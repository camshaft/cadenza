//! The running reducers — reducer tasks over a swappable runtime (`design/cadenza-platform.md` §3/§9).
//!
//! A reducer runs concurrently with the others and has a **mailbox**: events are delivered into it, and the
//! reducer folds them one at a time through its matching entry point. A reducer blocking on IO parks only
//! itself; the rest keep running.
//!
//! [`Runtime`] is the one trait the platform runs on. It is deliberately the *whole* lifecycle in one place,
//! because spawning a reducer, delivering to its mailbox, and removing it when it closes or crashes are
//! inseparable — a runtime that delivers to a reducer is the same thing that monitors it and reclaims it. An
//! in-memory runtime keeps reducers as async tasks with channel mailboxes and reclaims a task when its loop
//! ends; a durable-actor runtime keeps them as transactionally-stored actors, delivers by appending to the
//! actor's key, and reclaims via its supervisor — both implement the same trait, so the platform above does
//! not change.
//!
//! Two in-memory runtimes ship, one per executor: [`TokioRuntime`] (production) and [`BachRuntime`] (the
//! deterministic simulator, in tests). They are small enough to implement the trait directly rather than
//! share machinery. [`Reducers`] is the platform front over whichever runtime is installed: it derives a
//! reducer's id from its genesis, loads the program inside the reducer's own execution, and routes.

use crate::{
    ContractId, Deliver, Delivered, EventRegistry, Hash, Outcome, ProgramHash, ProgramStore,
    Reducer, ReducerId, Request, deliver_contract,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A future that loads and instantiates a reducer — run inside the reducer's own execution by a [`Runtime`],
/// so a slow program load never blocks the spawner or any peer. `None` if the program cannot be instantiated.
pub type Load = Pin<Box<dyn Future<Output = Option<Box<dyn Reducer>>> + Send>>;

/// How reducers are run, delivered to, and reclaimed — the swappable runtime the platform sits on. One trait
/// for the whole lifecycle: spawning a reducer, delivering events to its mailbox (the privileged routing act,
/// §4), and removing it when it closes or crashes are the same runtime's job and cannot be pulled apart.
pub trait Runtime: Send + Sync {
    /// Start running the reducer produced by `load` under `id`: give it a mailbox so [`deliver`] reaches it,
    /// drive its fold loop (delivering the `deliver`s it emits to the named peers), and reclaim it when its
    /// loop ends — a [`Break`](Outcome::Break), a closed mailbox, or a crash — so a dead reducer leaves no
    /// entry behind. Returns at once; the load and the loop run in the reducer's own execution.
    ///
    /// [`deliver`]: Runtime::deliver
    fn spawn(&self, id: ReducerId, load: Load);

    /// Deliver an event into a reducer's mailbox. `true` if the reducer is running and accepted it; `false`
    /// if no reducer is running under `target`.
    fn deliver(&self, target: ReducerId, event: Delivered) -> bool;

    /// Whether a reducer is currently running under `id`.
    fn contains(&self, id: ReducerId) -> bool;

    /// The number of running reducers.
    fn len(&self) -> usize;

    /// Whether no reducer is running.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The running reducers, over an installed [`Runtime`] and a shared [`ProgramStore`]. Derives a reducer's id
/// from its genesis, spawns reducers, and delivers events to them by id; the [`EventRegistry`] names which
/// program a contract's event reducer is spawned from.
pub struct Reducers {
    runtime: Arc<dyn Runtime>,
    programs: Arc<dyn ProgramStore>,
    events: EventRegistry,
}

impl Reducers {
    /// A registry over the given runtime, event registry, and shared program store.
    #[must_use]
    pub fn new(
        runtime: Arc<dyn Runtime>,
        events: EventRegistry,
        programs: Arc<dyn ProgramStore>,
    ) -> Self {
        Self {
            runtime,
            programs,
            events,
        }
    }

    /// The program a contract's event reducer is spawned from — the routing lookup on an emitted effect.
    #[must_use]
    pub fn route(&self, contract: ContractId) -> ProgramHash {
        self.events.resolve(contract)
    }

    /// Whether a reducer is currently running under `id`.
    #[must_use]
    pub fn contains(&self, id: ReducerId) -> bool {
        self.runtime.contains(id)
    }

    /// The number of running reducers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.runtime.len()
    }

    /// Whether no reducer is running.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.runtime.is_empty()
    }

    /// Spawn a reducer from `program`: derive its id from its genesis (the program plus a spawn `nonce` — the
    /// platform generates the id, a reducer never names its own) and start it on the runtime. Returns the id
    /// at once; the program is loaded inside the reducer's own execution, so the load blocks no one.
    pub fn spawn(&self, program: ProgramHash, nonce: &[u8]) -> ReducerId {
        let id = reducer_id(program, nonce);
        let programs = Arc::clone(&self.programs);
        let load: Load = Box::pin(async move { programs.spawn(program).await });
        self.runtime.spawn(id, load);
        id
    }

    /// Spawn the event reducer that governs `contract` (§4): route the contract to its program and spawn it
    /// with `nonce`. It persists like any reducer, shepherding the effect across the request/response cycle.
    pub fn spawn_event_reducer(&self, contract: ContractId, nonce: &[u8]) -> ReducerId {
        self.spawn(self.route(contract), nonce)
    }

    /// Deliver an event into a reducer's mailbox — the privileged routing act (§4). `false` if no reducer is
    /// running under `target`.
    pub fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
        self.runtime.deliver(target, event)
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

/// Fold one delivered event through a reducer's matching entry point — the executor-agnostic step both
/// in-memory runtimes drive.
async fn fold(reducer: &mut Box<dyn Reducer>, event: Delivered) -> (Vec<Request>, Outcome) {
    match event {
        Delivered::Message(message) => reducer.on_message(message).await,
        Delivered::Response(response) => reducer.on_response(response).await,
        Delivered::Notification(notification) => reducer.on_notification(notification).await,
    }
}

/// Removes a reducer's mailbox from its runtime's map when dropped — so a reducer whose loop ends *any* way,
/// including an unwinding crash, leaves no entry behind. Generic over the sender type so both in-memory
/// runtimes reuse it.
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

// ── the two in-memory runtimes, one per executor ─────────────────────────────────────────────────────────

/// The production in-memory runtime: each reducer an async task on tokio, with a channel mailbox.
pub struct TokioRuntime {
    reducers: Arc<Mutex<HashMap<ReducerId, tokio::sync::mpsc::UnboundedSender<Delivered>>>>,
}

impl TokioRuntime {
    /// A runtime with no running reducers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reducers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for TokioRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime for TokioRuntime {
    fn spawn(&self, id: ReducerId, load: Load) {
        let (sender, mut inbox) = tokio::sync::mpsc::unbounded_channel();
        self.reducers
            .lock()
            .expect("reducers lock")
            .insert(id, sender);
        let reducers = Arc::clone(&self.reducers);
        tokio::spawn(async move {
            // Deregister on any exit (loop end, break, or a crash unwinding through here).
            let _guard = DeregisterOnDrop {
                reducers: Arc::clone(&reducers),
                id,
            };
            let Some(mut reducer) = load.await else {
                return;
            };
            while let Some(event) = inbox.recv().await {
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
                            let _ = peer.send(deliver.event);
                        }
                    }
                }
                if let Outcome::Break { .. } = outcome {
                    break;
                }
            }
        });
    }

    fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
        let sender = self
            .reducers
            .lock()
            .expect("reducers lock")
            .get(&target)
            .cloned();
        match sender {
            Some(sender) => sender.send(event).is_ok(),
            None => false,
        }
    }

    fn contains(&self, id: ReducerId) -> bool {
        self.reducers
            .lock()
            .expect("reducers lock")
            .contains_key(&id)
    }

    fn len(&self) -> usize {
        self.reducers.lock().expect("reducers lock").len()
    }
}

/// The test in-memory runtime: each reducer an async task on the bach simulator, with a channel mailbox.
/// Gated with the test-support code, since bach only drives the reducers under test.
#[cfg(any(test, feature = "testing"))]
pub struct BachRuntime {
    reducers: Arc<Mutex<HashMap<ReducerId, bach::sync::mpsc::UnboundedSender<Delivered>>>>,
}

#[cfg(any(test, feature = "testing"))]
impl BachRuntime {
    /// A runtime with no running reducers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reducers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[cfg(any(test, feature = "testing"))]
impl Default for BachRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "testing"))]
impl Runtime for BachRuntime {
    fn spawn(&self, id: ReducerId, load: Load) {
        let (sender, mut inbox) = bach::sync::mpsc::unbounded_channel();
        self.reducers
            .lock()
            .expect("reducers lock")
            .insert(id, sender);
        let reducers = Arc::clone(&self.reducers);
        bach::task::spawn(async move {
            let _guard = DeregisterOnDrop {
                reducers: Arc::clone(&reducers),
                id,
            };
            let Some(mut reducer) = load.await else {
                return;
            };
            while let Some(event) = inbox.recv().await {
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
                            let _ = peer.send(deliver.event);
                        }
                    }
                }
                if let Outcome::Break { .. } = outcome {
                    break;
                }
            }
        });
    }

    fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
        let sender = self
            .reducers
            .lock()
            .expect("reducers lock")
            .get(&target)
            .cloned();
        match sender {
            Some(sender) => sender.send(event).is_ok(),
            None => false,
        }
    }

    fn contains(&self, id: ReducerId) -> bool {
        self.reducers
            .lock()
            .expect("reducers lock")
            .contains_key(&id)
    }

    fn len(&self) -> usize {
        self.reducers.lock().expect("reducers lock").len()
    }
}

#[cfg(test)]
mod tests {
    use super::{BachRuntime, Reducers, TokioRuntime, reducer_id};
    use crate::{
        Bytes, ContractId, Deliver, Delivered, EventRegistry, Hash, HostId, Message, Notification,
        Origin, Outcome, ProgramHash, Reducer, ReducerId, Request, Response,
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

                let reducers = Reducers::new(
                    Arc::new(BachRuntime::new()),
                    EventRegistry::new(prog(b"default")),
                    Arc::new(store),
                );
                let sink = reducers.spawn(prog(b"sink"), b"1");
                assert_eq!(sink, sink_id);
                let router = reducers.spawn(prog(b"router"), b"1");

                // Deliver a message to the router; it signals, then delivers to the sink, which signals.
                assert!(reducers.deliver(router, a_message(cid(b"http.get"))));
                assert_eq!(saw_rx.recv().await, Some(router));
                assert_eq!(saw_rx.recv().await, Some(sink));
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn delivering_to_an_unregistered_reducer_is_false() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let store = crate::testing::program::Store::new();
                let reducers = Reducers::new(
                    Arc::new(BachRuntime::new()),
                    EventRegistry::new(prog(b"default")),
                    Arc::new(store),
                );
                assert!(!reducers.deliver(reducer_id(prog(b"ghost"), b"1"), a_message(cid(b"c"))));
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
                let reducers = Reducers::new(
                    Arc::new(BachRuntime::new()),
                    EventRegistry::new(prog(b"default")),
                    Arc::new(store),
                );
                let id = reducers.spawn(prog(b"absent"), b"1");
                // The mailbox is registered synchronously, but the task's load fails and deregisters it.
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                assert!(!reducers.contains(id));
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }

    /// The production runtime drives the same behavior under tokio — spawn, deliver, and the fold loop.
    #[tokio::test]
    async fn the_tokio_runtime_runs_a_reducer_and_reclaims_it_on_break() {
        // A reducer that breaks on its first message (terminates).
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
        let reducers = Reducers::new(
            Arc::new(TokioRuntime::new()),
            EventRegistry::new(prog(b"default")),
            Arc::new(store),
        );
        let id = reducers.spawn(prog(b"term"), b"1");
        // Deliver a message; the reducer breaks, and its task reclaims the mailbox.
        assert!(reducers.deliver(id, a_message(cid(b"c"))));
        // Let the task run and reclaim; poll until it deregisters.
        for _ in 0..100 {
            if !reducers.contains(id) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!reducers.contains(id));
    }
}
