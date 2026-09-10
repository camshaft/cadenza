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
}
