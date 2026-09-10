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
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::net::TcpListener;

/// The HTTP edge: a [`Gateway`] + the [`ProgramStore`] its handlers instantiate from, bound to a listener.
/// `Send + Sync` (behind an `Arc`) so each accepted connection is served on its own task.
pub struct HttpEdge {
    gateway: Gateway,
    store: Arc<dyn ProgramStore>,
    /// A monotonic per-request counter seeding the correlation/session id. v0: a counter (distinct per
    /// request is all the runner needs to give each session a fresh id); the unguessable-token scheme the
    /// design reserves arrives with the in-platform trust model (auth is an external proxy in v0, D1).
    next_id: AtomicU64,
}

impl HttpEdge {
    /// An edge serving `gateway` over handlers instantiated from `store`.
    #[must_use]
    pub fn new(gateway: Gateway, store: Arc<dyn ProgramStore>) -> Self {
        Self {
            gateway,
            store,
            next_id: AtomicU64::new(0),
        }
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
    /// floor; a body-read error is a `400`; both are edge floors that never reach a handler.
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
        let body = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
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
}
