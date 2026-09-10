//! The mock control server (`DESIGN-http-outpost.md` §3) — behind the `host` feature.
//!
//! The real gateway dials a control server over WebSocket and, on the first frame, receives its route
//! table + fetches each handler component by hash (the outposts "fetch-a-program-by-content-hash"). This
//! is an IN-PROCESS mock of that: it holds the route table + the handler component bytes a control server
//! would ship, and ASSEMBLES a ready-to-serve [`HttpEdge`] from them — decode the route-table frame into a
//! [`Router`], seed the handler blobs + the value-heap runtime into the content store, and wire the
//! wasmtime [`wasm_store`]. It lets the whole control-driven path — control-frame → router → wasm handler
//! → response — be exercised end-to-end WITHOUT the real platform or a live network (the operator's
//! "mock control server so we can test the whole thing end-to-end easily").
//!
//! The WebSocket transport + a live control link is the P3 follow-on (design §3/D2); the wire is the same
//! `route-table` frame either way ([`crate::codec::encode_route_table`]).

use crate::codec::{Method, RouteFrame, encode_route_table};
use crate::edge::HttpEdge;
use crate::gateway::{Gateway, Router};
use crate::runner::HandlerRunner;
use crate::wasm::{spawn_epoch_ticker, wasm_store};
use bytes::Bytes;
use cdz_platform::{BlobStore, HostId, InMemoryBlobStore, ProgramHash, ProgramStore, ReducerId};
use std::sync::Arc;

/// One route a control server ships: a `(method, path)` served by the handler `handler_wasm` (the raw
/// component bytes — the gateway addresses it by its content hash), folding contract `contract`.
pub struct ControlRoute {
    pub method: Method,
    pub path: String,
    pub handler_wasm: Bytes,
    pub contract: Bytes,
}

/// A mock control server: the route table + handler component blobs it would ship, plus the dependency
/// components (the value-heap runtime + its NFC dep) a Cadenza handler imports. Assembles a ready-to-serve
/// [`HttpEdge`] from them.
pub struct MockControlServer {
    /// Dependency components every handler resolves against (the value-heap runtime + NFC) — seeded into
    /// the content store so the host composes each handler's `cadenza:runtime/heap` import by hash.
    deps: Vec<Bytes>,
    routes: Vec<ControlRoute>,
}

impl MockControlServer {
    /// A control server whose handlers resolve against `deps` (the runtime + NFC component bytes).
    #[must_use]
    pub fn new(deps: Vec<Bytes>) -> Self {
        Self {
            deps,
            routes: Vec::new(),
        }
    }

    /// Ship a route: `(method, path)` served by the handler component `handler_wasm`.
    #[must_use]
    pub fn route(
        mut self,
        method: Method,
        path: impl Into<String>,
        handler_wasm: Bytes,
        contract: Bytes,
    ) -> Self {
        self.routes.push(ControlRoute {
            method,
            path: path.into(),
            handler_wasm,
            contract,
        });
        self
    }

    /// The `route-table` frame this control server ships — each route's `handler` is the content hash of
    /// its component (the same `ProgramHash` the content store keys it by), so the decoded [`Router`]'s
    /// handler resolves against the seeded blob.
    #[must_use]
    pub fn route_table_frame(&self) -> Bytes {
        let frames: Vec<RouteFrame> = self
            .routes
            .iter()
            .map(|r| RouteFrame {
                method: r.method,
                path: r.path.clone(),
                handler: Bytes::copy_from_slice(ProgramHash::of(&r.handler_wasm).hash().as_bytes()),
                contract: r.contract.clone(),
            })
            .collect();
        encode_route_table(&frames)
    }

    /// Assemble a ready-to-serve [`HttpEdge`] as the gateway would on receiving this control server's frame:
    /// seed the dependency components + handler blobs into a content store, build the wasmtime handler store
    /// (driving the engine epoch on a detached ticker), decode the shipped route-table frame into a
    /// [`Router`], and wire the edge. `None` if the route-table frame is malformed (a bad handler hash).
    pub async fn build_edge(&self, host: HostId, router_id: ReducerId) -> Option<Arc<HttpEdge>> {
        let mut components: Vec<Bytes> = self.deps.clone();
        components.extend(self.routes.iter().map(|r| r.handler_wasm.clone()));
        assemble_edge(&self.route_table_frame(), &components, host, router_id).await
    }
}

/// Assemble a ready-to-serve [`HttpEdge`] from a `route-table` `frame` and the `components` it references —
/// the gateway's boot step, whatever the source of the frame + blobs (an in-process mock, or a deployment's
/// local content store). Seeds every component into a fresh content store (by content hash — the value-heap
/// runtime + NFC + each handler; the host composes each handler's `cadenza:runtime/heap` import from it),
/// builds the wasmtime handler store (driving the engine epoch on a detached ticker), decodes the frame into
/// a [`Router`], and wires the edge. `None` if the frame is malformed (a bad handler hash).
pub async fn assemble_edge(
    frame: &[u8],
    components: &[Bytes],
    host: HostId,
    router_id: ReducerId,
) -> Option<Arc<HttpEdge>> {
    let cas = InMemoryBlobStore::new();
    for c in components {
        cas.put(c.clone()).await;
    }
    let cas: Arc<dyn BlobStore> = Arc::new(cas);
    let store: Arc<dyn ProgramStore> = Arc::new(wasm_store(cas).ok()?);
    // Detached ticker: dropping the handle does not abort the task, so the engine epoch keeps advancing
    // for the edge's lifetime (a runaway handler still traps at its deadline).
    let _ = spawn_epoch_ticker(store.as_ref());

    // The router carries each route's contract-id (from the frame); the runner needs only the Origin.
    let router = Router::from_route_table(frame)?;
    let runner = HandlerRunner::new(host, router_id);
    Some(Arc::new(HttpEdge::new(Gateway::new(router, runner), store)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_platform::ContractId;
    use http_body_util::{BodyExt, Empty};
    use hyper::Request;
    use hyper_util::rt::TokioIo;

    /// THE MOCK-CONTROL-SERVER E2E (the operator's earliest-step directive): a control server ships a route
    /// table + a wasm handler blob; the gateway assembles its WHOLE serving stack from that frame and serves
    /// a real GET over a socket → the handler's 200. Exercises control-frame → Router::from_route_table →
    /// wasmtime spawn → fold → response, with NO hardcoded route. Env-gated on the component paths the nix
    /// check provides (skips cleanly when unset).
    #[tokio::test]
    async fn mock_control_server_assembles_and_serves_a_wasm_handler() {
        let (Ok(guest), Ok(rt), Ok(nfc)) = (
            std::env::var("CDZ_HTTP_POC_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!("mock_control_server: CDZ_HTTP_{{POC,RUNTIME,NFC}}_WASM unset — skipping");
            return;
        };
        let read = |p: &str| Bytes::from(std::fs::read(p).expect("read component"));

        let control = MockControlServer::new(vec![read(&rt), read(&nfc)]).route(
            Method::Get,
            "/hello",
            read(&guest),
            Bytes::copy_from_slice(
                ContractId::of(b"cdz-platform.http.request")
                    .hash()
                    .as_bytes(),
            ),
        );
        let edge = control
            .build_edge(HostId::of(b"edge-host"), ReducerId::of(b"router"))
            .await
            .expect("the shipped route table assembles an edge");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let resp = sender
            .send_request(
                Request::builder()
                    .method(hyper::Method::GET)
                    .uri("/hello")
                    .header("host", "test")
                    .body(Empty::<Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("send");
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.expect("body").to_bytes();

        assert_eq!(status, 200, "the control-shipped wasm handler answers 200");
        assert_eq!(body, Bytes::from_static(b"hello from a wasm handler"));

        // A path the control server did NOT ship is a 404 (the router only knows the shipped routes).
        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let resp = sender
            .send_request(
                Request::builder()
                    .method(hyper::Method::GET)
                    .uri("/absent")
                    .header("host", "test")
                    .body(Empty::<Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("send");
        assert_eq!(resp.status().as_u16(), 404, "an unshipped path is a 404");
    }

    /// FORWARD-PATH PROOF: a handler that READS the request. The echo guest `Value.decode`s the delivered
    /// `http-request` and answers a body naming the decoded method — so a 200 with `method=<M>` proves the
    /// gateway's Rust `encode_request` produces bytes the guest's `Value.decode` accepts (the boundary in
    /// the reading direction, which the request-ignoring PoC never exercised). Env-gated on the echo guest.
    #[tokio::test]
    async fn a_request_reading_handler_decodes_the_gateway_encoded_request() {
        let (Ok(echo), Ok(rt), Ok(nfc)) = (
            std::env::var("CDZ_HTTP_ECHO_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!("forward-path test: CDZ_HTTP_{{ECHO,RUNTIME,NFC}}_WASM unset — skipping");
            return;
        };
        let read = |p: &str| Bytes::from(std::fs::read(p).expect("read component"));
        let control = MockControlServer::new(vec![read(&rt), read(&nfc)]).route(
            Method::Post,
            "/m",
            read(&echo),
            Bytes::copy_from_slice(
                ContractId::of(b"cdz-platform.http.request")
                    .hash()
                    .as_bytes(),
            ),
        );
        let edge = control
            .build_edge(HostId::of(b"edge-host"), ReducerId::of(b"router"))
            .await
            .expect("edge");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let resp = sender
            .send_request(
                Request::builder()
                    .method(hyper::Method::POST)
                    .uri("/m")
                    .header("host", "test")
                    .body(Empty::<Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("send");
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        assert_eq!(
            status, 200,
            "the request decoded (400 would mean Value.decode rejected it)"
        );
        assert_eq!(
            body,
            Bytes::from_static(b"method=POST"),
            "the handler read the decoded method field"
        );
    }
}
