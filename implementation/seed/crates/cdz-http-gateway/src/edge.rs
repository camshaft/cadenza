//! The native HTTP edge (`DESIGN-http-outpost.md` §2, P1c-2).
//!
//! The one piece of native host plumbing: bind a TCP port, parse each inbound HTTP request into an
//! [`HttpRequest`], hand it to the [`Gateway`] (route → fold → response), and serialize the
//! [`HttpResponse`] back to the socket. It speaks NO routing, auth, or HTTP semantics beyond
//! parse/serialize — the reviewer bar from the federation doc ("does the host decide anything above the
//! socket? — it must not"). All policy is the gateway/handler fold.
//!
//! v0 is a `hyper` HTTP/1 server. A future slice may move the edge itself into a `wasi:http` reducer
//! (design D3); the handler contract (§5) is unaffected either way.

use crate::codec::{Header, HttpRequest, HttpResponse, Method};
use crate::effects::ControlSink;
use crate::gateway::Gateway;
use crate::root_driver::RootDriver;
use bytes::Bytes;
use cdz_platform::{ProgramHash, ProgramStore};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;

/// The default per-request body-size ceiling (1 MiB) — the edge reads at most this many bytes of a request
/// body before answering `413`, so a large/unbounded upload cannot exhaust the node's memory (design §10:
/// the edge bounds foreign input; a per-request fold cannot exhaust the node). Tune with
/// [`HttpEdge::with_max_body_bytes`].
pub const DEFAULT_MAX_BODY_BYTES: usize = 1 << 20;

/// The per-request wall-clock ceiling for the DUMB serving path (design §10): a root-router drive that never
/// resolves (an effect future that never lands — one the drive-loop fold ceiling does not catch because it
/// burns no folds) is abandoned here, so a stuck request cannot pin the connection or the node. The legacy
/// [`Gateway`] path carries its own timeout ([`Gateway::with_request_timeout`]).
pub const DEFAULT_DUMB_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How the edge serves an ordinary (non-ws) HTTP request.
enum HttpServe {
    /// The legacy model: a [`Gateway`] matches a route table / router guest and folds one handler
    /// (`Router`/`RouterReducer`/`DynamicRouter`). Retired once the dumb path fully subsumes it.
    Gateway(Gateway),
    /// The DUMB model (`DESIGN-http-outpost-drive-contract.md`, redirect inc-3c): the edge holds no route
    /// table — per request it drives the control-configured ROOT ROUTER, which routes internally by emitting
    /// effects. Holds the [`RootDriver`], the root-router [`ProgramHash`], the wall-clock timeout, and the
    /// [`ControlSink`] a `control.send` effect forwards to.
    Dumb {
        driver: RootDriver,
        root_router: ProgramHash,
        request_timeout: Duration,
        control: Arc<dyn ControlSink>,
    },
}

/// The HTTP edge: an [`HttpServe`] routing strategy + the [`ProgramStore`] its programs instantiate from,
/// bound to a listener. `Send + Sync` (behind an `Arc`) so each accepted connection is served on its own task.
pub struct HttpEdge {
    serve: HttpServe,
    store: Arc<dyn ProgramStore>,
    /// A monotonic per-request counter seeding the correlation/session id. v0: a counter (distinct per
    /// request is all the runner needs to give each session a fresh id); the unguessable-token scheme the
    /// design reserves arrives with the in-platform trust model (auth is an external proxy in v0, D1).
    next_id: AtomicU64,
    /// The per-request body-size ceiling (bytes); a body exceeding it is answered `413` before it reaches a
    /// handler ([`DEFAULT_MAX_BODY_BYTES`] unless overridden).
    max_body_bytes: usize,
}

impl HttpEdge {
    /// An edge serving `gateway` over programs instantiated from `store` (the legacy routing model), with the
    /// default body-size limit.
    #[must_use]
    pub fn new(gateway: Gateway, store: Arc<dyn ProgramStore>) -> Self {
        Self {
            serve: HttpServe::Gateway(gateway),
            store,
            next_id: AtomicU64::new(0),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// A DUMB edge (redirect inc-3c): per request it drives `root_router` from `store` via `driver`,
    /// forwarding a `control.send` effect to `control`. Holds no route table — the root router routes
    /// internally. Uses the default body-size limit + [`DEFAULT_DUMB_REQUEST_TIMEOUT`]. WebSocket upgrades are
    /// floored `404` in this mode until the dumb ws path lands (a later slice).
    #[must_use]
    pub fn dumb(
        driver: RootDriver,
        root_router: ProgramHash,
        control: Arc<dyn ControlSink>,
        store: Arc<dyn ProgramStore>,
    ) -> Self {
        Self {
            serve: HttpServe::Dumb {
                driver,
                root_router,
                request_timeout: DEFAULT_DUMB_REQUEST_TIMEOUT,
                control,
            },
            store,
            next_id: AtomicU64::new(0),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    /// Set the DUMB path's per-request wall-clock timeout (a drive that does not resolve within it is
    /// abandoned → `504`). No-op on a legacy [`Gateway`]-mode edge (that path carries its own timeout).
    #[must_use]
    pub fn with_dumb_request_timeout(mut self, timeout: Duration) -> Self {
        if let HttpServe::Dumb {
            request_timeout, ..
        } = &mut self.serve
        {
            *request_timeout = timeout;
        }
        self
    }

    /// Set the per-request body-size ceiling (bytes) — a request body larger than this is answered `413`.
    #[must_use]
    pub fn with_max_body_bytes(mut self, max_body_bytes: usize) -> Self {
        self.max_body_bytes = max_body_bytes;
        self
    }

    /// Accept connections on `listener` forever, serving each on its own task. Returns only on an accept
    /// error (the caller owns the listener lifecycle).
    ///
    /// # Errors
    /// Propagates a fatal [`std::io::Error`] from `TcpListener::accept`.
    pub async fn serve(self: Arc<Self>, listener: TcpListener) -> std::io::Result<()> {
        loop {
            let (stream, _peer) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let edge = Arc::clone(&self);
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let edge = Arc::clone(&edge);
                    async move { Ok::<_, Infallible>(edge.handle(req).await) }
                });
                // A per-connection serve error (a client hang-up, a malformed frame) is that connection's
                // business, not the edge's — log-free drop, keep accepting. `.with_upgrades()` lets a
                // WebSocket upgrade complete (so `hyper::upgrade::on` in the ws path resolves).
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .with_upgrades()
                    .await;
            });
        }
    }

    /// Map one hyper request onto an [`HttpRequest`], serve it through the gateway, and map the
    /// [`HttpResponse`] back. A WebSocket-upgrade request is handed to the per-connection ws path instead;
    /// an unrepresentable method (outside the 7 the contract models) is a `501` floor; a body over the size
    /// ceiling is a `413`; another body-read error is a `400`; each is an edge floor that never reaches a
    /// handler.
    async fn handle(&self, mut req: Request<Incoming>) -> Response<Full<Bytes>> {
        if is_websocket_upgrade(req.headers()) {
            return self.handle_ws_upgrade(&mut req).await;
        }
        let (parts, body) = req.into_parts();
        let Some(method) = method_from_hyper(&parts.method) else {
            return floor_response(501, "not implemented");
        };
        let path = parts.uri.path().to_string();
        let query = parts.uri.query().unwrap_or("").to_string();
        // Only well-formed (UTF-8-valued) headers cross; a non-UTF-8 header value is dropped rather than
        // failing the whole request (the handler validates what it needs, api-gateway §4).
        let headers = parts
            .headers
            .iter()
            .filter_map(|(name, value)| {
                value.to_str().ok().map(|v| Header {
                    name: name.as_str().to_string(),
                    value: v.to_string(),
                })
            })
            .collect();
        // Bound the body: read at most `max_body_bytes` before answering `413`, so a large/unbounded upload
        // cannot exhaust node memory (`Limited` errors with a `LengthLimitError` once the ceiling is passed).
        let body = match Limited::new(body, self.max_body_bytes).collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(err)
                if err
                    .downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some() =>
            {
                return floor_response(413, "payload too large");
            }
            Err(_) => return floor_response(400, "bad request body"),
        };
        let request = HttpRequest {
            method,
            path,
            query,
            headers,
            body,
        };

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let session = id.to_be_bytes();
        let response = match &self.serve {
            HttpServe::Gateway(gateway) => {
                gateway.serve(self.store.as_ref(), &session, &request).await
            }
            HttpServe::Dumb {
                driver,
                root_router,
                request_timeout,
                control,
            } => {
                // Drive the root router; a drive that never resolves is abandoned at the wall-clock ceiling
                // (dropping the future cancels it + releases the session) → 504, mirroring the gateway path.
                let drive = driver.serve(
                    Arc::clone(&self.store),
                    *root_router,
                    &session,
                    Arc::clone(control),
                    &request,
                );
                match tokio::time::timeout(*request_timeout, drive).await {
                    // The root router's own answer (its normal + error responses are its business).
                    Ok(Ok(resp)) => resp,
                    // A runtime failure driving the router (no such root program / did-not-close / runaway /
                    // malformed close) is the edge's last-resort 500 floor — the router never produced a
                    // usable response.
                    Ok(Err(_)) => HttpResponse {
                        status: 500,
                        headers: vec![Header {
                            name: "content-type".to_string(),
                            value: "text/plain; charset=utf-8".to_string(),
                        }],
                        body: Bytes::from_static(b"internal server error"),
                    },
                    Err(_elapsed) => HttpResponse {
                        status: 504,
                        headers: vec![Header {
                            name: "content-type".to_string(),
                            value: "text/plain; charset=utf-8".to_string(),
                        }],
                        body: Bytes::from_static(b"gateway timeout"),
                    },
                }
            }
        };
        to_hyper_response(response)
    }

    /// Handle a WebSocket-upgrade request: route its path (upgrades are `GET`), and on a match complete the
    /// handshake (`101`) while spawning the per-connection frame loop over the upgraded connection. `404`
    /// if no route matches, `400` if the request lacks a `Sec-WebSocket-Key`. Returns once routing resolves
    /// (a guest routing source consults the router reducer here); the loop runs on its own task after hyper
    /// finishes the upgrade.
    async fn handle_ws_upgrade(&self, req: &mut Request<Incoming>) -> Response<Full<Bytes>> {
        // The dumb model routes ws upgrades through the root router too (a `ws-upgrade` decision → a session
        // subprogram); that path is a later slice, so in dumb mode a ws upgrade is floored `404` for now.
        let HttpServe::Gateway(gateway) = &self.serve else {
            return floor_response(404, "not found");
        };
        let path = req.uri().path().to_string();
        let Some(key) = req.headers().get(hyper::header::SEC_WEBSOCKET_KEY).cloned() else {
            return floor_response(400, "missing sec-websocket-key");
        };
        // A ws upgrade is a GET; the matched route's handler is driven as a per-connection session. (The
        // route's http contract-id is unused here — a ws session folds ws-events, not http-requests.) The
        // connection sequence seeds both the router consult and the session id.
        let conn_seq = self.next_id.fetch_add(1, Ordering::Relaxed);
        let Some((program, _contract)) = gateway
            .match_route(
                self.store.as_ref(),
                &conn_seq.to_be_bytes(),
                Method::Get,
                &path,
            )
            .await
        else {
            return floor_response(404, "not found");
        };
        let on_upgrade = hyper::upgrade::on(req);
        let store = Arc::clone(&self.store);
        tokio::spawn(run_ws_session(on_upgrade, store, program, conn_seq));
        switching_protocols(&key)
    }
}

/// The raw `ws-send` contract-id marker a v0 session guest tags its pushes with: the ASCII string
/// `cdz-platform.ws.send` right-padded with `.` to exactly `Hash::LEN` (33) bytes. A Cadenza guest cannot
/// compute a tagged blake3 contract-id at the value level, so v0 fixes a literal 33-byte marker both sides
/// agree on (`guests/ws-echo/reducer.cdz`'s `ws-send-contract` emits these exact bytes); the real kernel
/// supplies a proper contract-id later (P5). The host surfaces an emitted request's contract as
/// `ContractId::from_hash(Hash::from_bytes(<these 33 bytes>))`, which `ContractId::try_from` reproduces — so
/// [`ws_send_contract`] below (built from the SAME bytes) is exactly the id the session filters pushes on.
const WS_SEND_CONTRACT_MARKER: &[u8; cdz_platform::Hash::LEN] =
    b"cdz-platform.ws.send.............";

/// The fixed contract-ids + node identity a ws session's events carry in v0 (a session guest decodes
/// `on_message` by payload and tags its pushes with the `ws-send` id; a live control/trust model supplies
/// these later). `send-contract` MUST match the id a session tags its `ws-send` requests with — for a wasm
/// guest that is the raw [`WS_SEND_CONTRACT_MARKER`] bytes, NOT `ContractId::of(...)` (which would hash the
/// string to different bytes and silently drop the guest's pushes).
fn ws_event_contract() -> cdz_platform::ContractId {
    cdz_platform::ContractId::of(b"cdz-platform.ws.event")
}
fn ws_send_contract() -> cdz_platform::ContractId {
    cdz_platform::ContractId::try_from(&WS_SEND_CONTRACT_MARKER[..])
        .expect("the ws-send marker is exactly Hash::LEN bytes")
}

/// Drive one WebSocket connection: await the upgrade, frame the connection, open a per-connection
/// [`WsSession`](crate::ws::WsSession), and pump inbound frames → `on_frame` → the session's `ws-send`
/// pushes, until the session closes or the socket does. Best-effort — any error ends the connection.
async fn run_ws_session(
    on_upgrade: hyper::upgrade::OnUpgrade,
    store: Arc<dyn ProgramStore>,
    program: cdz_platform::ProgramHash,
    conn_seq: u64,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let Ok(upgraded) = on_upgrade.await else {
        return;
    };
    let mut ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
        TokioIo::new(upgraded),
        tokio_tungstenite::tungstenite::protocol::Role::Server,
        None,
    )
    .await;

    let conn = Bytes::copy_from_slice(&conn_seq.to_be_bytes());
    let session_id = conn_seq.to_be_bytes();
    let Some((mut session, initial)) = crate::ws::WsSession::open(
        store.as_ref(),
        program,
        &session_id,
        conn,
        cdz_platform::HostId::of(b"cdz-http-gateway"),
        cdz_platform::ReducerId::of(b"ws-router"),
        ws_event_contract(),
        ws_send_contract(),
    )
    .await
    else {
        return;
    };
    for push in initial {
        if ws.send(Message::Binary(push.data.to_vec())).await.is_err() {
            return;
        }
    }

    while session.is_open() {
        let data = match ws.next().await {
            Some(Ok(Message::Binary(data))) => Bytes::from(data),
            Some(Ok(Message::Text(text))) => Bytes::from(text.into_bytes()),
            // Ping/Pong are handled by tungstenite; a Close or end-of-stream ends the session.
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(_)) => continue,
            Some(Err(_)) => break,
        };
        for push in session.on_frame(data).await {
            if ws.send(Message::Binary(push.data.to_vec())).await.is_err() {
                return;
            }
        }
    }
    for push in session.close().await {
        let _ = ws.send(Message::Binary(push.data.to_vec())).await;
    }
    let _ = ws.close(None).await;
}

/// Whether `headers` are a WebSocket upgrade request (RFC 6455): `Connection: Upgrade`, `Upgrade: websocket`,
/// a `Sec-WebSocket-Key`, and `Sec-WebSocket-Version: 13`. The edge routes such a request to a per-connection
/// [`WsSession`](crate::ws::WsSession) (the framing/upgrade wiring is a later slice); a non-upgrade request is
/// the ordinary request→response path.
#[must_use]
pub fn is_websocket_upgrade(headers: &hyper::HeaderMap) -> bool {
    use hyper::header::{CONNECTION, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE};
    // A comma-separated header lists `want` as one of its tokens (case-insensitive).
    let lists = |name: hyper::header::HeaderName, want: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.split(',').any(|t| t.trim().eq_ignore_ascii_case(want)))
    };
    lists(CONNECTION, "upgrade")
        && lists(UPGRADE, "websocket")
        && headers.contains_key(SEC_WEBSOCKET_KEY)
        && headers
            .get(SEC_WEBSOCKET_VERSION)
            .and_then(|v| v.to_str().ok())
            == Some("13")
}

/// The `101 Switching Protocols` response completing the WebSocket handshake for request key `key` (RFC
/// 6455: `Sec-WebSocket-Accept = base64(sha1(key + magic-GUID))`, via tungstenite's `derive_accept_key`).
#[must_use]
pub fn switching_protocols(key: &hyper::header::HeaderValue) -> Response<Full<Bytes>> {
    use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, UPGRADE};
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    Response::builder()
        .status(hyper::StatusCode::SWITCHING_PROTOCOLS)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(Full::new(Bytes::new()))
        .expect("a static 101 handshake response is always valid")
}

/// The [`Method`] for a hyper method, or `None` for one the `http-request` contract does not model (the
/// edge answers those `501`).
fn method_from_hyper(m: &hyper::Method) -> Option<Method> {
    Some(match *m {
        hyper::Method::GET => Method::Get,
        hyper::Method::POST => Method::Post,
        hyper::Method::PUT => Method::Put,
        hyper::Method::DELETE => Method::Delete,
        hyper::Method::PATCH => Method::Patch,
        hyper::Method::HEAD => Method::Head,
        hyper::Method::OPTIONS => Method::Options,
        _ => return None,
    })
}

/// Serialize an [`HttpResponse`] into a hyper response. A handler that produced an invalid HTTP status or
/// header (unrepresentable on the wire) collapses to a `500` floor rather than panicking.
fn to_hyper_response(resp: HttpResponse) -> Response<Full<Bytes>> {
    let mut builder = Response::builder().status(resp.status);
    for h in &resp.headers {
        builder = builder.header(h.name.as_str(), h.value.as_str());
    }
    builder
        .body(Full::new(resp.body))
        .unwrap_or_else(|_| floor_response(500, "invalid handler response"))
}

/// A host FLOOR hyper response — a plain-text status the edge synthesizes without invoking a handler.
fn floor_response(status: u16, message: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from_static(message.as_bytes())))
        .expect("a static floor response is always valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::encode_response;
    use crate::gateway::{Route, Router};
    use crate::runner::HandlerRunner;
    use async_trait::async_trait;
    use cdz_platform::testing::program::Store;
    use cdz_platform::{
        ContractId, HostId, Message, Notification, Outcome, ProgramHash, Reducer, ReducerId,
        Request as PRequest, Response as PResponse,
    };
    use http_body_util::Empty;
    use hyper::client::conn::http1 as client_http1;

    /// A native handler that closes with a `200` echoing the request path in the body — proves the whole
    /// edge→gateway→runner→handler→response path over a real socket.
    struct PathEchoHandler;
    #[async_trait]
    impl Reducer for PathEchoHandler {
        async fn on_message(&mut self, m: Message) -> (Vec<PRequest>, Outcome) {
            let body = match crate::codec::decode_request(&m.payload) {
                Some(req) => req.path.into_bytes(),
                None => b"decode-failed".to_vec(),
            };
            let resp = HttpResponse {
                status: 200,
                headers: vec![Header {
                    name: "content-type".to_string(),
                    value: "text/plain".to_string(),
                }],
                body: Bytes::from(body),
            };
            (
                vec![],
                Outcome::Break {
                    schema: ContractId::of(b"cdz-platform.http.response"),
                    reason: encode_response(&resp),
                },
            )
        }
        async fn on_response(&mut self, _r: PResponse) -> (Vec<PRequest>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<PRequest>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// Stand up the edge on an ephemeral port and return its address.
    async fn spawn_edge() -> std::net::SocketAddr {
        let handler = ProgramHash::of(b"path-echo");
        let mut store = Store::new();
        store.register(handler, || Box::new(PathEchoHandler));
        let runner = HandlerRunner::new(
            HostId::of(b"edge-test-host"),
            ReducerId::of(b"edge-test-router"),
        );
        let gateway = Gateway::new(
            Router::new(vec![Route::new(
                Method::Get,
                "/echo",
                handler,
                ContractId::of(b"cdz-platform.http.request"),
            )]),
            runner,
        );
        let edge = Arc::new(HttpEdge::new(gateway, Arc::new(store)));

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));
        addr
    }

    /// Send one GET over a real TCP connection and return `(status, body)`.
    async fn get(addr: std::net::SocketAddr, path: &str) -> (u16, Bytes) {
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let io = TokioIo::new(stream);
        let (mut sender, conn) = client_http1::handshake::<_, Empty<Bytes>>(io)
            .await
            .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method(hyper::Method::GET)
            .uri(path)
            .header("host", "test")
            .body(Empty::<Bytes>::new())
            .expect("request");
        let resp = sender.send_request(req).await.expect("send");
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        (status, body)
    }

    /// A no-op [`ControlSink`] for the dumb-edge tests (they do not exercise `control.send`).
    struct NullSink;
    #[async_trait]
    impl ControlSink for NullSink {
        async fn send(&self, _msg: crate::codec::ControlUp) {}
    }

    /// Stand up a DUMB edge (drives `root` as the root router per request) on an ephemeral port; return its
    /// address. `root` is registered iff `present` — an absent root exercises the `500` floor.
    async fn spawn_dumb_edge(present: bool) -> std::net::SocketAddr {
        let root = ProgramHash::of(b"dumb-root-router");
        let mut store = Store::new();
        if present {
            store.register(root, || Box::new(PathEchoHandler));
        }
        let driver = RootDriver::new(
            HostId::of(b"dumb-edge-host"),
            ContractId::of(b"cdz-platform.http.request"),
            ContractId::of(b"cdz-platform.http.request"),
        );
        let edge = Arc::new(HttpEdge::dumb(
            driver,
            root,
            Arc::new(NullSink),
            Arc::new(store),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));
        addr
    }

    #[tokio::test]
    async fn dumb_edge_serves_by_driving_the_root_router() {
        // No route table on the edge — the root router (here, one that answers directly) is driven per
        // request; its 200 comes back over a real socket, on ANY path (routing is the router's business).
        let addr = spawn_dumb_edge(true).await;
        let (status, body) = get(addr, "/anything").await;
        assert_eq!(status, 200);
        assert_eq!(body, Bytes::from_static(b"/anything"));
    }

    #[tokio::test]
    async fn dumb_edge_floors_a_missing_root_router() {
        // A misconfigured edge whose root-router hash names no program floors 500 (the drive fails to
        // instantiate — a runtime failure, not a router response).
        let addr = spawn_dumb_edge(false).await;
        let (status, _body) = get(addr, "/anything").await;
        assert_eq!(status, 500);
    }

    #[tokio::test]
    async fn dumb_edge_times_out_a_never_resolving_root_router() {
        // A root router whose drive never resolves (an effect/await that never lands — burns no folds, so
        // the fold ceiling does not catch it) is abandoned at the wall-clock timeout → 504.
        struct HangsForever;
        #[async_trait]
        impl Reducer for HangsForever {
            async fn on_message(&mut self, _m: Message) -> (Vec<PRequest>, Outcome) {
                std::future::pending::<()>().await;
                unreachable!()
            }
            async fn on_response(&mut self, _r: PResponse) -> (Vec<PRequest>, Outcome) {
                (vec![], Outcome::Continue)
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<PRequest>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }
        let root = ProgramHash::of(b"hangs");
        let mut store = Store::new();
        store.register(root, || Box::new(HangsForever));
        let driver = RootDriver::new(
            HostId::of(b"dumb-edge-host"),
            ContractId::of(b"cdz-platform.http.request"),
            ContractId::of(b"cdz-platform.http.request"),
        );
        let edge = Arc::new(
            HttpEdge::dumb(driver, root, Arc::new(NullSink), Arc::new(store))
                .with_dumb_request_timeout(Duration::from_millis(50)),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));
        let (status, _body) = get(addr, "/anything").await;
        assert_eq!(status, 504);
    }

    #[tokio::test]
    async fn dumb_edge_gives_each_request_a_fresh_root_session() {
        // Per-request isolation: the dumb edge spawns a FRESH root-router instance per request, so
        // per-instance state never leaks between requests. A router counting its own folds answers "1"
        // every time — never "2".
        struct CountingRouter {
            count: u32,
        }
        #[async_trait]
        impl Reducer for CountingRouter {
            async fn on_message(&mut self, _m: Message) -> (Vec<PRequest>, Outcome) {
                self.count += 1;
                let resp = HttpResponse {
                    status: 200,
                    headers: vec![],
                    body: Bytes::from(self.count.to_string().into_bytes()),
                };
                (
                    vec![],
                    Outcome::Break {
                        schema: ContractId::of(b"cdz-platform.http.response"),
                        reason: encode_response(&resp),
                    },
                )
            }
            async fn on_response(&mut self, _r: PResponse) -> (Vec<PRequest>, Outcome) {
                (vec![], Outcome::Continue)
            }
            async fn on_notification(&mut self, _n: Notification) -> (Vec<PRequest>, Outcome) {
                (vec![], Outcome::Continue)
            }
        }
        let root = ProgramHash::of(b"counting-router");
        let mut store = Store::new();
        store.register(root, || Box::new(CountingRouter { count: 0 }));
        let driver = RootDriver::new(
            HostId::of(b"dumb-edge-host"),
            ContractId::of(b"cdz-platform.http.request"),
            ContractId::of(b"cdz-platform.http.request"),
        );
        let edge = Arc::new(HttpEdge::dumb(
            driver,
            root,
            Arc::new(NullSink),
            Arc::new(store),
        ));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));
        let (s1, b1) = get(addr, "/a").await;
        let (s2, b2) = get(addr, "/b").await;
        assert_eq!((s1, s2), (200, 200));
        assert_eq!(b1, Bytes::from_static(b"1"));
        assert_eq!(
            b2,
            Bytes::from_static(b"1"),
            "each request is a fresh session"
        );
    }

    #[tokio::test]
    async fn a_matched_route_is_served_over_a_real_socket() {
        let addr = spawn_edge().await;
        let (status, body) = get(addr, "/echo").await;
        assert_eq!(status, 200);
        assert_eq!(body, Bytes::from_static(b"/echo"));
    }

    #[tokio::test]
    async fn an_unmatched_path_is_a_404_over_the_socket() {
        let addr = spawn_edge().await;
        let (status, _body) = get(addr, "/nope").await;
        assert_eq!(status, 404);
    }

    /// Send one POST with a body over a real TCP connection and return the status.
    async fn post(addr: std::net::SocketAddr, path: &str, body: Vec<u8>) -> u16 {
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut sender, conn) = client_http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method(hyper::Method::POST)
            .uri(path)
            .header("host", "test")
            .body(Full::new(Bytes::from(body)))
            .expect("request");
        sender
            .send_request(req)
            .await
            .expect("send")
            .status()
            .as_u16()
    }

    #[tokio::test]
    async fn an_oversized_body_is_a_413() {
        // An edge with an 8-byte body ceiling. The limit is enforced in the edge BEFORE routing, so it
        // holds regardless of the route; a native handler keeps this test wasm-free.
        let handler = ProgramHash::of(b"path-echo");
        let mut store = Store::new();
        store.register(handler, || Box::new(PathEchoHandler));
        let gateway = Gateway::new(
            Router::new(vec![Route::new(
                Method::Post,
                "/up",
                handler,
                ContractId::of(b"cdz-platform.http.request"),
            )]),
            HandlerRunner::new(HostId::of(b"h"), ReducerId::of(b"r")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, Arc::new(store)).with_max_body_bytes(8));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        // Over the ceiling → 413, before the request ever reaches a handler.
        assert_eq!(post(addr, "/up", vec![b'x'; 100]).await, 413);
        // Under the ceiling → the body is accepted and the request routes to the handler (200).
        assert_eq!(post(addr, "/up", b"tiny".to_vec()).await, 200);
    }

    #[test]
    fn maps_the_seven_modelled_methods_and_rejects_others() {
        assert_eq!(method_from_hyper(&hyper::Method::GET), Some(Method::Get));
        assert_eq!(
            method_from_hyper(&hyper::Method::OPTIONS),
            Some(Method::Options)
        );
        // CONNECT/TRACE are not modelled by the http-request contract → the edge answers 501.
        assert_eq!(method_from_hyper(&hyper::Method::CONNECT), None);
        assert_eq!(method_from_hyper(&hyper::Method::TRACE), None);
    }

    #[test]
    fn detects_and_completes_a_websocket_handshake() {
        use hyper::header::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        h.insert("connection", HeaderValue::from_static("Upgrade"));
        h.insert("upgrade", HeaderValue::from_static("websocket"));
        h.insert(
            "sec-websocket-key",
            HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        h.insert("sec-websocket-version", HeaderValue::from_static("13"));
        assert!(is_websocket_upgrade(&h));

        // The RFC 6455 §1.3 example: this key derives exactly this accept.
        let resp = switching_protocols(h.get("sec-websocket-key").unwrap());
        assert_eq!(resp.status(), hyper::StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            resp.headers().get("sec-websocket-accept").unwrap(),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );

        // A plain request is not an upgrade; nor is one missing the v13 version token.
        let mut plain = HeaderMap::new();
        plain.insert("host", HeaderValue::from_static("x"));
        assert!(!is_websocket_upgrade(&plain));
        h.remove("sec-websocket-version");
        assert!(!is_websocket_upgrade(&h));
    }

    /// A native WebSocket session that echoes each inbound frame back as a `ws-send` (tagged with the
    /// `ws-send` contract-id the edge filters on).
    struct WsEcho;
    #[async_trait]
    impl Reducer for WsEcho {
        async fn on_message(&mut self, m: Message) -> (Vec<PRequest>, Outcome) {
            use crate::codec::{WsEvent, WsSend, decode_ws_event, encode_ws_send};
            match decode_ws_event(&m.payload) {
                Some(WsEvent::Frame { conn, data }) => (
                    vec![PRequest {
                        id: ws_send_contract(),
                        payload: encode_ws_send(&WsSend { conn, data }),
                        continuation_token: Bytes::new(),
                        deadline: None,
                    }],
                    Outcome::Continue,
                ),
                _ => (vec![], Outcome::Continue),
            }
        }
        async fn on_response(&mut self, _r: PResponse) -> (Vec<PRequest>, Outcome) {
            (vec![], Outcome::Continue)
        }
        async fn on_notification(&mut self, _n: Notification) -> (Vec<PRequest>, Outcome) {
            (vec![], Outcome::Continue)
        }
    }

    /// THE WEBSOCKET E2E: a real ws client connects to a ws endpoint over a socket, sends a frame, and gets
    /// it echoed — exercising the edge's upgrade handshake → per-connection WsSession → frame loop. A native
    /// WsEcho session keeps this wasm-free.
    #[tokio::test]
    async fn a_websocket_endpoint_echoes_over_a_real_socket() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let session = ProgramHash::of(b"ws-echo");
        let mut store = Store::new();
        store.register(session, || Box::new(WsEcho));
        let gateway = Gateway::new(
            Router::new(vec![Route::new(
                Method::Get,
                "/ws",
                session,
                ContractId::of(b"cdz-platform.ws.event"),
            )]),
            HandlerRunner::new(HostId::of(b"h"), ReducerId::of(b"r")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, Arc::new(store)));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut ws, _resp) = tokio_tungstenite::client_async(format!("ws://{addr}/ws"), stream)
            .await
            .expect("ws client handshake");
        ws.send(Message::Binary(b"hello ws".to_vec()))
            .await
            .expect("send frame");
        let echoed = ws.next().await.expect("a frame").expect("ok frame");
        assert_eq!(echoed, Message::Binary(b"hello ws".to_vec()));
        let _ = ws.close(None).await;
    }

    /// A guard on the raw `ws-send` marker the edge filters a session's pushes on: exactly `Hash::LEN` bytes
    /// (the const's type already enforces this at compile time; assert it explicitly too) and the documented
    /// `cdz-platform.ws.send` prefix, so an accidental edit that drifts from `guests/ws-echo/reducer.cdz`'s
    /// `ws-send-contract` literal fails here rather than silently dropping every wasm-guest push.
    #[test]
    fn the_ws_send_marker_is_a_hash_len_prefixed_id() {
        assert_eq!(WS_SEND_CONTRACT_MARKER.len(), cdz_platform::Hash::LEN);
        assert!(WS_SEND_CONTRACT_MARKER.starts_with(b"cdz-platform.ws.send"));
        // The edge builds its filter id from these exact bytes — the same reconstruction the host applies to
        // a guest's emitted 33-byte contract, so a real wasm guest's pushes match.
        assert_eq!(
            ws_send_contract(),
            ContractId::try_from(&WS_SEND_CONTRACT_MARKER[..]).unwrap()
        );
    }
}

#[cfg(all(test, feature = "host"))]
mod host_e2e {
    use super::*;
    use crate::gateway::{Gateway, Route, Router};
    use crate::runner::HandlerRunner;
    use crate::wasm::{spawn_epoch_ticker, wasm_store};
    use cdz_platform::{
        BlobStore, ContractId, HostId, InMemoryBlobStore, ProgramHash, ProgramStore, ReducerId,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    /// THE FULL WEBSOCKET STACK, END-TO-END, WITH A REAL WASM SESSION GUEST (`DESIGN-http-outpost.md` §6):
    /// a real ws client connects over a socket → the edge's RFC-6455 upgrade → `run_ws_session` → a
    /// per-connection [`WsSession`](crate::ws::WsSession) folding the content-addressed wasm ws-echo guest
    /// (`guests/ws-echo/reducer.cdz`) on the wasmtime store → the guest's `ws-send` push framed back to the
    /// client. This is the ws analogue of `wasm::tests::poc_wasm_handler_served_over_a_socket`, and it is the
    /// test that exercises the edge's `ws_send_contract()` against a guest emitting the RAW 33-byte marker —
    /// the path the #8602 native e2e could not cover (its `WsEcho` used the edge's own `ContractId::of` id
    /// symmetrically, so a `ContractId::of`/raw-marker mismatch would have gone unnoticed there). The guest
    /// imports the value-heap runtime, which the host composes from the CAS by hash, so runtime + NFC are
    /// seeded alongside. Skips cleanly when any env var is unset so `cargo test --features host` passes
    /// without them (the fleet nix check sets all three).
    #[tokio::test]
    async fn wasm_ws_endpoint_echoes_over_a_real_socket() {
        let (Ok(guest_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_WS_ECHO_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "wasm_ws_endpoint_echoes_over_a_real_socket: CDZ_HTTP_WS_ECHO_WASM/RUNTIME_WASM/NFC_WASM \
                 unset — skipping (the nix check sets all three)"
            );
            return;
        };
        let guest = std::fs::read(&guest_path).expect("read ws-echo guest wasm");

        // Seed the value-heap runtime + its NFC dep (so the host composes the guest's `cadenza:runtime/heap`
        // import by hash) and the ws-session guest itself into the content store.
        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(Bytes::from(std::fs::read(dep).expect("read dep component")))
                .await;
        }
        cas.put(Bytes::from(guest.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let program = ProgramHash::of(&guest);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // Route a ws endpoint at GET /ws to the wasm session guest. (The route's http contract-id is unused
        // by a ws session; the edge folds ws-events, tagged with the fixed ws contract-ids.)
        let gateway = Gateway::new(
            Router::new(vec![Route::new(
                Method::Get,
                "/ws",
                program,
                ContractId::of(b"cdz-platform.ws.event"),
            )]),
            HandlerRunner::new(HostId::of(b"cdz-http-gateway"), ReducerId::of(b"ws-router")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, store));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut ws, _resp) = tokio_tungstenite::client_async(format!("ws://{addr}/ws"), stream)
            .await
            .expect("ws client handshake");
        ws.send(Message::Binary(b"hello wasm ws".to_vec()))
            .await
            .expect("send frame");
        let echoed = ws.next().await.expect("a frame").expect("ok frame");
        assert_eq!(
            echoed,
            Message::Binary(b"hello wasm ws".to_vec()),
            "the wasm ws-echo guest's push must reach the client — proves the edge's ws_send_contract() \
             matches the guest's raw 33-byte marker"
        );
        let _ = ws.close(None).await;
    }
}
