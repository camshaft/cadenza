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

// The kinded byte [`Artifact`] crossing the tool boundary now lives in the shared `cadenza-compile-abi`
// crate (a compile-boundary type: `compile` takes `&[Artifact]` in + returns them out, and `cdz`
// builds/reads them across the delegate boundary). Re-exported here so `crate::Artifact` /
// `rcdzc::Artifact` stay byte-stable and every consumer keeps resolving. The `Diagnostic` cluster +
// `CompileOutput` below stay until the v-inference-paired orphan-rule slice.
pub use cadenza_compile_abi::abi::Artifact;

// The diagnostic-cluster boundary types — Diagnostic / DiagnosticFix / Severity / FixKind / WRAP_HOLE +
// the wrap_prefix_suffix helper — now live in the shared `cadenza-compile-abi` crate. Re-exported here
// so `crate::{Diagnostic, ...}` / `rcdzc::{Diagnostic, ...}` stay byte-stable for every internal +
// sidecar consumer. The rcdzc-internal `diag::{Reject, Fix, Code}` -> boundary-type conversions are
// FREE FNS in `crate::abi_bridge` (the orphan rule: those boundary types are foreign to rcdzc now).
// CompileOutput STAYS here (its `#[cfg(test)]` field can't cross the crate boundary — a dep isn't
// built with the dependent's test cfg); it just carries the re-exported `Diagnostic` + `Artifact`.
pub use cadenza_compile_abi::{
    Diagnostic, DiagnosticFix, FixKind, Severity, WRAP_HOLE, wrap_prefix_suffix,
};

/// The output of a compilation: the produced artifacts and the always-live diagnostics channel. A RECORD
/// pairing a list of kinded output artifacts with a list of diagnostics — two DISTINCT channels, not
/// mutually-exclusive arms: the derived component is one artifact (kind `"component"`) in the list, a
/// debug sidecar another, and a warning rides alongside a produced component. Success/failure is READ
/// from the outputs (`artifact("component")` present + no error) rather than an in-band sentinel.
/// On success the produced artifacts carry the content-addressed component (its runtime import name
/// embeds the content address) alongside the manifest its imports are bound against; on failure the
/// output carries machine-readable `Diagnostic`s (code + span + message), never an opaque error.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The build tool MUST produce, on success, a content-addressed component together with the capability manifest against which its imports are bound.
//= spec/capabilities/agent-authoring.md#every-compiler-output-is-machine-readable
//# The compiler MUST expose the capability manifest it produced in a machine-readable form.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The build tool MUST produce, on failure, machine-readable diagnostics rather than an opaque error.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The build tool's derivation entry MUST return a record pairing a list of kinded output artifacts with a list of diagnostics, so that the byte outputs and the diagnostics are distinct channels rather than mutually exclusive arms of one result.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The derived component MUST be one artifact in the output artifact list, identified by its kind, so that a byte output that is not the component — a debug-information sidecar, a source map, the capability manifest — is another artifact of the same shape rather than a second return type, and the set of output kinds is open to additive extension.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The tool MUST signal a successful derivation by the presence of a component artifact in the output together with the absence of any error-severity diagnostic, and a failed derivation by the absence of a component artifact together with at least one error-severity diagnostic, so that success and failure are read from the produced artifacts and diagnostics rather than from an in-band sentinel such as an empty byte sequence.
//= spec/contracts/build-tool-interface.md#the-tool-produces-a-component-a-manifest-and-diagnostics
//# The tool MUST be able to return diagnostics alongside a produced component, so that a derivation that succeeds while reporting non-error diagnostics — a warning — carries both the component and those diagnostics rather than having to discard one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompileOutput {
    pub artifacts: Vec<Artifact>,
    pub diagnostics: Vec<Diagnostic>,
    /// Test-only: the `Db::cse_partition_core_eq_calls` count from this compile — the within-bucket
    /// `core_eq` comparisons the wasm CSE class-partition made. The counter lives on the `Db` (which the
    /// emit path drops before returning), so it is surfaced here for the regression-guard test
    /// (`a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time`) to read a value from exactly
    /// one compile — a per-`Db` metric rather than a parallel-test-contaminated process-global atomic.
    /// `0` on any construction path that ran no emit (a query-only or early-fail output).
    #[cfg(test)]
    pub cse_partition_core_eq_calls: u64,
}

impl CompileOutput {
    /// Whether any diagnostic is an error (the failure predicate).
    pub fn has_error(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// The bytes of the first artifact of the given kind, if present and no error was reported.
    pub fn artifact(&self, kind: &str) -> Option<&[u8]> {
        if self.has_error() {
            return None;
        }
        self.artifacts
            .iter()
            .find(|a| a.kind == kind)
            .map(|a| a.bytes.as_slice())
    }
}
