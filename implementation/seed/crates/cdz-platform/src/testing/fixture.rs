//! The reducer-echo guest fixture, resolved from the content-addressed store.
//!
//! The reducer-echo guest is a minimal wasm reducer *component* the host driver instantiates to prove
//! itself against a real component rather than a mock. Per the operator ruling there is no committed
//! `.wasm`: the component is built reproducibly by the flake (`nix build .#reducer-echo`) and by
//! `cargo xtask build`, and materialized into the content-addressed store — one `<store>/<blake3-hex>.wasm`
//! file plus a `reducer_echo = "<hash>"` line in the store's `runtime.toml`. The host-driver end-to-end
//! test therefore loads the component *by that hash* instead of `include_bytes!`-ing a checked-in binary.
//!
//! This mirrors `cdz-run`'s store-resolution shape: a line-based `runtime.toml` lookup followed by a
//! content-address re-verify (the store filename is the blake3 hex of the bytes, so a corrupt or
//! substituted entry can't load silently).

use crate::Bytes;
use std::path::PathBuf;

/// The reducer-echo guest component's bytes, resolved from the content-addressed store.
///
/// Returns `None` — the caller *skips* (the same way the store-dependent heap tests self-skip) — when the
/// guest can't be resolved for a benign reason: no store configured, or a store that predates the
/// `reducer_echo` entry (e.g. one built before the reproducible-guest flake change is present). Once the
/// `reducer_echo = "<hash>"` line IS in the manifest, the component itself must resolve: a missing or
/// content-address-mismatched `<hash>.wasm` is a corrupt store and **panics**. The `--features host` CI job
/// exports `CDZ_STORE=<the nix component store>` (which carries the entry), so it runs non-vacuously.
/// Refresh the store with `cargo xtask build` (or point `CDZ_STORE` at `nix build .#store`).
#[must_use]
pub fn reducer_echo_component_bytes() -> Option<Bytes> {
    let store = store_dir()?;
    // Benign misses (no manifest, or no `reducer_echo` line yet) → `None` → the caller skips.
    let manifest = std::fs::read_to_string(store.join("runtime.toml")).ok()?;
    let hash = manifest.lines().find_map(|line| {
        line.trim()
            .strip_prefix("reducer_echo")
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .map(|value| value.trim().trim_matches('"').to_owned())
    })?;
    // The entry names a hash, so the component must be present + intact — otherwise the store is corrupt.
    let bytes = std::fs::read(store.join(format!("{hash}.wasm")))
        .expect("the reducer-echo component is present in the store under its content hash");
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        hash,
        "reducer-echo store entry has the wrong content address (corrupt store)",
    );
    Some(Bytes::from(bytes))
}

/// The store directory: `CDZ_STORE` when set, otherwise the compiled-default `<repo>/target/cadenza-store`
/// but only when it actually holds a `runtime.toml` (absent → `None`, so the caller skips rather than
/// fails). Same location + default as `cdz-run`'s store resolution.
fn store_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CDZ_STORE") {
        return Some(PathBuf::from(dir));
    }
    // <crate>/../../../.. == the repo root: implementation/seed/crates/cdz-platform → repo.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)?
        .to_path_buf();
    let store = repo.join("target/cadenza-store");
    store.join("runtime.toml").is_file().then_some(store)
}

#[cfg(test)]
mod tests {
    /// With a store configured — locally after `cargo xtask build`, or the flake's host-feature job with
    /// `CDZ_STORE=<component store>` — the reducer-echo guest resolves, its content address re-verifies (the
    /// assert inside the loader), and its bytes are a wasm binary. With no store the loader returns `None`
    /// and the test skips (the storeless per-crate gate runs this way).
    #[test]
    fn reducer_echo_resolves_and_verifies_from_the_store() {
        let Some(bytes) = super::reducer_echo_component_bytes() else {
            return;
        };
        assert!(bytes.len() > 8, "the component is non-empty");
        assert_eq!(&bytes[..4], b"\0asm", "wasm magic number");
    }
}
