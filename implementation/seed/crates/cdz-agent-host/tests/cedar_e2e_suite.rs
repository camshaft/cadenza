//! Single integration-test binary aggregating the Cedar-fixture E2E suites.
//!
//! `cedar_authz_e2e`, `capability_manifest_e2e`, and `live_transport_e2e` each drove a separate
//! `tests/*.rs` binary — three full links of `cdz-agent-host` (a heavy async/wasmtime crate) + three
//! codegen cycles per `cargo test`. They share the `tests/common/` Cedar-guest-component helper and no
//! per-binary state, so `mod`-ing them from files under `tests/suite/` (a SUBDIR Cargo does NOT
//! auto-compile as its own binary) collapses the three links into one while keeping every test
//! function + module path + env-gated (`CEDAR_POLICY_COMPONENT`) skip semantics identical.
//!
//! `live_transport_e2e` carries `#![cfg(feature = "live-net")]`, so its submodule compiles only under
//! that feature (unchanged from when it was its own binary). The two files that use the shared helper
//! reach it via `#[path = "../common/mod.rs"] mod common;` (common/ stays put, single-sourced).
//!
//! NOTE (v-agent-harness-host coordination): `policy_swap_e2e.rs` is deliberately NOT aggregated here —
//! it is being deleted by their queued conversion (6a93300a8, replaced with a host.rs unit test), so
//! touching it would conflict. Once that lands there are just these three; if a future Cedar-fixture
//! E2E is added, drop it in `tests/suite/` and `mod` it below — do NOT add a new top-level `tests/*.rs`.

// The shared `tests/common/` helper, declared ONCE for the whole binary — the aggregated files reach it
// via `crate::common::…`. (Declaring it per-file with `#[path]` would load `common/mod.rs` as two
// distinct modules in one binary — clippy's `duplicate_mod`, which the `-D warnings` gate rejects.)
#[path = "common/mod.rs"]
mod common;

mod suite {
    mod capability_manifest_e2e;
    mod cedar_authz_e2e;
    mod live_transport_e2e;
}
