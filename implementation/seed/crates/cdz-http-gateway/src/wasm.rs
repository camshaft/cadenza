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

    /// THE P2 FINALE (DESIGN-http-outpost.md §4): routing-as-a-fold serving a REAL request over a REAL socket
    /// with REAL guests — client → edge → the router GOVERNING PROGRAM consults its baked table → the gateway
    /// spawns the decided handler → its http-response → socket. No static route table anywhere: the route
    /// decision comes entirely from folding the request through the wasm router guest. The router's baked
    /// `root` marker is bound to the PoC handler component's real `ProgramHash` (the deployment binding); both
    /// guests + the value-heap runtime + NFC are seeded into one CAS. Skips cleanly when any env var is unset.
    #[tokio::test]
    async fn real_router_guest_routes_a_real_handler_over_a_socket() {
        use crate::edge::HttpEdge;
        use crate::gateway::{Gateway, RouterReducer};
        use crate::runner::HandlerRunner;
        use cdz_platform::{ContractId, HostId, ProgramHash, ReducerId};
        use http_body_util::{BodyExt, Empty};
        use hyper::Request;
        use hyper_util::rt::TokioIo;
        use std::collections::HashMap;

        let (Ok(router_path), Ok(poc_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_WASM"),
            std::env::var("CDZ_HTTP_POC_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "real_router_guest_routes_a_real_handler_over_a_socket: \
                 CDZ_HTTP_ROUTER_WASM/POC_WASM/RUNTIME_WASM/NFC_WASM unset — skipping"
            );
            return;
        };
        let router = std::fs::read(&router_path).expect("read router guest wasm");
        let poc = std::fs::read(&poc_path).expect("read PoC handler wasm");

        // One CAS holds the router guest, the PoC handler, and the value-heap runtime + NFC both import.
        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(router.clone())).await;
        cas.put(bytes::Bytes::from(poc.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let router_program = ProgramHash::of(&router);
        let poc_program = ProgramHash::of(&poc);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // Bind the router guest's baked `root` marker (it routes GET / there) to the real PoC handler hash.
        let mut handlers = HashMap::new();
        handlers.insert(
            bytes::Bytes::from_static(b"cdz-http.handler.root............"),
            poc_program,
        );
        let router_reducer = RouterReducer::new(
            router_program,
            ContractId::of(b"cdz-platform.http.request"),
            HostId::of(b"edge-host"),
            ReducerId::of(b"gateway"),
            handlers,
        );
        let gateway = Gateway::with_router_reducer(
            router_reducer,
            HandlerRunner::new(HostId::of(b"edge-host"), ReducerId::of(b"gateway")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, store));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        async fn get(addr: std::net::SocketAddr, path: &str) -> (u16, bytes::Bytes) {
            let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake::<_, Empty<bytes::Bytes>>(TokioIo::new(
                    stream,
                ))
                .await
                .expect("handshake");
            tokio::spawn(async move {
                let _ = conn.await;
            });
            let resp = sender
                .send_request(
                    Request::builder()
                        .method(hyper::Method::GET)
                        .uri(path)
                        .header("host", "test")
                        .body(Empty::<bytes::Bytes>::new())
                        .expect("request"),
                )
                .await
                .expect("send");
            let status = resp.status().as_u16();
            let body = resp.into_body().collect().await.expect("body").to_bytes();
            (status, body)
        }

        // GET / : the router guest folds the request, decides `root`, the gateway spawns the bound PoC
        // handler, and its response rides back to the socket — routing-as-a-fold, end to end.
        let (status, body) = get(addr, "/").await;
        assert_eq!(
            status, 200,
            "the router-guest-routed wasm handler answers 200"
        );
        assert_eq!(
            body,
            bytes::Bytes::from_static(b"hello from a wasm handler")
        );

        // GET /nope : the router guest returns the no-match sentinel → the gateway's 404 floor.
        let (status, _) = get(addr, "/nope").await;
        assert_eq!(status, 404, "an unrouted path is a 404 floor");
    }

    /// P3 STATE-IMPORT PROOF (DESIGN-http-outpost.md §3/§4): the kv-probe guest holds STATE across the
    /// get/put within a fold via the platform `state` capability, driven through the REAL wasm store — the
    /// store hands the reducer a fresh `InMemoryKvStore` as its `state` backend, so this proves the whole
    /// host-import path works end to end (the mechanism the stateful router P3b needs). The guest
    /// `state.put(b"cell", payload)` then `state.get(b"cell")` and closes with the value read back; a correct
    /// round-trip returns the payload verbatim. Seeds the value-heap runtime + NFC; skips when env is unset.
    #[tokio::test]
    async fn wasm_kv_probe_round_trips_a_value_through_state() {
        use cdz_platform::{
            ContractId, HostId, Message, Origin, Outcome, ProgramHash, ReducerId, ReducerKind,
            SpawnContext,
        };

        let (Ok(guest_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_KV_PROBE_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "wasm_kv_probe_round_trips_a_value_through_state: \
                 CDZ_HTTP_KV_PROBE_WASM/RUNTIME_WASM/NFC_WASM unset — skipping"
            );
            return;
        };
        let guest = std::fs::read(&guest_path).expect("read kv-probe guest wasm");

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

        let mut reducer = store
            .spawn(
                program,
                SpawnContext {
                    id: ReducerId::of(b"kv-sess"),
                    kind: ReducerKind::Ordinary,
                    limits: None,
                },
            )
            .await
            .expect("kv-probe instantiates");
        let (_requests, outcome) = reducer
            .on_message(Message {
                id: ContractId::of(b"cdz-platform.kv.probe"),
                payload: bytes::Bytes::from_static(b"round-trip-me"),
                from: Origin {
                    reducer: ReducerId::of(b"driver"),
                    host: HostId::of(b"edge-host"),
                },
                continuation_token: bytes::Bytes::new(),
            })
            .await;

        // The guest put the payload into state then got it back and closed with it — a correct round-trip
        // returns the payload verbatim (proving state persisted across the put→get within the fold).
        match outcome {
            Outcome::Break { reason, .. } => assert_eq!(
                reason,
                bytes::Bytes::from_static(b"round-trip-me"),
                "state.get returns exactly what state.put stored"
            ),
            Outcome::Continue => panic!("kv-probe must Break with the value read back from state"),
        }
    }

    /// P3b PIVOT PROOF: the DYNAMIC (stateless) router guest routes over a LIVE table passed in the message,
    /// driven through the real wasm store — routing-as-a-fold without host-state, so it actually INSTANTIATES
    /// + runs (unlike the KV-state router, which the compiler bug blocks). The gateway would hold the table
    /// and send a `RouteQuery{request, table}`; here the test builds that envelope directly. Asserts the
    /// decoded routing decision matches the shipped table (GET /echo → echo handler; GET /nope → no-match),
    /// and that swapping the table changes the route (the "live table" property). Seeds runtime + NFC.
    #[tokio::test]
    async fn wasm_dynamic_router_routes_over_a_passed_in_table() {
        use crate::codec::{
            HttpRequest, Method, RouteFrame, decode_decision, encode_request, encode_route_query,
            encode_route_table,
        };
        use cdz_platform::{
            ContractId, HostId, Message, Origin, Outcome, ProgramHash, ReducerId, ReducerKind,
            SpawnContext,
        };

        let (Ok(guest_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_DYNAMIC_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "wasm_dynamic_router_routes_over_a_passed_in_table: \
                 CDZ_HTTP_ROUTER_DYNAMIC_WASM/RUNTIME_WASM/NFC_WASM unset — skipping"
            );
            return;
        };
        let guest = std::fs::read(&guest_path).expect("read dynamic router guest wasm");

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

        let echo_handler = bytes::Bytes::from_static(b"cdz-http.handler.echo............");
        let req_contract = bytes::Bytes::from_static(b"cdz-platform.http.request........");
        let request = |m: Method, p: &str| {
            encode_request(&HttpRequest {
                method: m,
                path: p.to_string(),
                query: String::new(),
                headers: vec![],
                body: bytes::Bytes::new(),
            })
        };

        // Route one (request, table) query through a fresh instance (a one-shot fold → Break decision).
        async fn route(
            store: &dyn ProgramStore,
            program: ProgramHash,
            id: &[u8],
            query: bytes::Bytes,
        ) -> crate::codec::RouteDecision {
            let mut r = store
                .spawn(
                    program,
                    SpawnContext {
                        id: ReducerId::of(id),
                        kind: ReducerKind::Ordinary,
                        limits: None,
                    },
                )
                .await
                .expect("dynamic router instantiates");
            let (_reqs, outcome) = r
                .on_message(Message {
                    id: ContractId::of(b"cdz-platform.http.route-query"),
                    payload: query,
                    from: Origin {
                        reducer: ReducerId::of(b"gateway"),
                        host: HostId::of(b"edge-host"),
                    },
                    continuation_token: bytes::Bytes::new(),
                })
                .await;
            let Outcome::Break { reason, .. } = outcome else {
                panic!("the router must Break with a routing decision");
            };
            decode_decision(&reason).expect("decision decodes")
        }

        // A table routing GET /echo → the echo handler.
        let table = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/echo".to_string(),
            handler: echo_handler.clone(),
            contract: req_contract.clone(),
        }]);

        // GET /echo → matched to the echo handler + its contract.
        let d = route(
            store.as_ref(),
            program,
            b"q1",
            encode_route_query(&request(Method::Get, "/echo"), &table),
        )
        .await;
        assert_eq!(d.handler, echo_handler, "the live table routes GET /echo");
        assert_eq!(d.contract, req_contract);

        // GET /nope → no route in the table → the no-match sentinel.
        let d = route(
            store.as_ref(),
            program,
            b"q2",
            encode_route_query(&request(Method::Get, "/nope"), &table),
        )
        .await;
        assert!(!d.is_match(), "an unlisted path is the no-match sentinel");

        // POST /echo → method mismatch (table has GET /echo only) → no match.
        let d = route(
            store.as_ref(),
            program,
            b"q3",
            encode_route_query(&request(Method::Post, "/echo"), &table),
        )
        .await;
        assert!(!d.is_match(), "POST /echo does not match GET /echo");

        // LIVE TABLE: a DIFFERENT table (routing GET /v2) → the SAME guest routes by whatever it's handed.
        let v2_handler = bytes::Bytes::from_static(b"cdz-http.handler.v2..............");
        let table2 = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/v2".to_string(),
            handler: v2_handler.clone(),
            contract: req_contract.clone(),
        }]);
        let d = route(
            store.as_ref(),
            program,
            b"q4",
            encode_route_query(&request(Method::Get, "/v2"), &table2),
        )
        .await;
        assert_eq!(
            d.handler, v2_handler,
            "routing is a pure fn of the passed-in table — a new table routes anew"
        );
    }

    /// THE P3 CAPSTONE (DESIGN-http-outpost.md §3/§4): the WHOLE control-driven chain over a real socket with
    /// real guests — a control server ships a route table over ws → the gateway DIALS it (`control_link`) →
    /// builds a [`DynamicRouter`](crate::gateway::DynamicRouter) over the fetched table → a real client GET is
    /// routed by the router-dynamic GUEST (folding the shipped table) to the bound handler, which the gateway
    /// spawns → its response rides back to the socket. Nothing is baked: the route table comes off the wire,
    /// routing is a wasm fold, and the handler is content-addressed. (Handler bytes are bound to the shipped
    /// marker here; fetching them by hash from the CAS over the link is the next slice.) Skips when env unset.
    #[tokio::test]
    async fn control_shipped_table_routes_a_real_handler_over_a_socket() {
        use crate::codec::{Method, RouteFrame, encode_route_table};
        use crate::control_link::fetch_route_table;
        use crate::edge::HttpEdge;
        use crate::gateway::{DynamicRouter, Gateway};
        use crate::runner::HandlerRunner;
        use cdz_platform::{ContractId, HostId, ProgramHash, ReducerId};
        use futures_util::SinkExt;
        use http_body_util::{BodyExt, Empty};
        use hyper::Request;
        use hyper_util::rt::TokioIo;
        use std::collections::HashMap;
        use tokio_tungstenite::tungstenite::Message;

        let (Ok(router_path), Ok(poc_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_DYNAMIC_WASM"),
            std::env::var("CDZ_HTTP_POC_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "control_shipped_table_routes_a_real_handler_over_a_socket: \
                 CDZ_HTTP_ROUTER_DYNAMIC_WASM/POC_WASM/RUNTIME_WASM/NFC_WASM unset — skipping"
            );
            return;
        };
        let router = std::fs::read(&router_path).expect("read dynamic router guest");
        let poc = std::fs::read(&poc_path).expect("read PoC handler");

        // One CAS holds the router guest, the PoC handler, and the value-heap runtime + NFC.
        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(router.clone())).await;
        cas.put(bytes::Bytes::from(poc.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let router_program = ProgramHash::of(&router);
        let poc_program = ProgramHash::of(&poc);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        // The route table the control server ships: GET / → the `root` handler marker.
        let root_marker = bytes::Bytes::from_static(b"cdz-http.handler.root............");
        let frame = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: root_marker.clone(),
            contract: bytes::Bytes::from_static(b"cdz-platform.http.request........"),
        }]);

        // A stub ws control server: on connect, push the route-table frame (control server §3).
        let ctl = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind control");
        let ctl_addr = ctl.local_addr().expect("ctl addr");
        let frame_for_server = frame.clone();
        tokio::spawn(async move {
            let (s, _) = ctl.accept().await.expect("ctl accept");
            let mut ws = tokio_tungstenite::accept_async(s)
                .await
                .expect("ctl handshake");
            let _ = ws.send(Message::Binary(frame_for_server.to_vec())).await;
            let _ = ws.close(None).await;
        });

        // The gateway dials the control server and builds a DynamicRouter over the fetched table.
        let frames = fetch_route_table(ctl_addr)
            .await
            .expect("control server ships a route table");
        let mut handlers = HashMap::new();
        handlers.insert(root_marker, poc_program); // bind the shipped marker → the real PoC handler
        let dynamic = DynamicRouter::new(
            router_program,
            ContractId::of(b"cdz-platform.http.route-query"),
            HostId::of(b"edge-host"),
            ReducerId::of(b"gateway"),
            handlers,
            encode_route_table(&frames),
        );
        let gateway = Gateway::with_dynamic_router(
            dynamic,
            HandlerRunner::new(HostId::of(b"edge-host"), ReducerId::of(b"gateway")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, store));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        async fn get(addr: std::net::SocketAddr, path: &str) -> (u16, bytes::Bytes) {
            let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake::<_, Empty<bytes::Bytes>>(TokioIo::new(
                    stream,
                ))
                .await
                .expect("handshake");
            tokio::spawn(async move {
                let _ = conn.await;
            });
            let resp = sender
                .send_request(
                    Request::builder()
                        .method(hyper::Method::GET)
                        .uri(path)
                        .header("host", "test")
                        .body(Empty::<bytes::Bytes>::new())
                        .expect("request"),
                )
                .await
                .expect("send");
            let status = resp.status().as_u16();
            let body = resp.into_body().collect().await.expect("body").to_bytes();
            (status, body)
        }

        // GET / : the control-shipped table routes it (via the router guest) to the PoC handler → 200.
        let (status, body) = get(addr, "/").await;
        assert_eq!(
            status, 200,
            "the control-shipped route reaches the wasm handler"
        );
        assert_eq!(
            body,
            bytes::Bytes::from_static(b"hello from a wasm handler")
        );

        // GET /nope : not in the shipped table → the router's no-match → the gateway's 404 floor.
        let (status, _) = get(addr, "/nope").await;
        assert_eq!(status, 404, "an unrouted path is a 404 floor");
    }

    /// THE P3 LIVE-UPDATE CAPSTONE (DESIGN-http-outpost.md §3): a route-table UPDATE re-routes a RUNNING
    /// server, end to end. A control server seeds a table (GET / → the PoC handler) then LATER pushes a new
    /// table (GET / → the ECHO handler); a persistent `run_control_link` task folds each into the gateway's
    /// live table cell — so the SAME `GET /`, over the same server, first returns the PoC body and then the
    /// echo body, with no restart. Proves control-shipped live routing composes over the real edge + guest.
    #[tokio::test]
    async fn a_live_route_table_update_reroutes_a_running_server() {
        use crate::codec::{Method, RouteFrame, encode_route_table};
        use crate::control_link::run_control_link;
        use crate::edge::HttpEdge;
        use crate::gateway::{DynamicRouter, Gateway};
        use crate::runner::HandlerRunner;
        use cdz_platform::{ContractId, HostId, ProgramHash, ReducerId};
        use futures_util::SinkExt;
        use http_body_util::{BodyExt, Empty};
        use hyper::Request;
        use hyper_util::rt::TokioIo;
        use std::collections::HashMap;
        use tokio_tungstenite::tungstenite::Message;

        let (Ok(router_path), Ok(poc_path), Ok(echo_path), Ok(runtime_path), Ok(nfc_path)) = (
            std::env::var("CDZ_HTTP_ROUTER_DYNAMIC_WASM"),
            std::env::var("CDZ_HTTP_POC_WASM"),
            std::env::var("CDZ_HTTP_ECHO_WASM"),
            std::env::var("CDZ_HTTP_RUNTIME_WASM"),
            std::env::var("CDZ_HTTP_NFC_WASM"),
        ) else {
            eprintln!(
                "a_live_route_table_update_reroutes_a_running_server: \
                 CDZ_HTTP_ROUTER_DYNAMIC_WASM/POC_WASM/ECHO_WASM/RUNTIME_WASM/NFC_WASM unset — skipping"
            );
            return;
        };
        let router = std::fs::read(&router_path).expect("read dynamic router");
        let poc = std::fs::read(&poc_path).expect("read PoC handler");
        let echo = std::fs::read(&echo_path).expect("read echo handler");

        let mut cas = InMemoryBlobStore::new();
        for dep in [&runtime_path, &nfc_path] {
            cas.put(bytes::Bytes::from(
                std::fs::read(dep).expect("read dep component"),
            ))
            .await;
        }
        cas.put(bytes::Bytes::from(router.clone())).await;
        cas.put(bytes::Bytes::from(poc.clone())).await;
        cas.put(bytes::Bytes::from(echo.clone())).await;
        let cas: Arc<dyn BlobStore> = Arc::new(cas);
        let router_program = ProgramHash::of(&router);
        let store: Arc<dyn ProgramStore> =
            Arc::new(wasm_store(Arc::clone(&cas)).expect("wasm store"));
        let _ticker = spawn_epoch_ticker(store.as_ref());

        let poc_marker = bytes::Bytes::from_static(b"cdz-http.handler.root............");
        let echo_marker = bytes::Bytes::from_static(b"cdz-http.handler.echo............");
        let req_c = bytes::Bytes::from_static(b"cdz-platform.http.request........");
        let table_poc = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: poc_marker.clone(),
            contract: req_c.clone(),
        }]);
        let table_echo = encode_route_table(&[RouteFrame {
            method: Method::Get,
            path: "/".to_string(),
            handler: echo_marker.clone(),
            contract: req_c.clone(),
        }]);

        // A stub control server: push table_poc on connect, then AWAIT a signal to push table_echo (a live
        // update mid-run), then close.
        let ctl = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind control");
        let ctl_addr = ctl.local_addr().expect("ctl addr");
        let (push_echo_tx, push_echo_rx) = tokio::sync::oneshot::channel::<()>();
        let (tp, te) = (table_poc.clone(), table_echo.clone());
        tokio::spawn(async move {
            let (s, _) = ctl.accept().await.expect("ctl accept");
            let mut ws = tokio_tungstenite::accept_async(s)
                .await
                .expect("ctl handshake");
            let _ = ws.send(Message::Binary(tp.to_vec())).await;
            let _ = push_echo_rx.await; // wait for the test to request the live update
            let _ = ws.send(Message::Binary(te.to_vec())).await;
            let _ = ws.close(None).await;
        });

        // Build the dynamic-router edge with an EMPTY initial table; the control link seeds + updates it.
        let mut handlers = HashMap::new();
        handlers.insert(poc_marker, ProgramHash::of(&poc));
        handlers.insert(echo_marker, ProgramHash::of(&echo));
        let dynamic = DynamicRouter::new(
            router_program,
            ContractId::of(b"cdz-platform.http.route-query"),
            HostId::of(b"edge-host"),
            ReducerId::of(b"gateway"),
            handlers,
            bytes::Bytes::new(),
        );
        let cell = dynamic.table_cell();
        let gateway = Gateway::with_dynamic_router(
            dynamic,
            HandlerRunner::new(HostId::of(b"edge-host"), ReducerId::of(b"gateway")),
        );
        let edge = Arc::new(HttpEdge::new(gateway, store));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(edge.serve(listener));

        // The persistent control link seeds + live-updates the gateway's route table.
        tokio::spawn(run_control_link(ctl_addr, cell.clone()));

        // Deterministically wait until the live cell holds `expected` (no sleep-based race).
        async fn await_table(cell: &crate::gateway::RouteTableCell, expected: &bytes::Bytes) {
            for _ in 0..2000 {
                if *cell.lock().expect("lock") == *expected {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("the live route table never reached the expected frame");
        }
        async fn get(addr: std::net::SocketAddr, path: &str) -> (u16, bytes::Bytes) {
            let stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
            let (mut sender, conn) =
                hyper::client::conn::http1::handshake::<_, Empty<bytes::Bytes>>(TokioIo::new(
                    stream,
                ))
                .await
                .expect("handshake");
            tokio::spawn(async move {
                let _ = conn.await;
            });
            let resp = sender
                .send_request(
                    Request::builder()
                        .method(hyper::Method::GET)
                        .uri(path)
                        .header("host", "test")
                        .body(Empty::<bytes::Bytes>::new())
                        .expect("request"),
                )
                .await
                .expect("send");
            let status = resp.status().as_u16();
            let body = resp.into_body().collect().await.expect("body").to_bytes();
            (status, body)
        }

        // Phase 1: the control link seeded table_poc → GET / reaches the PoC handler.
        await_table(&cell, &table_poc).await;
        let (status, body) = get(addr, "/").await;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            bytes::Bytes::from_static(b"hello from a wasm handler"),
            "before the update, GET / routes to the PoC handler"
        );

        // Phase 2: trigger the live update; once the cell holds table_echo, the SAME GET / re-routes to the
        // echo handler — no restart.
        let _ = push_echo_tx.send(());
        await_table(&cell, &table_echo).await;
        let (status, body) = get(addr, "/").await;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            bytes::Bytes::from_static(b"method=GET"),
            "after the live update, the same GET / routes to the echo handler"
        );
    }
}
