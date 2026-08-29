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

/// A diagnostic's severity. An error denies the artifact; a warning rides alongside a produced one —
/// the distinction is per-diagnostic, not which arm of a union was taken. Severity is a SEPARATE field
/// from the diagnostic's kind (reject/decline/trap): a consumer reads failure-ness from `severity`, not
/// from whether the "no" was a rejection or a decline.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-carries-a-severity
//# Every diagnostic the compiler emits MUST carry a severity that distinguishes an error, which denies a produced component, from a non-error such as a warning, which may accompany a produced component, so that a consumer decides from the diagnostic itself whether the outcome it reports is a failure.
///
//= spec/capabilities/diagnostics.md#every-diagnostic-carries-a-severity
//# The severity a diagnostic carries MUST be independent of the diagnostic's kind, so that whether an outcome is a failure is read from the severity rather than inferred from whether the outcome is a rejection, a decline, or a trap.
//= spec/contracts/build-tool-interface.md#a-diagnostic-carries-a-severity-a-code-and-a-message
//# A diagnostic the build tool produces MUST carry a severity that distinguishes an error — one that denies a component artifact — from a non-error such as a warning that accompanies a produced component, so that a consumer decides from the diagnostic itself whether the derivation failed.
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
///
/// The fix AND its verified-or-applicability status (`verified`, from `Fix::applicability`) ride in this
/// machine-readable record, so an agent consumes the route to a compliant program programmatically.
//= spec/capabilities/agent-authoring.md#a-diagnostic-s-fix-is-machine-readable
//# A diagnostic's proposed fix and its verified-or-applicability status MUST be part of the compiler's machine-readable output, so an agent consumes the route to a compliant program programmatically.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DiagnosticFix {
    /// A one-line human label for the edit (`replace with `foo``, `add the missing match arms`).
    pub label: String,
    /// How to apply the edit at `node`: `Replace` swaps the node's surface spelling for `replacement`;
    /// `InsertInto` appends `replacement` (rendered child forms) as new children of the list node.
    pub kind: FixKind,
    /// The AST node index (`StructId.0`) the edit targets — the node replaced, or the list appended into.
    pub node: u32,
    /// The edit's surface payload: for `Replace`, the spelling to put in `node`'s place; for
    /// `InsertInto`, the child form(s) to append (a space-joined list of complete `(…)` s-expressions,
    /// each directly splice-able before the target list's closing paren).
    pub replacement: String,
    /// `true` iff the compiler PROVED the fix correct (machine-applicable); `false` for a heuristic an
    /// agent should confirm before applying (`spec/capabilities/diagnostics.md` §An Unconfirmed Fix
    /// Carries An Applicability Marker).
    pub verified: bool,
}

/// The ellipsis placeholder marking where a `Wrap` fix's ORIGINAL node text goes inside its
/// `replacement` — `(Some …)` means "put `(Some ` before the node's text and `)` after". A single
/// character (U+2026) that does not occur in Cadenza source, so it never collides with real spelling.
pub const WRAP_HOLE: char = '…';

/// Reshape a `Wrap` fix's `replacement` for the target SURFACE and split it into the `(prefix, suffix)` a
/// consumer wraps the original node text with — `prefix + <node text> + suffix`. THE way a machine
/// consumer (the `cdz check --json` fix object, the `cdz-wasm` guide quick-fix) should present a wrap:
/// NEVER hand out the raw `replacement` bearing the [`WRAP_HOLE`] sentinel, because an agent splicing that
/// string over the node's byte range would write a literal `…` and corrupt the source. Splitting on the
/// sentinel here yields the two literal sides instead.
///
/// The compiler renders a wrap in S-EXPR form `(<ctor> …)`; on the ML surface a constructor application is
/// `<ctor>(…)`, not juxtaposition, so `is_ml` first rewrites `(<name> <HOLE>)` → `<name>(<HOLE>)` (only the
/// constructor-wrap shape the fix producers emit; any other shape passes through). Then the surface form
/// splits on the hole. A `replacement` with no hole (should not happen for a real wrap) returns
/// `(whole, "")` — the consumer still applies `whole` as a prefix, degrading safely rather than panicking.
pub fn wrap_prefix_suffix(replacement: &str, is_ml: bool) -> (String, String) {
    let hole = WRAP_HOLE.to_string();
    let surface = if is_ml {
        // Reshape ONLY a bare single-ctor wrap `(<name> …)` → `<name>(…)` (ML uses call syntax, not
        // juxtaposition). The remainder after the name must be EXACTLY the hole — a multi-token prefix like
        // a `(host (E) …)` delegation is NOT a ctor application (`host` is a form), so it is left as-is
        // rather than mangled into `host((E) …)`.
        match replacement
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|inner| inner.split_once(' '))
        {
            Some((ctor, rest))
                if !ctor.is_empty() && !ctor.contains(['(', ')', ' ']) && rest == hole =>
            {
                format!("{ctor}({rest})") // `(Some …)` → `Some(…)`
            }
            _ => replacement.to_string(),
        }
    } else {
        replacement.to_string()
    };
    match surface.split_once(WRAP_HOLE) {
        Some((prefix, suffix)) => (prefix.to_string(), suffix.to_string()),
        None => (surface, String::new()),
    }
}

/// How a [`DiagnosticFix`] applies its `replacement` at its `node` — the ABI projection of a
/// [`crate::diag::Edit`]'s shape, so a consumer performs the right tree op without re-deriving it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FixKind {
    /// Replace the target node's surface spelling with `replacement`.
    Replace,
    /// Append `replacement` (rendered child forms) at the end of the target list node's children.
    InsertInto,
    /// Wrap the target node: `replacement` contains exactly one [`WRAP_HOLE`] (`…`) marking where the
    /// node's ORIGINAL text goes — the consumer replaces the node's span with `replacement` with the
    /// hole substituted by the original text (`(Some …)` → `(Some <expr>)`).
    Wrap,
    /// Delete the target node from its enclosing list (its span plus one adjacent separating space, so
    /// the list stays well-formed). `replacement` is empty — the edit is fully described by the node.
    Delete,
}

impl DiagnosticFix {
    /// The ABI projection of a compiler-internal [`crate::diag::Fix`] — lower its structural edit to a
    /// `(kind, node, replacement)` triple and its applicability to the `verified` flag.
    pub fn from_fix(fix: &crate::diag::Fix) -> DiagnosticFix {
        let (kind, node, replacement) = match &fix.edit {
            crate::diag::Edit::ReplaceNode { at, replacement } => {
                (FixKind::Replace, at.0, replacement.clone())
            }
            crate::diag::Edit::InsertArms { at, arms } => {
                (FixKind::InsertInto, at.0, arms.join(" "))
            }
            crate::diag::Edit::Wrap { at, prefix, suffix } => {
                (FixKind::Wrap, at.0, format!("{prefix}{WRAP_HOLE}{suffix}"))
            }
            crate::diag::Edit::Delete { at } => (FixKind::Delete, at.0, String::new()),
        };
        DiagnosticFix {
            label: fix.label.clone(),
            kind,
            node,
            replacement,
            verified: matches!(fix.applicability, crate::diag::Applicability::Verified),
        }
    }
}

/// A machine-readable diagnostic: severity + a stable code (or `None` for an uncoded decline) + a
/// human message + the AST NODE INDEX it is about (for source mapping) + an optional structural fix.
/// This STRUCT (not the human-formatted text a CLI prints) is the diagnostic's canonical form — a
/// consumer branches on its fields rather than parsing prose:
///
//= spec/capabilities/diagnostics.md#diagnostics-are-machine-readable
//# The compiler MUST expose its diagnostics in a machine-readable form rather than only as human-formatted text.
//= spec/capabilities/agent-authoring.md#every-compiler-output-is-machine-readable
//# The compiler MUST expose its diagnostics in a machine-readable form.
//= spec/contracts/build-tool-interface.md#a-diagnostic-carries-a-severity-a-code-and-a-message
//# A diagnostic MUST carry the machine-readable code and message fixed by the diagnostics-schema, so that a diagnostic in this interface is the same machine-actionable record the rest of the specification uses.
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
    /// span-free and its Cadenza port unburdened by source-position plumbing. The node index IS the
    /// diagnostic's span identifier — it names the exact construct the diagnostic concerns, which the
    /// consumer resolves to a precise source region:
    ///
    //= spec/capabilities/diagnostics.md#every-diagnostic-has-a-precise-span
    //# Every diagnostic the compiler emits MUST carry a source span identifying the construct it concerns.
    //= constitution.md#xi-diagnostics-are-machine-actionable
    //# Every diagnostic the compiler emits MUST carry a precise source span.
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
            fix: reject.fix.as_ref().map(|f| DiagnosticFix::from_fix(f)),
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

#[cfg(test)]
mod wrap_prefix_suffix_tests {
    use super::wrap_prefix_suffix;

    #[test]
    fn sexpr_splits_the_ctor_wrap_on_the_hole() {
        // The compiler's s-expr wrap `(Some …)` splits into the two literal sides an agent wraps the node
        // text with: `(Some ` + <text> + `)`. The `…` sentinel never survives into either side.
        let (prefix, suffix) = wrap_prefix_suffix("(Some …)", false);
        assert_eq!(prefix, "(Some ");
        assert_eq!(suffix, ")");
        assert!(!prefix.contains('…') && !suffix.contains('…'));
    }

    #[test]
    fn ml_reshapes_to_call_syntax_before_splitting() {
        // On ML a constructor application is `Some(…)`, not juxtaposition — so `(Some …)` reshapes to
        // `Some(…)` first, then splits: `Some(` + <text> + `)`. An agent on ML thus produces valid syntax.
        let (prefix, suffix) = wrap_prefix_suffix("(Some …)", true);
        assert_eq!(prefix, "Some(");
        assert_eq!(suffix, ")");
    }

    #[test]
    fn a_dotted_member_ctor_wrap_reshapes_to_ml_call_syntax() {
        // A wrap whose "ctor" is a DOTTED MEMBER op — `(Symbol.of …)` (the Symbol-vs-String CDZ0202 fix),
        // `(Int64.of …)` / `(Float64.of …)` (the numeric-coercion CDZ0301 fixes) — is still a single-token
        // constructor-application shape (the `.` is part of the name, not a separator), so ML reshapes it
        // to call syntax `Symbol.of(…)` and both surfaces split cleanly on the hole. Pins that the
        // reshape guard (`!ctor.contains(['(', ')', ' '])`) admits a dotted op name rather than mangling
        // it — the rendering these member-op wrap fixes rely on to present a valid ML quick-fix.
        assert_eq!(
            wrap_prefix_suffix("(Symbol.of …)", false),
            ("(Symbol.of ".to_string(), ")".to_string())
        );
        assert_eq!(
            wrap_prefix_suffix("(Symbol.of …)", true),
            ("Symbol.of(".to_string(), ")".to_string())
        );
        assert_eq!(
            wrap_prefix_suffix("(Int64.of …)", true),
            ("Int64.of(".to_string(), ")".to_string())
        );
    }

    #[test]
    fn a_multi_token_wrap_keeps_its_shape() {
        // A wrap whose prefix carries more than a bare ctor (`(host (E) …)`) is not the ML reshape shape,
        // so ML leaves it as-is and both surfaces split on the hole identically.
        assert_eq!(
            wrap_prefix_suffix("(host (E) …)", false),
            ("(host (E) ".to_string(), ")".to_string())
        );
        assert_eq!(
            wrap_prefix_suffix("(host (E) …)", true),
            ("(host (E) ".to_string(), ")".to_string())
        );
    }

    #[test]
    fn a_replacement_with_no_hole_degrades_to_a_bare_prefix() {
        // Defensive: a wrap replacement missing the sentinel (should not happen) returns (whole, "") so a
        // consumer applies it as a prefix rather than the helper panicking.
        assert_eq!(
            wrap_prefix_suffix("(Some x)", false),
            ("(Some x)".to_string(), String::new())
        );
    }
}
