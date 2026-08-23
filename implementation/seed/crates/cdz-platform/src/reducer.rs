//! The reducer interface (`design/cadenza-platform.md` §3).
//!
//! There is one kind of participant: a reducer. Anything that takes part — a session running an agent's
//! task, a handler that answers a single contract, the boundary that performs input and output — is a
//! reducer with the same interface: it receives an event, updates its own state, and emits effect
//! requests. There is no separate executor.
//!
//! Everything a reducer receives is an event carrying a contract-id, and it is one of a few kinds, so a
//! reducer has three entry points and the runtime calls the one that fits:
//! - [`on_response`](Reducer::on_response) — the output side: a reply to a request this reducer performed.
//! - [`on_message`](Reducer::on_message) — the input side: an effect another reducer performed on this
//!   one (or a message sent to it), carrying its [`Origin`] — the sending reducer and the host that ran
//!   it — so the reducer can authenticate and route on who sent it and from where.
//! - [`on_notification`](Reducer::on_notification) — the control-plane side: an unsolicited platform event
//!   such as a new handler becoming available or a lifecycle event, typed by contract-id like anything
//!   else, so one entry point carries every kind and a reducer handles or ignores each by its id.
//!
//! Both return the same product: the [`Request`]s to perform next, and an [`Outcome`] (`Continue` to keep
//! running, or `Break` to terminate the reducer carrying a typed reason). Each call is a pure function of
//! its input and the reducer's current state; a fresh instance runs each call and holds no memory between
//! calls. Correlation is by the `continuation_token` a reducer chooses on a request and gets back on the
//! response — not the `id` (the contract-id is shared by every request of that contract).
//!
//! Identity by hash, payload by value in bytes: every event carries an `id` (the contract-id, which is the
//! hash of the schema) plus a `payload`. The payload is the value in its canonical encoding — carried as
//! [`Bytes`] because the runtime routes on the `id` and never decodes a payload; a reducer that needs the
//! structured value decodes it (and derefs the schema from the store by `id`) itself, lazily.

use crate::{Bytes, ContractId, HostId, ReducerId};
use async_trait::async_trait;
use std::time::Duration;

/// A runtime-level failure of a request — distinct from a handler's own domain error, which is a normal
/// answer that rides in `Ok(output)` (§3). These are the only two ways a dispatch fails without an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The request carried a deadline that elapsed with no answer. The dispatch is cancelled — no late
    /// answer will ever fold — so a reducer never has to handle a response arriving after it gave up.
    Timeout,
    /// No handler is registered for the contract; nothing could answer it.
    MissingHandler,
}

/// What a reduce call decides about the reducer's own life (§3). A reducer ends *itself* only by returning
/// `Break`; it never ends itself via an effect. Because the return is a product of `(requests, outcome)`,
/// a reducer can emit final requests and `Break` in one call (send a result, notify a peer, then close);
/// those final requests dispatch but their responses never fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Keep the reducer running.
    Continue,
    /// Terminate the reducer, carrying the reason for closure as a typed value: the schema hash of the
    /// reason plus the reason value's canonical bytes. The runtime imposes no normal-vs-error taxonomy —
    /// a clean completion and a failure are both `Break`s, distinguished only by the reason a subscribing
    /// supervisor decodes.
    Break {
        /// The contract-id (schema hash) of the reason value.
        schema: ContractId,
        /// The reason value in its canonical encoding.
        reason: Bytes,
    },
}

/// A request a reducer emits: perform the contract `id` with `payload`, and correlate the eventual answer
/// by `continuation_token` (§3/§4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The contract-id: the hash of the schema being requested.
    pub id: ContractId,
    /// The input value, in its canonical encoding.
    pub payload: Bytes,
    /// The token the reducer chooses to correlate the eventual response back to this request. Unique per
    /// outstanding request in a session; returned verbatim on the response.
    pub continuation_token: Bytes,
    /// An optional per-request deadline. With `Some(d)`, no answer within `d` delivers `Err(Timeout)` and
    /// cancels the dispatch; `None` means no timeout. This is the reducer's own opt-in anti-stuck control.
    pub deadline: Option<Duration>,
}

/// The answer to a request this reducer performed, delivered to [`on_response`](Reducer::on_response). The
/// correlation (`id`, `continuation_token`) is on the `Response` itself — not inside the `Result` — so a
/// failure is matched back to its originating request the same way a success is (§4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The contract-id this answers (schema hash).
    pub id: ContractId,
    /// The token from the originating request — present on both `Ok` and `Err`.
    pub continuation_token: Bytes,
    /// The result: `Ok` the contract's output value (canonical bytes; a handler's *domain* failure rides
    /// here too, since answering with an error is still answering), or `Err` a runtime-level failure.
    pub payload: Result<Bytes, Error>,
}

/// The source of a [`Message`] (§3): the sending reducer and the host that ran it. The kernel stamps both
/// as envelope metadata a reducer cannot forge, so a reducer authenticates and routes on who sent an effect
/// and from where. Carrying the `host` alongside the `reducer` is the hook for federated trust: a receiver
/// can gate on reducer-on-host, and a grant attributed to an `Origin` stays attributable across a
/// federation (§4/§11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Origin {
    /// The sending reducer's id.
    pub reducer: ReducerId,
    /// The host (node/runtime) that ran the sending reducer.
    pub host: HostId,
}

/// An effect another reducer performed on this one, delivered to [`on_message`](Reducer::on_message). It
/// carries its [`Origin`] (`from`) — the sending reducer and its host — as envelope metadata so the reducer
/// can authenticate and route on who sent it and from where, the reason `on_message` is distinct from
/// `on_response`. The reducer answers by emitting its reply, correlated by this `continuation_token`, which
/// the runtime routes back to the caller's `on_response`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The contract-id of the effect being performed on this reducer.
    pub id: ContractId,
    /// The input value of that effect, in its canonical encoding.
    pub payload: Bytes,
    /// The source — the sending reducer and the host that ran it — to authenticate or route on.
    pub from: Origin,
    /// The caller's token; the reducer's reply is correlated back to the caller by it.
    pub continuation_token: Bytes,
}

/// An unsolicited platform control-plane event, delivered to [`on_notification`](Reducer::on_notification):
/// a new handler becoming available (the trigger for propagation, §3), or a lifecycle event a reducer
/// subscribed to (spawned / closed / failed, §7). It is shaped like a response without a
/// `continuation_token` — a contract-id and a plain typed payload. There is no `continuation_token`
/// (nothing of the reducer's is being answered) and no `Result` (a notification is an event that happened,
/// not the success-or-failure of a request); an error condition, where one applies, lives in the payload's
/// own schema. Because it is typed by `id`, one entry point carries every kind of platform event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    /// The contract-id of the notification's schema.
    pub id: ContractId,
    /// The notification value, in its canonical encoding.
    pub payload: Bytes,
}

/// The one interface every ordinary participant implements (§3): receive an event, update state, emit
/// requests. The three entry points return the same product — the requests to perform and the [`Outcome`].
/// Async because a reduce call may await the reducer's direct accesses (its key-value store, the content
/// store); it never blocks the runtime.
///
/// The methods take `&mut self`: a reducer folds an event into its own state, so it mutates. Each reducer
/// lives in the runtime's registry with a single owner — its own task drives it, one event at a time — so
/// exclusive access is the natural fit and the trait does not force interior mutability on implementations.
/// An implementation is still free to use interior mutability if it wants.
///
/// `Send` (not `Sync`): a reducer is moved into its task and driven only through `&mut` from that one task;
/// it is never shared across threads, so `Send` (to hand it to the task) is all the runtime needs. Requiring
/// `Sync` would rule out a wasm-backed reducer, which owns a wasmtime `Store` — `Send` but not `Sync` — with
/// no benefit, since nothing ever holds a reducer behind a shared reference.
///
/// The system reducer that shepherds an effect (§4) is a distinct, privileged interface, not this one.
#[async_trait]
pub trait Reducer: Send {
    /// React to a [`Response`] — a reply to a request this reducer performed.
    async fn on_response(&mut self, response: Response) -> (Vec<Request>, Outcome);

    /// React to a [`Message`] — an effect performed on this reducer by another, carrying its [`Origin`].
    async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome);

    /// React to a [`Notification`] — an unsolicited platform control-plane event. A reducer that has no
    /// interest in a given notification simply returns `(Vec::new(), Outcome::Continue)`; ignoring the
    /// control plane is safe (for handler-availability, not propagating fails closed, §3).
    async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome);
}

#[cfg(test)]
mod tests {
    use super::{Message, Notification, Origin, Outcome, Reducer, Request, Response};
    use crate::{Bytes, ContractId, HostId, ReducerId};

    // Typed-id helpers over distinct hashes.
    fn cid(tag: &[u8]) -> ContractId {
        ContractId::of(tag)
    }
    fn rid(tag: &[u8]) -> ReducerId {
        ReducerId::of(tag)
    }
    fn hid(tag: &[u8]) -> HostId {
        HostId::of(tag)
    }

    /// A reducer that actually folds state: it counts the messages it has seen (in `&mut self`), forwards
    /// each to a downstream contract carrying the running count, and closes once it has seen `close_at`.
    /// It also reacts to one control-plane notification — the `propagate` contract — by forwarding it
    /// downstream, and ignores every other notification. The behavior on any given event depends on the
    /// accumulated state and the event's contract-id, so it exercises real folding rather than a fixed shape.
    struct Counter {
        seen: u32,
        close_at: u32,
        downstream: ContractId,
        propagate: ContractId,
    }

    #[async_trait::async_trait]
    impl Reducer for Counter {
        async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome) {
            self.seen += 1;
            let request = Request {
                id: self.downstream,
                payload: Bytes::copy_from_slice(&self.seen.to_le_bytes()),
                continuation_token: message.continuation_token,
                deadline: None,
            };
            let outcome = if self.seen >= self.close_at {
                Outcome::Break {
                    schema: message.id,
                    reason: Bytes::from_static(b"reached limit"),
                }
            } else {
                Outcome::Continue
            };
            (vec![request], outcome)
        }

        async fn on_response(&mut self, _response: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }

        async fn on_notification(&mut self, notification: Notification) -> (Vec<Request>, Outcome) {
            // Only the `propagate` notification does anything: forward it downstream. Every other
            // control-plane event is ignored (returns no requests), which is the safe default.
            if notification.id == self.propagate {
                let request = Request {
                    id: self.downstream,
                    payload: notification.payload,
                    continuation_token: Bytes::from_static(b"propagate"),
                    deadline: None,
                };
                (vec![request], Outcome::Continue)
            } else {
                (Vec::new(), Outcome::Continue)
            }
        }
    }

    fn counter(close_at: u32) -> Counter {
        Counter {
            seen: 0,
            close_at,
            downstream: cid(b"downstream"),
            propagate: cid(b"propagate"),
        }
    }

    fn msg(token: &'static [u8]) -> Message {
        Message {
            id: cid(b"inbound"),
            payload: Bytes::from_static(b"x"),
            from: Origin {
                reducer: rid(b"peer"),
                host: hid(b"host-a"),
            },
            continuation_token: Bytes::from_static(token),
        }
    }

    #[tokio::test]
    async fn folds_state_across_calls_and_breaks_at_the_limit() {
        let mut r = counter(3);
        // The running count carried on each forwarded request reflects accumulated state: 1, then 2.
        let (out1, o1) = r.on_message(msg(b"t1")).await;
        assert_eq!(out1[0].payload, Bytes::copy_from_slice(&1u32.to_le_bytes()));
        assert_eq!(o1, Outcome::Continue);
        let (out2, o2) = r.on_message(msg(b"t2")).await;
        assert_eq!(out2[0].payload, Bytes::copy_from_slice(&2u32.to_le_bytes()));
        assert_eq!(o2, Outcome::Continue);
        // The third message hits the limit, so the reducer closes — an outcome that depends on state.
        let (out3, o3) = r.on_message(msg(b"t3")).await;
        assert_eq!(out3[0].payload, Bytes::copy_from_slice(&3u32.to_le_bytes()));
        assert!(matches!(o3, Outcome::Break { .. }));
    }

    #[tokio::test]
    async fn a_forwarded_request_carries_the_callers_token_for_correlation() {
        let mut r = counter(100);
        // The reducer routes to its downstream contract and threads the caller's token through, so the
        // eventual answer correlates back — the routing behavior, not the struct shape.
        let (requests, _) = r.on_message(msg(b"correlate-me")).await;
        assert_eq!(requests[0].id, cid(b"downstream"));
        assert_eq!(
            requests[0].continuation_token,
            Bytes::from_static(b"correlate-me")
        );
    }

    #[tokio::test]
    async fn reacts_to_a_control_plane_notification_and_ignores_unknown_ones() {
        let mut r = counter(100);
        // The `propagate` notification is acted on: forwarded downstream carrying its payload.
        let (out, o) = r
            .on_notification(Notification {
                id: cid(b"propagate"),
                payload: Bytes::from_static(b"new-handler"),
            })
            .await;
        assert_eq!(out[0].id, cid(b"downstream"));
        assert_eq!(out[0].payload, Bytes::from_static(b"new-handler"));
        assert_eq!(o, Outcome::Continue);
        // An unrelated notification is ignored — no requests — and a notification never counts as a message.
        let (out2, _) = r
            .on_notification(Notification {
                id: cid(b"some-lifecycle-event"),
                payload: Bytes::from_static(b"ignored"),
            })
            .await;
        assert!(out2.is_empty());
        assert_eq!(r.seen, 0, "notifications are not messages");
    }

    /// A reducer that authenticates on the message's `Origin`: it forwards only effects that came from a
    /// trusted host, and denies (drops) the rest. This exercises `from` carrying the host, the hook for
    /// federated trust — routing on where an effect came from, not only which reducer sent it.
    struct HostGate {
        trusted_host: HostId,
        downstream: ContractId,
    }

    #[async_trait::async_trait]
    impl Reducer for HostGate {
        async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome) {
            if message.from.host == self.trusted_host {
                let request = Request {
                    id: self.downstream,
                    payload: message.payload,
                    continuation_token: message.continuation_token,
                    deadline: None,
                };
                (vec![request], Outcome::Continue)
            } else {
                (Vec::new(), Outcome::Continue)
            }
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (Vec::new(), Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn a_reducer_can_authenticate_on_the_origin_host() {
        let mut gate = HostGate {
            trusted_host: hid(b"host-a"),
            downstream: cid(b"downstream"),
        };
        let from_trusted = Message {
            id: cid(b"inbound"),
            payload: Bytes::from_static(b"v"),
            from: Origin {
                reducer: rid(b"peer"),
                host: hid(b"host-a"),
            },
            continuation_token: Bytes::from_static(b"t"),
        };
        // Same sending reducer, different host — the host is what the gate keys on.
        let from_untrusted = Message {
            from: Origin {
                reducer: rid(b"peer"),
                host: hid(b"host-b"),
            },
            ..from_trusted.clone()
        };
        let (out_ok, _) = gate.on_message(from_trusted).await;
        assert_eq!(
            out_ok.len(),
            1,
            "an effect from the trusted host is forwarded"
        );
        let (out_denied, _) = gate.on_message(from_untrusted).await;
        assert!(
            out_denied.is_empty(),
            "an effect from another host is dropped"
        );
    }
}
