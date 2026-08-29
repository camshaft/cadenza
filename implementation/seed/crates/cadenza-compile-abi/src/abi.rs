//! The build-tool ABI boundary types — kinded artifacts in, `{artifacts, diagnostics}` out.
//!
//! So far this module holds the kinded [`Artifact`] (the byte artifact crossing the tool boundary).
//! A later slice moves the `{artifacts, diagnostics}` `CompileOutput` result + the `Diagnostic` cluster
//! here (that one is paired with v-inference's orphan-rule refactor of the `Diagnostic`/`DiagnosticFix`
//! conversions that couple to rcdzc-internal `diag::Reject`/`diag::Fix`).

/// A kinded byte artifact crossing the tool boundary. The canonical program input is the artifact of
/// `kind == "ast"`; a derived WebAssembly component is `kind == "component"`; other backends tag their
/// own kinds. A kind the tool does not recognize is a diagnostic, not a silent drop. `compile` takes a
/// `&[Artifact]` — a LIST of these — so the input channel is an open kinded set (add a source unit, a
/// `spans`/`sidecar` input) without changing the entry's arity, and a consumer selects by kind not
/// position.
//= spec/contracts/build-tool-interface.md#the-tool-s-inputs-are-a-kinded-artifact-list
//# The build tool's derivation entry MUST take its inputs as a list of kinded artifacts, each a named kind paired with its bytes, so that the canonical source tree is one artifact among an open set and the input channel admits further inputs — additional source units of a multi-unit program, a build cache, or a previously derived dependency — without changing the entry's arity.
//= spec/contracts/build-tool-interface.md#the-tool-s-inputs-are-a-kinded-artifact-list
//# The kind of an artifact MUST identify how its bytes are interpreted, so that a consumer selects an input by kind rather than by position, and an input kind the tool does not recognize is reported as a diagnostic rather than silently ignored.
// An `Artifact` carries only `bytes` (with a kind tag) across the tool boundary — the compiler's
// derivation interface takes and returns BYTE SEQUENCES, never a live in-memory toolchain value, so no
// internal representation crosses the boundary:
//= spec/capabilities/self-hosting-surface.md#a-toolchain-s-internal-values-do-not-cross-the-boundary
//# A compiler's derivation interface MUST accept its input and produce its output as byte sequences at the component boundary, so that a toolchain's internal values do not cross it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Artifact {
    pub kind: String,
    pub name: String,
    pub bytes: Vec<u8>,
}

impl Artifact {
    pub fn new(kind: impl Into<String>, name: impl Into<String>, bytes: Vec<u8>) -> Artifact {
        Artifact {
            kind: kind.into(),
            name: name.into(),
            bytes,
        }
    }

    /// The canonical-binary-AST input artifact kind.
    pub const KIND_AST: &'static str = "ast";
}
