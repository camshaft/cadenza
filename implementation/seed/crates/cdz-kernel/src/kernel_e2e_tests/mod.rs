//! In-crate reducer / kernel end-to-end test suites.
//!
//! These exercise the kernel's public API (`crate::kernel`, `reducer`, `wasm_host`, `executor`, …) over
//! the whole fold/replay/component path. They were originally `tests/*.rs` CARGO INTEGRATION tests (each
//! a SEPARATE test binary = an extra full crate link), but the standing directive is NO integration
//! tests: they carry no subprocess and need no separate binary, so they live here as ordinary
//! `#[cfg(test)]` in-crate units (compiled with the lib, reachable via `crate::` paths). Coverage is
//! unchanged from the integration form.
//!
//! The four wasm-component suites remain gated on their v-nix fixture env vars (`REDUCER_GUEST_COMPONENT`,
//! `REDUCER_CADENZA_COMPONENT` / `_B2_` / `_B3_`, `RUNTIME_HEAP_COMPONENT`, `CDZ_STORE`) — unset → skip
//! cleanly, exactly as before. All fixture references are env-var-based (no `tests/`-relative or
//! manifest-relative path joins), so the relocation to `src/` changes nothing about how they resolve.
//! The `tests/fixtures/` guest-component sources stay where they are (they are guest crates, not test
//! binaries).

mod async_component_reducer_e2e;
mod component_reducer_e2e;
mod loop_and_recovery;
mod reducer_cadenza_b1_e2e;
mod reducer_cadenza_b2_e2e;
mod reducer_cadenza_b3_e2e;
mod replay_determinism;
