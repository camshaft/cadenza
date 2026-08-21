//! The running reducers — a registry of reducer tasks (`design/cadenza-platform.md` §3/§9).
//!
//! Each reducer runs as its own async task that owns its state and has a **mailbox**: an unbounded channel
//! whose receiver the task drains, folding one delivered event at a time through the reducer's matching
//! entry point. A reducer blocking on IO (an edge reducer's network or subprocess call) parks only its own
//! task; every other reducer keeps running, and the work spreads across the executor's threads. This is the
//! actor model — no single-threaded scheduler, no shared tick.
//!
//! [`Reducers`] is the registry those tasks share: a map from [`ReducerId`] to the reducer's mailbox sender,
//! behind an `Arc<Mutex<…>>` so any task can look up a peer and deliver to it. Delivering an event is that
//! lookup-and-send — the privileged routing act (§4); a response is a [`Delivered::Response`] delivered to
//! the caller's mailbox (responses route as ordinary deliveries, §4). Spawning generates the reducer's id
//! from its genesis and registers the mailbox synchronously, then spawns the task; the task loads the
//! program from the shared [`ProgramStore`] and runs its loop — so a program load never blocks the spawner
//! or any other reducer.

use crate::{
    ContractId, Deliver, Delivered, EventRegistry, Hash, Outcome, ProgramHash, ProgramStore,
    Reducer, ReducerId, deliver_contract, rt,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A reducer's mailbox sender — where its inbound events are delivered. Cloneable, so many peers can hold a
/// handle to the same reducer's mailbox.
type Mailbox = rt::mpsc::UnboundedSender<Delivered>;

/// The shared map of running reducers to their mailboxes. `Arc<Mutex<…>>` because every reducer task holds a
/// clone and reads it to deliver to peers; the lock is held only to clone a sender or edit the map, never
/// across an await.
type Mailboxes = Arc<Mutex<HashMap<ReducerId, Mailbox>>>;

/// The registry of running reducer tasks (§3): which reducers are alive and how to deliver to each. Also
/// holds the shared [`ProgramStore`] that instantiates a program into a reducer, and the [`EventRegistry`]
/// that names which program a contract's event reducer is spawned from.
pub struct Reducers {
    mailboxes: Mailboxes,
    programs: Arc<dyn ProgramStore>,
    events: EventRegistry,
}

impl Reducers {
    /// A registry with no running reducers, over the given event registry and shared program store.
    #[must_use]
    pub fn new(events: EventRegistry, programs: Arc<dyn ProgramStore>) -> Self {
        Self {
            mailboxes: Arc::new(Mutex::new(HashMap::new())),
            programs,
            events,
        }
    }

    /// The program a contract's event reducer is spawned from — the routing lookup on an emitted effect.
    #[must_use]
    pub fn route(&self, contract: ContractId) -> ProgramHash {
        self.events.resolve(contract)
    }

    /// Whether a reducer is currently registered (its task alive) under `id`.
    #[must_use]
    pub fn contains(&self, id: ReducerId) -> bool {
        self.mailboxes
            .lock()
            .expect("mailboxes lock")
            .contains_key(&id)
    }

    /// The number of running reducers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.mailboxes.lock().expect("mailboxes lock").len()
    }

    /// Whether no reducer is running.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.mailboxes.lock().expect("mailboxes lock").is_empty()
    }

    /// Spawn a reducer from `program`: derive its id from its genesis (the program plus a spawn `nonce` — the
    /// registry generates the id, a reducer never names its own), register its mailbox, and spawn its task.
    /// Returns the id immediately; the task loads the program from the shared store and runs its loop, so the
    /// async load blocks no one. The reducer persists (its task alive, its mailbox registered) until it
    /// breaks or its program fails to load. Registering the same id again replaces the mailbox.
    pub fn spawn(&self, program: ProgramHash, nonce: &[u8]) -> ReducerId {
        let id = reducer_id(program, nonce);
        let (mailbox, inbox) = rt::mpsc::unbounded_channel();
        self.mailboxes
            .lock()
            .expect("mailboxes lock")
            .insert(id, mailbox);

        let mailboxes = Arc::clone(&self.mailboxes);
        let programs = Arc::clone(&self.programs);
        rt::spawn(async move {
            match programs.spawn(program).await {
                Some(reducer) => reducer_loop(id, reducer, inbox, mailboxes).await,
                // The program could not be instantiated — deregister the mailbox we reserved.
                None => drop_mailbox(&mailboxes, id),
            }
        });
        id
    }

    /// Spawn the event reducer that governs `contract` (§4): route the contract to its program and spawn it
    /// with `nonce`. Persisted like any reducer, so it shepherds the effect across the request/response cycle
    /// rather than living for a single step.
    pub fn spawn_event_reducer(&self, contract: ContractId, nonce: &[u8]) -> ReducerId {
        self.spawn(self.route(contract), nonce)
    }

    /// Deliver an event into a reducer's mailbox — the privileged routing act (§4). `true` if the reducer is
    /// running and its mailbox accepted the event; `false` if no reducer is registered under `target` (its
    /// task never spawned or has ended).
    pub fn deliver(&self, target: ReducerId, event: Delivered) -> bool {
        deliver_to(&self.mailboxes, target, event)
    }
}

/// Look up `target`'s mailbox and deliver `event` into it. The lock is taken only to clone the sender (never
/// held across the send), so delivering never blocks the map. `false` if `target` is not registered.
fn deliver_to(mailboxes: &Mailboxes, target: ReducerId, event: Delivered) -> bool {
    let mailbox = mailboxes
        .lock()
        .expect("mailboxes lock")
        .get(&target)
        .cloned();
    match mailbox {
        Some(mailbox) => mailbox.send(event).is_ok(),
        None => false,
    }
}

/// Remove a reducer's mailbox from the registry — it has ended (broke, or its program failed to load).
fn drop_mailbox(mailboxes: &Mailboxes, id: ReducerId) {
    mailboxes.lock().expect("mailboxes lock").remove(&id);
}

/// A running reducer's task: drain the mailbox, folding each delivered event through the reducer, delivering
/// each `deliver` it emits to the named peer's mailbox, and acting on the outcome. A [`Break`](Outcome::Break)
/// ends the reducer; the loop also ends when the mailbox closes (all senders dropped). Either way the reducer
/// is deregistered on exit. An emitted request that is not a `deliver` is the reducer's own effect; routing
/// it to the event reducer that governs its contract is the next slice.
async fn reducer_loop(
    id: ReducerId,
    mut reducer: Box<dyn Reducer>,
    mut inbox: rt::mpsc::UnboundedReceiver<Delivered>,
    mailboxes: Mailboxes,
) {
    while let Some(event) = inbox.recv().await {
        let (requests, outcome) = match event {
            Delivered::Message(message) => reducer.on_message(message).await,
            Delivered::Response(response) => reducer.on_response(response).await,
            Delivered::Notification(notification) => reducer.on_notification(notification).await,
        };
        for request in requests {
            if request.id == deliver_contract()
                && let Some(deliver) = Deliver::decode(&request.payload)
            {
                deliver_to(&mailboxes, deliver.target, deliver.event);
            }
        }
        if let Outcome::Break { .. } = outcome {
            break;
        }
    }
    drop_mailbox(&mailboxes, id);
}

/// Derive a reducer's id from its genesis — the program it runs plus a spawn nonce — by content hash, so an
/// id is reproducible from its genesis and a reducer never names its own. (A richer genesis, e.g. a parent
/// link for the spawn tree of §7, folds in here later without changing the shape.)
fn reducer_id(program: ProgramHash, nonce: &[u8]) -> ReducerId {
    let mut genesis = program.hash().as_bytes().to_vec();
    genesis.extend_from_slice(nonce);
    ReducerId::from_hash(Hash::of(&genesis))
}

#[cfg(test)]
mod tests {
    use super::{Reducers, reducer_id};
    use crate::{
        Bytes, ContractId, Deliver, Delivered, EventRegistry, Hash, HostId, Message, Notification,
        Origin, Outcome, ProgramHash, Reducer, ReducerId, Request, Response, rt,
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

    /// A reducer that, on each message, signals receipt on a test channel and optionally delivers a message
    /// to a fixed peer — enough to observe the actor flow end to end.
    struct Probe {
        saw: rt::mpsc::UnboundedSender<ReducerId>,
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
                // The sink's id is deterministic from its genesis, so the router can target it up front.
                let sink_id = reducer_id(prog(b"sink"), b"1");
                let (saw, mut saw_rx) = rt::mpsc::unbounded_channel();

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

                let reducers = Reducers::new(EventRegistry::new(prog(b"default")), Arc::new(store));
                let sink = reducers.spawn(prog(b"sink"), b"1");
                assert_eq!(sink, sink_id);
                let router = reducers.spawn(prog(b"router"), b"1");
                assert_eq!(reducers.len(), 2);

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
                let reducers = Reducers::new(EventRegistry::new(prog(b"default")), Arc::new(store));
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
                let reducers = Reducers::new(EventRegistry::new(prog(b"default")), Arc::new(store));
                let id = reducers.spawn(prog(b"absent"), b"1");
                // The mailbox is registered synchronously, but the task's load fails and deregisters it.
                // Yield so the spawned task runs; then it is gone.
                bach::time::sleep(core::time::Duration::from_millis(1)).await;
                assert!(!reducers.contains(id));
            }
            .group("reducers")
            .primary()
            .spawn();
        });
    }
}
