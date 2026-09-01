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

    /// The GUEST export RESULT-TYPE map artifact kind (bytes-second run-wiring): `<name>\t<Ty::render_name>`
    /// lines an IN-PROCESS consumer reads to disambiguate a WIT-erased leaf at render (`render_val_typed`).
    /// ALSO embedded as a `cdz-result-type` component custom section for the multi-process (piped) run path.
    pub const KIND_RESULT_TYPES: &'static str = "result-types";
}

/// The input-artifact kind naming a package's ENTRY file — its bytes ARE the entry name. Rides the input
/// artifact stream like `ast`/`sidecar` (`DESIGN-package-linking.md` §3c): a `.find(kind == KIND_ENTRY)`,
/// no change to `compile`'s signature. A compile-BOUNDARY kind (the front-end builds it, the compiler
/// reads it), so it lives here; `rcdzc::link` `pub use`s it (byte-stable for the linker + `compile`).
pub const KIND_ENTRY: &str = "entry";

/// The input-artifact kind naming the INTERFACE a PROVIDER component publishes its exports under (X4b) —
/// its bytes are the interface name (`cadenza:pkg/iface`) a peer consumer binds to. A compile-BOUNDARY
/// kind like [`KIND_ENTRY`]; `rcdzc::link` `pub use`s it.
pub const KIND_COMPONENT_NAME: &str = "component-name";

/// Build the [`KIND_ENTRY`] input artifact naming a package's entry file — its bytes are the entry name.
/// A boundary artifact-builder the FRONT-END (`cdz`) uses to deliver a package the same way the
/// artifacts-in compiler path does, so it lives here rather than in `rcdzc::cli` (which `pub use`s it).
pub fn entry_artifact(name: &str) -> Artifact {
    Artifact::new(KIND_ENTRY, "entry", crate::name_wire::encode_name(name))
}

/// Build the [`KIND_COMPONENT_NAME`] input artifact naming a provider's published interface (X4b) — its
/// bytes are the interface name. The boundary companion of [`entry_artifact`].
pub fn component_name_artifact(iface: &str) -> Artifact {
    Artifact::new(
        KIND_COMPONENT_NAME,
        "component-name",
        crate::name_wire::encode_name(iface),
    )
}

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
    /// Attach a proposed structural fix — the fluent form a producer uses. Takes the ABI-projected
    /// [`DiagnosticFix`] directly (both are plain boundary types, so this stays an inherent method); the
    /// rcdzc-side conversion from a compiler-internal `diag::Fix` lives in `rcdzc::abi_bridge`.
    pub fn with_fix(mut self, fix: DiagnosticFix) -> Diagnostic {
        self.fix = Some(fix);
        self
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
    /// A DIAGNOSTIC METRIC: the `rcdzc` Db's `cse_partition_core_eq_calls` count from this compile — the
    /// within-bucket `core_eq` comparisons the wasm CSE class-partition made. The counter lives on the
    /// `Db` (which the emit path drops before returning), so it is surfaced here for `rcdzc`'s CSE-partition
    /// regression-guard test (`a_wide_arithmetic_body_partitions_cse_candidates_in_bounded_time`) to read a
    /// value from exactly one compile — a per-`Db` metric rather than a parallel-test-contaminated
    /// process-global atomic. `0` on any construction path that ran no emit (query-only / early-fail) and
    /// in a non-`rcdzc` consumer (which never runs the emit). Always-present (not `#[cfg(test)]`) because a
    /// cross-crate `#[cfg(test)]` field cannot be set from a dependent's tests — 8 harmless bytes, always
    /// `0` outside `rcdzc`'s emit path.
    pub cse_partition_core_eq_calls: u64,
    /// A DIAGNOSTIC METRIC: the `rcdzc` Db's `value_range_uncached_calls` count from this compile — how many
    /// times `lower::value_range_uncached` ran (i.e. a `value_range` query that MISSED its refinement-free
    /// memo). `value_range` recurses `LocalRef → initializer`, so without the memo it is O(N²) over a
    /// sequential-dependency chain; the memo makes uncached calls ~O(nodes). Surfaced here (the `Db` is
    /// dropped before returning) for the regression-guard test to assert a LINEAR bound — a future
    /// un-memoization flips it back to quadratic. `0` outside `rcdzc`'s lowering path. Always-present (same
    /// cross-crate-`#[cfg(test)]` reason as `cse_partition_core_eq_calls` above) — 8 harmless bytes.
    pub value_range_uncached_calls: u64,
    /// A DIAGNOSTIC METRIC: the `rcdzc` Db's `param_apply_extra_handled_calls` count from this compile — how
    /// many times `effects::param_apply_extra_handled` ran its BODY (a call that MISSED its
    /// `(callee_body, arity, depth)` memo). That fn's transitive follow re-enters itself per sub-callee AND
    /// `walk(head)` re-descends the same body, so without the memo it is 2^N over a nested applied-lambda
    /// chain (the seq-203 compile-hang, #5755); the memo makes body-runs ~O(nodes). Surfaced here (the `Db`
    /// is dropped before returning) for the regression-guard test to assert a LINEAR bound — a future
    /// un-memoization flips it back to exponential. `0` outside `rcdzc`'s lowering path. Always-present (same
    /// cross-crate-`#[cfg(test)]` reason as the metrics above) — 8 harmless bytes.
    pub param_apply_extra_handled_calls: u64,
    /// A DIAGNOSTIC METRIC: the `rcdzc` Db's `is_cse_shareable_uncached_calls` count from this compile — how
    /// many times `backend::wasm::select::is_cse_shareable` ran its inner body (a query that MISSED the
    /// `is_cse_shareable_memo`). The straight-line CSE driver queries the predicate per candidate node and it
    /// recurses over the node's whole Core subtree, so without the id-keyed memo a deeply-nested expression
    /// re-walks overlapping subtrees per enclosing node → O(N²)+ emit; the memo makes inner-runs ~O(nodes).
    /// Surfaced here (the `Db` is dropped before returning) for the regression-guard test to assert a LINEAR
    /// bound — a future un-memoization flips it back to quadratic. `0` outside `rcdzc`'s emit path.
    /// Always-present (same cross-crate-`#[cfg(test)]` reason as the metrics above) — 8 harmless bytes.
    pub is_cse_shareable_uncached_calls: u64,
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
