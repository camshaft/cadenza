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
pub fn policy_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CEDAR_POLICY_COMPONENT").ok()?; // unset → skip (None)
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("CEDAR_POLICY_COMPONENT is set to {path:?} but the component can't be read: {e}")
    }))
}
