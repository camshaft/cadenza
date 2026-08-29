//! The build-tool ABI — kinded artifacts in, {artifacts, diagnostics} out.
//!
//! Compilation is artifacts-in, artifacts-out, with an always-live diagnostics channel
//! (`build-tool-interface.md`): the derived component is not a privileged return value — it is the
//! artifact of `kind == "component"`. Success = the requested artifact is present and no diagnostic
//! has error severity; failure = the artifact is absent and ≥1 error diagnostic. This is the same
//! interface the eventual self-hosted compiler exports, so every toolchain entry speaks one
//! vocabulary. A backend tags its artifact by kind (`backends-and-targets.md` §The Emitted Artifact
//! Is Self-Describing By Kind), so a consumer distinguishes outputs by kind rather than by inspecting
//! bytes.

// The whole compile-BOUNDARY vocabulary now lives in the shared `cadenza-compile-abi` crate (so `cdz`
// can speak it in a `!standalone` build without linking `rcdzc`); this module just RE-EXPORTS it so
// `crate::{Artifact, Diagnostic, CompileOutput, …}` / `rcdzc::{…}` stay byte-stable for every internal +
// sidecar consumer. The kinded `Artifact`, the `Diagnostic` cluster (Diagnostic/DiagnosticFix/Severity/
// FixKind/WRAP_HOLE + `wrap_prefix_suffix`), and the `{artifacts, diagnostics}` `CompileOutput` all moved.
// The rcdzc-internal `diag::{Reject, Fix, Code}` -> boundary-type conversions are FREE FNS in
// `crate::abi_bridge` (the orphan rule: those boundary types are foreign to `rcdzc` now). `CompileOutput`'s
// `cse_partition_core_eq_calls` metric is now an always-present `u64` (was `#[cfg(test)]`) — a
// cross-crate `#[cfg(test)]` field can't be set from `rcdzc`'s tests, so it is always-present + `0`
// outside the emit path; `rcdzc`'s CSE regression test reads it unchanged.
pub use cadenza_compile_abi::abi::Artifact;
pub use cadenza_compile_abi::{
    CompileOutput, Diagnostic, DiagnosticFix, FixKind, Severity, WRAP_HOLE, wrap_prefix_suffix,
};
