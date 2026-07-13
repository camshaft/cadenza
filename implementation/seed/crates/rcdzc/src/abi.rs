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

/// A machine-readable, agent-actionable REPAIR carried by a diagnostic — the ABI projection of a
/// [`crate::diag::Fix`] (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix).
/// Span-free like the diagnostic: the edit names a NODE INDEX the consumer maps to a text region, so an
/// agent applies the structural edit (replace node `node` with `replacement`) directly rather than
/// re-deriving the repair from the message prose.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagnosticFix {
    /// A one-line human label for the edit (`replace with `foo``).
    pub label: String,
    /// The AST node index (`StructId.0`) the edit targets — the node whose surface spelling is replaced.
    pub node: u32,
    /// The surface spelling to put in its place (the suggested name / wrapped form).
    pub replacement: String,
    /// `true` iff the compiler PROVED the fix correct (machine-applicable); `false` for a heuristic an
    /// agent should confirm before applying (`spec/capabilities/diagnostics.md` §An Unconfirmed Fix
    /// Carries An Applicability Marker).
    pub verified: bool,
}

impl DiagnosticFix {
    /// The ABI projection of a compiler-internal [`crate::diag::Fix`] — lower its structural edit to a
    /// node index + replacement spelling and its applicability to the `verified` flag.
    pub fn from_fix(fix: &crate::diag::Fix) -> DiagnosticFix {
        let (node, replacement) = match &fix.edit {
            crate::diag::Edit::ReplaceNode { at, replacement } => (at.0, replacement.clone()),
        };
        DiagnosticFix {
            label: fix.label.clone(),
            node,
            replacement,
            verified: matches!(fix.applicability, crate::diag::Applicability::Verified),
        }
    }
}

/// A machine-readable diagnostic: severity + a stable code (or `None` for an uncoded decline) + a
/// human message + the AST NODE INDEX it is about (for source mapping) + an optional structural fix.
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
    /// A proposed structural repair, if the producer knew one — the "route to a fix" an agent applies
    /// directly. `None` when the compiler has no actionable suggestion.
    pub fix: Option<DiagnosticFix>,
}

impl Diagnostic {
    /// Build an error diagnostic from a produced "no" (a reject carries a code; a decline does not),
    /// carrying its origin node index for the consumer to resolve to a span, and its proposed fix.
    pub fn from_reject(reject: &crate::diag::Reject) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code: reject.code.map(|c| c.code().to_string()),
            message: reject.message.clone(),
            node: reject.at.map(|id| id.0),
            fix: reject.fix.as_ref().map(DiagnosticFix::from_fix),
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
            fix: None,
        }
    }

    /// Attach a proposed structural fix — the fluent form a producer uses when, alongside the
    /// diagnostic, it can name the repair (`Diagnostic::warning(..).with_fix(Fix::replace_verified(..))`).
    pub fn with_fix(mut self, fix: &crate::diag::Fix) -> Diagnostic {
        self.fix = Some(DiagnosticFix::from_fix(fix));
        self
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
