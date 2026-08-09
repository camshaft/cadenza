//! Shared `#[cfg(test)]` support for the crate's in-crate tests (relocated from the old
//! `tests/common/mod.rs` when the integration-test binaries were converted to in-crate units under the
//! operator's no-integration-tests mandate). Compiled only under `cfg(test)`; not part of the crate's API.

/// Load a real WASM REDUCER component (a `wit/reducer.wit` guest) for the tests that run a resolved program
/// end to end — the §4c publish→consume arcs blob-fetch this + run it as an
/// [`AsyncComponentReducer`](cdz_kernel::wasm_host::AsyncComponentReducer). The nix build produces such a
/// component (e.g. rcdzc→wasm of a tiny reducer); a plain `cargo test` doesn't have one.
///
/// Skip/read contract: returns `None` ONLY when `CDZ_LIVE_REDUCER_COMPONENT` is UNSET — a plain local
/// `cargo test` without the wasm toolchain, where the caller SKIPS cleanly. When the var IS set the file MUST
/// read: a missing/unreadable/corrupt component is a CI misconfiguration or a broken lift, and swallowing it
/// (`.ok()`) would silently skip the CI-gated test + let a broken component pass green (PR#1332). So this
/// PANICS on a read error when the var is set — fail loud, same discipline as the S1 store-guard.
///
/// `#[allow(dead_code)]`: used by only some in-crate test modules; the support module compiles once for the
/// whole `cfg(test)` build, so a helper not called by every module would otherwise trip `-D warnings`.
#[allow(dead_code)]
pub(crate) fn reducer_component_bytes() -> Option<Vec<u8>> {
    let path = std::env::var("CDZ_LIVE_REDUCER_COMPONENT").ok()?; // unset → skip (None)
    Some(std::fs::read(&path).unwrap_or_else(|e| {
        panic!("CDZ_LIVE_REDUCER_COMPONENT is set to {path:?} but the component can't be read: {e}")
    }))
}
