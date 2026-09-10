//! The wasmtime-backed handler store (`DESIGN-http-outpost.md` §0.1/§4, P1c-3) — behind the `host` feature.
//!
//! Reuses `cdz-platform`'s `WasmProgramStore` (its wasmtime host driving the reducer WIT world) so the
//! gateway instantiates a real content-addressed wasm handler per request with NO core platform — the
//! decoupling the design turns on. Each spawned handler session gets a FRESH in-memory `state` KV
//! (per-request isolation, design §4) and reads its component from the shared content store (`cas`).
//!
//! The compute bound (the epoch deadline `WasmProgramStore` arms) only bites if the engine epoch is
//! advanced, and the standalone gateway must drive that itself (production drives it via the platform
//! `Runtime`) — [`spawn_epoch_ticker`] is that driver.

use cdz_platform::{
    BlobStore, InMemoryBlobStore, InMemoryKvStore, InMemoryReducerGraph, KvStore, ProgramStore,
    ReducerGraph, ReducerId, ResourceLimits, WasmProgramStore,
};
use std::sync::Arc;

/// Build a wasmtime-backed [`ProgramStore`](cdz_platform::ProgramStore) for HTTP handlers. `cas` holds the
/// handler components (addressed by `ProgramHash`); the instantiator reads a component from it on `spawn`.
/// Each spawned handler gets:
/// - a FRESH [`InMemoryKvStore`] as its `state` — born empty, dies with the request (per-request isolation);
/// - an empty `blobs` backend — the inline PoC handler (§7) fetches none; a blob-fetching handler is a
///   later variant that wires the shared CAS here;
/// - a fresh graph (an ordinary handler does not route through it).
///
/// Uses the default [`ResourceLimits`] (epoch deadline + memory ceiling); a caller tuning per-request
/// budgets can build `WasmProgramStore::with_resource_limits` directly.
///
/// # Errors
/// A `wasmtime::Error` if the engine/component-model host cannot be constructed.
pub fn wasm_store(cas: Arc<dyn BlobStore>) -> Result<WasmProgramStore, wasmtime::Error> {
    WasmProgramStore::with_resource_limits(
        cas,
        Arc::new(|_id: ReducerId| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>),
        Arc::new(|_id: ReducerId| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>),
        Arc::new(|_id: ReducerId| Arc::new(InMemoryReducerGraph::new()) as Arc<dyn ReducerGraph>),
        ResourceLimits::default(),
    )
}

/// Drive the engine's epoch ticker on a background task so a runaway handler fold TRAPS at its deadline
/// rather than only yielding (the standalone-gateway counterpart of the platform `Runtime`'s ticker).
/// Returns the task handle (abort/drop it to stop), or `None` if `store` has no wasm engine (a native
/// store). Call once per store.
#[must_use]
pub fn spawn_epoch_ticker(store: &dyn ProgramStore) -> Option<tokio::task::JoinHandle<()>> {
    let (period, tick) = store.epoch_incrementer()?;
    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(period);
        loop {
            interval.tick().await;
            tick();
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_platform::{ProgramHash, ReducerKind, SpawnContext};

    fn empty_cas() -> Arc<dyn BlobStore> {
        Arc::new(InMemoryBlobStore::new())
    }

    #[tokio::test]
    async fn constructs_a_wasm_store() {
        // The construction wiring (the three factories + resource limits + the wasmtime engine) is the
        // non-trivial part — assert it links and builds.
        let store = wasm_store(empty_cas());
        assert!(store.is_ok(), "wasm store construction should succeed");
    }

    #[tokio::test]
    async fn an_absent_program_does_not_instantiate() {
        let store = wasm_store(empty_cas()).expect("store");
        let absent = ProgramHash::of(b"no-such-handler");
        assert!(!store.contains(absent).await, "empty CAS holds no program");
        let spawned = store
            .spawn(
                absent,
                SpawnContext {
                    id: ReducerId::of(b"req"),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await;
        assert!(spawned.is_none(), "an unknown program yields no reducer");
    }

    #[tokio::test]
    async fn the_wasm_store_has_an_epoch_ticker() {
        let store = wasm_store(empty_cas()).expect("store");
        // A wasm-backed store exposes an epoch incrementer (a native store returns None); the ticker task
        // spawns and is abortable.
        let ticker = spawn_epoch_ticker(&store);
        assert!(ticker.is_some(), "a wasm store drives an epoch ticker");
        ticker.unwrap().abort();
    }

    /// THE END-TO-END PAYOFF (DESIGN-http-outpost.md §7): a REAL content-addressed wasm handler served over
    /// a real socket — edge -> router -> wasmtime spawn -> fold -> http-response -> socket. The handler
    /// component imports the value-heap runtime (`cadenza:runtime/heap@…+<hash>`), which the host COMPOSES
    /// from the CAS by hash, so the runtime + its NFC dep must be seeded alongside the guest (the itest's
    /// resolve_deps/cas.put pattern). All three component paths come from env vars the fleet nix check sets
    /// (the PoC handler = mkCadenzaGuest, runtime = packages.runtime, nfc = packages.nfc); the test skips
    /// cleanly when any is unset so `cargo test --features host` passes without them.
    #[tokio::test]
    async fn poc_wasm_handler_served_over_a_socket() {
        use crate::codec::Method;
        use crate::edge::HttpEdge;
        use crate::gateway::{Gateway, Route, Router};
        use crate::runner::HandlerRunner;
        use cdz_platform::{ContractId, HostId, ProgramHash, ReducerId};
        use http_body_util::{BodyExt, Empty};
        use hyper::Request;
        use hyper_util::rt::TokioIo;

        let (Ok(guest_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_POC_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "poc_wasm_handler_served_over_a_socket: CDZ_HTTP_POC_WASM/RUNTIME_WASM/NFC_WASM unset — \
                 skipping (the nix check sets all three)"
            );
            return;
        };
        let guest = std::fs::read(&guest_path).expect("read PoC handler wasm");

        // Seed the value-heap runtime + its NFC dep (so the host composes the guest's `cadenza:runtime/heap`
        // import by hash) and the handler component itself into the content store.
        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(guest.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let program = ProgramHash::of(&guest);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        let runner = HandlerRunner::new(HostId::of(b"edge-host"), ReducerId::of(b"router"));
        let gateway = Gateway::new(
            Router::new(vec![Route::new(
                Method::Get,
                "/",
                program,
                ContractId::of(b"cdz-platform.http.request"),
            )]),
            runner,
        );
        let edge = Arc::new(HttpEdge::new(gateway, store));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake::<_, Empty<bytes::Bytes>>(TokioIo::new(stream))
                .await
                .expect("handshake");
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let resp = sender
            .send_request(
                Request::builder()
                    .method(hyper::Method::GET)
                    .uri("/")
                    .header("host", "test")
                    .body(Empty::<bytes::Bytes>::new())
                    .expect("request"),
            )
            .await
            .expect("send");
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.expect("body").to_bytes();

        assert_eq!(status, 200, "the wasm PoC handler answers 200");
        assert_eq!(
            body,
            bytes::Bytes::from_static(b"hello from a wasm handler")
        );
    }

    /// P2 RUNTIME PROOF (DESIGN-http-outpost.md §4): the ROUTER governing program (guests/router/reducer.cdz)
    /// folds real requests on the wasmtime store — routing-as-a-fold executes, and the Rust codec reads the
    /// router's `Value.encode`d `Decision` back (a cross-compiler pin of `decode_decision`). Each request
    /// spawns a fresh router instance (a one-shot fold: `on_message` → `Break` with the decision). Asserts the
    /// baked table: `GET /` → the root handler marker, `POST /echo` → the echo marker, an unlisted route →
    /// the empty-handler no-match sentinel. (The handler markers are the guest's baked PLACEHOLDERS; the
    /// gateway-consults-router wiring slice reconciles them with real handler `ProgramHash`es.) Seeds the
    /// value-heap runtime + NFC alongside the guest; skips cleanly when any env var is unset.
    #[tokio::test]
    async fn wasm_router_folds_the_baked_route_table() {
        use crate::codec::{HttpRequest, Method, RouteDecision, decode_decision, encode_request};
        use cdz_platform::{
            ContractId, HostId, Message, Origin, Outcome, ProgramHash, ReducerId, ReducerKind,
            SpawnContext,
        };

        let (Ok(router_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "wasm_router_folds_the_baked_route_table: CDZ_HTTP_ROUTER_WASM/RUNTIME_WASM/NFC_WASM unset \
                 — skipping (the nix check sets all three)"
            );
            return;
        };
        let router = std::fs::read(&router_path).expect("read router guest wasm");

        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(router.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let program = ProgramHash::of(&router);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // The router dispatches on the decoded request; the delivered contract-id is immaterial to it.
        let request_contract = ContractId::of(b"cdz-platform.http.request");

        // Fold one request through a FRESH router instance (a one-shot fold) → its routing decision.
        async fn route(
            store: &dyn ProgramStore,
            program: ProgramHash,
            request_contract: ContractId,
            id: &[u8],
            method: Method,
            path: &str,
        ) -> RouteDecision {
            let req = HttpRequest {
                method,
                path: path.to_string(),
                query: String::new(),
                headers: vec![],
                body: bytes::Bytes::new(),
            };
            let mut reducer = store
                .spawn(
                    program,
                    SpawnContext {
                        id: ReducerId::of(id),
                        kind: ReducerKind::Ordinary,
                        limits: None,
                    },
                )
                .await
                .expect("router instantiates");
            let (_requests, outcome) = reducer
                .on_message(Message {
                    id: request_contract,
                    payload: encode_request(&req),
                    from: Origin {
                        reducer: ReducerId::of(b"gateway"),
                        host: HostId::of(b"edge-host"),
                    },
                    continuation_token: bytes::Bytes::new(),
                })
                .await;
            match outcome {
                Outcome::Break { reason, .. } => {
                    decode_decision(&reason).expect("router's decision decodes")
                }
                Outcome::Continue => panic!("the router must Break with a routing decision"),
            }
        }

        // GET / → the root handler, folding the http-request contract (the guest's baked markers).
        let d = route(
            store.as_ref(),
            program,
            request_contract,
            b"r1",
            Method::Get,
            "/",
        )
        .await;
        assert!(d.is_match(), "GET / matches a route");
        assert_eq!(
            d.handler,
            bytes::Bytes::from_static(b"cdz-http.handler.root............")
        );
        assert_eq!(
            d.contract,
            bytes::Bytes::from_static(b"cdz-platform.http.request........")
        );

        // POST /echo → the echo handler.
        let d = route(
            store.as_ref(),
            program,
            request_contract,
            b"r2",
            Method::Post,
            "/echo",
        )
        .await;
        assert!(d.is_match(), "POST /echo matches a route");
        assert_eq!(
            d.handler,
            bytes::Bytes::from_static(b"cdz-http.handler.echo............")
        );

        // GET /nope → no route: the empty-handler no-match sentinel (→ the gateway's 404 floor).
        let d = route(
            store.as_ref(),
            program,
            request_contract,
            b"r3",
            Method::Get,
            "/nope",
        )
        .await;
        assert!(!d.is_match(), "an unlisted path is the no-match sentinel");

        // A method mismatch on a known path is also no-match (POST / is not routed; only GET / is).
        let d = route(
            store.as_ref(),
            program,
            request_contract,
            b"r4",
            Method::Post,
            "/",
        )
        .await;
        assert!(!d.is_match(), "POST / is not a route (only GET /)");
    }

    /// P2c PROOF: the gateway's [`RouterReducer`](crate::gateway::RouterReducer) CONSULTS the real router
    /// guest over the wasmtime store and maps its decision to a spawnable handler — the full host-side of
    /// "routing is a governing program." Binds the guest's baked handler markers to real handler
    /// `ProgramHash`es; asserts `GET /` → the root-bound hash + the http-request contract, `POST /echo` →
    /// the echo-bound hash, and an unlisted route → `None` (→ the gateway's 404). Seeds runtime + NFC; skips
    /// cleanly when any env var is unset.
    #[tokio::test]
    async fn router_reducer_consults_the_real_guest() {
        use crate::codec::Method;
        use crate::gateway::RouterReducer;
        use cdz_platform::{ContractId, HostId, ProgramHash, ReducerId};
        use std::collections::HashMap;

        let (Ok(router_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "router_reducer_consults_the_real_guest: CDZ_HTTP_ROUTER_WASM/RUNTIME_WASM/NFC_WASM unset \
                 — skipping (the nix check sets all three)"
            );
            return;
        };
        let router = std::fs::read(&router_path).expect("read router guest wasm");

        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(router.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let router_program = ProgramHash::of(&router);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // Bind the guest's baked handler markers to the real handler hashes this deployment would spawn.
        let root_handler = ProgramHash::of(b"the-root-handler-component");
        let echo_handler = ProgramHash::of(b"the-echo-handler-component");
        let mut handlers = HashMap::new();
        handlers.insert(
            bytes::Bytes::from_static(b"cdz-http.handler.root............"),
            root_handler,
        );
        handlers.insert(
            bytes::Bytes::from_static(b"cdz-http.handler.echo............"),
            echo_handler,
        );
        let request_contract =
            ContractId::try_from(&b"cdz-platform.http.request........"[..]).unwrap();
        let rr = RouterReducer::new(
            router_program,
            ContractId::of(b"cdz-platform.http.request"),
            HostId::of(b"edge-host"),
            ReducerId::of(b"gateway"),
            handlers,
        );

        // GET / → the root-bound handler hash + the http-request contract the decision carries.
        assert_eq!(
            rr.match_route(store.as_ref(), b"q1", Method::Get, "/")
                .await,
            Some((root_handler, request_contract)),
            "the gateway consults the guest and binds GET / to the root handler"
        );
        // POST /echo → the echo-bound handler hash.
        assert_eq!(
            rr.match_route(store.as_ref(), b"q2", Method::Post, "/echo")
                .await,
            Some((echo_handler, request_contract))
        );
        // GET /nope → no route.
        assert!(
            rr.match_route(store.as_ref(), b"q3", Method::Get, "/nope")
                .await
                .is_none(),
            "an unlisted route is not usable"
        );
    }
}
