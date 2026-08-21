//! The running reducers — reducer tasks over a swappable runtime (`design/cadenza-platform.md` §3/§9).
//!
//! A reducer runs concurrently with the others and has a **mailbox**: events are delivered into it, and the
//! reducer folds them one at a time through its matching entry point. A reducer blocking on IO parks only
//! itself; the rest keep running.
//!
//! [`Runtime`] is the one high-level trait the platform runs on — spawning a reducer, delivering to its
//! mailbox, and reclaiming it when it closes or crashes are the same runtime's job and cannot be pulled
//! apart. Its operations are **async and fallible**: an in-memory runtime answers them at once and never
//! fails, but a durable-actor runtime awaits a replicated store and can fail, so the trait admits both. It is
//! used behind `dyn` so the platform is not generic over the backend.
//!
//! Underneath, the two in-memory runtimes differ only in their executor — tokio in production, the bach
//! simulator in tests — so that difference is a small **static** [`Executor`] trait (spawn + channel), and a
//! single generic [`TaskRuntime<E>`] implements the whole [`Runtime`] over it. [`TokioRuntime`] and
//! [`BachRuntime`] are just `TaskRuntime` over the two executors, with no duplicated logic. A different
//! backend (durable, transactionally-stored actors, delivering by appending to an actor's key) implements
//! [`Runtime`] directly instead.
//!
//! [`Reducers`] is the platform front over whichever runtime is installed: it derives a reducer's id from its
//! genesis, loads the program inside the reducer's own execution, and routes via the [`EventRegistry`].

use crate::{
    ContractId, Deliver, Delivered, EventRegistry, Hash, Outcome, ProgramHash, ProgramStore,
    Reducer, ReducerId, Request, deliver_contract,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A future that loads and instantiates a reducer — run inside the reducer's own execution by a [`Runtime`],
/// so a slow program load never blocks the spawner or any peer. `None` if the program cannot be instantiated.
pub type Load = Pin<Box<dyn Future<Output = Option<Box<dyn Reducer>>> + Send>>;

/// A failure carrying out a runtime operation — a backend's store or transport error. The in-memory runtimes
/// never return one (their operations are infallible); a durable-actor backend surfaces its failures here.
#[non_exhaustive]
#[derive(Debug)]
pub enum RuntimeError {
    /// The runtime backend failed (e.g. a durable-store or transport error).
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::Backend(e) => write!(f, "runtime backend error: {e}"),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RuntimeError::Backend(e) => Some(&**e),
        }
    }
}

/// How reducers are run, delivered to, and reclaimed — the swappable runtime the platform sits on. One trait
/// for the whole lifecycle. Async and fallible so a durable-actor backend (awaiting a replicated store) fits
/// alongside an in-memory one.
#[async_trait]
pub trait Runtime: Send + Sync {
    /// Start running the reducer produced by `load` under `id`: give it a mailbox so [`deliver`] reaches it,
    /// drive its fold loop (delivering the `deliver`s it emits to the named peers), and reclaim it when its
    /// loop ends — a [`Break`](Outcome::Break), a closed mailbox, or a crash — so a dead reducer leaves no
    /// entry behind. The load and the loop run in the reducer's own execution, so this does not await them.
    ///
    /// [`deliver`]: Runtime::deliver
    async fn spawn(&self, id: ReducerId, load: Load) -> Result<(), RuntimeError>;

    /// Deliver an event into a reducer's mailbox. `Ok(true)` if the reducer is running and accepted it,
    /// `Ok(false)` if no reducer is running under `target`, `Err` on a backend failure.
    async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, RuntimeError>;

    /// Whether a reducer is currently running under `id`.
    async fn contains(&self, id: ReducerId) -> Result<bool, RuntimeError>;
}

/// The executor a [`TaskRuntime`] runs on — the small, static difference between the in-memory runtimes:
/// spawning a task and a mailbox channel. Static (a generic bound, never `dyn`), so `TaskRuntime` composes
/// over it with no dynamic dispatch and no duplicated runtime logic.
pub trait Executor: Send + Sync + 'static {
    /// A mailbox sender — cloneable so many peers can hold a handle to one reducer's mailbox.
    type Sender: Clone + Send + Sync + 'static;
    /// A mailbox receiver — the reducer's own end, drained by its loop.
    type Receiver: Send + 'static;

    /// Create a reducer's mailbox: an unbounded channel of delivered events.
    fn channel() -> (Self::Sender, Self::Receiver);
    /// Deliver an event into a mailbox. `false` if the receiving end is gone.
    fn send(sender: &Self::Sender, event: Delivered) -> bool;
    /// Receive the next delivered event, or `None` when the mailbox closes.
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_;
    /// Spawn `future` as a task on the executor.
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F);
}

/// Removes a reducer's mailbox from the runtime's map when dropped — so a reducer whose loop ends *any* way,
/// including an unwinding crash, leaves no entry behind.
struct DeregisterOnDrop<S> {
    reducers: Arc<Mutex<HashMap<ReducerId, S>>>,
    id: ReducerId,
}

impl<S> Drop for DeregisterOnDrop<S> {
    fn drop(&mut self) {
        self.reducers
            .lock()
            .expect("reducers lock")
            .remove(&self.id);
    }
}

/// The in-memory [`Runtime`], generic over its [`Executor`]: each reducer an async task draining a channel
/// mailbox. [`TokioRuntime`] and [`BachRuntime`] are this over the two executors.
pub struct TaskRuntime<E: Executor> {
    reducers: Arc<Mutex<HashMap<ReducerId, E::Sender>>>,
}

impl<E: Executor> TaskRuntime<E> {
    /// A runtime with no running reducers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reducers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<E: Executor> Default for TaskRuntime<E> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<E: Executor> Runtime for TaskRuntime<E> {
    async fn spawn(&self, id: ReducerId, load: Load) -> Result<(), RuntimeError> {
        let (sender, mut inbox) = E::channel();
        self.reducers
            .lock()
            .expect("reducers lock")
            .insert(id, sender);
        let reducers = Arc::clone(&self.reducers);
        E::spawn(async move {
            // Reclaim the mailbox on any exit — loop end, break, a failed load, or an unwinding crash.
            let _guard = DeregisterOnDrop {
                reducers: Arc::clone(&reducers),
                id,
            };
            let Some(mut reducer) = load.await else {
                return;
            };
            while let Some(event) = E::recv(&mut inbox).await {
                let (requests, outcome) = fold(&mut reducer, event).await;
                for request in requests {
                    if request.id == deliver_contract()
                        && let Some(deliver) = Deliver::decode(&request.payload)
                    {
                        let peer = reducers
                            .lock()
                            .expect("reducers lock")
                            .get(&deliver.target)
                            .cloned();
                        if let Some(peer) = peer {
                            E::send(&peer, deliver.event);
                        }
                    }
                }
                if let Outcome::Break { .. } = outcome {
                    break;
                }
            }
        });
        Ok(())
    }

    async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, RuntimeError> {
        let sender = self
            .reducers
            .lock()
            .expect("reducers lock")
            .get(&target)
            .cloned();
        Ok(match sender {
            Some(sender) => E::send(&sender, event),
            None => false,
        })
    }

    async fn contains(&self, id: ReducerId) -> Result<bool, RuntimeError> {
        Ok(self
            .reducers
            .lock()
            .expect("reducers lock")
            .contains_key(&id))
    }
}

/// Fold one delivered event through a reducer's matching entry point — the executor-agnostic step the loop
/// drives.
async fn fold(reducer: &mut Box<dyn Reducer>, event: Delivered) -> (Vec<Request>, Outcome) {
    match event {
        Delivered::Message(message) => reducer.on_message(message).await,
        Delivered::Response(response) => reducer.on_response(response).await,
        Delivered::Notification(notification) => reducer.on_notification(notification).await,
    }
}

/// The production executor: tokio tasks and channels.
pub struct TokioExecutor;

impl Executor for TokioExecutor {
    type Sender = tokio::sync::mpsc::UnboundedSender<Delivered>;
    type Receiver = tokio::sync::mpsc::UnboundedReceiver<Delivered>;

    fn channel() -> (Self::Sender, Self::Receiver) {
        tokio::sync::mpsc::unbounded_channel()
    }
    fn send(sender: &Self::Sender, event: Delivered) -> bool {
        sender.send(event).is_ok()
    }
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_ {
        receiver.recv()
    }
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
        tokio::spawn(future);
    }
}

/// The production in-memory runtime: reducers as tokio tasks.
pub type TokioRuntime = TaskRuntime<TokioExecutor>;

/// The test executor: bach-simulator tasks and channels (deterministic). Gated with the test-support code,
/// since bach only drives the reducers under test.
#[cfg(any(test, feature = "testing"))]
pub struct BachExecutor;

#[cfg(any(test, feature = "testing"))]
impl Executor for BachExecutor {
    type Sender = bach::sync::mpsc::UnboundedSender<Delivered>;
    type Receiver = bach::sync::mpsc::UnboundedReceiver<Delivered>;

    fn channel() -> (Self::Sender, Self::Receiver) {
        bach::sync::mpsc::unbounded_channel()
    }
    fn send(sender: &Self::Sender, event: Delivered) -> bool {
        sender.send(event).is_ok()
    }
    fn recv(receiver: &mut Self::Receiver) -> impl Future<Output = Option<Delivered>> + Send + '_ {
        receiver.recv()
    }
    fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
        bach::task::spawn(future);
    }
}

/// The test in-memory runtime: reducers as bach-simulator tasks.
#[cfg(any(test, feature = "testing"))]
pub type BachRuntime = TaskRuntime<BachExecutor>;

/// The running reducers, over an installed [`Runtime`], a shared [`EventRegistry`], and a shared
/// [`ProgramStore`]. Derives a reducer's id from its genesis, spawns reducers, and delivers events by id.
pub struct Reducers {
    runtime: Arc<dyn Runtime>,
    events: Arc<dyn EventRegistry>,
    programs: Arc<dyn ProgramStore>,
}

impl Reducers {
    /// A registry over the given runtime, event registry, and shared program store.
    #[must_use]
    pub fn new(
        runtime: Arc<dyn Runtime>,
        events: Arc<dyn EventRegistry>,
        programs: Arc<dyn ProgramStore>,
    ) -> Self {
        Self {
            runtime,
            events,
            programs,
        }
    }

    /// The program a contract's event reducer is spawned from — the routing lookup on an emitted effect.
    pub async fn route(&self, contract: ContractId) -> ProgramHash {
        self.events.resolve(contract).await
    }

    /// Whether a reducer is currently running under `id`.
    pub async fn contains(&self, id: ReducerId) -> Result<bool, RuntimeError> {
        self.runtime.contains(id).await
    }

    /// Spawn a reducer from `program`: derive its id from its genesis (the program plus a spawn `nonce` — the
    /// platform generates the id, a reducer never names its own) and start it on the runtime. Returns the id;
    /// the program is loaded inside the reducer's own execution, so the load blocks no one.
    pub async fn spawn(
        &self,
        program: ProgramHash,
        nonce: &[u8],
    ) -> Result<ReducerId, RuntimeError> {
        let id = reducer_id(program, nonce);
        let programs = Arc::clone(&self.programs);
        let load: Load = Box::pin(async move { programs.spawn(program).await });
        self.runtime.spawn(id, load).await?;
        Ok(id)
    }

    /// Spawn the event reducer that governs `contract` (§4): route the contract to its program and spawn it
    /// with `nonce`. It persists like any reducer, shepherding the effect across the request/response cycle.
    pub async fn spawn_event_reducer(
        &self,
        contract: ContractId,
        nonce: &[u8],
    ) -> Result<ReducerId, RuntimeError> {
        let program = self.route(contract).await;
        self.spawn(program, nonce).await
    }

    /// Deliver an event into a reducer's mailbox — the privileged routing act (§4). `Ok(false)` if no reducer
    /// is running under `target`.
    pub async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, RuntimeError> {
        self.runtime.deliver(target, event).await
    }
}

/// Derive a reducer's id from its genesis — the program it runs plus a spawn nonce — by content hash, so an
/// id is reproducible from its genesis and a reducer never names its own. (A richer genesis, e.g. a parent
/// link for the spawn tree of §7, folds in here later without changing the shape.)
#[must_use]
pub fn reducer_id(program: ProgramHash, nonce: &[u8]) -> ReducerId {
    let mut genesis = program.hash().as_bytes().to_vec();
    genesis.extend_from_slice(nonce);
    ReducerId::from_hash(Hash::of(&genesis))
}

#[cfg(test)]
mod tests {
    use super::{BachRuntime, Reducers, TokioRuntime, reducer_id};
    use crate::{
        Bytes, ContractId, Deliver, Delivered, Hash, HostId, InMemoryEventRegistry, Message,
        Notification, Origin, Outcome, ProgramHash, Reducer, ReducerId, Request, Response,
    };
    use std::sync::Arc;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::from_hash(Hash::of(tag))
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::from_hash(Hash::of(tag))
    }
    fn origin() -> Origin {
        Origin {
            reducer: ReducerId::from_hash(Hash::of(b"peer")),
            host: HostId::from_hash(Hash::of(b"host-a")),
        }
    }
    fn a_message(id: ContractId) -> Delivered {
        Delivered::Message(Message {
            id,
            payload: Bytes::from_static(b"e"),
            from: origin(),
            continuation_token: Bytes::from_static(b"t"),
        })
    }
    fn events() -> Arc<InMemoryEventRegistry> {
        Arc::new(InMemoryEventRegistry::new(prog(b"default")))
    }

    /// A reducer that signals receipt on a bach channel and optionally delivers a message to a fixed peer.
    struct Probe {
        saw: bach::sync::mpsc::UnboundedSender<ReducerId>,
        me: ReducerId,
        deliver_to: Option<ReducerId>,
    }

    #[async_trait::async_trait]
    impl Reducer for Probe {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            let _ = self.saw.send(self.me);
            let requests = match self.deliver_to {
                Some(target) => vec![
                    Deliver {
                        target,
                        event: a_message(cid(b"forwarded")),
                    }
                    .into_request(),
                ],
                None => Vec::new(),
            };
            (requests, Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[test]
    fn a_reducer_task_receives_delivered_messages_and_its_deliver_reaches_a_peer() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let sink_id = reducer_id(prog(b"sink"), b"1");
                let (saw, mut saw_rx) = bach::sync::mpsc::unbounded_channel();

                let saw_sink = saw.clone();
                let saw_router = saw.clone();
                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"sink"), move || {
                    Box::new(Probe {
                        saw: saw_sink.clone(),
                        me: sink_id,
                        deliver_to: None,
                    })
                });
                store.register(prog(b"router"), move || {
                    Box::new(Probe {
                        saw: saw_router.clone(),
                        me: reducer_id(prog(b"router"), b"1"),
                        deliver_to: Some(sink_id),
                    })
                });

                let reducers =
                    Reducers::new(Arc::new(BachRuntime::new()), events(), Arc::new(store));
                let sink = reducers.spawn(prog(b"sink"), b"1").await.unwrap();
                assert_eq!(sink, sink_id);
                let router = reducers.spawn(prog(b"router"), b"1").await.unwrap();

                // Deliver a message to the router; it signals, then delivers to the sink, which signals.
                assert!(
                    reducers
                        .deliver(router, a_message(cid(b"http.get")))
                        .await
                        .unwrap()
                );
                assert_eq!(saw_rx.recv().await, Some(router));
                assert_eq!(saw_rx.recv().await, Some(sink));
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn delivering_to_an_unregistered_reducer_is_ok_false() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let store = crate::testing::program::Store::new();
                let reducers =
                    Reducers::new(Arc::new(BachRuntime::new()), events(), Arc::new(store));
                assert!(
                    !reducers
                        .deliver(reducer_id(prog(b"ghost"), b"1"), a_message(cid(b"c")))
                        .await
                        .unwrap()
                );
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn spawning_an_unregistered_program_leaves_no_running_reducer() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let store = crate::testing::program::Store::new(); // no factories
                let reducers =
                    Reducers::new(Arc::new(BachRuntime::new()), events(), Arc::new(store));
                let id = reducers.spawn(prog(b"absent"), b"1").await.unwrap();
                // The mailbox is registered synchronously, but the task's load fails and deregisters it.
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                assert!(!reducers.contains(id).await.unwrap());
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }

    /// The production runtime drives the same behavior under tokio — spawn, deliver, and break-reclaim.
    #[tokio::test]
    async fn the_tokio_runtime_runs_a_reducer_and_reclaims_it_on_break() {
        struct Terminating;
        #[async_trait::async_trait]
        impl Reducer for Terminating {
            async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
                (
                    Vec::new(),
                    Outcome::Break {
                        schema: cid(b"done"),
                        reason: Bytes::from_static(b"bye"),
                    },
                )
            }
            async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
                (Vec::new(), Outcome::Continue)
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
                (Vec::new(), Outcome::Continue)
            }
        }

        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"term"), || Box::new(Terminating));
        let reducers = Reducers::new(Arc::new(TokioRuntime::new()), events(), Arc::new(store));
        let id = reducers.spawn(prog(b"term"), b"1").await.unwrap();
        assert!(reducers.deliver(id, a_message(cid(b"c"))).await.unwrap());
        // Let the task run and reclaim; poll until it deregisters.
        for _ in 0..100 {
            if !reducers.contains(id).await.unwrap() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!reducers.contains(id).await.unwrap());
    }
}
