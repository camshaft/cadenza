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

/// A kinded byte artifact crossing the tool boundary. The canonical program input is the artifact of
/// `kind == "ast"`; a derived WebAssembly component is `kind == "component"`; other backends tag their
/// own kinds. A kind the tool does not recognize is a diagnostic, not a silent drop.
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

/// A diagnostic's severity. An error denies the artifact; a warning rides alongside a produced one —
/// the distinction is per-diagnostic, not which arm of a union was taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

/// A machine-readable diagnostic: severity + a stable code (or `None` for an uncoded decline) + a
/// human message + the AST NODE INDEX it is about (for source mapping).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The stable `CDZ####` code, or `None` for an uncoded decline (a not-yet-supported construct).
    pub code: Option<String>,
    pub message: String,
    /// The AST node index (`StructId.0`) this diagnostic is about, or `None` if unanchored. The
    /// compiler emits only the node IDENTITY, never a source position — the consumer (which parsed the
    /// text and holds the span table keyed by this same index) maps it to a text region
    /// (`query-engine.md` §Provenance Is Recovered By Back-Reference). This keeps the compiler
    /// span-free and its Cadenza port unburdened by source-position plumbing.
    pub node: Option<u32>,
}

impl Diagnostic {
    /// Build an error diagnostic from a produced "no" (a reject carries a code; a decline does not),
    /// carrying its origin node index for the consumer to resolve to a span.
    pub fn from_reject(reject: &crate::diag::Reject) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: reject.code.map(|c| c.code().to_string()),
            message: reject.message.clone(),
            node: reject.at.map(|id| id.0),
        }
    }

    /// Build a WARNING diagnostic — a non-error that rides alongside a produced artifact (it does not
    /// deny the component; `has_error` ignores it). The compiler emits these for a program that is
    /// well-formed but suspect, e.g. a provably-trapping computation eliminated because its value is
    /// unobserved (`core-semantics.md` §A Trap Occurs Only Where Its Computation Is Observed).
    pub fn warning(
        code: crate::diag::Code,
        message: impl Into<String>,
        node: Option<crate::ast::StructId>,
    ) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            code: Some(code.code().to_string()),
            message: message.into(),
            node: node.map(|id| id.0),
        }
    }
}

/// The output of a compilation: the produced artifacts and the always-live diagnostics channel.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CompileOutput {
    pub artifacts: Vec<Artifact>,
    pub diagnostics: Vec<Diagnostic>,
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
