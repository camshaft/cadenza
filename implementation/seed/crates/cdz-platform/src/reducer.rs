//! The reducer interface (`design/cadenza-platform.md` §3).
//!
//! There is one kind of participant: a reducer. Anything that takes part — a session running an agent's
//! task, a handler that answers a single contract, the boundary that performs input and output — is a
//! reducer with the same interface: it receives an event, updates its own state, and emits effect
//! requests. There is no separate executor.
//!
//! Everything a reducer receives is an event carrying a contract-id, and it is one of two kinds, so a
//! reducer has two entry points and the runtime calls the one that fits:
//! - [`on_response`](Reducer::on_response) — the output side: a reply to a request this reducer performed.
//! - [`on_message`](Reducer::on_message) — the input side: an effect another reducer performed on this
//!   one (or a message sent to it), carrying its source so the reducer can authenticate and route on who
//!   sent it.
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

use crate::{Bytes, Hash};
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
        schema: Hash,
        /// The reason value in its canonical encoding.
        reason: Bytes,
    },
}

/// A request a reducer emits: perform the contract `id` with `payload`, and correlate the eventual answer
/// by `continuation_token` (§3/§4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The contract-id: the hash of the schema being requested.
    pub id: Hash,
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
    pub id: Hash,
    /// The token from the originating request — present on both `Ok` and `Err`.
    pub continuation_token: Bytes,
    /// The result: `Ok` the contract's output value (canonical bytes; a handler's *domain* failure rides
    /// here too, since answering with an error is still answering), or `Err` a runtime-level failure.
    pub payload: Result<Bytes, Error>,
}

/// An effect another reducer performed on this one, delivered to [`on_message`](Reducer::on_message). It
/// carries its source (`from`) as envelope metadata so the reducer can authenticate and route on who sent
/// it — the reason `on_message` is distinct from `on_response`. The reducer answers by emitting its reply,
/// correlated by this `continuation_token`, which the runtime routes back to the caller's `on_response`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The contract-id of the effect being performed on this reducer.
    pub id: Hash,
    /// The input value of that effect, in its canonical encoding.
    pub payload: Bytes,
    /// The source reducer's id — envelope metadata to authenticate or route on.
    pub from: Hash,
    /// The caller's token; the reducer's reply is correlated back to the caller by it.
    pub continuation_token: Bytes,
}

/// The one interface every participant implements (§3): receive an event, update state, emit requests.
/// `Send + Sync` so the runtime can share and schedule it. The two entry points return the same product —
/// the requests to perform and the [`Outcome`]. Async because a reduce call may await the reducer's direct
/// accesses (its key-value store, the content store); it never blocks the runtime.
///
/// The methods take `&mut self`: a reducer folds an event into its own state, so it mutates. Each reducer
/// lives in the runtime's registry with a single owner (the event loop drives one reducer at a time), so
/// exclusive access is the natural fit and the trait does not force interior mutability on implementations.
/// An implementation is still free to use interior mutability if it wants.
#[async_trait]
pub trait Reducer: Send + Sync {
    /// React to a [`Response`] — a reply to a request this reducer performed.
    async fn on_response(&mut self, response: Response) -> (Vec<Request>, Outcome);

    /// React to a [`Message`] — an effect performed on this reducer by another, carrying its source.
    async fn on_message(&mut self, message: Message) -> (Vec<Request>, Outcome);
}

#[cfg(test)]
mod tests {
    use super::{Message, Outcome, Reducer, Request, Response};
    use crate::{Bytes, Hash};

    /// A reducer that actually folds state: it counts the messages it has seen (in `&mut self`), forwards
    /// each to a downstream contract carrying the running count, and closes once it has seen `close_at`.
    /// The behavior on any given event depends on the accumulated state, so it exercises real folding
    /// across calls rather than a fixed shape.
    struct Counter {
        seen: u32,
        close_at: u32,
        downstream: Hash,
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
    }

    fn msg(token: &'static [u8]) -> Message {
        Message {
            id: Hash::of(b"inbound"),
            payload: Bytes::from_static(b"x"),
            from: Hash::of(b"peer"),
            continuation_token: Bytes::from_static(token),
        }
    }

    #[tokio::test]
    async fn folds_state_across_calls_and_breaks_at_the_limit() {
        let mut r = Counter {
            seen: 0,
            close_at: 3,
            downstream: Hash::of(b"downstream"),
        };
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
        let mut r = Counter {
            seen: 0,
            close_at: 100,
            downstream: Hash::of(b"downstream"),
        };
        // The reducer routes to its downstream contract and threads the caller's token through, so the
        // eventual answer correlates back — the routing behavior, not the struct shape.
        let (requests, _) = r.on_message(msg(b"correlate-me")).await;
        assert_eq!(requests[0].id, Hash::of(b"downstream"));
        assert_eq!(
            requests[0].continuation_token,
            Bytes::from_static(b"correlate-me")
        );
    }
}
