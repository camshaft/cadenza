//! The reducer system — running reducers, delivering to them, reclaiming them (`design/cadenza-platform.md`
//! §3/§7/§9).
//!
//! A reducer runs concurrently with the others and has a **mailbox**: events are delivered into it, and it
//! folds them one at a time through its matching entry point. A reducer blocking on IO parks only itself.
//!
//! [`System`] is the one cohesive interface the platform runs reducers through — spawn a reducer under the
//! configuration a [`Spawn`] describes, deliver an event to a reducer, ask whether one is running. Its
//! operations are async and fallible so a durable-actor backend (awaiting a replicated store, and able to
//! fail) fits the same trait as an in-memory one; it is used behind `dyn`, so the platform holds an
//! `Arc<dyn System>` and is not generic over the backend. Tracking the spawn tree is the system's own job —
//! it records the spawn edge, the supervision links, and the reducer's [kind](ReducerKind) as it spawns,
//! through the [`ReducerGraph`](crate::ReducerGraph) it holds (a local map here, a replicated structure on a
//! durable backend). Deriving a reducer's id from its genesis and routing a contract to its program are the
//! layer *above* this — they compose a `System` with the other substrates, rather than living inside it.
//!
//! [`TaskSystem`] is the in-memory `System`, generic over its [`Runtime`](crate::Runtime) (tokio in
//! production, bach in tests): each reducer is a task draining a channel mailbox, loading its program inside
//! its own task so a slow load blocks no one.

use crate::{
    Bytes, ContractId, Deliver, Delivered, EdgeKind, Lifecycle, Outcome, ProgramHash, ProgramStore,
    Reducer, ReducerGraph, ReducerId, Request, Runtime, deliver_contract,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

/// What a reducer was spawned as — which fixes how the system tracks it and what it is allowed to do
/// (`design/cadenza-platform.md` §3/§4/§5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReducerKind {
    /// An ordinary reducer. Its effects are routed *for* it — it emits requests and the system reducer
    /// handling its events carries them out — so an ordinary reducer may not [`deliver`](System::deliver)
    /// itself; the system ignores a deliver it emits.
    Ordinary,
    /// A privileged event/system reducer: the one kind that may [`deliver`](System::deliver), since routing an
    /// event into another reducer's log is the one privileged act (§4). Its authority comes from the trust
    /// root, not its parent, so it does not inherit the parent's permission model — the system tracks it under
    /// the reducer that requested it, but as a privileged node.
    Event,
}

/// The supervision links a spawn establishes — one per direction, independently (`design/cadenza-platform.md`
/// §7). A link is a pure subscription: it asks the system to deliver the peer's [`Lifecycle`] event into a
/// reducer's mailbox when it exits, and nothing more. The system enacts no reaction of its own — each reducer
/// decides in its own fold what a peer's exit means for it, including returning its own `Break`, which then
/// reaches *that* reducer's watchers, so a cascade is the same mechanism rather than a separate system act.
/// The two directions are independent: a parent may watch its child without the child watching the parent,
/// either way, both, or neither. Each link becomes a [`watch_exit`](EdgeKind::watch_exit) edge in the graph,
/// so supervision is fixed atomically with the spawn and cannot be set by a later call that races the child's
/// first events or its exit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Links {
    /// Deliver the child's lifecycle event to its parent — a parent supervising the child it spawned.
    pub parent_watches_child: bool,
    /// Deliver the parent's lifecycle event to the child — a child that reacts to its parent ending.
    pub child_watches_parent: bool,
}

impl Links {
    /// No supervision link in either direction — neither reducer is told of the other's exit.
    pub const NONE: Links = Links {
        parent_watches_child: false,
        child_watches_parent: false,
    };
}

/// A spawn request — everything a reducer is configured with, together, so its lineage (`parent`), privilege
/// (`kind`), and supervision (`links`) are fixed atomically with it and cannot be set by later calls that
/// could race its first events or its exit. The `id` is derived above from the reducer's genesis; the system
/// just runs what this describes. Named fields, not positional arguments, so `id` and `parent` — both
/// [`ReducerId`] — cannot be transposed at a call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// The reducer's id, derived by the layer above from its genesis.
    pub id: ReducerId,
    /// The program it runs, by content hash.
    pub program: ProgramHash,
    /// The reducer that spawned it. A **root** created at genesis is its own parent (`parent == id`).
    pub parent: ReducerId,
    /// Its privilege: only an [`Event`](ReducerKind::Event) reducer's delivers are carried out.
    pub kind: ReducerKind,
    /// The supervision links between it and its parent, one per direction (ignored for a root, which has no
    /// parent to link with).
    pub links: Links,
}

/// How reducers are run, delivered to, and reclaimed — the whole lifecycle in one interface. Spawning a
/// reducer, delivering to its mailbox, and reclaiming it on close or crash are the same system's job. Async
/// and fallible so a durable-actor backend fits alongside an in-memory one; used behind `dyn`.
#[async_trait]
pub trait System: Send + Sync {
    /// Start running the reducer the [`Spawn`] describes: give it a mailbox so [`deliver`] reaches it, record
    /// its place in the spawn tree and its supervision links, load and drive it, and reclaim it when it
    /// closes or crashes. The system chooses how and where to load the program — in the reducer's own local
    /// task, or by handing the program to a remote host that instantiates it — so [`Spawn`] carries the
    /// program's id, not an opaque loader.
    ///
    /// [`deliver`]: System::deliver
    async fn spawn(&self, spawn: Spawn) -> Result<(), SystemError>;

    /// Deliver an event into a reducer's mailbox. `Ok(true)` if the reducer is running and accepted it,
    /// `Ok(false)` if no reducer is running under `target`, `Err` on a backend failure.
    async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, SystemError>;

    /// Whether a reducer is currently running under `id`.
    async fn contains(&self, id: ReducerId) -> Result<bool, SystemError>;
}

/// The in-memory [`System`], generic over its [`Runtime`](crate::Runtime): each reducer an async task on the
/// runtime, draining a channel mailbox. It holds the program store and loads a spawned reducer's program
/// inside that reducer's own task, so a slow load blocks no one.
pub struct TaskSystem<R: Runtime> {
    reducers: Arc<Mutex<HashMap<ReducerId, R::Sender>>>,
    programs: Arc<dyn ProgramStore>,
    graph: Arc<dyn ReducerGraph>,
}

impl<R: Runtime> TaskSystem<R> {
    /// A system with no running reducers, loading programs from `programs` and tracking the spawn tree and
    /// supervision links in `graph`. Whoever composes the system keeps a clone of `graph` to read the tree
    /// (e.g. the privileged event reducer walking ancestors); the system holds it to record spawns, resolve a
    /// terminating reducer's watchers, and reclaim its edges.
    #[must_use]
    pub fn new(programs: Arc<dyn ProgramStore>, graph: Arc<dyn ReducerGraph>) -> Self {
        Self {
            reducers: Arc::new(Mutex::new(HashMap::new())),
            programs,
            graph,
        }
    }
}

#[async_trait]
impl<R: Runtime> System for TaskSystem<R> {
    async fn spawn(&self, spawn: Spawn) -> Result<(), SystemError> {
        let Spawn {
            id,
            program,
            parent,
            kind,
            links,
        } = spawn;
        // Record the spawn tree and supervision edges before the task starts. A root has no parent edge; a
        // child links to its parent, plus a watch_exit edge per requested supervision direction (§7).
        self.graph.insert(id).await;
        if parent != id {
            self.graph.link(id, parent, EdgeKind::spawn()).await;
            if links.parent_watches_child {
                self.graph.link(parent, id, EdgeKind::watch_exit()).await;
            }
            if links.child_watches_parent {
                self.graph.link(id, parent, EdgeKind::watch_exit()).await;
            }
        }
        let (sender, mut inbox) = R::channel();
        self.reducers
            .lock()
            .expect("reducers lock")
            .insert(id, sender);
        let reducers = Arc::clone(&self.reducers);
        let programs = Arc::clone(&self.programs);
        let graph = Arc::clone(&self.graph);
        R::spawn(async move {
            // Reclaim the mailbox on any exit — loop end, break, a failed load, or an unwinding crash.
            let _guard = DeregisterOnDrop {
                reducers: Arc::clone(&reducers),
                id,
            };
            // The typed reason this reducer closed with, captured when it returns `Break` so its watchers can
            // be told how it ended.
            let mut break_reason: Option<(ContractId, Bytes)> = None;
            // Load the program inside the reducer's own task, so a slow load blocks no one. A failed load
            // falls straight through to the cleanup below.
            if let Some(mut reducer) = programs.spawn(program).await {
                while let Some(event) = R::recv(&mut inbox).await {
                    let (requests, outcome) = fold(&mut reducer, event).await;
                    for request in requests {
                        // Delivering an event into another reducer's log is the one privileged act (§4): only
                        // an event reducer may do it. An ordinary reducer's deliver is routed for it by its
                        // event reducer, so a deliver it emits directly is ignored here.
                        if kind == ReducerKind::Event
                            && request.id == deliver_contract()
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
                    if let Outcome::Break { schema, reason } = outcome {
                        break_reason = Some((schema, reason));
                        break;
                    }
                }
            }
            // On a clean close, tell every watcher how this reducer ended, then leave the graph. Watchers are
            // read before removal, since removal drops the watch_exit edges. A crash unwinds past this — its
            // watchers are not told here (crash notification is host-observed, a later slice) and its graph
            // node lingers until a supervisor reaps it; the mailbox, though, is always reclaimed by the guard.
            if let Some((schema, reason)) = break_reason {
                let notice = Delivered::Notification(
                    Lifecycle::Exited {
                        reducer: id,
                        schema,
                        reason,
                    }
                    .into_notification(),
                );
                for watcher in graph.watchers(id).await {
                    let mailbox = reducers
                        .lock()
                        .expect("reducers lock")
                        .get(&watcher)
                        .cloned();
                    if let Some(mailbox) = mailbox {
                        R::send(&mailbox, notice.clone());
                    }
                }
            }
            graph.remove(id).await;
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

#[cfg(test)]
mod tests {
    use super::{Links, ReducerKind, Spawn, System, TaskSystem};
    use crate::{
        BachRuntime, Bytes, ContractId, Deliver, Delivered, Hash, HostId, InMemoryReducerGraph,
        Lifecycle, Message, Notification, Origin, Outcome, ProgramHash, Reducer, ReducerGraph,
        ReducerId, Request, Response, TokioRuntime, lifecycle_contract,
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
    /// A root spawn (its own parent) with no supervision links — the default shape most tests want.
    fn root(id: ReducerId, program: ProgramHash, kind: ReducerKind) -> Spawn {
        Spawn {
            id,
            program,
            parent: id,
            kind,
            links: Links::NONE,
        }
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

                let graph = Arc::new(InMemoryReducerGraph::new());
                let system =
                    TaskSystem::<BachRuntime>::new(Arc::new(store), Arc::clone(&graph) as _);
                // The router is a privileged event reducer (its deliver is honored) and a root; the sink is
                // an ordinary reducer spawned under it.
                system
                    .spawn(root(router, prog(b"router"), ReducerKind::Event))
                    .await
                    .unwrap();
                system
                    .spawn(Spawn {
                        id: sink,
                        program: prog(b"sink"),
                        parent: router,
                        kind: ReducerKind::Ordinary,
                        links: Links::NONE,
                    })
                    .await
                    .unwrap();

                // Deliver a message to the router; it signals, then delivers to the sink, which signals.
                assert!(
                    system
                        .deliver(router, a_message(cid(b"http.get")))
                        .await
                        .unwrap()
                );
                assert_eq!(saw_rx.recv().await, Some(router));
                assert_eq!(saw_rx.recv().await, Some(sink));
                // The spawn was recorded in the tree: the router is a root, the sink its child.
                assert_eq!(graph.parent(sink).await, Some(router));
                assert!(graph.ancestors(sink).await.contains(&router));
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn an_ordinary_reducers_deliver_is_not_honored() {
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

                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                );
                // This time the router is an ordinary reducer, so the deliver it emits is dropped.
                system
                    .spawn(root(router, prog(b"router"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                system
                    .spawn(Spawn {
                        id: sink,
                        program: prog(b"sink"),
                        parent: router,
                        kind: ReducerKind::Ordinary,
                        links: Links::NONE,
                    })
                    .await
                    .unwrap();

                assert!(
                    system
                        .deliver(router, a_message(cid(b"http.get")))
                        .await
                        .unwrap()
                );
                // The router runs and signals; give the sim time to route anything it emitted.
                assert_eq!(saw_rx.recv().await, Some(router));
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                // The sink never ran — the ordinary router's deliver was not carried out.
                assert!(saw_rx.try_recv().is_err());
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
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                );
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
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                );
                system
                    .spawn(root(rid(b"x"), prog(b"absent"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
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
        let system =
            TaskSystem::<TokioRuntime>::new(Arc::new(store), Arc::new(InMemoryReducerGraph::new()));
        let id = rid(b"term-1");
        system
            .spawn(root(id, prog(b"term"), ReducerKind::Ordinary))
            .await
            .unwrap();
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

    /// A reducer that records every notification it receives on a bach channel, so a test can observe a
    /// lifecycle notification the system delivers to it. It never delivers or closes itself.
    struct Watcher {
        heard: bach::sync::mpsc::UnboundedSender<Notification>,
    }
    #[async_trait::async_trait]
    impl Reducer for Watcher {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, n: Notification) -> (Vec<Request>, Outcome) {
            let _ = self.heard.send(n);
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A reducer that closes on its first message, carrying a fixed typed reason — so its exit reason is
    /// known: schema `finished`, reason `done`.
    struct Closer;
    #[async_trait::async_trait]
    impl Reducer for Closer {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"finished"),
                    reason: Bytes::from_static(b"done"),
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

    /// Register a `watcher` program and a `closer` program in a fresh store-backed system.
    fn watcher_and_closer(
        heard: bach::sync::mpsc::UnboundedSender<Notification>,
    ) -> TaskSystem<BachRuntime> {
        let mut store = crate::testing::program::Store::new();
        store.register(prog(b"watcher"), move || {
            Box::new(Watcher {
                heard: heard.clone(),
            })
        });
        store.register(prog(b"closer"), || Box::new(Closer));
        TaskSystem::<BachRuntime>::new(Arc::new(store), Arc::new(InMemoryReducerGraph::new()))
    }

    /// Spawn a watching parent and a closing child under it, deliver one message to the child so it closes,
    /// and return what (if anything) the parent heard. `links` selects the supervision direction.
    async fn watch_child_exit(links: Links) -> Option<Notification> {
        let parent = rid(b"parent");
        let child = rid(b"child");
        let (heard, mut heard_rx) = bach::sync::mpsc::unbounded_channel();
        let system = watcher_and_closer(heard);
        system
            .spawn(root(parent, prog(b"watcher"), ReducerKind::Ordinary))
            .await
            .unwrap();
        system
            .spawn(Spawn {
                id: child,
                program: prog(b"closer"),
                parent,
                kind: ReducerKind::Ordinary,
                links,
            })
            .await
            .unwrap();

        // Drive the child to its close, then give the sim time to route any lifecycle notification.
        assert!(system.deliver(child, a_message(cid(b"go"))).await.unwrap());
        bach::time::sleep(core::time::Duration::from_millis(1)).await;
        heard_rx.try_recv().ok()
    }

    #[test]
    fn a_child_that_breaks_notifies_a_watching_parent() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // The parent watches the child, so it hears the child's typed exit as a lifecycle
                // notification naming the child.
                let heard = watch_child_exit(Links {
                    parent_watches_child: true,
                    child_watches_parent: false,
                })
                .await
                .expect("the parent heard the child's exit");
                assert_eq!(heard.id, lifecycle_contract());
                assert_eq!(
                    Lifecycle::decode(&heard.payload),
                    Some(Lifecycle::Exited {
                        reducer: rid(b"child"),
                        schema: cid(b"finished"),
                        reason: Bytes::from_static(b"done"),
                    })
                );
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn an_unlinked_child_does_not_notify_its_parent() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // With no supervision link, a child closes without telling its parent.
                assert_eq!(watch_child_exit(Links::NONE).await, None);
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn a_child_watching_its_parent_hears_the_parents_exit() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // The other direction, independently: the child watches the parent, and the parent (not the
                // child) is the one that closes. The child hears the parent's exit.
                let parent = rid(b"parent");
                let child = rid(b"child");
                let (heard, mut heard_rx) = bach::sync::mpsc::unbounded_channel();

                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"closer"), || Box::new(Closer));
                store.register(prog(b"watcher"), move || {
                    Box::new(Watcher {
                        heard: heard.clone(),
                    })
                });
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                );
                // Parent is the closer (root); child is the watcher, subscribed to the parent's exit.
                system
                    .spawn(root(parent, prog(b"closer"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                system
                    .spawn(Spawn {
                        id: child,
                        program: prog(b"watcher"),
                        parent,
                        kind: ReducerKind::Ordinary,
                        links: Links {
                            parent_watches_child: false,
                            child_watches_parent: true,
                        },
                    })
                    .await
                    .unwrap();

                // Close the parent; the child hears the parent's typed exit naming the parent.
                assert!(system.deliver(parent, a_message(cid(b"go"))).await.unwrap());
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                let heard = heard_rx.try_recv().expect("the child heard the parent");
                assert_eq!(
                    Lifecycle::decode(&heard.payload),
                    Some(Lifecycle::Exited {
                        reducer: parent,
                        schema: cid(b"finished"),
                        reason: Bytes::from_static(b"done"),
                    })
                );
            }
            .group("system")
            .primary()
            .spawn();
        });
    }
}
