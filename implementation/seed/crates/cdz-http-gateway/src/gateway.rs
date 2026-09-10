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

use crate::codec::{Header, HttpRequest, HttpResponse, Method};
use crate::runner::HandlerRunner;
use cdz_platform::{ProgramHash, ProgramStore};

/// One route: an exact `(method, path)` served by the handler component `handler`. Path-pattern matching
/// (params, prefixes) is a later slice — v0 is exact-match, mirroring the design's minimal first cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub method: Method,
    pub path: String,
    pub handler: ProgramHash,
}

impl Route {
    /// A route serving `handler` at exactly `(method, path)`.
    #[must_use]
    pub fn new(method: Method, path: impl Into<String>, handler: ProgramHash) -> Self {
        Self {
            method,
            path: path.into(),
            handler,
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
    /// `http-route-table.cdz`) — decode it and map each route's `handler` bytes to a [`ProgramHash`].
    /// `None` if the frame is malformed or any `handler` is not a valid hash (`Hash::LEN` bytes). The
    /// per-route `contract` id is not yet used by the router (the runner folds a fixed request contract-id
    /// for now); wiring per-route contracts is a follow-up.
    #[must_use]
    pub fn from_route_table(frame: &[u8]) -> Option<Router> {
        let routes = crate::codec::decode_route_table(frame)?
            .into_iter()
            .map(|r| {
                Some(Route {
                    method: r.method,
                    path: r.path,
                    handler: ProgramHash::try_from(r.handler.as_ref()).ok()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Router::new(routes))
    }

    /// The handler for `(method, path)`, or `None` if no route matches.
    #[must_use]
    pub fn match_route(&self, method: Method, path: &str) -> Option<ProgramHash> {
        self.routes
            .iter()
            .find(|r| r.method == method && r.path == path)
            .map(|r| r.handler)
    }
}

/// The request-serving core: a [`Router`] plus the [`HandlerRunner`] that drives the matched handler.
pub struct Gateway {
    router: Router,
    runner: HandlerRunner,
}

impl Gateway {
    /// A gateway routing through `router` and folding matched requests with `runner`.
    #[must_use]
    pub fn new(router: Router, runner: HandlerRunner) -> Self {
        Self { router, runner }
    }

    /// Serve one request to a response: match a route (→ `404` floor on no match), fold it through the
    /// handler (→ `500` floor on a [`FoldError`](crate::runner::FoldError)), else the handler's response.
    /// `request_id` is the unguessable per-request correlation token (seeds the handler session's id).
    /// Infallible at this layer — every path yields an [`HttpResponse`] (the edge always answers the socket).
    pub async fn serve(
        &self,
        store: &dyn ProgramStore,
        request_id: &[u8],
        req: &HttpRequest,
    ) -> HttpResponse {
        let Some(handler) = self.router.match_route(req.method, &req.path) else {
            return floor(404, "not found");
        };
        match self.runner.fold(store, handler, request_id, req).await {
            Ok(resp) => resp,
            Err(_) => floor(500, "internal server error"),
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

    fn runner() -> HandlerRunner {
        HandlerRunner::new(
            HostId::of(b"test-host"),
            ReducerId::of(b"test-router"),
            ContractId::of(b"cdz-platform.http.request"),
        )
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
    async fn a_matched_route_serves_the_handler_response() {
        let ok = ProgramHash::of(b"ok-handler");
        let mut store = Store::new();
        store.register(ok, || Box::new(OkHandler));
        let gw = Gateway::new(
            Router::new(vec![Route::new(Method::Get, "/", ok)]),
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
            Router::new(vec![Route::new(Method::Get, "/", ok)]),
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
            Router::new(vec![Route::new(Method::Get, "/", ok)]),
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
            Router::new(vec![Route::new(Method::Get, "/", stuck)]),
            runner(),
        );

        let resp = gw.serve(&store, b"req-4", &get("/")).await;
        assert_eq!(resp.status, 500);
    }

    #[test]
    fn builds_a_router_from_a_route_table_frame() {
        use crate::codec::{RouteFrame, encode_route_table};
        let h1 = ProgramHash::of(b"handler-one");
        let h2 = ProgramHash::of(b"handler-two");
        let frame = encode_route_table(&[
            RouteFrame {
                method: Method::Get,
                path: "/".to_string(),
                handler: Bytes::copy_from_slice(h1.hash().as_bytes()),
                contract: Bytes::from_static(b"contract-1"),
            },
            RouteFrame {
                method: Method::Post,
                path: "/mcp".to_string(),
                handler: Bytes::copy_from_slice(h2.hash().as_bytes()),
                contract: Bytes::from_static(b"contract-2"),
            },
        ]);
        let router = Router::from_route_table(&frame).expect("route table builds a router");
        assert_eq!(router.match_route(Method::Get, "/"), Some(h1));
        assert_eq!(router.match_route(Method::Post, "/mcp"), Some(h2));
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
            contract: Bytes::from_static(b"c"),
        }]);
        assert!(Router::from_route_table(&frame).is_none());
    }

    #[test]
    fn first_matching_route_wins() {
        let a = ProgramHash::of(b"a");
        let b = ProgramHash::of(b"b");
        let router = Router::new(vec![
            Route::new(Method::Get, "/x", a),
            Route::new(Method::Get, "/x", b),
        ]);
        assert_eq!(router.match_route(Method::Get, "/x"), Some(a));
        assert_eq!(router.match_route(Method::Get, "/y"), None);
    }
}
