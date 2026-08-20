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
#[async_trait]
pub trait Reducer: Send + Sync {
    /// React to a [`Response`] — a reply to a request this reducer performed.
    async fn on_response(&self, response: Response) -> (Vec<Request>, Outcome);

    /// React to a [`Message`] — an effect performed on this reducer by another, carrying its source.
    async fn on_message(&self, message: Message) -> (Vec<Request>, Outcome);
}

#[cfg(test)]
mod tests {
    use super::{Error, Message, Outcome, Reducer, Request, Response};
    use crate::{Bytes, Hash};

    /// A tiny reducer exercising the interface: on a message it performs one downstream request (echoing
    /// the payload to a fixed contract) and keeps running; on a response it closes, carrying the answer as
    /// the break reason. Enough to pin the ABI shapes and both entry points.
    struct EchoThenClose {
        downstream: Hash,
    }

    #[async_trait::async_trait]
    impl Reducer for EchoThenClose {
        async fn on_message(&self, message: Message) -> (Vec<Request>, Outcome) {
            let req = Request {
                id: self.downstream,
                payload: message.payload,
                continuation_token: message.continuation_token,
                deadline: None,
            };
            (vec![req], Outcome::Continue)
        }

        async fn on_response(&self, response: Response) -> (Vec<Request>, Outcome) {
            // close, carrying whatever we got back as the reason (its schema is the answered contract-id).
            let reason = response
                .payload
                .unwrap_or_else(|_| Bytes::from_static(b"failed"));
            (
                Vec::new(),
                Outcome::Break {
                    schema: response.id,
                    reason,
                },
            )
        }
    }

    #[tokio::test]
    async fn on_message_emits_a_correlated_request_and_continues() {
        let r = EchoThenClose {
            downstream: Hash::of(b"downstream.contract"),
        };
        let msg = Message {
            id: Hash::of(b"inbound.contract"),
            payload: Bytes::from_static(b"hello"),
            from: Hash::of(b"peer-reducer"),
            continuation_token: Bytes::from_static(b"tok-1"),
        };
        let (requests, outcome) = r.on_message(msg).await;
        assert_eq!(outcome, Outcome::Continue);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, Hash::of(b"downstream.contract"));
        assert_eq!(requests[0].payload, Bytes::from_static(b"hello"));
        // the caller's token is threaded onto the emitted request for correlation.
        assert_eq!(requests[0].continuation_token, Bytes::from_static(b"tok-1"));
        assert_eq!(requests[0].deadline, None);
    }

    #[tokio::test]
    async fn on_response_breaks_with_the_answer_as_reason() {
        let contract = Hash::of(b"downstream.contract");
        let r = EchoThenClose {
            downstream: contract,
        };
        let resp = Response {
            id: contract,
            continuation_token: Bytes::from_static(b"tok-1"),
            payload: Ok(Bytes::from_static(b"world")),
        };
        let (requests, outcome) = r.on_response(resp).await;
        assert!(requests.is_empty());
        assert_eq!(
            outcome,
            Outcome::Break {
                schema: contract,
                reason: Bytes::from_static(b"world"),
            }
        );
    }

    #[tokio::test]
    async fn a_runtime_failure_is_matched_to_its_request_by_token() {
        // Err carries the correlation (id + token) the same as Ok, so a timeout is matched to its request.
        let contract = Hash::of(b"c");
        let r = EchoThenClose {
            downstream: contract,
        };
        let resp = Response {
            id: contract,
            continuation_token: Bytes::from_static(b"tok-9"),
            payload: Err(Error::Timeout),
        };
        // the response still correlates (token present on the error path).
        assert_eq!(resp.continuation_token, Bytes::from_static(b"tok-9"));
        let (_, outcome) = r.on_response(resp).await;
        assert!(matches!(outcome, Outcome::Break { .. }));
    }

    /// The reducer is usable as a trait object (dyn Reducer) — the registry will hold backends this way.
    #[tokio::test]
    async fn reducer_is_dyn_safe() {
        let r: Box<dyn Reducer> = Box::new(EchoThenClose {
            downstream: Hash::of(b"c"),
        });
        let msg = Message {
            id: Hash::of(b"c"),
            payload: Bytes::from_static(b"x"),
            from: Hash::of(b"p"),
            continuation_token: Bytes::from_static(b"t"),
        };
        let (requests, _) = r.on_message(msg).await;
        assert_eq!(requests.len(), 1);
    }
}
