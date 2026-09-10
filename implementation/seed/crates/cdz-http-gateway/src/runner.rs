//! The per-request handler runner (`DESIGN-http-outpost.md` §4, P1b).
//!
//! The spine minus the socket: given a handler's [`ProgramHash`] and an [`HttpRequest`], instantiate a
//! fresh per-request reducer session, deliver the request as its first `on_message`, and read the
//! `http-response` off the session's closing `Break`. Generic over [`ProgramStore`] — the standalone
//! gateway supplies the wasmtime-backed `cdz_platform::WasmProgramStore` (behind `cdz-platform`'s `host`
//! feature; wired in a later slice), while tests supply the native `cdz_platform::testing::program::Store`,
//! so the runner's plumbing + the codec are provable end-to-end without wasmtime or a compiled guest.
//!
//! Per-request isolation is the store's job: a fresh instance per `spawn` with its own `state`/`blobs`
//! (design §4). Resource bounding (the epoch deadline + memory ceiling the wasm store arms) and the 404/500
//! floors are later slices (P1d).

use crate::codec::{HttpRequest, HttpResponse, decode_response, encode_request};
use bytes::Bytes;
use cdz_platform::{
    ContractId, HostId, Message, Origin, Outcome, ProgramHash, ProgramStore, ReducerId,
    ReducerKind, SpawnContext,
};
use std::fmt;

/// Why folding a request through a handler failed. Not itself an HTTP status — the caller (the edge)
/// maps it to a floor response (a `500`); a handler's own error is a normal `http-response` it returns,
/// not one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// The store could not instantiate the program — an unknown/unregistered handler hash (a routing
    /// misconfiguration).
    UnknownProgram,
    /// The handler returned `Outcome::Continue` rather than closing with a response. A one-shot HTTP
    /// handler is expected to `Break` with its `http-response`; a looping program went quiescent with no
    /// pending effect and never produced one.
    HandlerDidNotClose,
    /// The handler closed, but its `Break` reason did not decode as a valid `http-response`.
    MalformedResponse,
    /// A looping program kept emitting effects past the drive-loop ceiling without ever closing — a
    /// runaway. Bounded so a misbehaving program cannot spin the driver forever (only reachable via the
    /// looping [`RootDriver`](crate::root_driver::RootDriver), not the one-shot `fold`).
    Runaway,
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            FoldError::UnknownProgram => "handler program is not instantiable (unknown hash)",
            FoldError::HandlerDidNotClose => "handler did not close with a response",
            FoldError::MalformedResponse => "handler's close reason is not a valid http-response",
            FoldError::Runaway => "program exceeded the drive-loop ceiling without closing",
        };
        f.write_str(s)
    }
}

impl std::error::Error for FoldError {}

/// Folds each inbound [`HttpRequest`] through a per-request handler session.
///
/// Holds the envelope metadata every delivered request carries: the `host` (the node running the gateway)
/// and `router` (the sender) stamped as the request's [`Origin`]. The delivered message's contract-id is
/// PER-ROUTE (passed to [`fold`](HandlerRunner::fold) from the matched route), so one runner serves every
/// route. `host`/`router` are configuration, not per-request state.
pub struct HandlerRunner {
    host: HostId,
    router: ReducerId,
}

impl HandlerRunner {
    /// A runner stamping requests with `host`/`router` as their [`Origin`].
    #[must_use]
    pub fn new(host: HostId, router: ReducerId) -> Self {
        Self { host, router }
    }

    /// Instantiate a fresh session of `program`, deliver `req` as its `on_message` (with contract-id
    /// `request_contract`, the matched route's), and return the `http-response` it closes with.
    /// `request_id` is the unguessable per-request correlation token — it seeds the session's
    /// [`ReducerId`], so each request gets a distinct instance with its own state.
    ///
    /// # Errors
    /// [`FoldError::UnknownProgram`] if the store cannot instantiate `program`;
    /// [`FoldError::HandlerDidNotClose`] if the handler did not `Break`;
    /// [`FoldError::MalformedResponse`] if the close reason is not a valid `http-response`.
    pub async fn fold(
        &self,
        store: &dyn ProgramStore,
        program: ProgramHash,
        request_contract: ContractId,
        request_id: &[u8],
        req: &HttpRequest,
    ) -> Result<HttpResponse, FoldError> {
        let mut reducer = store
            .spawn(
                program,
                SpawnContext {
                    id: ReducerId::of(request_id),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await
            .ok_or(FoldError::UnknownProgram)?;

        let (_requests, outcome) = reducer
            .on_message(Message {
                id: request_contract,
                payload: encode_request(req),
                from: Origin {
                    reducer: self.router,
                    host: self.host,
                },
                continuation_token: Bytes::new(),
            })
            .await;

        match outcome {
            Outcome::Break { reason, .. } => {
                decode_response(&reason).ok_or(FoldError::MalformedResponse)
            }
            Outcome::Continue => Err(FoldError::HandlerDidNotClose),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Header, HttpResponse, Method, encode_response};
    use async_trait::async_trait;
    use cdz_platform::testing::program::Store;
    use cdz_platform::{Notification, Reducer, Request, Response};

    fn runner() -> HandlerRunner {
        HandlerRunner::new(HostId::of(b"test-host"), ReducerId::of(b"test-router"))
    }

    /// A stand-in request contract-id for the fold tests (the runner delivers it as the Message.id; these
    /// handlers decode by payload, so the exact id is immaterial).
    fn a_contract() -> ContractId {
        ContractId::of(b"cdz-platform.http.request")
    }

    fn a_request() -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            path: "/hello".to_string(),
            query: String::new(),
            headers: vec![],
            body: Bytes::from_static(b"ping"),
        }
    }

    /// A native handler that decodes the delivered `http-request` and closes with a `200` that echoes the
    /// path (as a header) and the body — the full runner→encode→handler→decode→encode→runner→decode path.
    struct EchoHandler;
    #[async_trait]
    impl Reducer for EchoHandler {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let resp = match crate::codec::decode_request(&m.payload) {
                Some(req) => HttpResponse {
                    status: 200,
                    headers: vec![Header {
                        name: "x-echo-path".to_string(),
                        value: req.path,
                    }],
                    body: req.body,
                },
                None => HttpResponse {
                    status: 400,
                    headers: vec![],
                    body: Bytes::from_static(b"bad request"),
                },
            };
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"cdz-platform.http.response"),
                    reason: encode_response(&resp),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A handler that never closes — exercises [`FoldError::HandlerDidNotClose`].
    struct NeverCloses;
    #[async_trait]
    impl Reducer for NeverCloses {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A handler that closes with a garbage reason — exercises [`FoldError::MalformedResponse`].
    struct GarbageReason;
    #[async_trait]
    impl Reducer for GarbageReason {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"nonsense"),
                    reason: Bytes::from_static(b"not a valid response value"),
                },
            )
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    #[tokio::test]
    async fn folds_a_request_through_a_handler() {
        let mut store = Store::new();
        let program = ProgramHash::of(b"echo-handler");
        store.register(program, || Box::new(EchoHandler));

        let resp = runner()
            .fold(&store, program, a_contract(), b"req-1", &a_request())
            .await
            .expect("handler responds");

        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, Bytes::from_static(b"ping"));
        assert_eq!(resp.headers.len(), 1);
        assert_eq!(resp.headers[0].name, "x-echo-path");
        assert_eq!(resp.headers[0].value, "/hello");
    }

    #[tokio::test]
    async fn unknown_program_is_an_error() {
        let store = Store::new(); // nothing registered
        let err = runner()
            .fold(
                &store,
                ProgramHash::of(b"absent"),
                a_contract(),
                b"req-2",
                &a_request(),
            )
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::UnknownProgram);
    }

    #[tokio::test]
    async fn a_handler_that_never_closes_is_an_error() {
        let mut store = Store::new();
        let program = ProgramHash::of(b"never");
        store.register(program, || Box::new(NeverCloses));
        let err = runner()
            .fold(&store, program, a_contract(), b"req-3", &a_request())
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::HandlerDidNotClose);
    }

    #[tokio::test]
    async fn a_malformed_close_reason_is_an_error() {
        let mut store = Store::new();
        let program = ProgramHash::of(b"garbage");
        store.register(program, || Box::new(GarbageReason));
        let err = runner()
            .fold(&store, program, a_contract(), b"req-4", &a_request())
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::MalformedResponse);
    }

    /// Each `fold` spawns a FRESH instance (per-request isolation): two requests to the same program
    /// through the native store get independent reducers, so nothing leaks between them.
    #[tokio::test]
    async fn each_request_gets_a_fresh_session() {
        let mut store = Store::new();
        let program = ProgramHash::of(b"echo-handler");
        store.register(program, || Box::new(EchoHandler));

        let mut first = a_request();
        first.path = "/a".to_string();
        let mut second = a_request();
        second.path = "/b".to_string();

        let r1 = runner()
            .fold(&store, program, a_contract(), b"r1", &first)
            .await
            .unwrap();
        let r2 = runner()
            .fold(&store, program, a_contract(), b"r2", &second)
            .await
            .unwrap();
        assert_eq!(r1.headers[0].value, "/a");
        assert_eq!(r2.headers[0].value, "/b");
    }
}
