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
    BlobStore, InMemoryBlobStore, InMemoryKvStore, InMemoryReducerGraph, KvStore, ProgramStore,
    ReducerGraph, ReducerId, WasmProgramStore,
};
use std::sync::Arc;

/// Build a wasm program store over the HTTP CAS at `cas_url` (authenticated with `cas_credential` on reads).
/// The returned store instantiates a content-addressed wasm reducer per `spawn`. Each reducer is handed a
/// fresh in-memory blob + kv scratch and one shared in-memory reducer graph.
///
/// # Errors
/// Returns the [`wasmtime::Error`] if the wasm engine/linkers cannot be built.
pub fn build_store(
    cas_url: &str,
    cas_credential: &[u8],
) -> Result<Arc<dyn ProgramStore>, wasmtime::Error> {
    let cas: Arc<dyn BlobStore> = Arc::new(
        HttpBlobStore::new(cas_url)
            .with_read_credential(String::from_utf8_lossy(cas_credential).into_owned()),
    );
    // Per-reducer scratch: a fresh in-memory blob + kv store each. The gateway's driven programs keep their
    // own state; nothing here is node-wide.
    let make_blobs: Arc<dyn Fn(ReducerId) -> Box<dyn BlobStore> + Send + Sync> =
        Arc::new(|_id| Box::new(InMemoryBlobStore::new()) as Box<dyn BlobStore>);
    let make_kv: Arc<dyn Fn(ReducerId) -> Box<dyn KvStore> + Send + Sync> =
        Arc::new(|_id| Box::new(InMemoryKvStore::new()) as Box<dyn KvStore>);
    // One shared routing graph: the gateway drives a single reducer per connection over its own mailbox +
    // effect resolver, so no reducer needs the node's live routing substrate — a bare shared graph is enough.
    let graph: Arc<dyn ReducerGraph> = Arc::new(InMemoryReducerGraph::new());
    let make_graph: Arc<dyn Fn(ReducerId) -> Arc<dyn ReducerGraph> + Send + Sync> =
        Arc::new(move |_id| Arc::clone(&graph));

    let store = WasmProgramStore::new(cas, make_blobs, make_kv, make_graph)?;
    Ok(Arc::new(store))
}
