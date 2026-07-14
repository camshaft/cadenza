//! The host layer that runs a compiled component — resolve the value-heap runtime from the
//! content-addressed store (only if the component records one), then invoke the entry via `cdz_run`.
//!
//! Mirrors `cdz-run`'s own store resolution (`crates/cdz-run/src/main.rs`): a scalar/const component
//! imports nothing and needs no runtime, so a missing store is not an error there; a component that
//! DOES require a runtime resolves it BY CONTENT ADDRESS — the exact hash the component records must be
//! present, never a substitution. The `CADENZA_STORE` env var overrides the default store location (the
//! same knob `xtask` sets), so a REPL launched outside the repo can point at a populated store.

use std::path::PathBuf;

/// Run a compiled component's entry, resolving the value-heap runtime by content address if required.
/// Returns the rendered outcome (`Value`/`Trap`) or an error (a missing/mismatched runtime, an invalid
/// component). The entry is the sole export (`cdz-repl-eval`), selected by `cdz_run`'s default.
pub fn run_component(component_bytes: &[u8]) -> anyhow::Result<cdz_run::Outcome> {
    // Resolve the runtime ONLY if the component records a requirement (a scalar result needs none).
    let runtime = match cdz_run::required_runtime(component_bytes)? {
        Some(req) => Some(resolve_runtime(&req)?),
        None => None,
    };
    // Cache the compiled runtime in the store dir (as `cdz-run` does) — byte-identical across programs,
    // so a REPL session compiles it once then deserializes it on every later heap-valued line.
    let runtime_cache_dir = if runtime.is_some() {
        Some(store_dir())
    } else {
        None
    };
    let opts = cdz_run::RunOpts {
        runtime,
        runtime_cache_dir,
        ..Default::default()
    };
    cdz_run::run(component_bytes, &opts)
}

/// Resolve the runtime bytes the component requires, BY CONTENT ADDRESS, from the store. Refuses if the
/// exact hash is absent (never substitutes a different runtime — the component-abi content-address
/// contract), pointing at `cargo xtask build` to populate it.
fn resolve_runtime(req: &cdz_run::RuntimeReq) -> anyhow::Result<Vec<u8>> {
    if req.hash.is_empty() {
        anyhow::bail!(
            "the program imports the value-heap runtime but records no content address to resolve it by"
        );
    }
    let store = store_dir();
    let path = store.join(format!("{}.wasm", req.hash));
    if !path.exists() {
        anyhow::bail!(
            "no runtime of content address {} in the store at {} — build it with `cargo xtask build` \
             (or set CADENZA_STORE to a populated store)",
            req.hash,
            store.display()
        );
    }
    std::fs::read(&path)
        .map_err(|e| anyhow::anyhow!("reading stored runtime {}: {e}", path.display()))
}

/// The content-addressed store directory: `CADENZA_STORE` if set, else `<repo>/target/cadenza-store`
/// resolved from this crate's manifest (crate lives at `<repo>/implementation/seed/crates/cdz-calc`).
fn store_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CADENZA_STORE") {
        return PathBuf::from(dir);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // <repo>/implementation/seed/crates/cdz-calc → up 4 → <repo>
    let repo = manifest
        .ancestors()
        .nth(4)
        .unwrap_or(&manifest)
        .to_path_buf();
    repo.join("target/cadenza-store")
}
