//! Shared helpers for the `cdz-agent-host` integration tests. A `tests/common/mod.rs` is the idiomatic
//! spot for code shared across integration-test binaries — it is a submodule (not its own test binary),
//! so it compiles once per consuming test and never runs as a standalone test target.

/// Load the lifted Cedar policy component the CI job built, for the e2es gated on a real Cedar decision.
///
/// Returns `None` ONLY when `CEDAR_POLICY_COMPONENT` is UNSET — a plain local `cargo test` without the
/// wasm toolchain, where the caller SKIPS cleanly. When the var IS set the file MUST read: a
/// missing/unreadable/corrupt component is a CI misconfiguration or a broken lift, and swallowing it
/// (`.ok()`) would silently skip the CI-gated e2e + let a broken component pass green (PR#1332). So this
/// PANICS on a read error when the var is set — fail loud, same discipline as the S1 store-guard.
///
/// One source of truth for the skip/read contract, shared by `cedar_authz_e2e` (a real agent gated by a
/// real decision) and `capability_manifest_e2e` (a capability manifest projected against a real decision).
///
/// `#[allow(dead_code)]`: `tests/common/mod.rs` compiles once PER integration-test binary, so a helper used
/// by only some binaries is "unused" in the others — expected for a shared-helpers module, not real dead
/// code (would otherwise trip `-D warnings` in the binaries that don't call it).
#[allow(dead_code)]
pub fn policy_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CEDAR_POLICY_COMPONENT").ok()?; // unset → skip (None)
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("CEDAR_POLICY_COMPONENT is set to {path:?} but the component can't be read: {e}")
    }))
}

/// Load a real WASM REDUCER component (a `wit/reducer.wit` guest) for the e2es that run a resolved
/// program end-to-end — the §4c publish→consume demo blob-fetches this + runs it as an
/// [`AsyncComponentReducer`](cdz_kernel::wasm_host::AsyncComponentReducer). The nix build produces such a
/// component (e.g. rcdzc→wasm of a tiny reducer); a plain `cargo test` doesn't have one.
///
/// Same skip/read contract as [`policy_component_bytes`]: `None` ONLY when `CDZ_LIVE_REDUCER_COMPONENT` is
/// UNSET (caller SKIPS cleanly); when the var IS set the file MUST read (a missing/corrupt component is a
/// misconfig — PANIC, never a silent skip that could pass a broken blob green).
///
/// `#[allow(dead_code)]` for the same reason as [`policy_component_bytes`]: shared across test binaries,
/// used by only some.
#[allow(dead_code)]
pub fn reducer_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CDZ_LIVE_REDUCER_COMPONENT").ok()?; // unset → skip (None)
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("CDZ_LIVE_REDUCER_COMPONENT is set to {path:?} but the component can't be read: {e}")
    }))
}

/// Load the lifted `cadenza:syntax` guest component the CI job built (v-nix `packages.syntax-guest`, wired as
/// `CDZ_SYNTAX_COMPONENT` in flake.nix) — the TARGET component for the signature-query part-1 E2E: the host
/// reflects ITS exported funcs (parse/query/doc) into a `ComponentSignature` and folds it back.
///
/// Same skip/read contract as the siblings: `None` ONLY when `CDZ_SYNTAX_COMPONENT` is UNSET (caller SKIPS
/// cleanly on a plain `cargo test` with no wasm toolchain); when SET the file MUST read (a missing/corrupt
/// component is a misconfig — PANIC, never a silent skip that could pass a broken lift green).
///
/// `#[allow(dead_code)]` for the same shared-across-binaries reason as the siblings.
#[allow(dead_code)]
pub fn syntax_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CDZ_SYNTAX_COMPONENT").ok()?; // unset → skip (None)
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("CDZ_SYNTAX_COMPONENT is set to {path:?} but the component can't be read: {e}")
    }))
}
