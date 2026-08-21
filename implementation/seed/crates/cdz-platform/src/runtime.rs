//! The runtime — the kernel's reducer-execution engine (`design/cadenza-platform.md` §3/§9).
//!
//! The kernel's job is a loop: it holds the running reducers, a queue of pending **deliveries** (an event to
//! fold into a reducer's log), and drains that queue one delivery at a time. Each step folds an event into
//! its target reducer, which emits requests; a request that is a `deliver` (the privileged routing act, §4)
//! is itself a delivery, enqueued to run in turn. Reacting to an appended event is the only way a reducer
//! runs (principle 5), and running one may append more — so the whole system advances by repeatedly ticking
//! the queue until it is empty.
//!
//! A reducer enters the running set only by being **spawned** from a program: [`spawn`](Runtime::spawn)
//! instantiates the program and derives the reducer's id by hashing its genesis (the program plus a spawn
//! nonce) — a reducer never names its own id. It then persists in the running set, addressed by that id, so
//! later events (a response to a request it made, a message from a peer) are delivered back to the same
//! instance.
//!
//! This is the native, in-memory engine. Routing an ordinary effect to the event reducer that shepherds it
//! (spawning that event reducer and delivering the effect to it) is the next slice, layered on `spawn` and
//! the queue; here a delivery is enqueued directly (via [`enqueue`](Runtime::enqueue)) or by a reducer
//! emitting a `deliver`.

use crate::{
    ContractId, Deliver, Delivered, EventRegistry, Hash, Outcome, ProgramHash, ProgramStore,
    Reducer, ReducerId, deliver_contract,
};
use std::collections::{HashMap, VecDeque};

/// The in-memory runtime: the running reducers keyed by [`ReducerId`], the [`EventRegistry`] naming which
/// program a contract's event reducer is spawned from, the [`ProgramStore`] that instantiates a program into
/// a reducer, and the queue of pending deliveries the engine drains. Reducers are spawned into the running
/// set (their id generated from their genesis) and persist there; the queue drives them by delivering events.
pub struct Runtime {
    reducers: HashMap<ReducerId, Box<dyn Reducer>>,
    events: EventRegistry,
    programs: Box<dyn ProgramStore>,
    queue: VecDeque<Deliver>,
}

impl Runtime {
    /// A runtime with no running reducers and an empty queue, over the given event registry (which names the
    /// default event reducer program and any overrides) and program store (which instantiates a program into
    /// a reducer).
    #[must_use]
    pub fn new(events: EventRegistry, programs: Box<dyn ProgramStore>) -> Self {
        Self {
            reducers: HashMap::new(),
            events,
            programs,
            queue: VecDeque::new(),
        }
    }

    /// Spawn a reducer from `program`: instantiate it, derive its id by hashing its genesis (the program it
    /// runs plus the spawn `nonce`), register it in the running set, and return the id. `None` if the program
    /// store cannot instantiate `program`. The id is generated here from the genesis — a reducer never names
    /// its own id, and the same `(program, nonce)` always derives the same id (so a genesis is reproducible).
    pub async fn spawn(&mut self, program: ProgramHash, nonce: &[u8]) -> Option<ReducerId> {
        let reducer = self.programs.spawn(program).await?;
        let id = reducer_id(program, nonce);
        self.reducers.insert(id, reducer);
        Some(id)
    }

    /// Spawn the event reducer that governs `contract` (§4): route the contract to its program and spawn it
    /// with `nonce`. `None` if that program cannot be instantiated. Registered and persisted like any spawned
    /// reducer — it shepherds the effect across the request/response cycle, so it must outlive a single step.
    pub async fn spawn_event_reducer(
        &mut self,
        contract: ContractId,
        nonce: &[u8],
    ) -> Option<ReducerId> {
        self.spawn(self.route(contract), nonce).await
    }

    /// Remove a reducer from the running set — it has terminated — returning it if it was present.
    pub fn remove(&mut self, id: ReducerId) -> Option<Box<dyn Reducer>> {
        self.reducers.remove(&id)
    }

    /// Whether a reducer is registered under `id`.
    #[must_use]
    pub fn contains(&self, id: ReducerId) -> bool {
        self.reducers.contains_key(&id)
    }

    /// The program a contract's event reducer is spawned from — the routing lookup on an emitted effect.
    /// Always resolves (the event registry has a default), so this returns a [`ProgramHash`], not an option.
    #[must_use]
    pub fn route(&self, contract: ContractId) -> ProgramHash {
        self.events.resolve(contract)
    }

    /// Enqueue a delivery — an event to fold into a reducer's log. This is the unit of work the engine
    /// drains: it seeds the queue (an initial message to a reducer, a response) and is what a reducer's
    /// emitted `deliver` becomes. Delivering is the one privileged routing act (§4); the queue is where a
    /// delivery waits its turn.
    pub fn enqueue(&mut self, deliver: Deliver) {
        self.queue.push_back(deliver);
    }

    /// Whether the queue has no pending deliveries — the engine is quiescent.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }

    /// Process one queued delivery. Pop the next delivery, fold its event into the target reducer, enqueue
    /// each `deliver` the reducer emits (the routing act — it waits its turn in the queue), and act on the
    /// reducer's [`Outcome`] (a [`Break`](Outcome::Break) retires the reducer from the running set). Returns
    /// the outcome, or `None` if the queue is empty. A delivery to a reducer no longer in the running set is
    /// dropped, reported as [`Continue`](Outcome::Continue) so draining proceeds.
    ///
    /// An emitted request that is not a `deliver` is the reducer's own effect; routing it to the event
    /// reducer that governs its contract is the next slice, so it is not acted on here yet.
    pub async fn tick(&mut self) -> Option<Outcome> {
        let Deliver { target, event } = self.queue.pop_front()?;
        let Some(reducer) = self.reducers.get_mut(&target) else {
            // The target has been retired (or never existed); its delivery is dropped, draining continues.
            return Some(Outcome::Continue);
        };
        let (requests, outcome) = match event {
            Delivered::Message(message) => reducer.on_message(message).await,
            Delivered::Response(response) => reducer.on_response(response).await,
            Delivered::Notification(notification) => reducer.on_notification(notification).await,
        };
        for request in requests {
            // A `deliver` is the routing act: enqueue the delivery it carries to run in turn. Any other
            // request is the reducer's own effect — event-reducer routing for it is the next slice.
            if request.id == deliver_contract()
                && let Some(deliver) = Deliver::decode(&request.payload)
            {
                self.queue.push_back(deliver);
            }
        }
        if let Outcome::Break { .. } = outcome {
            self.reducers.remove(&target);
        }
        Some(outcome)
    }
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
    use super::Runtime;
    use crate::{
        Bytes, ContractId, Deliver, Delivered, EventRegistry, Hash, HostId, Message, Notification,
        Origin, Outcome, ProgramHash, Reducer, ReducerId, Request, Response,
    };

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
    fn message(id: ContractId) -> Delivered {
        Delivered::Message(Message {
            id,
            payload: Bytes::from_static(b"e"),
            from: origin(),
            continuation_token: Bytes::from_static(b"t"),
        })
    }
    fn runtime() -> Runtime {
        Runtime::new(
            EventRegistry::new(prog(b"default-event-program")),
            Box::new(crate::testing::program::Store::new()),
        )
    }

    /// A reducer that, on each message, optionally emits a `deliver` to a fixed target — enough to drive the
    /// queue (a reducer whose emitted deliver enqueues the next delivery).
    struct Noting {
        deliver_to: Option<ReducerId>,
    }

    #[async_trait::async_trait]
    impl Reducer for Noting {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let requests = match self.deliver_to {
                Some(target) => vec![
                    Deliver {
                        target,
                        event: Delivered::Message(m),
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

    /// A reducer that terminates: its first message breaks with a given schema/reason.
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

    #[tokio::test]
    async fn spawn_generates_a_reproducible_id_and_persists_the_reducer() {
        let mut programs = crate::testing::program::Store::new();
        programs.register(prog(b"p"), || Box::new(Noting { deliver_to: None }));
        let mut rt = Runtime::new(EventRegistry::new(prog(b"default")), Box::new(programs));

        let id = rt.spawn(prog(b"p"), b"nonce-1").await.expect("registered");
        assert!(rt.contains(id));
        // The id is derived from the genesis: same program + nonce derives the same id, a different nonce a
        // different one — the runtime generated it, the reducer did not name it.
        assert_eq!(super::reducer_id(prog(b"p"), b"nonce-1"), id);
        assert_ne!(super::reducer_id(prog(b"p"), b"nonce-2"), id);
        // A program with no factory cannot be spawned.
        assert!(rt.spawn(prog(b"absent"), b"n").await.is_none());
    }

    #[tokio::test]
    async fn tick_on_an_empty_queue_is_none() {
        let mut rt = runtime();
        assert!(rt.is_idle());
        assert!(rt.tick().await.is_none());
    }

    #[tokio::test]
    async fn tick_folds_a_delivery_and_enqueues_the_delivers_the_reducer_emits() {
        // A `router` that on its message delivers to a `sink`. The sink's id is deterministic from its
        // genesis, so the router's factory can target it up front. Delivering to the router then ticking
        // should, in turn, deliver to the sink — the queue self-feeding via the router's emitted `deliver`.
        let sink_id = super::reducer_id(prog(b"sink"), b"1");
        let mut programs = crate::testing::program::Store::new();
        programs.register(prog(b"sink"), || Box::new(Noting { deliver_to: None }));
        programs.register(prog(b"router"), move || {
            Box::new(Noting {
                deliver_to: Some(sink_id),
            })
        });
        let mut rt = Runtime::new(EventRegistry::new(prog(b"default")), Box::new(programs));

        let sink = rt.spawn(prog(b"sink"), b"1").await.expect("sink spawns");
        assert_eq!(sink, sink_id);
        let router = rt
            .spawn(prog(b"router"), b"1")
            .await
            .expect("router spawns");

        // Seed the queue with a message to the router; tick processes it and enqueues the deliver to the sink.
        rt.enqueue(Deliver {
            target: router,
            event: message(cid(b"http.get")),
        });
        assert!(matches!(rt.tick().await, Some(Outcome::Continue))); // router ran, enqueued deliver-to-sink
        assert!(!rt.is_idle()); // the deliver-to-sink is queued
        assert!(matches!(rt.tick().await, Some(Outcome::Continue))); // sink ran
        assert!(rt.is_idle()); // quiescent
    }

    #[tokio::test]
    async fn a_break_outcome_retires_the_reducer() {
        let mut programs = crate::testing::program::Store::new();
        programs.register(prog(b"term"), || Box::new(Terminating));
        let mut rt = Runtime::new(EventRegistry::new(prog(b"default")), Box::new(programs));
        let id = rt.spawn(prog(b"term"), b"1").await.expect("spawns");
        assert!(rt.contains(id));
        rt.enqueue(Deliver {
            target: id,
            event: message(cid(b"c")),
        });
        assert!(matches!(rt.tick().await, Some(Outcome::Break { .. })));
        // Break retired it from the running set.
        assert!(!rt.contains(id));
    }

    #[tokio::test]
    async fn a_delivery_to_a_retired_reducer_is_dropped_and_draining_continues() {
        let mut rt = runtime();
        // Deliver to an id that was never spawned; tick drops it and reports Continue (not idle/None).
        rt.enqueue(Deliver {
            target: ReducerId::from_hash(Hash::of(b"ghost")),
            event: message(cid(b"c")),
        });
        assert!(matches!(rt.tick().await, Some(Outcome::Continue)));
        assert!(rt.is_idle());
        assert!(rt.tick().await.is_none());
    }
}
