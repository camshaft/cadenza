//! The reducer system — running reducers, delivering to them, reclaiming them (`design/cadenza-platform.md`
//! §3/§9).
//!
//! A reducer runs concurrently with the others and has a **mailbox**: events are delivered into it, and it
//! folds them one at a time through its matching entry point. A reducer blocking on IO parks only itself.
//!
//! [`System`] is the one cohesive interface the platform runs reducers through — spawn a reducer under an id
//! on a program, deliver an event to a reducer, ask whether one is running. Its operations are async and
//! fallible so a durable-actor backend (awaiting a replicated store, and able to fail) fits the same trait as
//! an in-memory one; it is used behind `dyn`, so the platform holds an `Arc<dyn System>` and is not generic
//! over the backend. Deriving a reducer's id from its genesis, recording the spawn tree, and routing a
//! contract to its program are the layer *above* this — they compose a `System` with the other substrates,
//! rather than living inside it.
//!
//! Two things are submodules here:
//!  - [`runtime`] — the [`Runtime`] trait, the async runtime a task-based system runs on (spawn a task, make
//!    a mailbox channel). [`TokioRuntime`] in production, [`BachRuntime`] (the simulator) in tests. Static (a
//!    generic bound), so the system composes over it with no dynamic dispatch and no duplicated logic.
//!  - [`task`] — [`TaskSystem`], the in-memory `System`, generic over its `Runtime`: each reducer a task
//!    draining a channel mailbox, loading its program inside its own task so a slow load blocks no one.

#[cfg(any(test, feature = "testing"))]
pub use runtime::BachRuntime;
pub use runtime::{Runtime, TokioRuntime};
pub use task::TaskSystem;

use crate::{Delivered, ProgramHash, ReducerId};
use async_trait::async_trait;

/// A failure carrying out a system operation — a backend's store or transport error. The in-memory system
/// never returns one; a durable-actor backend surfaces its failures here.
#[non_exhaustive]
#[derive(Debug)]
pub enum SystemError {
    /// The system backend failed (e.g. a durable-store or transport error).
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for SystemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemError::Backend(e) => write!(f, "system backend error: {e}"),
        }
    }
}

impl std::error::Error for SystemError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SystemError::Backend(e) => Some(&**e),
        }
    }
}

/// How reducers are run, delivered to, and reclaimed — the whole lifecycle in one interface. Spawning a
/// reducer, delivering to its mailbox, and reclaiming it on close or crash are the same system's job. Async
/// and fallible so a durable-actor backend fits alongside an in-memory one; used behind `dyn`.
#[async_trait]
pub trait System: Send + Sync {
    /// Start running a reducer under `id` on `program`: give it a mailbox so [`deliver`] reaches it, load and
    /// drive it, and reclaim it when it closes or crashes. The system chooses how and where to load `program`
    /// — in the reducer's own local task, or by handing the program to a remote host that instantiates it —
    /// so it takes the program's id, not an opaque loader. The `id` is derived by the layer above (from the
    /// reducer's genesis); the system just runs it.
    ///
    /// [`deliver`]: System::deliver
    async fn spawn(&self, id: ReducerId, program: ProgramHash) -> Result<(), SystemError>;

    /// Deliver an event into a reducer's mailbox. `Ok(true)` if the reducer is running and accepted it,
    /// `Ok(false)` if no reducer is running under `target`, `Err` on a backend failure.
    async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, SystemError>;

    /// Whether a reducer is currently running under `id`.
    async fn contains(&self, id: ReducerId) -> Result<bool, SystemError>;
}

/// The [`Runtime`] trait — the async runtime a task-based [`System`] runs on — and its two implementations,
/// [`TokioRuntime`] and [`BachRuntime`].
mod runtime {
    use crate::Delivered;
    use std::future::Future;

    /// The async runtime a [`TaskSystem`](super::TaskSystem) runs on: spawning a task and a mailbox channel.
    /// A static trait (a generic bound, never `dyn`), so the system composes over it with no dynamic dispatch
    /// and no duplicated runtime logic. tokio and bach are runtimes; a durable backend is a whole
    /// [`System`](super::System), not a `Runtime`.
    pub trait Runtime: Send + Sync + 'static {
        /// A mailbox sender — cloneable so many peers can hold a handle to one reducer's mailbox.
        type Sender: Clone + Send + Sync + 'static;
        /// A mailbox receiver — the reducer's own end, drained by its loop.
        type Receiver: Send + 'static;

        /// Create a reducer's mailbox: an unbounded channel of delivered events.
        fn channel() -> (Self::Sender, Self::Receiver);
        /// Deliver an event into a mailbox. `false` if the receiving end is gone.
        fn send(sender: &Self::Sender, event: Delivered) -> bool;
        /// Receive the next delivered event, or `None` when the mailbox closes.
        fn recv(
            receiver: &mut Self::Receiver,
        ) -> impl Future<Output = Option<Delivered>> + Send + '_;
        /// Spawn `future` as a task on the runtime.
        fn spawn<F: Future<Output = ()> + Send + 'static>(future: F);
    }

    /// The production runtime: tokio tasks and channels.
    pub struct TokioRuntime;

    impl Runtime for TokioRuntime {
        type Sender = tokio::sync::mpsc::UnboundedSender<Delivered>;
        type Receiver = tokio::sync::mpsc::UnboundedReceiver<Delivered>;

        fn channel() -> (Self::Sender, Self::Receiver) {
            tokio::sync::mpsc::unbounded_channel()
        }
        fn send(sender: &Self::Sender, event: Delivered) -> bool {
            sender.send(event).is_ok()
        }
        fn recv(
            receiver: &mut Self::Receiver,
        ) -> impl Future<Output = Option<Delivered>> + Send + '_ {
            receiver.recv()
        }
        fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
            tokio::spawn(future);
        }
    }

    /// The test runtime: bach-simulator tasks and channels (deterministic). Gated with the test-support code,
    /// since bach only drives the reducers under test.
    #[cfg(any(test, feature = "testing"))]
    pub struct BachRuntime;

    #[cfg(any(test, feature = "testing"))]
    impl Runtime for BachRuntime {
        type Sender = bach::sync::mpsc::UnboundedSender<Delivered>;
        type Receiver = bach::sync::mpsc::UnboundedReceiver<Delivered>;

        fn channel() -> (Self::Sender, Self::Receiver) {
            bach::sync::mpsc::unbounded_channel()
        }
        fn send(sender: &Self::Sender, event: Delivered) -> bool {
            sender.send(event).is_ok()
        }
        fn recv(
            receiver: &mut Self::Receiver,
        ) -> impl Future<Output = Option<Delivered>> + Send + '_ {
            receiver.recv()
        }
        fn spawn<F: Future<Output = ()> + Send + 'static>(future: F) {
            bach::task::spawn(future);
        }
    }
}

/// [`TaskSystem`] — the in-memory [`System`](super::System), generic over its [`Runtime`](super::Runtime).
mod task {
    use super::runtime::Runtime;
    use super::{System, SystemError};
    use crate::{
        Deliver, Delivered, Outcome, ProgramHash, ProgramStore, Reducer, ReducerId, Request,
        deliver_contract,
    };
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// The in-memory system: each reducer an async task on the runtime `R`, draining a channel mailbox. It
    /// holds the program store and loads a spawned reducer's program inside that reducer's own task.
    pub struct TaskSystem<R: Runtime> {
        reducers: Arc<Mutex<HashMap<ReducerId, R::Sender>>>,
        programs: Arc<dyn ProgramStore>,
    }

    impl<R: Runtime> TaskSystem<R> {
        /// A system with no running reducers, loading programs from `programs`.
        #[must_use]
        pub fn new(programs: Arc<dyn ProgramStore>) -> Self {
            Self {
                reducers: Arc::new(Mutex::new(HashMap::new())),
                programs,
            }
        }
    }

    #[async_trait]
    impl<R: Runtime> System for TaskSystem<R> {
        async fn spawn(&self, id: ReducerId, program: ProgramHash) -> Result<(), SystemError> {
            let (sender, mut inbox) = R::channel();
            self.reducers
                .lock()
                .expect("reducers lock")
                .insert(id, sender);
            let reducers = Arc::clone(&self.reducers);
            let programs = Arc::clone(&self.programs);
            R::spawn(async move {
                // Reclaim the mailbox on any exit — loop end, break, a failed load, or an unwinding crash.
                let _guard = DeregisterOnDrop {
                    reducers: Arc::clone(&reducers),
                    id,
                };
                // Load the program inside the reducer's own task, so a slow load blocks no one.
                let Some(mut reducer) = programs.spawn(program).await else {
                    return;
                };
                while let Some(event) = R::recv(&mut inbox).await {
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
                                R::send(&peer, deliver.event);
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

        async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, SystemError> {
            let sender = self
                .reducers
                .lock()
                .expect("reducers lock")
                .get(&target)
                .cloned();
            Ok(match sender {
                Some(sender) => R::send(&sender, event),
                None => false,
            })
        }

        async fn contains(&self, id: ReducerId) -> Result<bool, SystemError> {
            Ok(self
                .reducers
                .lock()
                .expect("reducers lock")
                .contains_key(&id))
        }
    }

    /// Fold one delivered event through a reducer's matching entry point.
    async fn fold(reducer: &mut Box<dyn Reducer>, event: Delivered) -> (Vec<Request>, Outcome) {
        match event {
            Delivered::Message(message) => reducer.on_message(message).await,
            Delivered::Response(response) => reducer.on_response(response).await,
            Delivered::Notification(notification) => reducer.on_notification(notification).await,
        }
    }

    /// Removes a reducer's mailbox from the system's map when dropped — so a reducer whose task ends any way,
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
}

#[cfg(test)]
mod tests {
    use super::{BachRuntime, System, TaskSystem, TokioRuntime};
    use crate::{
        Bytes, ContractId, Deliver, Delivered, Hash, HostId, Message, Notification, Origin,
        Outcome, ProgramHash, Reducer, ReducerId, Request, Response,
    };
    use std::sync::Arc;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::from_hash(Hash::of(tag))
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::from_hash(Hash::of(tag))
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::from_hash(Hash::of(tag))
    }
    fn origin() -> Origin {
        Origin {
            reducer: rid(b"peer"),
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
                let sink = rid(b"sink");
                let router = rid(b"router");
                let (saw, mut saw_rx) = bach::sync::mpsc::unbounded_channel();

                let saw_sink = saw.clone();
                let saw_router = saw.clone();
                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"sink"), move || {
                    Box::new(Probe {
                        saw: saw_sink.clone(),
                        me: sink,
                        deliver_to: None,
                    })
                });
                store.register(prog(b"router"), move || {
                    Box::new(Probe {
                        saw: saw_router.clone(),
                        me: router,
                        deliver_to: Some(sink),
                    })
                });

                let system = TaskSystem::<BachRuntime>::new(Arc::new(store));
                system.spawn(sink, prog(b"sink")).await.unwrap();
                system.spawn(router, prog(b"router")).await.unwrap();

                // Deliver a message to the router; it signals, then delivers to the sink, which signals.
                assert!(
                    system
                        .deliver(router, a_message(cid(b"http.get")))
                        .await
                        .unwrap()
                );
                assert_eq!(saw_rx.recv().await, Some(router));
                assert_eq!(saw_rx.recv().await, Some(sink));
            }
            .group("system")
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
                let system = TaskSystem::<BachRuntime>::new(Arc::new(store));
                assert!(
                    !system
                        .deliver(rid(b"ghost"), a_message(cid(b"c")))
                        .await
                        .unwrap()
                );
            }
            .group("system")
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
                let system = TaskSystem::<BachRuntime>::new(Arc::new(store));
                system.spawn(rid(b"x"), prog(b"absent")).await.unwrap();
                // The mailbox is registered synchronously, but the task's load fails and deregisters it.
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                assert!(!system.contains(rid(b"x")).await.unwrap());
            }
            .group("system")
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
        let system = TaskSystem::<TokioRuntime>::new(Arc::new(store));
        let id = rid(b"term-1");
        system.spawn(id, prog(b"term")).await.unwrap();
        assert!(system.deliver(id, a_message(cid(b"c"))).await.unwrap());
        // Let the task run and reclaim; poll until it deregisters.
        for _ in 0..100 {
            if !system.contains(id).await.unwrap() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(!system.contains(id).await.unwrap());
    }
}
