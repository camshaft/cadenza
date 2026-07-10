//! The build-tool ABI — kinded artifacts in, {artifacts, diagnostics} out.
//!
//! `build-tool-interface.md` (Amendment 0.8.0; learning
//! `2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result`):
//! compilation is **artifacts-in, artifacts-out, with a diagnostics channel that is always live**.
//! The derived component is not a privileged return value — it is the artifact of `kind ==
//! "component"`. Success = a component artifact is present and no diagnostic has error severity;
//! failure = no component artifact and ≥1 error diagnostic. This is the SAME interface the eventual
//! self-hosted compiler exports, so every entry in the toolchain speaks one vocabulary.
//!
//! The old crate realizes only the degenerate `list<u8> -> result<list<u8>, list<diagnostic>>` case;
//! rcdzc uses the general artifact interface from the start (`compile`), and offers the degenerate
//! `compile_bytes` convenience derived from it for the gates that hand in a lone AST.

/// A kinded byte artifact crossing the tool boundary. The canonical source tree is the artifact of
/// `kind == "ast"` (or `"source"`); the derived module is `kind == "component"`; sidecar outputs
/// (DWARF, source map, manifest) are their own kinds. A kind the tool does not recognize is a
/// diagnostic, not a silent drop (reject-don't-miscompile).
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub kind: String,
    pub bytes: Vec<u8>,
}

impl Artifact {
    pub fn new(kind: impl Into<String>, bytes: Vec<u8>) -> Artifact {
        Artifact {
            kind: kind.into(),
            bytes,
        }
    }
    /// The canonical-binary-AST input artifact.
    pub const KIND_AST: &'static str = "ast";
    /// The derived WebAssembly-component output artifact.
    pub const KIND_COMPONENT: &'static str = "component";
}

/// A diagnostic's severity. An error denies the component; a warning rides alongside a produced one.
/// The error-vs-warning distinction lives HERE (per-diagnostic), not in which arm of a union was
/// taken — so a derivation can both succeed and report warnings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A machine-readable diagnostic (constitution XI): severity + a stable code + a human message. No
/// span yet — the compiler works on the canonical AST, which carries no source text.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The stable CDZ code, or `None` for an uncoded decline (a not-yet-supported construct).
    pub code: Option<String>,
    pub message: String,
}

/// The output of a compilation: the produced artifacts and the always-live diagnostics channel.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileOutput {
    pub artifacts: Vec<Artifact>,
    pub diagnostics: Vec<Diagnostic>,
}

impl CompileOutput {
    /// The produced component's bytes, if a `component` artifact is present AND no diagnostic has
    /// error severity (the success predicate). `None` otherwise — a failed or diagnostic-only run.
    pub fn component(&self) -> Option<&[u8]> {
        if self
            .diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
        {
            return None;
        }
        self.artifacts
            .iter()
            .find(|a| a.kind == Artifact::KIND_COMPONENT)
            .map(|a| a.bytes.as_slice())
    }
}
