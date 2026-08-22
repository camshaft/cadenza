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
//! `Arc<dyn System>` and is not generic over the backend. The system records the spawn tree, the supervision
//! links, and each reducer's [kind](ReducerKind) in the [`ReducerGraph`](crate::ReducerGraph) it holds, and
//! it owns dispatch's first act: when a reducer emits an ordinary effect, the system looks up the system
//! reducer for that contract in the [`EventRegistry`](crate::EventRegistry), instantiates a fresh per-event
//! context, and delivers the effect to it (§4). Deriving a *top-level* reducer's id from its genesis is the
//! layer above this; the per-event context ids the system derives itself, from each reducer's rolling hash.
//!
//! [`TaskSystem`] is the in-memory `System`, generic over its [`Runtime`](crate::Runtime) (tokio in
//! production, bach in tests): each reducer is a task draining a channel mailbox, loading its program inside
//! its own task so a slow load blocks no one.

use crate::cancel::CancelScope;
use crate::{
    Bytes, ContractId, Deliver, Delivered, EdgeKind, EventRegistry, FireAfter, Fired, Genesis,
    Hash, HashTag, HostId, Lifecycle, Message, Origin, Outcome, ProgramHash, ProgramStore, Reducer,
    ReducerGraph, ReducerId, Request, Response, Runtime, Spawned, deliver_contract, timer_contract,
};
use async_trait::async_trait;
use futures_util::FutureExt; // catch_unwind, to turn a fold panic into a Crashed notification (§7)
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// The spawn nonce from its genesis. The `id` already commits to it (the id is the hash of the genesis),
    /// but the system also needs it directly: it seeds the reducer's **rolling hash**, from which each
    /// per-event context id the reducer's effects spawn is derived (§4).
    pub nonce: Bytes,
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
/// runtime, draining a channel mailbox. Its state lives in a [`Shared`] behind an `Arc`, so a running
/// reducer's own task can spawn and deliver in turn — the recursion the router needs to route an emitted
/// effect to the system reducer that shepherds it. It loads a spawned reducer's program inside that reducer's
/// own task, so a slow load blocks no one.
pub struct TaskSystem<R: Runtime> {
    shared: Arc<Shared<R>>,
}

impl<R: Runtime> TaskSystem<R> {
    /// A system with no running reducers. `programs` instantiates a reducer from its program hash; `graph`
    /// tracks the spawn tree, supervision links, and handler chains; `events` maps a contract to the system
    /// reducer that shepherds it (§4); and `host` is this node's id, stamped as the `from` host on every
    /// message the system routes, so an effect is attributable to a reducer-on-a-host (§3).
    #[must_use]
    pub fn new(
        programs: Arc<dyn ProgramStore>,
        graph: Arc<dyn ReducerGraph>,
        events: Arc<dyn EventRegistry>,
        host: HostId,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                reducers: Arc::new(Mutex::new(HashMap::new())),
                programs,
                graph,
                events,
                host,
            }),
        }
    }
}

#[async_trait]
impl<R: Runtime> System for TaskSystem<R> {
    async fn spawn(&self, spawn: Spawn) -> Result<(), SystemError> {
        Arc::clone(&self.shared).launch(spawn).await;
        Ok(())
    }

    async fn deliver(&self, target: ReducerId, event: Delivered) -> Result<bool, SystemError> {
        Ok(self.shared.send(target, event))
    }

    async fn contains(&self, id: ReducerId) -> Result<bool, SystemError> {
        Ok(self.shared.contains(id))
    }
}

/// The system's shared state, held behind an `Arc` so a running reducer's own task can route the effects it
/// emits — spawning the system reducer for a contract and delivering to it — without routing through a
/// separate, central router.
struct Shared<R: Runtime> {
    reducers: Arc<Mutex<HashMap<ReducerId, R::Sender>>>,
    programs: Arc<dyn ProgramStore>,
    graph: Arc<dyn ReducerGraph>,
    events: Arc<dyn EventRegistry>,
    host: HostId,
}

impl<R: Runtime> Shared<R> {
    /// Deliver an event into a reducer's mailbox; `false` if none is running under `target`.
    fn send(&self, target: ReducerId, event: Delivered) -> bool {
        let sender = self
            .reducers
            .lock()
            .expect("reducers lock")
            .get(&target)
            .cloned();
        match sender {
            Some(sender) => R::send(&sender, event),
            None => false,
        }
    }

    /// Whether a reducer is running under `id`.
    fn contains(&self, id: ReducerId) -> bool {
        self.reducers
            .lock()
            .expect("reducers lock")
            .contains_key(&id)
    }

    /// Start running the reducer a [`Spawn`] describes: record its place in the graph, give it a mailbox,
    /// deliver its birth notification, then drive it in its own task — folding events, routing the effects it
    /// emits, honoring its delivers if it is privileged, and reclaiming it and notifying its watchers when it
    /// closes. Takes `self` by `Arc` so the task can retain it to route in turn. Returns a boxed future
    /// (rather than an `async fn`) so the recursion — a reducer's task launching the system reducer for an
    /// effect it emits — goes through a type-erased indirection, which both breaks the infinite future type
    /// and lets the `Send` bound be stated explicitly instead of inferred through the cycle.
    fn launch(self: Arc<Self>, spawn: Spawn) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let Spawn {
                id,
                program,
                nonce,
                parent,
                kind,
                links,
            } = spawn;
            // Record the spawn tree and supervision edges before the task starts (§7). A root has no parent edge;
            // a child links to its parent, plus a watch_exit edge per requested supervision direction.
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
            // The birth notification is the reducer's first event: its own id and parent (§7), sent before the
            // task drains so it folds first once the program has loaded.
            R::send(
                &sender,
                Delivered::Notification(Spawned { id, parent }.into_notification()),
            );
            self.reducers
                .lock()
                .expect("reducers lock")
                .insert(id, sender);
            let shared = self;
            R::spawn(async move {
                // Reclaim the mailbox on any exit — loop end, break, a failed load, or an unwinding crash.
                let _guard = DeregisterOnDrop {
                    reducers: Arc::clone(&shared.reducers),
                    id,
                };
                // The cancellation scope for this reducer's in-flight timer tasks. Dropping it when the task
                // ends any way — clean close, break, mailbox close, or an unwinding crash — cancels every arm
                // still pending, so no timer outlives the reducer that armed it (§6/§7).
                let timers = CancelScope::new();
                let mut break_reason: Option<(ContractId, Bytes)> = None;
                // The rolling hash each per-event context id is derived from — seeded with this reducer's nonce
                // and advanced once per routed effect, so every effect gets a distinct, replay-stable context
                // (§4).
                let mut rolling = Hash::of(HashTag::SystemProperty, &nonce);
                // Load the program inside the reducer's own task, so a slow load blocks no one.
                if let Some(mut reducer) = shared.programs.spawn(program).await {
                    while let Some(event) = R::recv(&mut inbox).await {
                        let (requests, outcome) =
                            match std::panic::AssertUnwindSafe(fold(&mut reducer, event))
                                .catch_unwind()
                                .await
                            {
                                Ok(folded) => folded,
                                // A panic in a fold is an uncontrolled crash: stop draining and fall through
                                // to the `Crashed` notification below (`break_reason` stays `None`).
                                Err(_panic) => break,
                            };
                        for request in requests {
                            if request.id == deliver_contract() {
                                // Delivering an event into another reducer's log is the one privileged act (§4):
                                // honored only from an event reducer. An ordinary reducer's deliver is routed for
                                // it, so a deliver it emits directly is ignored.
                                if kind == ReducerKind::Event
                                    && let Some(deliver) = Deliver::decode(&request.payload)
                                {
                                    shared.send(deliver.target, deliver.event);
                                }
                            } else if request.id == timer_contract() {
                                // Arm a fire-after timer (§6) — any reducer may. The runtime waits the
                                // duration on its clock, then wakes the arming reducer with the `Fired` event
                                // AS THE RESPONSE to this request, correlated by the request's own standard
                                // continuation-token (not a bespoke field). The recorded fire time is stamped
                                // from the runtime's clock; the reducer folds it without reading a clock
                                // itself. Enforcing a request deadline on top of this raw wake is the system
                                // reducer's policy (§4), not the kernel's.
                                if let Some(arm) = FireAfter::decode(&request.payload) {
                                    let shared = Arc::clone(&shared);
                                    let armed = id;
                                    let token = request.continuation_token;
                                    // Spawn the wait-then-wake future through this reducer's cancel scope, so
                                    // it is aborted if the reducer exits before it fires (§6/§7). The scope
                                    // only wraps the future; the runtime still does the spawning.
                                    R::spawn(timers.wrap(async move {
                                        R::sleep(Duration::from_nanos(arm.duration)).await;
                                        let fired = Fired {
                                            fired_time: R::now(),
                                        };
                                        shared.send(
                                            armed,
                                            Delivered::Response(Response {
                                                id: timer_contract(),
                                                continuation_token: token,
                                                payload: Ok(fired.encode()),
                                            }),
                                        );
                                    }));
                                }
                            } else {
                                // An ordinary effect: the kernel looks up the system reducer for the contract,
                                // instantiates a fresh per-event context, and delivers the effect to it (§4).
                                rolling = Hash::of(HashTag::SystemProperty, rolling.as_bytes());
                                let context_nonce = Bytes::copy_from_slice(rolling.as_bytes());
                                let program = shared.events.resolve(request.id).await;
                                let context = Genesis {
                                    program,
                                    nonce: context_nonce.clone(),
                                    parent: id,
                                }
                                .id();
                                // Spawn the system reducer (privileged) if this context is not already running,
                                // then deliver the effect to it stamped with the emitter's origin.
                                if !shared.contains(context) {
                                    // `launch` returns a boxed future, so this recursive call is type-erased.
                                    Arc::clone(&shared)
                                        .launch(Spawn {
                                            id: context,
                                            program,
                                            nonce: context_nonce,
                                            parent: id,
                                            kind: ReducerKind::Event,
                                            links: Links::NONE,
                                        })
                                        .await;
                                }
                                shared.send(
                                    context,
                                    Delivered::Message(Message {
                                        id: request.id,
                                        payload: request.payload,
                                        from: Origin {
                                            reducer: id,
                                            host: shared.host,
                                        },
                                        continuation_token: request.continuation_token,
                                    }),
                                );
                            }
                        }
                        if let Outcome::Break { schema, reason } = outcome {
                            break_reason = Some((schema, reason));
                            break;
                        }
                    }
                }
                // Tell every watcher how this reducer ended, then leave the graph (§7): a clean Break is an
                // `Exited` carrying its typed reason; any other end — a fold that panicked (caught above), the
                // mailbox closing, or a program that failed to load — is a `Crashed` naming only who ended. The
                // mailbox is always reclaimed by the guard.
                let ended = match break_reason {
                    Some((schema, reason)) => Lifecycle::Exited {
                        reducer: id,
                        schema,
                        reason,
                    },
                    None => Lifecycle::Crashed { reducer: id },
                };
                let notice = Delivered::Notification(ended.into_notification());
                for watcher in shared.graph.watchers(id).await {
                    shared.send(watcher, notice.clone());
                }
                shared.graph.remove(id).await;
            });
        })
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
        BachRuntime, Bytes, ContractId, Deliver, Delivered, FireAfter, Fired, HostId,
        InMemoryEventRegistry, InMemoryReducerGraph, Lifecycle, Message, Notification, Origin,
        Outcome, ProgramHash, Reducer, ReducerGraph, ReducerId, Request, Response, Spawned,
        TokioRuntime, lifecycle_contract, spawned_contract, timer_contract,
    };
    use std::sync::Arc;

    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::of(tag)
    }
    /// A root spawn (its own parent) with no supervision links — the default shape most tests want.
    fn root(id: ReducerId, program: ProgramHash, kind: ReducerKind) -> Spawn {
        Spawn {
            id,
            program,
            nonce: Bytes::from_static(b"nonce"),
            parent: id,
            kind,
            links: Links::NONE,
        }
    }
    fn origin() -> Origin {
        Origin {
            reducer: rid(b"peer"),
            host: HostId::of(b"host-a"),
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
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::clone(&graph) as _,
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
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
                        nonce: Bytes::from_static(b"nonce"),
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
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
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
                        nonce: Bytes::from_static(b"nonce"),
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
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
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
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
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
        let system = TaskSystem::<TokioRuntime>::new(
            Arc::new(store),
            Arc::new(InMemoryReducerGraph::new()),
            Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
            HostId::of(b"node"),
        );
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

    /// A reducer that panics on its first message — an uncontrolled crash, distinct from a typed `Break`.
    struct Panicker;
    #[async_trait::async_trait]
    impl Reducer for Panicker {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            panic!("panicker crashed");
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// Register a `watcher` program, a `closer` program, and a `panicker` program in a fresh store-backed
    /// system.
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
        store.register(prog(b"panicker"), || Box::new(Panicker));
        TaskSystem::<BachRuntime>::new(
            Arc::new(store),
            Arc::new(InMemoryReducerGraph::new()),
            Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
            HostId::of(b"node"),
        )
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
                nonce: Bytes::from_static(b"nonce"),
                parent,
                kind: ReducerKind::Ordinary,
                links,
            })
            .await
            .unwrap();

        // Drive the child to its close, then give the sim time to route any lifecycle notification.
        assert!(system.deliver(child, a_message(cid(b"go"))).await.unwrap());
        bach::time::sleep(core::time::Duration::from_millis(1)).await;
        // Skip the parent's own birth notification; return the first lifecycle event it heard, if any.
        while let Ok(n) = heard_rx.try_recv() {
            if n.id == lifecycle_contract() {
                return Some(n);
            }
        }
        None
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
    fn a_child_that_panics_notifies_a_watching_parent_with_crashed() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // A fold panic is an uncontrolled crash: the system catches it and tells the watching parent
                // via a `Crashed` lifecycle notification naming the child — distinct from a typed `Exited`,
                // and it does not take down the system.
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
                        program: prog(b"panicker"),
                        nonce: Bytes::from_static(b"nonce"),
                        parent,
                        kind: ReducerKind::Ordinary,
                        links: Links {
                            parent_watches_child: true,
                            child_watches_parent: false,
                        },
                    })
                    .await
                    .unwrap();

                // Deliver a message so the child panics. Waiting on the channel (bach advances time; its
                // deadlock detection covers a crash that never surfaces), skip the parent's own birth
                // notification and find the child's lifecycle event.
                assert!(system.deliver(child, a_message(cid(b"go"))).await.unwrap());
                let crashed = loop {
                    let n = heard_rx
                        .recv()
                        .await
                        .expect("the parent heard the child's crash");
                    if n.id == lifecycle_contract() {
                        break Lifecycle::decode(&n.payload);
                    }
                };
                assert_eq!(crashed, Some(Lifecycle::Crashed { reducer: child }));
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
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
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
                        nonce: Bytes::from_static(b"nonce"),
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
                // Skip the child's own birth notification; find the parent's lifecycle event.
                let mut exit = None;
                while let Ok(n) = heard_rx.try_recv() {
                    if n.id == lifecycle_contract() {
                        exit = Some(n);
                        break;
                    }
                }
                let heard = exit.expect("the child heard the parent's exit");
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

    #[test]
    fn a_spawned_reducer_is_told_its_id_and_parent_first() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // A parent and a child, each recording its notifications on its own channel, so the child's
                // first event can be read without interleaving. The child is spawned under the parent.
                let parent = rid(b"parent");
                let child = rid(b"child");
                let (p_heard, _p_rx) = bach::sync::mpsc::unbounded_channel();
                let (c_heard, mut c_rx) = bach::sync::mpsc::unbounded_channel();

                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"parent-prog"), move || {
                    Box::new(Watcher {
                        heard: p_heard.clone(),
                    })
                });
                store.register(prog(b"child-prog"), move || {
                    Box::new(Watcher {
                        heard: c_heard.clone(),
                    })
                });

                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
                system
                    .spawn(root(parent, prog(b"parent-prog"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                system
                    .spawn(Spawn {
                        id: child,
                        program: prog(b"child-prog"),
                        nonce: Bytes::from_static(b"nonce"),
                        parent,
                        kind: ReducerKind::Ordinary,
                        links: Links::NONE,
                    })
                    .await
                    .unwrap();
                bach::time::sleep(core::time::Duration::from_millis(1)).await;

                // The child's very first event is its birth notification, naming itself and its parent.
                let birth = c_rx.try_recv().expect("the child heard its birth");
                assert_eq!(birth.id, spawned_contract());
                assert_eq!(
                    Spawned::decode(&birth.payload),
                    Some(Spawned { id: child, parent })
                );
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    /// A reducer that, on its first message, emits one ordinary (non-deliver) effect against `contract`,
    /// then closes — so the system must route that effect to the system reducer for the contract.
    struct Emitter {
        contract: ContractId,
    }
    #[async_trait::async_trait]
    impl Reducer for Emitter {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            let request = Request {
                id: self.contract,
                payload: Bytes::from_static(b"effect"),
                continuation_token: Bytes::from_static(b"k"),
                deadline: None,
            };
            (
                vec![request],
                Outcome::Break {
                    schema: cid(b"done"),
                    reason: Bytes::from_static(b"emitted"),
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

    /// A stand-in system reducer: it records the origin and contract of the effect routed to it, then closes.
    struct SystemStub {
        saw: bach::sync::mpsc::UnboundedSender<(ReducerId, ContractId)>,
    }
    #[async_trait::async_trait]
    impl Reducer for SystemStub {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let _ = self.saw.send((m.from.reducer, m.id));
            (
                Vec::new(),
                Outcome::Break {
                    schema: cid(b"shepherded"),
                    reason: Bytes::new(),
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

    #[test]
    fn the_system_routes_an_ordinary_effect_to_the_system_reducer_for_its_contract() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let emitter = rid(b"emitter");
                let http = cid(b"http.get");
                let (saw, mut saw_rx) = bach::sync::mpsc::unbounded_channel();

                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"emitter"), move || {
                    Box::new(Emitter { contract: http })
                });
                store.register(prog(b"sys"), move || {
                    Box::new(SystemStub { saw: saw.clone() })
                });

                // The event registry routes every contract to the `sys` program by default.
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
                system
                    .spawn(root(emitter, prog(b"emitter"), ReducerKind::Ordinary))
                    .await
                    .unwrap();

                // Deliver a message so the emitter emits its effect; the system routes it to a freshly
                // spawned system reducer, which sees it as a message from the emitter on the http.get
                // contract.
                assert!(
                    system
                        .deliver(emitter, a_message(cid(b"go")))
                        .await
                        .unwrap()
                );
                assert_eq!(saw_rx.recv().await, Some((emitter, http)));
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    /// A reducer that arms a fire-after timer on its first message, then waits; when the timer fires (as the
    /// response to that arm), it records the recorded fire time and the echoed continuation-token, then closes.
    struct Armer {
        duration: u64,
        token: Bytes,
        heard: bach::sync::mpsc::UnboundedSender<(Bytes, u64)>,
    }
    #[async_trait::async_trait]
    impl Reducer for Armer {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            // Arm the timer (durations are nanoseconds), correlating the wake with the standard
            // continuation-token every request carries — no bespoke token rides in the value.
            let arm = FireAfter {
                duration: self.duration,
            };
            (
                vec![arm.into_request(self.token.clone())],
                Outcome::Continue,
            )
        }
        async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
            if r.id == timer_contract()
                && let Ok(payload) = &r.payload
                && let Some(fired) = Fired::decode(payload)
            {
                let _ = self.heard.send((r.continuation_token, fired.fired_time));
                return (
                    Vec::new(),
                    Outcome::Break {
                        schema: cid(b"woke"),
                        reason: Bytes::new(),
                    },
                );
            }
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    /// A reducer that arms a timer and closes in the same fold — so it exits with a fire-after still pending,
    /// which the system must cancel.
    struct ArmThenClose {
        duration: u64,
    }
    #[async_trait::async_trait]
    impl Reducer for ArmThenClose {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            let arm = FireAfter {
                duration: self.duration,
            };
            (
                vec![arm.into_request(Bytes::from_static(b"pending"))],
                Outcome::Break {
                    schema: cid(b"closed"),
                    reason: Bytes::new(),
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

    #[test]
    fn a_reducer_that_arms_a_fire_after_timer_is_woken_after_the_duration() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                let armer = rid(b"armer");
                let (heard, mut heard_rx) = bach::sync::mpsc::unbounded_channel();
                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"armer"), move || {
                    Box::new(Armer {
                        duration: 10_000_000,
                        token: Bytes::from_static(b"awaiting"),
                        heard: heard.clone(),
                    })
                });
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
                system
                    .spawn(root(armer, prog(b"armer"), ReducerKind::Ordinary))
                    .await
                    .unwrap();

                // Deliver a message so it arms a 10ms timer, then wait on the wake: bach advances simulated
                // time to fire it (and its deadlock detection would catch a timer that never fires). The wake
                // is the response to the arm, carrying the arm's standard continuation-token back and a
                // recorded fire time at or past the 10ms (10_000_000ns) it waited on the deterministic clock.
                assert!(system.deliver(armer, a_message(cid(b"go"))).await.unwrap());
                let (token, fired_time) = heard_rx.recv().await.expect("the timer fired");
                assert_eq!(token, Bytes::from_static(b"awaiting"));
                assert!(fired_time >= 10_000_000, "recorded fire time {fired_time}");
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn concurrent_timers_fire_independently_each_woken_with_its_own_token() {
        // Two reducers arm timers of different durations at once. Each must be woken on its OWN schedule and
        // with its OWN continuation-token — pinning that a per-arm sleep task is isolated (no cross-talk) and
        // the wake correlates by the arm's token, not by which timer happened to fire.
        use bach::ext::*;
        bach::sim(|| {
            async {
                let (heard, mut heard_rx) = bach::sync::mpsc::unbounded_channel();
                let mut store = crate::testing::program::Store::new();
                // A short (10ms) and a long (50ms) timer, each with a distinct token.
                let short_heard = heard.clone();
                store.register(prog(b"short"), move || {
                    Box::new(Armer {
                        duration: 10_000_000,
                        token: Bytes::from_static(b"short"),
                        heard: short_heard.clone(),
                    })
                });
                store.register(prog(b"long"), move || {
                    Box::new(Armer {
                        duration: 50_000_000,
                        token: Bytes::from_static(b"long"),
                        heard: heard.clone(),
                    })
                });
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
                let short = rid(b"short");
                let long = rid(b"long");
                system
                    .spawn(root(short, prog(b"short"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                system
                    .spawn(root(long, prog(b"long"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                // Both arm their timers. Waiting on the channel, the short (10ms) timer must wake FIRST — its
                // task sleeps independently of the long (50ms) one — each with its own continuation-token and
                // a recorded fire time past its own duration. The strict order + distinct tokens pin per-arm
                // task isolation (no cross-talk) and token-correlation.
                assert!(system.deliver(short, a_message(cid(b"go"))).await.unwrap());
                assert!(system.deliver(long, a_message(cid(b"go"))).await.unwrap());

                let (token, fired_time) =
                    heard_rx.recv().await.expect("the short timer fired first");
                assert_eq!(token, Bytes::from_static(b"short"));
                assert!((10_000_000..50_000_000).contains(&fired_time));

                let (token, fired_time) = heard_rx.recv().await.expect("the long timer fired next");
                assert_eq!(token, Bytes::from_static(b"long"));
                assert!(fired_time >= 50_000_000, "recorded fire time {fired_time}");
            }
            .group("system")
            .primary()
            .spawn();
        });
    }

    #[test]
    fn a_reducers_pending_timer_is_cancelled_when_it_exits() {
        use bach::ext::*;
        bach::sim(|| {
            async {
                // A reducer arms a 100ms timer then closes in the same fold, so it exits with the arm still
                // pending — the system must cancel it, not let it fire into the void (or a later reducer). To
                // observe that, we re-use the SAME id for a fresh reducer that would forward any fire it
                // receives: if the pending timer had NOT been cancelled, its late `Fired` would land on the
                // reused id and show up on the channel.
                let id = rid(b"ephemeral");
                let (heard, mut heard_rx) = bach::sync::mpsc::unbounded_channel();
                let mut store = crate::testing::program::Store::new();
                store.register(prog(b"arm-then-close"), || {
                    Box::new(ArmThenClose {
                        duration: 100_000_000,
                    })
                });
                // The catcher never arms its own timer (it is delivered no message); it only forwards a `Fired`
                // response — which should never arrive, because the earlier arm was cancelled on exit.
                store.register(prog(b"catcher"), move || {
                    Box::new(Armer {
                        duration: 0,
                        token: Bytes::new(),
                        heard: heard.clone(),
                    })
                });
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(store),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(prog(b"sys"))),
                    HostId::of(b"node"),
                );
                system
                    .spawn(root(id, prog(b"arm-then-close"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                // Arm the 100ms timer and close; give the sim a moment for the reducer to fold, exit, and
                // cancel its pending arm.
                assert!(system.deliver(id, a_message(cid(b"go"))).await.unwrap());
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                // Re-use the id for a fresh reducer that would catch a leaked late fire.
                system
                    .spawn(root(id, prog(b"catcher"), ReducerKind::Ordinary))
                    .await
                    .unwrap();
                // Advance well past the original 100ms duration. A cancelled arm never fires, so nothing
                // reaches the reused id. (A negative assertion needs a timed check, not a blocking `recv`.)
                bach::time::sleep(core::time::Duration::from_millis(200)).await;
                assert!(
                    heard_rx.try_recv().is_err(),
                    "the pending timer was cancelled on exit, so no fire reached the reused id"
                );
            }
            .group("system")
            .primary()
            .spawn();
        });
    }
}
