//! The runtime — the kernel's reducer-execution core (`design/cadenza-platform.md` §3/§9).
//!
//! The kernel is a reducer-execution engine: it holds the running reducers, runs one reducer step given an
//! event, and routes an emitted effect's contract to the program its event reducer is spawned from (via the
//! [`EventRegistry`]). This is that core for the native, in-memory build — before wasm loading, a reducer
//! is any [`Reducer`] value held in a map by its [`ReducerId`].
//!
//! Primitives live here. **Routing** ([`route`](Runtime::route)) is the lookup on an emitted effect: the
//! [`ContractId`] resolves to the [`ProgramHash`] the kernel spawns that contract's event reducer from.
//! **Spawning** ([`spawn_event_reducer`](Runtime::spawn_event_reducer)) instantiates that program into a
//! fresh reducer (§4 — the system reducer is instantiated once per event). **Running** ([`run`](Runtime::run))
//! folds an event through a reducer's matching entry point and returns what it emitted. **Carrying out**
//! ([`carry_out`](Runtime::carry_out)) recognizes a `deliver` request — the one privileged primitive that
//! injects an event into another reducer's log (§4) — and runs its target. Assembling these into the full
//! dispatch loop (run the spawned event reducer on the effect, feed its emitted requests back) is a later
//! slice.

use crate::{
    ContractId, Deliver, Delivered, EventRegistry, Outcome, ProgramHash, ProgramStore, Reducer,
    ReducerId, Request, deliver_contract,
};
use std::collections::HashMap;

/// The in-memory runtime: the set of running reducers keyed by [`ReducerId`], the [`EventRegistry`] that
/// says which program a contract's event reducer is spawned from, and the [`ProgramStore`] that instantiates
/// a program hash into a fresh reducer. It runs reducer steps, routes contracts, and spawns the event reducer
/// a route resolves to; a spawned event reducer is then run like any other reducer.
pub struct Runtime {
    reducers: HashMap<ReducerId, Box<dyn Reducer>>,
    events: EventRegistry,
    programs: Box<dyn ProgramStore>,
}

impl Runtime {
    /// A runtime with no running reducers yet, over the given event registry (which names the default event
    /// reducer program and any overrides) and the given program store (which instantiates a program hash
    /// into a reducer — the default event reducer's factory is registered in it at setup).
    #[must_use]
    pub fn new(events: EventRegistry, programs: Box<dyn ProgramStore>) -> Self {
        Self {
            reducers: HashMap::new(),
            events,
            programs,
        }
    }

    /// Register a running reducer under its id, returning the reducer it replaced (if any).
    pub fn register(
        &mut self,
        id: ReducerId,
        reducer: Box<dyn Reducer>,
    ) -> Option<Box<dyn Reducer>> {
        self.reducers.insert(id, reducer)
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

    /// The program a contract's event reducer is spawned from — the kernel's routing lookup on an emitted
    /// effect. Always resolves (the event registry has a default), so this returns a [`ProgramHash`], not an
    /// option; the kernel spawns a fresh event reducer from it per event.
    #[must_use]
    pub fn route(&self, contract: ContractId) -> ProgramHash {
        self.events.resolve(contract)
    }

    /// Spawn the event reducer that governs `contract`: route the contract to its program, then instantiate
    /// that program into a fresh reducer (§4 — the system reducer is instantiated once per event). `None` if
    /// the resolved program has no registered factory — a misconfiguration, since routing always resolves to
    /// a program but the kernel may not know how to instantiate it. The spawned reducer is not registered in
    /// the running set; it is ephemeral, run for this one event.
    pub async fn spawn_event_reducer(&self, contract: ContractId) -> Option<Box<dyn Reducer>> {
        self.programs.spawn(self.route(contract)).await
    }

    /// Run one step of the reducer `target`: fold `event` through its matching entry point and return the
    /// requests it emits and its outcome. `None` if no reducer is registered under `target`.
    pub async fn run(
        &mut self,
        target: ReducerId,
        event: Delivered,
    ) -> Option<(Vec<Request>, Outcome)> {
        let reducer = self.reducers.get_mut(&target)?;
        Some(match event {
            Delivered::Message(message) => reducer.on_message(message).await,
            Delivered::Response(response) => reducer.on_response(response).await,
            Delivered::Notification(notification) => reducer.on_notification(notification).await,
        })
    }

    /// Carry out an emitted request if it is a **deliver** — the routing act (§4): recognize the built-in
    /// [`deliver_contract`], decode the [`Deliver`] envelope, and run the target with the delivered event.
    /// Returns the target's result, or `None` if the request is not a deliver, its envelope is malformed, or
    /// no reducer is registered under the target. (Only the event reducer the [`EventRegistry`] names may
    /// legitimately emit a deliver; enforcing that privilege is the kernel's, above this method.)
    pub async fn carry_out(&mut self, request: Request) -> Option<(Vec<Request>, Outcome)> {
        if request.id != deliver_contract() {
            return None;
        }
        let envelope = Deliver::decode(&request.payload)?;
        self.run(envelope.target, envelope.event).await
    }
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
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::from_hash(Hash::of(tag))
    }
    fn prog(tag: &[u8]) -> ProgramHash {
        ProgramHash::from_hash(Hash::of(tag))
    }

    /// A reducer that emits a distinct request per entry point, so a test can tell which one the runtime
    /// dispatched to.
    struct Probe;

    fn tag(name: &'static [u8]) -> Request {
        Request {
            id: cid(name),
            payload: Bytes::from_static(name),
            continuation_token: Bytes::from_static(b"t"),
            deadline: None,
        }
    }

    #[async_trait::async_trait]
    impl Reducer for Probe {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![tag(b"on_message")], Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![tag(b"on_response")], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![tag(b"on_notification")], Outcome::Continue)
        }
    }

    fn runtime() -> Runtime {
        Runtime::new(
            EventRegistry::new(prog(b"default-event-program")),
            Box::new(crate::programs::testing::Store::new()),
        )
    }

    fn origin() -> Origin {
        Origin {
            reducer: rid(b"peer"),
            host: HostId::from_hash(Hash::of(b"host-a")),
        }
    }

    #[tokio::test]
    async fn run_dispatches_each_event_to_its_entry_point() {
        let mut rt = runtime();
        let id = rid(b"probe");
        rt.register(id, Box::new(Probe));
        assert!(rt.contains(id));

        let (msg_out, _) = rt
            .run(
                id,
                Delivered::Message(Message {
                    id: cid(b"c"),
                    payload: Bytes::from_static(b"x"),
                    from: origin(),
                    continuation_token: Bytes::from_static(b"t"),
                }),
            )
            .await
            .expect("reducer is registered");
        assert_eq!(msg_out[0].id, cid(b"on_message"));

        let (resp_out, _) = rt
            .run(
                id,
                Delivered::Response(Response {
                    id: cid(b"c"),
                    continuation_token: Bytes::from_static(b"t"),
                    payload: Ok(Bytes::from_static(b"y")),
                }),
            )
            .await
            .unwrap();
        assert_eq!(resp_out[0].id, cid(b"on_response"));

        let (notif_out, _) = rt
            .run(
                id,
                Delivered::Notification(Notification {
                    id: cid(b"c"),
                    payload: Bytes::from_static(b"z"),
                }),
            )
            .await
            .unwrap();
        assert_eq!(notif_out[0].id, cid(b"on_notification"));
    }

    #[tokio::test]
    async fn running_an_unregistered_reducer_is_none() {
        let mut rt = runtime();
        let out = rt
            .run(
                rid(b"ghost"),
                Delivered::Notification(Notification {
                    id: cid(b"c"),
                    payload: Bytes::from_static(b""),
                }),
            )
            .await;
        assert!(out.is_none());
    }

    #[test]
    fn route_resolves_a_contract_to_its_event_reducer_program() {
        // With no override, every contract routes to the default program; an override wins for its contract.
        let mut events = EventRegistry::new(prog(b"default"));
        events.set_override(cid(b"session.spawn"), prog(b"session-program"));
        let rt = Runtime::new(events, Box::new(crate::programs::testing::Store::new()));
        assert_eq!(rt.route(cid(b"http.get")), prog(b"default"));
        assert_eq!(rt.route(cid(b"session.spawn")), prog(b"session-program"));
    }

    #[tokio::test]
    async fn spawn_event_reducer_instantiates_the_program_a_contract_routes_to() {
        // The default program governs any contract; an override redirects one. Register a factory for each
        // program, then spawning for a contract instantiates the program that contract routes to — verified
        // by running the fresh reducer and reading which entry point it dispatched to.
        let mut events = EventRegistry::new(prog(b"default-event-program"));
        events.set_override(cid(b"session.spawn"), prog(b"session-program"));
        let mut programs = crate::programs::testing::Store::new();
        programs.register(prog(b"default-event-program"), || Box::new(Probe));
        programs.register(prog(b"session-program"), || Box::new(Probe));
        let rt = Runtime::new(events, Box::new(programs));

        // A contract with no override spawns the default program's reducer; the override spawns its own.
        for contract in [cid(b"http.get"), cid(b"session.spawn")] {
            let mut reducer = rt
                .spawn_event_reducer(contract)
                .await
                .expect("the routed program has a registered factory");
            let (out, _) = reducer
                .on_message(Message {
                    id: contract,
                    payload: Bytes::from_static(b"e"),
                    from: origin(),
                    continuation_token: Bytes::from_static(b"t"),
                })
                .await;
            assert_eq!(out[0].id, cid(b"on_message"));
        }
    }

    #[tokio::test]
    async fn spawn_event_reducer_is_none_when_the_routed_program_has_no_factory() {
        // Routing always resolves to a program, but if the store cannot instantiate it the event reducer
        // cannot be spawned — a misconfiguration surfaced as `None`, not a panic.
        let rt = Runtime::new(
            EventRegistry::new(prog(b"unregistered-default")),
            Box::new(crate::programs::testing::Store::new()),
        );
        assert!(rt.spawn_event_reducer(cid(b"http.get")).await.is_none());
    }

    #[tokio::test]
    async fn a_removed_reducer_no_longer_runs() {
        let mut rt = runtime();
        let id = rid(b"probe");
        rt.register(id, Box::new(Probe));
        assert!(rt.remove(id).is_some());
        assert!(!rt.contains(id));
        let out = rt
            .run(
                id,
                Delivered::Message(Message {
                    id: cid(b"c"),
                    payload: Bytes::from_static(b"x"),
                    from: origin(),
                    continuation_token: Bytes::from_static(b"t"),
                }),
            )
            .await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn carry_out_delivers_a_deliver_request_to_its_target() {
        let mut rt = runtime();
        let target = rid(b"probe");
        rt.register(target, Box::new(Probe));
        // A deliver request (as an event reducer would emit) that injects a message into the target's log.
        let request = Deliver {
            target,
            event: Delivered::Message(Message {
                id: cid(b"http.get"),
                payload: Bytes::from_static(b"x"),
                from: origin(),
                continuation_token: Bytes::from_static(b"t"),
            }),
        }
        .into_request();
        let (out, _) = rt
            .carry_out(request)
            .await
            .expect("delivered to the target");
        // It reached the target's on_message entry point.
        assert_eq!(out[0].id, cid(b"on_message"));
    }

    #[tokio::test]
    async fn carry_out_ignores_a_non_deliver_request() {
        let mut rt = runtime();
        rt.register(rid(b"probe"), Box::new(Probe));
        // A request against some other contract is not a deliver — the runtime does not carry it out.
        let not_a_deliver = Request {
            id: cid(b"some.contract"),
            payload: Bytes::from_static(b""),
            continuation_token: Bytes::from_static(b""),
            deadline: None,
        };
        assert!(rt.carry_out(not_a_deliver).await.is_none());
    }
}
