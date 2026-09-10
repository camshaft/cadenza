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
}
