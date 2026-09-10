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
use crate::gateway::Gateway;
use bytes::Bytes;
use cdz_platform::ProgramStore;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpListener;

/// The default per-request body-size ceiling (1 MiB) — the edge reads at most this many bytes of a request
/// body before answering `413`, so a large/unbounded upload cannot exhaust the node's memory (design §10:
/// the edge bounds foreign input; a per-request fold cannot exhaust the node). Tune with
/// [`HttpEdge::with_max_body_bytes`].
pub const DEFAULT_MAX_BODY_BYTES: usize = 1 << 20;

/// The HTTP edge: a [`Gateway`] + the [`ProgramStore`] its handlers instantiate from, bound to a listener.
/// `Send + Sync` (behind an `Arc`) so each accepted connection is served on its own task.
pub struct HttpEdge {
    gateway: Gateway,
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
    /// An edge serving `gateway` over handlers instantiated from `store`, with the default body-size limit.
    #[must_use]
    pub fn new(gateway: Gateway, store: Arc<dyn ProgramStore>) -> Self {
        Self {
            gateway,
            store,
            next_id: AtomicU64::new(0),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
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
                // business, not the edge's — log-free drop, keep accepting.
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await;
            });
        }
    }

    /// Map one hyper request onto an [`HttpRequest`], serve it through the gateway, and map the
    /// [`HttpResponse`] back. An unrepresentable method (outside the 7 the contract models) is a `501`
    /// floor; a body over the size ceiling is a `413`; another body-read error is a `400`; each is an edge
    /// floor that never reaches a handler.
    async fn handle(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
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
        let response = self
            .gateway
            .serve(self.store.as_ref(), &id.to_be_bytes(), &request)
            .await;
        to_hyper_response(response)
    }
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
}
