//! The wasmtime-backed program store (`host` feature) — how the gateway turns a control-shipped
//! `ProgramHash` into a running **wasm** reducer (`DESIGN-http-outpost-drive-contract.md` §1/§4).
//!
//! [`build_store`] wraps the control-supplied HTTP CAS ([`HttpBlobStore`]) in a [`WasmProgramStore`]: it
//! fetches a program's wasm component from the CAS by hash and instantiates it on the platform's wasmtime
//! engine, so `store.spawn(hash, ctx)` yields a real [`Reducer`](cdz_platform::Reducer) the gateway drives
//! over its mailbox loop. Each spawned reducer gets its own in-memory `blobs`/`kv` scratch and shares one
//! in-memory routing graph — the gateway drives ONE reducer per connection through its own mailbox +
//! [`GatewayResolver`](crate::resolver::GatewayResolver), not the node's routing substrate, so bare
//! in-memory backends suffice (no node-wide graph to mutate).
//!
//! Gated behind `host` (⇒ `cdz-platform/host` + wasmtime) so the codec/edge spine still builds
//! wasmtime-free; the `cdz-http-gateway` binary always enables it (its `[[bin]]` requires `host`).

use crate::HttpBlobStore;
use cdz_platform::{
    BlobStore, InMemoryKvStore, InMemoryReducerGraph, KvStore, ProgramStore, ReducerGraph,
    ReducerId, WasmProgramStore,
};
use std::sync::Arc;

/// The wasm program store plus a read-capable handle to the same CAS (the [`build_store`] return pair).
/// Aliased to keep the signature under clippy's `type_complexity` bar.
type StoreHandles = (Arc<dyn ProgramStore>, Arc<dyn BlobStore>);

/// Build a wasm program store over the HTTP CAS at `cas_url`, plus a read-capable handle to that same CAS.
/// The returned store instantiates a content-addressed wasm reducer per `spawn`; the returned [`BlobStore`] is
/// how the edge resolves a `CasRef` response body (§6) — a handler answers with a blob hash and the gateway
/// fetches it here. Each reducer's guest `blobs` import is backed by the shared WRITE-CAPABLE CAS (so
/// `blobs.put` persists — e.g. a compile route publishing a component), while `kv` and the reducer graph stay
/// in-memory scratch. `cas_credential` authenticates both reads and writes.
///
/// # Errors
/// Returns the [`wasmtime::Error`] if the wasm engine/linkers cannot be built.
pub fn build_store(cas_url: &str, cas_credential: &[u8]) -> Result<StoreHandles, wasmtime::Error> {
    let cas: Arc<dyn BlobStore> = Arc::new(
        HttpBlobStore::new(cas_url)
            .with_read_credential(String::from_utf8_lossy(cas_credential).into_owned()),
    );
    // A read-capable handle to the same CAS the edge keeps for `CasRef` body resolution (§6).
    let cas_bodies = Arc::clone(&cas);
    // The guest `blobs` import is backed by the SHARED, WRITE-CAPABLE CAS (not per-reducer scratch), so a
    // handler's `blobs.put` actually persists — e.g. a compile route that runs the parser + `rcdzc` via the
    // `run` import and publishes the compiled component, whose returned `ProgramHash` must then resolve for
    // anyone. Reads are open; writes use the config's (write-capable) credential. `HttpBlobStore` is `Clone`
    // (one pooled connection shared across every reducer's handle).
    let cas_blobs = HttpBlobStore::new(cas_url)
        .with_write_credential(String::from_utf8_lossy(cas_credential).into_owned());
    let make_blobs: Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync> =
        Arc::new(move |_id| Box::new(cas_blobs.clone()) as Box<dyn BlobStore>);
    // Per-reducer `kv` stays in-memory scratch (a driven program keeps its own transient state; nothing
    // node-wide), as does the routing graph below.
    let make_kv: Arc<dyn Fn(ReducerId) -> Box<dyn KvStore> + Send + Sync> =
        Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>);
    // One shared routing graph: the gateway drives a single reducer per connection over its own mailbox +
    // effect resolver, so no reducer needs the node's live routing substrate — a bare shared graph is enough.
    let graph: Arc<dyn ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
    let make_graph: Arc<dyn Fn(ReducerId) -> Arc<dyn ReducerGraph> + Send + Sync> =
        Arc::new(move |_id| Arc::clone(&graph));

    let store = WasmProgramStore::new(cas, make_blobs, make_kv, make_graph)?;
    Ok((Arc::new(store), cas_bodies))
}
