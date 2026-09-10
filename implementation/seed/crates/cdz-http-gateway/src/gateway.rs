//! The router + request-serving core (`DESIGN-http-outpost.md` §4, P1c).
//!
//! Above the [`runner`](crate::runner) and below the socket: match an inbound [`HttpRequest`] against a
//! route table to a handler [`ProgramHash`], fold it through the [`HandlerRunner`], and produce the
//! [`HttpResponse`] — synthesizing the host FLOOR responses (a `404` for no route, a `500` for a faulted
//! handler) the design reserves for the edge. The normal error responses are the handler's own business
//! (an `http-response` it returns); these floors are only the last resort.
//!
//! The route table is a STATIC config-seeded table for now (design D5: the P2 stand-in before the
//! control link lands). P2 lifts routing into a governing-program router reducer; P3 ships the table over
//! the control link. This module is the socket-independent core — the tokio/hyper edge (P1c-2) binds it
//! to a listener, and a mock control server (P1c-3) seeds the table for end-to-end tests.

use crate::codec::{Header, HttpRequest, HttpResponse, Method, decode_decision, encode_request};
use crate::runner::HandlerRunner;
use bytes::Bytes;
use cdz_platform::{
    ContractId, HostId, Message, Origin, Outcome, ProgramHash, ProgramStore, ReducerId,
    ReducerKind, SpawnContext,
};
use std::collections::HashMap;
use std::time::Duration;

/// The default per-request wall-clock ceiling (30s): a handler fold that has not produced a response within
/// it is abandoned and answered `504`. The wasm store's epoch deadline traps a CPU-*spinning* fold, but a
/// fold that legitimately YIELDS (an `await` that never resolves — e.g. a handler blocked on a slow host
/// call) burns no epoch and would otherwise hold the connection forever; this bounds that (design §10: the
/// edge bounds foreign work so one request cannot exhaust the node). Tune with
/// [`Gateway::with_request_timeout`].
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One route: an exact `(method, path)` served by the handler component `handler`, which folds the
/// contract `contract` (the contract-id delivered as the request's `Message.id` — different handlers fold
/// different contracts, e.g. an MCP handler vs an HTML one). Path-pattern matching (params, prefixes) is a
/// later slice — v0 is exact-match, mirroring the design's minimal first cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub method: Method,
    pub path: String,
    pub handler: ProgramHash,
    pub contract: ContractId,
}

impl Route {
    /// A route serving `handler` (folding `contract`) at exactly `(method, path)`.
    #[must_use]
    pub fn new(
        method: Method,
        path: impl Into<String>,
        handler: ProgramHash,
        contract: ContractId,
    ) -> Self {
        Self {
            method,
            path: path.into(),
            handler,
            contract,
        }
    }
}

/// The route table: an ordered list of exact `(method, path)` routes. The first match wins (so an earlier
/// route shadows a later duplicate) — deterministic and order-preserving, matching how the control server
/// ships the table.
#[derive(Debug, Clone, Default)]
pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    /// A router over `routes`, first-match-wins.
    #[must_use]
    pub fn new(routes: Vec<Route>) -> Self {
        Self { routes }
    }

    /// Build a router from an `http-route-table` control frame (the payload the control server ships,
    /// `http-route-table.cdz`) — decode it and map each route's `handler`/`contract` bytes to their typed
    /// ids. `None` if the frame is malformed or any `handler`/`contract` is not a valid hash (`Hash::LEN`
    /// bytes).
    #[must_use]
    pub fn from_route_table(frame: &[u8]) -> Option<Router> {
        let routes = crate::codec::decode_route_table(frame)?
            .into_iter()
            .map(|r| {
                Some(Route {
                    method: r.method,
                    path: r.path,
                    handler: ProgramHash::try_from(r.handler.as_ref()).ok()?,
                    contract: ContractId::try_from(r.contract.as_ref()).ok()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Router::new(routes))
    }

    /// The `(handler, contract)` for `(method, path)`, or `None` if no route matches.
    #[must_use]
    pub fn match_route(&self, method: Method, path: &str) -> Option<(ProgramHash, ContractId)> {
        self.routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .map(|r| (r.handler, r.contract))
    }
}

/// Routing lifted into a GOVERNING PROGRAM (`DESIGN-http-outpost.md` §4, P2): instead of matching a
/// `(method, path)` against a static in-process [`Router`] table, the gateway CONSULTS a router-reducer wasm
/// guest (`guests/router/reducer.cdz`) — it folds the request and answers a routing [`RouteDecision`]. This
/// is the host side of that consultation, generic over [`ProgramStore`] (a wasmtime router in production, a
/// native reducer in tests).
///
/// The router's decision names the handler by a stable SYMBOLIC marker (the guest bakes markers, not
/// content hashes it cannot know) plus the contract-id the handler folds; `handlers` binds each marker to
/// the real spawnable [`ProgramHash`] of the deployed handler component (the deployment binding — later the
/// control link ships the markers alongside the handler blobs the CAS resolves). A decision whose handler
/// marker is unbound, or the no-match sentinel (empty handler), routes to `None` (→ the gateway's 404).
pub struct RouterReducer {
    /// The router governing program (a content-addressed wasm reducer in the store).
    program: ProgramHash,
    /// The contract-id delivered as the request's `Message.id` when the router folds it.
    request_contract: ContractId,
    host: HostId,
    /// The `ReducerId` stamped as the request's origin when delivered to the router.
    origin: ReducerId,
    /// Binds each decision handler-marker to the real [`ProgramHash`] the gateway spawns for that route.
    handlers: HashMap<Bytes, ProgramHash>,
}

impl RouterReducer {
    /// A router that consults `program`, delivering requests on `request_contract` from `host`/`origin`, and
    /// binds decision handler-markers to spawnable handler hashes via `handlers`.
    #[must_use]
    pub fn new(
        program: ProgramHash,
        request_contract: ContractId,
        host: HostId,
        origin: ReducerId,
        handlers: HashMap<Bytes, ProgramHash>,
    ) -> Self {
        Self {
            program,
            request_contract,
            host,
            origin,
            handlers,
        }
    }

    /// Consult the router for `(method, path)`: spawn a fresh router instance, deliver the request, read its
    /// closing [`RouteDecision`], and map a matched decision's handler-marker to the bound
    /// `(ProgramHash, ContractId)`. `None` on no route (the no-match sentinel), an unbound handler-marker, a
    /// router that could not instantiate / did not `Break` with a decodable decision, or a malformed
    /// contract-id — every one is a "no usable route" the gateway answers `404`.
    pub async fn match_route(
        &self,
        store: &dyn ProgramStore,
        request_id: &[u8],
        method: Method,
        path: &str,
    ) -> Option<(ProgramHash, ContractId)> {
        let req = HttpRequest {
            method,
            path: path.to_string(),
            query: String::new(),
            headers: vec![],
            body: Bytes::new(),
        };
        let mut router = store
            .spawn(
                self.program,
                SpawnContext {
                    id: ReducerId::of(request_id),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await?;
        let (_requests, outcome) = router
            .on_message(Message {
                id: self.request_contract,
                payload: encode_request(&req),
                from: Origin {
                    reducer: self.origin,
                    host: self.host,
                },
                continuation_token: Bytes::new(),
            })
            .await;
        let Outcome::Break { reason, .. } = outcome else {
            return None; // a router that does not close with a decision routes nowhere
        };
        let decision = decode_decision(&reason)?;
        if !decision.is_match() {
            return None; // the no-match sentinel (empty handler)
        }
        let handler = *self.handlers.get(&decision.handler)?; // unbound marker → no usable route
        let contract = ContractId::try_from(decision.contract.as_ref()).ok()?;
        Some((handler, contract))
    }
}

/// How the gateway resolves a `(method, path)` to a handler: either a STATIC in-process [`Router`] table
/// (v0's config-seeded stand-in) or a [`RouterReducer`] — routing lifted into a governing-program guest (the
/// P2 thesis). The serving path resolves through this uniformly; `Static` ignores the store/request-id.
enum Routing {
    Static(Router),
    Guest(RouterReducer),
}

impl Routing {
    /// Resolve `(method, path)` to `(handler, contract)`. `Static` is a synchronous table lookup; `Guest`
    /// consults the router reducer over `store` (spawning it under `request_id`).
    async fn resolve(
        &self,
        store: &dyn ProgramStore,
        request_id: &[u8],
        method: Method,
        path: &str,
    ) -> Option<(ProgramHash, ContractId)> {
        match self {
            Routing::Static(router) => router.match_route(method, path),
            Routing::Guest(reducer) => reducer.match_route(store, request_id, method, path).await,
        }
    }
}

/// The request-serving core: a routing source (a static [`Router`] table or a [`RouterReducer`] governing
/// program) plus the [`HandlerRunner`] that drives the matched handler.
pub struct Gateway {
    routing: Routing,
    runner: HandlerRunner,
    /// The per-request wall-clock ceiling; a fold exceeding it is answered `504`
    /// ([`DEFAULT_REQUEST_TIMEOUT`] unless overridden).
    request_timeout: Duration,
}

impl Gateway {
    /// A gateway routing through the STATIC `router` table and folding matched requests with `runner`, with
    /// the default per-request timeout.
    #[must_use]
    pub fn new(router: Router, runner: HandlerRunner) -> Self {
        Self {
            routing: Routing::Static(router),
            runner,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// A gateway routing through a [`RouterReducer`] governing program (routing-as-a-fold, P2) — it consults
    /// the router guest per request. Otherwise identical to [`new`](Gateway::new).
    #[must_use]
    pub fn with_router_reducer(router: RouterReducer, runner: HandlerRunner) -> Self {
        Self {
            routing: Routing::Guest(router),
            runner,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }

    /// Set the per-request wall-clock ceiling — a fold that has not produced a response within it is
    /// abandoned and answered `504`.
    #[must_use]
    pub fn with_request_timeout(mut self, request_timeout: Duration) -> Self {
        self.request_timeout = request_timeout;
        self
    }

    /// The `(handler, contract)` a `(method, path)` routes to, or `None` — for the edge to match a
    /// WebSocket-upgrade request's path (the ws frame loop drives the handler as a per-connection session
    /// rather than folding a request→response). Async + store-taking because a [`RouterReducer`] routing
    /// source consults the router guest (a `Static` source ignores both); `request_id` seeds that consult.
    pub async fn match_route(
        &self,
        store: &dyn ProgramStore,
        request_id: &[u8],
        method: Method,
        path: &str,
    ) -> Option<(ProgramHash, ContractId)> {
        self.routing.resolve(store, request_id, method, path).await
    }

    /// Serve one request to a response: match a route (→ `404` floor on no match), fold it through the
    /// handler (→ `500` floor on a [`FoldError`](crate::runner::FoldError), `504` if the fold exceeds the
    /// request timeout), else the handler's response. `request_id` is the unguessable per-request
    /// correlation token (seeds the handler session's id — and the router-consult, for a guest routing
    /// source). Infallible at this layer — every path yields an [`HttpResponse`] (the edge always answers).
    pub async fn serve(
        &self,
        store: &dyn ProgramStore,
        request_id: &[u8],
        req: &HttpRequest,
    ) -> HttpResponse {
        let Some((handler, contract)) = self
            .routing
            .resolve(store, request_id, req.method, &req.path)
            .await
        else {
            return floor(404, "not found");
        };
        let fold = self.runner.fold(store, handler, contract, request_id, req);
        // A fold that never completes (an `await` that never resolves — one the epoch deadline does not
        // trap because it burns no CPU) is abandoned at the ceiling; dropping the future cancels the fold
        // and releases its session, so a stuck handler cannot pin the connection or the node.
        match tokio::time::timeout(self.request_timeout, fold).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(_)) => floor(500, "internal server error"),
            Err(_elapsed) => floor(504, "gateway timeout"),
        }
    }
}

/// A host FLOOR response — a plain-text `status`. The router/handler own the normal error responses; this
/// is only the no-route / faulted-handler last resort.
fn floor(status: u16, message: &'static str) -> HttpResponse {
    HttpResponse {
        status,
        headers: vec![Header {
            name: "content-type".to_string(),
            value: "text/plain; charset=utf-8".to_string(),
        }],
        body: bytes::Bytes::from_static(message.as_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode_response;
    use crate::runner::HandlerRunner;
    use async_trait::async_trait;
    use bytes::Bytes;
    use cdz_platform::testing::program::Store;
    use cdz_platform::{
        ContractId, HostId, Message, Notification, Outcome, Reducer, ReducerId, Request, Response,
    };

    /// A handler that closes with a fixed `200 "ok"`.
    struct OkHandler;
    #[async_trait]
    impl Reducer for OkHandler {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            let resp = HttpResponse {
                status: 200,
                headers: vec![],
                body: Bytes::from_static(b"ok"),
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

    /// A handler that never closes → the gateway must synthesize a `500` floor.
    struct StuckHandler;
    #[async_trait]
    impl Reducer for StuckHandler {
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

    /// A handler whose `on_message` never resolves (awaits forever without burning CPU) → the gateway must
    /// abandon the fold at the request timeout and synthesize a `504` floor. Models a handler blocked on an
    /// `await` the epoch deadline cannot trap.
    struct HangsForever;
    #[async_trait]
    impl Reducer for HangsForever {
        async fn on_message(&mut self, _m: Message) -> (Vec<Request>, Outcome) {
            std::future::pending::<()>().await;
            unreachable!("pending never resolves")
        }
        async fn on_response(&mut self, _r: Response) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<Request>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// A native stand-in for the router governing program: decodes the request and closes with a
    /// [`RouteDecision`](crate::codec::RouteDecision) — `GET /ping` → the `h-ping` marker, `GET /unbound` →
    /// an `h-unbound` marker (deliberately not bound in the test's handler map), anything else → the empty
    /// no-match sentinel. Lets [`RouterReducer`] be exercised without wasm.
    struct NativeRouter;
    #[async_trait]
    impl Reducer for NativeRouter {
        async fn on_message(&mut self, m: Message) -> (Vec<Request>, Outcome) {
            use crate::codec::{RouteDecision, decode_request, encode_decision};
            let decision = match decode_request(&m.payload) {
                Some(req) if req.method == Method::Get && req.path == "/ping" => RouteDecision {
                    handler: Bytes::from_static(b"h-ping"),
                    contract: Bytes::from_static(b"cdz-platform.http.request........"),
                },
                Some(req) if req.method == Method::Get && req.path == "/unbound" => RouteDecision {
                    handler: Bytes::from_static(b"h-unbound"),
                    contract: Bytes::from_static(b"cdz-platform.http.request........"),
                },
                _ => RouteDecision {
                    handler: Bytes::new(),
                    contract: Bytes::new(),
                },
            };
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"cdz-platform.http.route"),
                    reason: encode_decision(&decision),
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

    fn runner() -> HandlerRunner {
        HandlerRunner::new(HostId::of(b"test-host"), ReducerId::of(b"test-router"))
    }

    /// A stand-in per-route contract-id for the serve tests.
    fn a_contract() -> ContractId {
        ContractId::of(b"cdz-platform.http.request")
    }

    fn get(path: &str) -> HttpRequest {
        HttpRequest {
            method: Method::Get,
            path: path.to_string(),
            query: String::new(),
            headers: vec![],
            body: Bytes::new(),
        }
    }

    #[tokio::test]
    async fn a_router_reducer_consults_the_guest_and_binds_the_handler() {
        let router_prog = ProgramHash::of(b"native-router");
        let ping_handler = ProgramHash::of(b"ping-handler");
        let mut store = Store::new();
        store.register(router_prog, || Box::new(NativeRouter));

        let mut handlers = HashMap::new();
        handlers.insert(Bytes::from_static(b"h-ping"), ping_handler);
        let rr = RouterReducer::new(
            router_prog,
            a_contract(),
            HostId::of(b"h"),
            ReducerId::of(b"gw"),
            handlers,
        );

        // A matched route → the bound handler hash + the decision's contract-id.
        let matched = rr.match_route(&store, b"q1", Method::Get, "/ping").await;
        assert_eq!(
            matched,
            Some((
                ping_handler,
                ContractId::try_from(&b"cdz-platform.http.request........"[..]).unwrap()
            )),
            "a matched route binds the marker to the real handler hash + carries the contract-id"
        );

        // The no-match sentinel (empty handler) → None.
        assert!(
            rr.match_route(&store, b"q2", Method::Get, "/nope")
                .await
                .is_none(),
            "the no-match sentinel routes nowhere"
        );

        // A matched decision whose handler-marker is NOT bound in the map → None (no usable route).
        assert!(
            rr.match_route(&store, b"q3", Method::Get, "/unbound")
                .await
                .is_none(),
            "an unbound handler-marker is not a usable route"
        );

        // An unknown router program (cannot instantiate) → None.
        let orphan = RouterReducer::new(
            ProgramHash::of(b"absent-router"),
            a_contract(),
            HostId::of(b"h"),
            ReducerId::of(b"gw"),
            HashMap::new(),
        );
        assert!(
            orphan
                .match_route(&store, b"q4", Method::Get, "/ping")
                .await
                .is_none(),
            "a router that cannot instantiate routes nowhere"
        );
    }

    #[tokio::test]
    async fn a_gateway_backed_by_a_router_reducer_serves_the_routed_handler() {
        // The full P2 serving path with routing lifted into a governing program: the gateway consults the
        // (native stand-in) router reducer, binds its decision to a handler, and folds it — no static table.
        let router_prog = ProgramHash::of(b"native-router");
        let ok_prog = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(router_prog, || Box::new(NativeRouter));
        store.register(ok_prog, || Box::new(OkHandler));

        let mut handlers = HashMap::new();
        handlers.insert(Bytes::from_static(b"h-ping"), ok_prog);
        let rr = RouterReducer::new(
            router_prog,
            a_contract(),
            HostId::of(b"h"),
            ReducerId::of(b"gw"),
            handlers,
        );
        let gw = Gateway::with_router_reducer(rr, runner());

        // GET /ping → the router routes to the ok handler → its 200 "ok".
        let resp = gw.serve(&store, b"req-1", &get("/ping")).await;
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, Bytes::from_static(b"ok"));

        // GET /nope → the router returns the no-match sentinel → the gateway's 404 floor.
        let resp = gw.serve(&store, b"req-2", &get("/nope")).await;
        assert_eq!(resp.status, 404);
    }

    #[tokio::test]
    async fn a_matched_route_serves_the_handler_response() {
        let ok = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(ok, || Box::new(OkHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", ok, a_contract())]),
            runner(),
        );

        let resp = gw.serve(&store, b"req-1", &get("/")).await;
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, Bytes::from_static(b"ok"));
    }

    #[tokio::test]
    async fn an_unmatched_path_is_a_404_floor() {
        let ok = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(ok, || Box::new(OkHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", ok, a_contract())]),
            runner(),
        );

        let resp = gw.serve(&store, b"req-2", &get("/missing")).await;
        assert_eq!(resp.status, 404);
    }

    #[tokio::test]
    async fn a_wrong_method_does_not_match() {
        let ok = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(ok, || Box::new(OkHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", ok, a_contract())]),
            runner(),
        );

        let mut post = get("/");
        post.method = Method::Post;
        let resp = gw.serve(&store, b"req-3", &post).await;
        assert_eq!(resp.status, 404, "a POST must not match a GET route");
    }

    #[tokio::test]
    async fn a_faulted_handler_is_a_500_floor() {
        let stuck = ProgramHash::of(b"stuck-handler");
        let mut store = Store::new();
        store.register(stuck, || Box::new(StuckHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", stuck, a_contract())]),
            runner(),
        );

        let resp = gw.serve(&store, b"req-4", &get("/")).await;
        assert_eq!(resp.status, 500);
    }

    #[tokio::test]
    async fn a_fold_exceeding_the_request_timeout_is_a_504_floor() {
        let hangs = ProgramHash::of(b"hangs-forever");
        let mut store = Store::new();
        store.register(hangs, || Box::new(HangsForever));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", hangs, a_contract())]),
            runner(),
        )
        .with_request_timeout(std::time::Duration::from_millis(50));

        let resp = gw.serve(&store, b"req-timeout", &get("/")).await;
        assert_eq!(
            resp.status, 504,
            "a fold that never resolves is abandoned at the timeout with a 504"
        );
    }

    #[tokio::test]
    async fn a_handler_within_the_timeout_still_serves_its_response() {
        // A generous timeout does not disturb a prompt handler (guards against the timeout floor firing on
        // the happy path).
        let ok = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(ok, || Box::new(OkHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", ok, a_contract())]),
            runner(),
        )
        .with_request_timeout(std::time::Duration::from_secs(5));

        let resp = gw.serve(&store, b"req-ok", &get("/")).await;
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, Bytes::from_static(b"ok"));
    }

    #[test]
    fn builds_a_router_from_a_route_table_frame() {
        use crate::codec::{RouteFrame, encode_route_table};
        let h1 = ProgramHash::of(b"handler-one");
        let h2 = ProgramHash::of(b"handler-two");
        let c1 = ContractId::of(b"contract-1");
        let c2 = ContractId::of(b"contract-2");
        let frame = encode_route_table(&[
            RouteFrame {
                method: Method::Get,
                path: "/".to_string(),
                handler: Bytes::copy_from_slice(h1.hash().as_bytes()),
                contract: Bytes::copy_from_slice(c1.hash().as_bytes()),
            },
            RouteFrame {
                method: Method::Post,
                path: "/mcp".to_string(),
                handler: Bytes::copy_from_slice(h2.hash().as_bytes()),
                contract: Bytes::copy_from_slice(c2.hash().as_bytes()),
            },
        ]);
        let router = Router::from_route_table(&frame).expect("route table builds a router");
        assert_eq!(router.match_route(Method::Get, "/"), Some((h1, c1)));
        assert_eq!(router.match_route(Method::Post, "/mcp"), Some((h2, c2)));
        assert_eq!(router.match_route(Method::Get, "/mcp"), None);
    }

    #[test]
    fn a_route_table_with_a_bad_handler_hash_is_rejected() {
        use crate::codec::{RouteFrame, encode_route_table};
        // A too-short handler is not a valid ProgramHash (Hash::LEN) → the whole frame is rejected.
        let frame = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: Bytes::from_static(b"too-short"),
            contract: Bytes::copy_from_slice(ContractId::of(b"c").hash().as_bytes()),
        }]);
        assert!(Router::from_route_table(&frame).is_none());
    }

    #[test]
    fn a_route_table_with_a_bad_contract_hash_is_rejected() {
        use crate::codec::{RouteFrame, encode_route_table};
        // A valid handler but a too-short contract-id → the whole frame is rejected.
        let frame = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: Bytes::copy_from_slice(ProgramHash::of(b"h").hash().as_bytes()),
            contract: Bytes::from_static(b"too-short"),
        }]);
        assert!(Router::from_route_table(&frame).is_none());
    }

    #[test]
    fn first_matching_route_wins() {
        let a = ProgramHash::of(b"a");
        let b = ProgramHash::of(b"b");
        let ca = ContractId::of(b"ca");
        let router = Router::new(vec![
            Route::new(Method::Get, "/x", a, ca),
            Route::new(Method::Get, "/x", b, ContractId::of(b"cb")),
        ]);
        assert_eq!(router.match_route(Method::Get, "/x"), Some((a, ca)));
        assert_eq!(router.match_route(Method::Get, "/y"), None);
    }
}
