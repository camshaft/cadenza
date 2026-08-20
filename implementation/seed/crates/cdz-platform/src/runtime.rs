//! The runtime — the kernel's reducer-execution core (`design/cadenza-platform.md` §3/§9).
//!
//! The kernel is a reducer-execution engine: it holds the running reducers, runs one reducer step given an
//! event, and routes an emitted effect's contract to the event reducer that governs it (via the
//! [`EventRegistry`]). This is that core for the native, in-memory build — before wasm loading, a reducer
//! is any [`Reducer`] value held in a map by its id.
//!
//! Two primitives live here. **Routing** ([`route`](Runtime::route)) is the lookup on an emitted effect:
//! the contract-id resolves to the event reducer that shepherds it. **Running** ([`run`](Runtime::run))
//! folds an event through a reducer's matching entry point and returns what it emitted. What the runtime
//! does with those emitted requests — above all carrying out `deliver`, the one privileged primitive that
//! injects an event into another reducer's log — is the next slice; this establishes the store and the two
//! primitives it builds on.

use crate::{EventRegistry, Hash, Message, Notification, Outcome, Reducer, Request, Response};
use std::collections::HashMap;

/// An event delivered to a reducer, selecting the entry point it folds through: the three kinds an ordinary
/// [`Reducer`] receives. The runtime dispatches each to `on_message` / `on_response` / `on_notification`.
pub enum Delivered {
    /// Deliver to `on_message` — an effect performed on the reducer.
    Message(Message),
    /// Deliver to `on_response` — a reply to a request the reducer performed.
    Response(Response),
    /// Deliver to `on_notification` — a platform control-plane event.
    Notification(Notification),
}

/// The in-memory runtime: the set of running reducers, keyed by id, plus the [`EventRegistry`] that says
/// which event reducer governs a contract. It runs reducer steps and routes contracts; the event reducer a
/// route resolves to is then run like any other reducer.
pub struct Runtime {
    reducers: HashMap<Hash, Box<dyn Reducer>>,
    events: EventRegistry,
}

impl Runtime {
    /// A runtime with no reducers yet, over the given event registry (which carries the default event
    /// reducer and any overrides).
    #[must_use]
    pub fn new(events: EventRegistry) -> Self {
        Self {
            reducers: HashMap::new(),
            events,
        }
    }

    /// Register a running reducer under its id, returning the reducer it replaced (if any).
    pub fn register(&mut self, id: Hash, reducer: Box<dyn Reducer>) -> Option<Box<dyn Reducer>> {
        self.reducers.insert(id, reducer)
    }

    /// Remove a reducer from the running set — it has terminated — returning it if it was present.
    pub fn remove(&mut self, id: Hash) -> Option<Box<dyn Reducer>> {
        self.reducers.remove(&id)
    }

    /// Whether a reducer is registered under `id`.
    #[must_use]
    pub fn contains(&self, id: Hash) -> bool {
        self.reducers.contains_key(&id)
    }

    /// The event reducer that governs `contract` — the kernel's routing lookup on an emitted effect. Always
    /// resolves (the event registry has a default), so this returns a reducer id, not an option.
    #[must_use]
    pub fn route(&self, contract: Hash) -> Hash {
        self.events.resolve(contract)
    }

    /// Run one step of the reducer `target`: fold `event` through its matching entry point and return the
    /// requests it emits and its outcome. `None` if no reducer is registered under `target`.
    pub async fn run(&mut self, target: Hash, event: Delivered) -> Option<(Vec<Request>, Outcome)> {
        let reducer = self.reducers.get_mut(&target)?;
        Some(match event {
            Delivered::Message(message) => reducer.on_message(message).await,
            Delivered::Response(response) => reducer.on_response(response).await,
            Delivered::Notification(notification) => reducer.on_notification(notification).await,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Delivered, Runtime};
    use crate::{
        Bytes, EventRegistry, Hash, Message, Notification, Origin, Outcome, Reducer, Request,
        Response,
    };

    /// A reducer that emits a distinct request per entry point, so a test can tell which one the runtime
    /// dispatched to.
    struct Probe;

    fn tag(name: &'static [u8]) -> Request {
        Request {
            id: Hash::of(name),
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
        Runtime::new(EventRegistry::new(Hash::of(b"default-event-reducer")))
    }

    fn origin() -> Origin {
        Origin {
            reducer: Hash::of(b"peer"),
            host: Hash::of(b"host-a"),
        }
    }

    #[tokio::test]
    async fn run_dispatches_each_event_to_its_entry_point() {
        let mut rt = runtime();
        let id = Hash::of(b"probe");
        rt.register(id, Box::new(Probe));
        assert!(rt.contains(id));

        let (msg_out, _) = rt
            .run(
                id,
                Delivered::Message(Message {
                    id: Hash::of(b"c"),
                    payload: Bytes::from_static(b"x"),
                    from: origin(),
                    continuation_token: Bytes::from_static(b"t"),
                }),
            )
            .await
            .expect("reducer is registered");
        assert_eq!(msg_out[0].id, Hash::of(b"on_message"));

        let (resp_out, _) = rt
            .run(
                id,
                Delivered::Response(Response {
                    id: Hash::of(b"c"),
                    continuation_token: Bytes::from_static(b"t"),
                    payload: Ok(Bytes::from_static(b"y")),
                }),
            )
            .await
            .unwrap();
        assert_eq!(resp_out[0].id, Hash::of(b"on_response"));

        let (notif_out, _) = rt
            .run(
                id,
                Delivered::Notification(Notification {
                    id: Hash::of(b"c"),
                    payload: Bytes::from_static(b"z"),
                }),
            )
            .await
            .unwrap();
        assert_eq!(notif_out[0].id, Hash::of(b"on_notification"));
    }

    #[tokio::test]
    async fn running_an_unregistered_reducer_is_none() {
        let mut rt = runtime();
        let out = rt
            .run(
                Hash::of(b"ghost"),
                Delivered::Notification(Notification {
                    id: Hash::of(b"c"),
                    payload: Bytes::from_static(b""),
                }),
            )
            .await;
        assert!(out.is_none());
    }

    #[test]
    fn route_resolves_a_contract_to_its_event_reducer() {
        // With no override, every contract routes to the default event reducer.
        let mut events = EventRegistry::new(Hash::of(b"default"));
        events.set_override(Hash::of(b"session.spawn"), Hash::of(b"session-reducer"));
        let rt = Runtime::new(events);
        assert_eq!(rt.route(Hash::of(b"http.get")), Hash::of(b"default"));
        assert_eq!(
            rt.route(Hash::of(b"session.spawn")),
            Hash::of(b"session-reducer")
        );
    }

    #[tokio::test]
    async fn a_removed_reducer_no_longer_runs() {
        let mut rt = runtime();
        let id = Hash::of(b"probe");
        rt.register(id, Box::new(Probe));
        assert!(rt.remove(id).is_some());
        assert!(!rt.contains(id));
        let out = rt
            .run(
                id,
                Delivered::Message(Message {
                    id: Hash::of(b"c"),
                    payload: Bytes::from_static(b"x"),
                    from: origin(),
                    continuation_token: Bytes::from_static(b"t"),
                }),
            )
            .await;
        assert!(out.is_none());
    }
}
