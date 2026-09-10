//! The per-request root-router drive (`DESIGN-http-outpost-drive-contract.md` §1, redirect inc-3c).
//!
//! The operator redirect makes the gateway DUMB: it holds no route table and no handler map. Per request it
//! spawns the ROOT ROUTER program (fetched from the CAS by the control-configured hash — the store's job),
//! DRIVES ITS LOOP (`drive_loop`), and reads the `http-response` off the terminal `Break`. The router does
//! all routing internally and reaches its answer by emitting effects the gateway resolves — chiefly
//! `dispatch` (spawn + drive a subprogram, e.g. the matched handler, and fold its response back) and
//! `control.send` (talk to the control server). This is the looping replacement for the one-shot
//! [`HandlerRunner`](crate::runner::HandlerRunner): where the runner folded ONE `on_message` and expected a
//! `Break`, the [`RootDriver`] folds a whole loop of effects/answers until the router closes.
//!
//! Socket/CAS-independent: generic over any [`ProgramStore`] (the standalone gateway supplies the
//! wasmtime-backed `WasmProgramStore` over the HTTP CAS; tests supply the native
//! `cdz_platform::testing::program::Store`), so the drive + the codec are provable end-to-end without
//! wasmtime or a compiled guest. Writing the response to the socket and routing `ControlDown` pushes to
//! sessions are the edge slices that consume this driver.

use crate::codec::{
    Header, HttpRequest, HttpResponse, decode_deny, decode_response, deny_contract, encode_request,
};
use crate::effects::{ControlSink, GatewayResolver};
use crate::loop_driver::{DriveEnd, drive_loop};
use crate::runner::FoldError;
use bytes::Bytes;
use cdz_platform::{
    ContractId, HostId, Message, Origin, ProgramHash, ProgramStore, ReducerId, ReducerKind,
    SpawnContext,
};
use std::sync::Arc;

/// The stable [`ReducerId`] the gateway edge stamps as the `Origin.reducer` of the request it delivers to a
/// root router — the router's messages come FROM the edge (there is no upstream reducer above the root).
const EDGE_REDUCER: &[u8] = b"cdz-http-gateway-edge";

/// A dispatch chain (root router → handler → …) is bounded so a misconfigured router cannot recurse forever.
const DEFAULT_MAX_DISPATCH_DEPTH: usize = 8;
/// The drive-loop fold ceiling: a router that keeps emitting effects without closing is a runaway
/// ([`FoldError::Runaway`]); bounded so it cannot spin the driver forever.
const DEFAULT_MAX_FOLDS: usize = 4096;

/// Drives the root router for each inbound [`HttpRequest`], one fresh session per request.
///
/// Holds only configuration: the `host` (the node running the gateway, stamped as the delivered request's
/// [`Origin`]), the contract-id the `http-request` is delivered under (the router's first `Message.id`), the
/// contract-id a dispatched subprogram receives its input under, and the recursion/fold budgets. The root
/// router hash + the CAS-backed store are supplied per `serve` (they change as the control server pushes a
/// new root-router hash / the CAS is reconfigured).
pub struct RootDriver {
    host: HostId,
    request_contract: ContractId,
    dispatch_request_contract: ContractId,
    max_depth: usize,
    max_folds: usize,
}

impl RootDriver {
    /// A driver stamping delivered requests with `host`, delivering the `http-request` under
    /// `request_contract` and a dispatched subprogram's input under `dispatch_request_contract`. Uses the
    /// default recursion/fold budgets (tune with [`with_budgets`](Self::with_budgets)).
    #[must_use]
    pub fn new(
        host: HostId,
        request_contract: ContractId,
        dispatch_request_contract: ContractId,
    ) -> Self {
        Self {
            host,
            request_contract,
            dispatch_request_contract,
            max_depth: DEFAULT_MAX_DISPATCH_DEPTH,
            max_folds: DEFAULT_MAX_FOLDS,
        }
    }

    /// Override the dispatch-recursion depth and drive-loop fold ceiling.
    #[must_use]
    pub fn with_budgets(mut self, max_depth: usize, max_folds: usize) -> Self {
        self.max_depth = max_depth;
        self.max_folds = max_folds;
        self
    }

    /// Spawn a fresh session of `root_router` from `store`, deliver `req` as its `on_message`, drive its loop
    /// — resolving each effect it emits with a [`GatewayResolver`] (dispatch enabled over `store`, control
    /// messages forwarded to `control`) — and return the `http-response` it closes with. `session` is the
    /// per-connection correlation id: it seeds the router's [`ReducerId`] (a distinct instance with its own
    /// state per connection) and stamps a `control.send`'s provenance.
    ///
    /// # Errors
    /// [`FoldError::UnknownProgram`] if the store cannot instantiate `root_router`;
    /// [`FoldError::HandlerDidNotClose`] if the router went quiescent without an answer;
    /// [`FoldError::Runaway`] if it exceeded the fold ceiling without closing;
    /// [`FoldError::MalformedResponse`] if its close reason is not a valid `http-response`.
    pub async fn serve(
        &self,
        store: Arc<dyn ProgramStore>,
        root_router: ProgramHash,
        session: &[u8],
        control: Arc<dyn ControlSink>,
        req: &HttpRequest,
    ) -> Result<HttpResponse, FoldError> {
        let mut router = store
            .spawn(
                root_router,
                SpawnContext {
                    id: ReducerId::of(session),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await
            .ok_or(FoldError::UnknownProgram)?;

        let resolver = GatewayResolver::new(
            Bytes::copy_from_slice(root_router.hash().as_bytes()),
            Bytes::copy_from_slice(session),
            control,
        )
        .with_dispatch(
            Arc::clone(&store),
            self.dispatch_request_contract,
            self.max_depth,
        );

        let first = Message {
            id: self.request_contract,
            payload: encode_request(req),
            from: Origin {
                reducer: ReducerId::of(EDGE_REDUCER),
                host: self.host,
            },
            continuation_token: Bytes::new(),
        };

        match drive_loop(&mut *router, first, &resolver, self.max_folds).await {
            // A `deny` terminal (design §2) rejects the request → a plain-text status floor; any other
            // close reason is decoded as an `http-response` (the schema stays response-agnostic so a guest
            // Break-ing with an http-response under any schema still serves — only `deny` is special-cased).
            Ok((schema, reason)) if schema == deny_contract() => decode_deny(&reason)
                .map(|d| deny_response(&d))
                .ok_or(FoldError::MalformedResponse),
            Ok((_schema, reason)) => decode_response(&reason).ok_or(FoldError::MalformedResponse),
            Err(DriveEnd::Quiescent) => Err(FoldError::HandlerDidNotClose),
            Err(DriveEnd::FoldLimit) => Err(FoldError::Runaway),
        }
    }
}

/// Render a [`Deny`](crate::codec::Deny) terminal as a plain-text [`HttpResponse`] the edge serves verbatim.
fn deny_response(deny: &crate::codec::Deny) -> HttpResponse {
    HttpResponse {
        status: deny.status,
        headers: vec![Header {
            name: "content-type".to_string(),
            value: "text/plain; charset=utf-8".to_string(),
        }],
        body: deny.reason.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{DispatchEffect, Header, Method, encode_dispatch, encode_response};
    use crate::effects::dispatch_contract;
    use async_trait::async_trait;
    use cdz_platform::testing::program::Store;
    use cdz_platform::{Notification, Outcome, Reducer, Request, Response};
    use std::sync::Mutex;

    /// A no-op [`ControlSink`] (the drive tests do not exercise `control.send`).
    #[derive(Default)]
    struct NullSink(Mutex<Vec<crate::codec::ControlUp>>);
    #[async_trait]
    impl ControlSink for NullSink {
        async fn send(&self, msg: crate::codec::ControlUp) {
            self.0.lock().expect("sink lock").push(msg);
        }
    }

    fn driver() -> RootDriver {
        RootDriver::new(
            HostId::of(b"test-host"),
            ContractId::of(b"cdz.http.request"),
            ContractId::of(b"cdz.http.request"),
        )
    }

    fn a_request(path: &str) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            path: path.to_string(),
            query: String::new(),
            headers: vec![],
            body: Bytes::from_static(b"ping"),
        }
    }

    fn ok_response(path: String) -> Bytes {
        encode_response(&HttpResponse {
            status: 200,
            headers: vec![Header {
                name: "x-path".to_string(),
                value: path,
            }],
            body: Bytes::from_static(b"ok"),
        })
    }

    /// A root router that answers the request DIRECTLY (no dispatch): decode it, Break with a 200 echoing
    /// the path. The simplest looping-program shape — one fold, then close.
    struct DirectRouter;
    #[async_trait]
    impl Reducer for DirectRouter {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            let path = crate::codec::decode_request(&m.payload)
                .map(|r| r.path)
                .unwrap_or_default();
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"cdz.http.response"),
                    reason: ok_response(path),
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
    async fn drives_a_direct_answering_root_router() {
        let mut store = Store::new();
        let router = ProgramHash::of(b"direct-router");
        store.register(router, || Box::new(DirectRouter));
        let store: Arc<dyn ProgramStore> = Arc::new(store);

        let resp = driver()
            .serve(
                store,
                router,
                b"sess-1",
                Arc::new(NullSink::default()),
                &a_request("/hi"),
            )
            .await
            .expect("router answers");
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers[0].value, "/hi");
    }

    #[tokio::test]
    async fn the_root_router_dispatches_to_a_handler_and_folds_its_response() {
        // The handler: decode the dispatched request, Break with a 200 echoing its path.
        struct Handler;
        #[async_trait]
        impl Reducer for Handler {
            async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
                let path = crate::codec::decode_request(&m.payload)
                    .map(|r| r.path)
                    .unwrap_or_default();
                (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"cdz.http.response"),
                        reason: ok_response(path),
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
        // The root router: dispatch the request (verbatim) to the handler, fold its response, Break with it.
        struct DispatchRouter {
            handler: Bytes,
        }
        #[async_trait]
        impl Reducer for DispatchRouter {
            async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
                (
                    vec![Request {
                        id: dispatch_contract(),
                        payload: encode_dispatch(&DispatchEffect {
                            subprogram: self.handler.clone(),
                            input: m.payload,
                        }),
                        continuation_token: Bytes::from_static(b"d1"),
                        deadline: None,
                    }],
                    Outcome::Continue,
                )
            }
            async fn on_response(&mut self, r: Response) -> (Vec<Request>, Outcome) {
                (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"cdz.http.response"),
                        reason: r.payload.unwrap_or_default(),
                    },
                )
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }

        let handler_hash = ProgramHash::of(b"the-handler");
        let mut store = Store::new();
        store.register(handler_hash, || Box::new(Handler));
        let router_hash = ProgramHash::of(b"dispatch-router");
        let handler_bytes = Bytes::copy_from_slice(handler_hash.hash().as_bytes());
        store.register(router_hash, move || {
            Box::new(DispatchRouter {
                handler: handler_bytes.clone(),
            })
        });
        let store: Arc<dyn ProgramStore> = Arc::new(store);

        let resp = driver()
            .serve(
                store,
                router_hash,
                b"sess-2",
                Arc::new(NullSink::default()),
                &a_request("/routed"),
            )
            .await
            .expect("router dispatches + folds the handler response");
        assert_eq!(resp.status, 200);
        assert_eq!(
            resp.headers[0].value, "/routed",
            "the handler saw the dispatched request and its response folded back through the router"
        );
    }

    #[tokio::test]
    async fn a_deny_terminal_becomes_a_status_floor_response() {
        use crate::codec::{Deny, deny_contract, encode_deny};
        // A root router that REJECTS the request: Break with a `deny` (403) instead of an http-response.
        struct Denier;
        #[async_trait]
        impl Reducer for Denier {
            async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
                (
                    vec![],
                    Outcome::Break {
                        schema: deny_contract(),
                        reason: encode_deny(&Deny {
                            status: 403,
                            reason: Bytes::from_static(b"forbidden"),
                        }),
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
        let mut store = Store::new();
        let router = ProgramHash::of(b"denier");
        store.register(router, || Box::new(Denier));
        let store: Arc<dyn ProgramStore> = Arc::new(store);

        let resp = driver()
            .serve(
                store,
                router,
                b"s",
                Arc::new(NullSink::default()),
                &a_request("/secret"),
            )
            .await
            .expect("a deny maps to a floor response, not an error");
        assert_eq!(resp.status, 403);
        assert_eq!(resp.body, Bytes::from_static(b"forbidden"));
    }

    #[tokio::test]
    async fn an_unknown_root_router_is_unknown_program() {
        let store: Arc<dyn ProgramStore> = Arc::new(Store::new());
        let err = driver()
            .serve(
                store,
                ProgramHash::of(b"absent"),
                b"s",
                Arc::new(NullSink::default()),
                &a_request("/"),
            )
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::UnknownProgram);
    }

    #[tokio::test]
    async fn a_router_that_goes_quiescent_did_not_close() {
        // A router that neither closes nor emits an effect goes quiescent — no answer.
        struct Quiet;
        #[async_trait]
        impl Reducer for Quiet {
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
        let mut store = Store::new();
        let router = ProgramHash::of(b"quiet");
        store.register(router, || Box::new(Quiet));
        let store: Arc<dyn ProgramStore> = Arc::new(store);
        let err = driver()
            .serve(
                store,
                router,
                b"s",
                Arc::new(NullSink::default()),
                &a_request("/"),
            )
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::HandlerDidNotClose);
    }

    #[tokio::test]
    async fn a_malformed_close_reason_is_malformed_response() {
        struct Garbage;
        #[async_trait]
        impl Reducer for Garbage {
            async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
                (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"nonsense"),
                        reason: Bytes::from_static(b"not a valid http-response value"),
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
        let mut store = Store::new();
        let router = ProgramHash::of(b"garbage");
        store.register(router, || Box::new(Garbage));
        let store: Arc<dyn ProgramStore> = Arc::new(store);
        let err = driver()
            .serve(
                store,
                router,
                b"s",
                Arc::new(NullSink::default()),
                &a_request("/"),
            )
            .await
            .unwrap_err();
        assert_eq!(err, FoldError::MalformedResponse);
    }
}
