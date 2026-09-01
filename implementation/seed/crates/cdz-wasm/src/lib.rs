//! cdz-wasm — the browser-facing wrapper around Cadenza's pure compiler core.
//!
//! This crate adds NO compiler logic. It only marshals between JavaScript and the two pure Rust
//! libraries the interactive guide drives:
//!
//!   - [`cadenza_syntax`] reads a text surface (ML or s-expression) into the one canonical binary
//!     AST and prints it back out in any surface — the substrate for both compilation input and the
//!     guide's global syntax toggle.
//!   - [`rcdzc`] lowers a binary AST to a WebAssembly component (`compile_component`).
//!
//! The guide runs this cdylib inside a Web Worker (loaded via `wasm-pack build --target web`), so the
//! CPU-bound compile stays off the UI thread. None of `rcdzc`'s host-boundary machinery (the 64 MB
//! stack worker in `rcdzc::host`, the CLI bin) is referenced — those are native-only and unneeded:
//! guide snippets are far below the compiler's recursive-descent depth guard, so `compile_component`
//! is called inline on the worker's own stack.

use cadenza_syntax::convert::{self, Format};
use wasm_bindgen::prelude::*;

/// Parse a surface-format name coming from JS (`"ml"`, `"sexpr"`/`"sexp"`, `"binary"`, `"debug"`,
/// `"flat"`) into the internal [`Format`]. Returns a JS error for an unknown name rather than
/// silently defaulting, so a typo in the caller surfaces immediately.
fn parse_format(name: &str) -> Result<Format, JsError> {
    Format::parse(name).ok_or_else(|| JsError::new(&format!("unknown syntax format: {name:?}")))
}

/// One compiler diagnostic, flattened for JavaScript. `code` is the stable `CDZ####` string (empty
/// for an uncoded decline — a construct the compiler does not yet support), `node` is the AST node
/// index the message is about (`u32::MAX` when unanchored), which the caller maps to a source span
/// using the span table it built while reading the same text.
#[wasm_bindgen(getter_with_clone)]
#[derive(Clone)]
pub struct Diagnostic {
    /// `true` for an error (denies the component), `false` for a warning (rides alongside it).
    pub error: bool,
    pub code: String,
    pub message: String,
    /// AST node index, or `u32::MAX` if the diagnostic is unanchored.
    pub node: u32,
    /// The source byte range `[from, to)` this diagnostic anchors to, resolved from the front-end span
    /// table in Rust (where offsets are UTF-8 bytes). `from == to == 0` when unanchored (no user span).
    /// The JS side converts these UTF-8 byte offsets to the editor's UTF-16 offsets for a squiggle.
    pub from: u32,
    pub to: u32,
    /// A proposed structural repair (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
    /// A Fix), surfaced to the guide's quick-fix affordance. Empty (`fix_kind == ""`) when the diagnostic
    /// carries no fix; otherwise applied per `fix_kind` over the target `[fix_from, fix_to)` byte range:
    ///   - `"replace"` — replace the range with `fix_replacement`;
    ///   - `"insert"` — insert `fix_replacement` (rendered child forms, e.g. missing match arms) just
    ///     before `fix_to` (the end of the target list, before its closing paren);
    ///   - `"delete"` — replace the range with the empty string;
    ///   - `"wrap"` — replace the range with `fix_prefix + <range text> + fix_suffix` (`fix_replacement`
    ///     is EMPTY for a wrap). The two sides are the surface-correct literals to wrap the node in
    ///     (`Some(` / `)` on ML, `(Some ` / `)` on s-expr) — split here so the JS side NEVER sees the
    ///     internal `…` hole sentinel it would otherwise have to strip (a raw splice would corrupt text).
    /// `fix_verified` distinguishes a machine-applicable fix from a heuristic the user should confirm.
    pub fix_replacement: String,
    /// For a `"wrap"` fix, the literal text to insert BEFORE the target range (empty for other kinds).
    pub fix_prefix: String,
    /// For a `"wrap"` fix, the literal text to insert AFTER the target range (empty for other kinds).
    pub fix_suffix: String,
    pub fix_from: u32,
    pub fix_to: u32,
    pub fix_verified: bool,
    pub fix_kind: String,
}

/// The outcome of a compile: on success `component` holds the WebAssembly component bytes and
/// `diagnostics` holds any warnings; on failure `component` is `None` and `diagnostics` holds the
/// errors. This mirrors the always-live-diagnostics ABI of [`rcdzc::CompileOutput`].
#[wasm_bindgen(getter_with_clone)]
pub struct CompileResult {
    /// The emitted component bytes, or `None` if compilation failed.
    pub component: Option<Vec<u8>>,
    /// Every diagnostic produced — errors (on failure) or warnings (on success).
    pub diagnostics: Vec<Diagnostic>,
}

#[wasm_bindgen]
impl CompileResult {
    /// Convenience for JS: did compilation produce a component?
    #[wasm_bindgen(getter)]
    pub fn ok(&self) -> bool {
        self.component.is_some()
    }
}

/// Flatten an `rcdzc::Diagnostic` for JS, resolving its node id to a source byte range through the
/// front-end span table (`None` → no span, e.g. a synthesized/prelude node or an unanchored decline).
fn to_js_diag(
    d: &rcdzc::Diagnostic,
    spans: Option<&cadenza_syntax::spans::SpanTable>,
    is_ml: bool,
) -> Diagnostic {
    let node = d.node.unwrap_or(u32::MAX);
    let span_of = |n: u32| -> (u32, u32) {
        spans
            .and_then(|s| s.get(cadenza_syntax::ast::StructId(n)))
            .map(|s| (s.start as u32, s.end as u32))
            .unwrap_or((0, 0))
    };
    let (from, to) = d.node.map(span_of).unwrap_or((0, 0));
    // Carry the structural fix (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
    // Fix) through to the guide's quick-fix affordance — its target node mapped to a byte range here. A
    // WRAP splits into surface-correct `prefix`/`suffix` (via the shared `rcdzc::wrap_prefix_suffix`) so
    // the JS quick-fix applies `prefix + <range text> + suffix` and never sees the `…` hole sentinel.
    let (fix_replacement, fix_prefix, fix_suffix, fix_from, fix_to, fix_verified, fix_kind) =
        match &d.fix {
            Some(f) => {
                let (ff, ft) = span_of(f.node);
                match f.kind {
                    rcdzc::FixKind::Wrap => {
                        let (prefix, suffix) = rcdzc::wrap_prefix_suffix(&f.replacement, is_ml);
                        (String::new(), prefix, suffix, ff, ft, f.verified, "wrap")
                    }
                    rcdzc::FixKind::Replace => (
                        f.replacement.clone(),
                        String::new(),
                        String::new(),
                        ff,
                        ft,
                        f.verified,
                        "replace",
                    ),
                    rcdzc::FixKind::InsertInto => (
                        f.replacement.clone(),
                        String::new(),
                        String::new(),
                        ff,
                        ft,
                        f.verified,
                        "insert",
                    ),
                    rcdzc::FixKind::Delete => (
                        f.replacement.clone(),
                        String::new(),
                        String::new(),
                        ff,
                        ft,
                        f.verified,
                        "delete",
                    ),
                }
            }
            None => (String::new(), String::new(), String::new(), 0, 0, false, ""),
        };
    Diagnostic {
        error: d.severity == rcdzc::Severity::Error,
        code: d.code.clone().unwrap_or_default(),
        message: d.message.clone(),
        node,
        from,
        to,
        fix_replacement,
        fix_prefix,
        fix_suffix,
        fix_from,
        fix_to,
        fix_verified,
        fix_kind: fix_kind.to_string(),
    }
}

/// The maximum size (UTF-8 bytes) of a SINGLE untrusted source the wasm boundary parses, checked BEFORE
/// parsing. This is the DoS backstop at the UNTRUSTED ingestion layer (browser input) — the correct layer
/// for a size limit: the reader builds an arena that is O(input), so bounding input bytes bounds arena
/// size (the real resource concern), and the reader itself is overflow-proof. NATIVE/trusted callers of
/// `cadenza_syntax` are UNBOUNDED; only this wasm boundary caps input. 1 MiB is generous — guide /
/// playground / CAD sources are KB-scale — while rejecting a pathological megabyte-deep-nest source.
pub const CDZ_WASM_MAX_SOURCE_BYTES: usize = 1 << 20;

/// The maximum AGGREGATE untrusted source size across a multi-module compile — the user `text` PLUS every
/// preloaded module source — checked BEFORE parsing in [`compile_with_preloaded`]. The per-source
/// [`CDZ_WASM_MAX_SOURCE_BYTES`] guard bounds each source individually; this bounds their SUM so that N
/// just-under-limit modules cannot aggregate into a many-sources DoS. 8 MiB accommodates a real preloaded
/// library set while staying bounded.
pub const CDZ_WASM_MAX_TOTAL_BYTES: usize = 8 << 20;

/// Reject an untrusted source exceeding [`CDZ_WASM_MAX_SOURCE_BYTES`] BEFORE parsing (the size guard at
/// the wasm ingestion boundary). The `Err(String)` surfaces through the existing parse-failure channel as
/// a codeless error diagnostic (see [`compile`]). The byte counts ride in the message.
fn check_source_size(text: &str) -> Result<(), String> {
    if text.len() > CDZ_WASM_MAX_SOURCE_BYTES {
        return Err(format!(
            "source exceeds the maximum size ({} bytes; limit {CDZ_WASM_MAX_SOURCE_BYTES} bytes)",
            text.len()
        ));
    }
    Ok(())
}

/// Parse `text` in `surface` into rcdzc's binary AST bytes AND the front-end span table (node id →
/// UTF-8 byte range) built from the SAME parse. The span table MUST be keyed by the CANONICAL node ids,
/// because `codec::encode` canonicalizes (`canon.rs`) and the compiler decodes+reports THOSE ids. A raw
/// span table can be keyed by pre-canonical ids, so a lookup by a compiler node id lands on the WRONG
/// node; both readers therefore CANONICALIZE the arena and REMAP the span table first (the same fix
/// `cdz`'s `load_program_spanned` applies). The ML reader is non-canonical because it builds an infix
/// operand before its operator head (`ml-parser-node-order`); the s-expr reader builds a LONE form
/// canonically, but its MULTI-form path wraps the roots in a synthetic `(do …)` whose head is built
/// LAST, which canonicalization reorders — so a multi-form guide program needs the remap too (matching
/// `cdz` M24; without it a `check`/underline lands on a neighbour's span). Only the s-expression and ML
/// surfaces have a reader; a binary or output-only (`debug`/`flat`) `from` has no source spans, so
/// `spans` is `None`.
fn parse_spanned(
    text: &str,
    from: Format,
) -> Result<(Vec<u8>, Option<cadenza_syntax::spans::SpanTable>), String> {
    // Size guard at the untrusted ingestion boundary: reject an over-limit source BEFORE parsing. Every
    // surface parse (the user model AND each preloaded module) funnels through here, so this one guard
    // bounds each individual untrusted source; the aggregate is bounded in `compile_with_preloaded`.
    check_source_size(text)?;
    match from {
        Format::Sexpr => {
            // A SINGLE top-level form stays bare (`read_spanned`); MULTIPLE forms (a guide snippet with
            // several `(def …)`/`(export …)`) wrap in a synthetic `(do …)` via `read_all_spanned` —
            // `read_spanned` errors on trailing input, so fall back to it. Then canonicalize + remap so
            // the span table is keyed by the ids the compiler reports (a lone form's map is identity, so
            // that case is byte-unchanged).
            let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(text) {
                Ok(pair) => pair,
                Err(_) => cadenza_syntax::sexpr::read_all_spanned(text).map_err(|e| e.0)?,
            };
            let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&raw_arenas);
            let spans = raw_spans.remap(&id_map, arenas.structure.len());
            Ok((cadenza_syntax::codec::encode(&arenas), Some(spans)))
        }
        Format::Ml => {
            let parsed = cadenza_syntax::parser::read_ml(text);
            if let Some(err) = parsed.errors.first() {
                // Drop the raw "at byte N" (a byte offset the editor can't place); the caller that
                // renders this as a diagnostic sets the source RANGE from the error's span
                // (`ml_parse_error_span`), so the guide can underline the exact syntax mistake.
                return Err(err.message.clone());
            }
            // Canonicalize + remap so the span table is keyed by the ids the compiler reports (see the
            // doc above) — the same fix `cdz`'s `load_program_spanned` applies for the ML surface.
            let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas);
            let spans = parsed.spans.remap(&id_map, arenas.structure.len());
            Ok((cadenza_syntax::codec::encode(&arenas), Some(spans)))
        }
        // No reader (or output-only) — fall back to the format-agnostic byte conversion, no spans.
        _ => convert::convert(text.as_bytes(), from, Format::Binary)
            .map(|b| (b, None))
            .map_err(|e| e.0),
    }
}

/// The UTF-8 byte range `[from, to)` of the FIRST ML parse error in `text`, or `None` when it parses
/// cleanly / is not the ML surface. The `compile` entry uses this to give a parse-error diagnostic a
/// source RANGE (so the guide editor underlines the mistake) rather than a positionless `(0, 0)`. Cheap
/// enough to re-read here: it runs only on the error path, when a component was not produced anyway.
fn ml_parse_error_span(text: &str, from: Format) -> Option<(u32, u32)> {
    if from != Format::Ml {
        return None;
    }
    let parsed = cadenza_syntax::parser::read_ml(text);
    parsed
        .errors
        .first()
        .map(|e| (e.span.start as u32, e.span.end as u32))
}

/// Decode `ast_bytes` into a `Db`, run one sidecar `Query`, and return its result artifact's raw BYTES.
/// This is the ONE path every IDE fact read goes through — the browser IDE speaks the same sidecar query
/// vocabulary as the `cdz` CLI (`cdz type-at`/`def`/`check`/`uses`), so a fact the editor shows equals
/// what the CLI would answer, by construction. (The span table stays a front-end concern: the consumer
/// maps a query's node ids to source ranges, per the query-engine contract.)
///
/// EVERY sidecar result artifact is now canonical BINARY AST (seq-254 "binary AST is THE data exchange
/// format"), so callers DECODE the bytes with the artifact's `*_wire` codec (`decode_diagnostics`,
/// `decode_type_at`, `decode_resolve`, `decode_exports`, `decode_highlight`, `decode_instantiations`, …)
/// rather than interpreting them as UTF-8 text — a `String::from_utf8` here would throw on a non-UTF-8
/// payload (Class A) or silently mis-parse a valid-by-luck one (Class B). (The former `run_query_text`
/// helper did exactly that and was removed once every consumer moved to a structured decode.)
fn run_query_bytes(ast_bytes: &[u8], query: &rcdzc::Query) -> Result<Vec<u8>, JsError> {
    let arenas = rcdzc::codec::decode(ast_bytes)
        .ok_or_else(|| JsError::new("internal: re-encoded AST failed to decode"))?;
    let mut db = rcdzc::db::Db::load(arenas);
    Ok(rcdzc::sidecar::run_query(&mut db, query).bytes)
}

/// Compile Cadenza source in the given surface format to a WebAssembly component.
///
/// The pipeline is exactly the reference toolchain's, run in-process: read `text` (in `from` format)
/// into the binary AST, then hand those AST bytes to `rcdzc::compile_component`. A read/parse error
/// is returned as a single error [`Diagnostic`] with no code, so the caller has one uniform channel.
#[wasm_bindgen]
pub fn compile(text: &str, from: &str) -> Result<CompileResult, JsError> {
    let from = parse_format(from)?;

    // Text surface -> canonical binary AST + the span table (so diagnostics carry source ranges). A
    // parse failure becomes a codeless error diagnostic.
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(msg) => {
            // A parse failure is a codeless error diagnostic — carry the error's SOURCE RANGE (from the
            // ML parser's span) so the guide editor underlines the exact syntax mistake, not a
            // positionless (0, 0). (The message no longer embeds the byte offset — the range carries it.)
            let (from_b, to_b) = ml_parse_error_span(text, from).unwrap_or((0, 0));
            return Ok(compile_error(msg, from_b, to_b));
        }
    };

    // Binary AST -> WebAssembly component. Use the full `compile` entry so warnings ride alongside a
    // successful component too; `push_spans_target` embeds DWARF when we have a span table (a text
    // surface — the guide's case), and `finish_compile` maps diagnostics + extracts the component.
    let mut inputs = vec![rcdzc::Artifact::new(
        rcdzc::Artifact::KIND_AST,
        "main",
        ast_bytes,
    )];
    let target = push_spans_target(&mut inputs, text, &spans);
    Ok(finish_compile(
        &inputs,
        target,
        spans.as_ref(),
        from == Format::Ml,
    ))
}

/// A codeless error [`CompileResult`] carrying one diagnostic anchored at `[from_b, to_b)` — the uniform
/// "no component, one message" channel `compile*` returns for a parse/read failure.
fn compile_error(message: String, from_b: u32, to_b: u32) -> CompileResult {
    CompileResult {
        component: None,
        diagnostics: vec![codeless_error(message, from_b, to_b)],
    }
}

/// Compile `text` (surface `from`) to a WebAssembly component with a set of PRELOADED library modules
/// linked alongside it — so the user's buffer can `import { … } from "<name>"` a module it never had to
/// author or paste in. This is the seam the `/cad` IDE needs: the CAD geometry library (v-cad's
/// `exact`/`units` modules) is supplied as preloaded modules, and the editor buffer holds ONLY the user's
/// model, not the library boilerplate.
///
/// The preloaded modules arrive as three parallel arrays (wasm-bindgen marshals `Vec<String>` directly,
/// no JSON): `preloaded_names[i]` is the module NAME an `import` resolves against (the artifact name —
/// `import from "exact"` binds to the preloaded module named `exact`), `preloaded_sources[i]` its source
/// text, `preloaded_formats[i]` its surface (`"ml"`/`"sexpr"`). The three MUST be equal length.
///
/// Mechanism: each preloaded source is parsed to an `ast`-kinded artifact named by its module name, the
/// user text is parsed to the `main` `ast` artifact, and a `KIND_ENTRY` marker names `main` the entry —
/// so `rcdzc::compile` LINKS them into one package (`DESIGN-package-linking.md`), resolving the user's
/// imports against the preloaded modules exactly as the native `cdz` multi-file compile does. A preloaded
/// module that fails to parse is a codeless error diagnostic naming the offending module (the whole
/// compile declines rather than silently dropping a library the user's model depends on).
///
/// NOTE: this links preloaded modules the user still IMPORTS by name. Making a preloaded module's exports
/// AMBIENT (in scope with no `import` line at all) is a resolution-policy decision (auto-inject the import
/// vs. a prelude-style install) pending an operator ruling; this seam delivers the module-crossing plumbing
/// either policy needs. Existing [`compile`] is unchanged (callers that pass no library keep using it).
#[wasm_bindgen]
pub fn compile_with_preloaded(
    text: &str,
    from: &str,
    preloaded_names: Vec<String>,
    preloaded_sources: Vec<String>,
    preloaded_formats: Vec<String>,
) -> Result<CompileResult, JsError> {
    let from = parse_format(from)?;
    if preloaded_names.len() != preloaded_sources.len()
        || preloaded_names.len() != preloaded_formats.len()
    {
        return Err(JsError::new(
            "compile_with_preloaded: preloaded_names/sources/formats must be equal length",
        ));
    }

    // AGGREGATE size guard: the per-source `check_source_size` (in `parse_spanned`) bounds `text` and each
    // preloaded source individually; this bounds their SUM so N just-under-limit modules can't aggregate
    // into a many-sources DoS. Checked BEFORE any parsing; surfaced as a codeless diagnostic (the same
    // channel a parse/size failure uses), not a JS exception.
    let total: usize = text.len() + preloaded_sources.iter().map(String::len).sum::<usize>();
    if total > CDZ_WASM_MAX_TOTAL_BYTES {
        return Ok(compile_error(
            format!(
                "aggregate source size exceeds the maximum ({total} bytes; limit {CDZ_WASM_MAX_TOTAL_BYTES} bytes)"
            ),
            0,
            0,
        ));
    }

    // No preloaded modules → nothing to link; the result is byte-identical to a plain single-file
    // compile (no KIND_ENTRY, no linkage), so keep the flat-namespace path exactly.
    if preloaded_names.is_empty() {
        return compile(text, from.name());
    }

    // Parse the user model into the `main` AST + its span table (a parse failure → codeless diagnostic
    // with the mistake's source range, matching `compile`).
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(msg) => {
            let (from_b, to_b) = ml_parse_error_span(text, from).unwrap_or((0, 0));
            return Ok(compile_error(msg, from_b, to_b));
        }
    };

    let mut inputs = vec![rcdzc::Artifact::new(
        rcdzc::Artifact::KIND_AST,
        "main",
        ast_bytes,
    )];
    // Embed DWARF for the USER model (the buffer the browser debugger steps through); preloaded library
    // modules carry no spans (they aren't the source under edit), so the debug info stays about the model.
    let target = push_spans_target(&mut inputs, text, &spans);

    // Each preloaded module → an `ast` artifact NAMED by its module name (the link target of an
    // `import from "<name>"`). A parse failure names the module so the user knows which library broke.
    for ((name, source), fmt) in preloaded_names
        .iter()
        .zip(preloaded_sources.iter())
        .zip(preloaded_formats.iter())
    {
        let pf = parse_format(fmt)?;
        match parse_spanned(source, pf) {
            Ok((bytes, _)) => {
                inputs.push(rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, name, bytes));
            }
            Err(msg) => {
                return Ok(compile_error(
                    format!("preloaded module `{name}` failed to parse: {msg}"),
                    0,
                    0,
                ));
            }
        }
    }

    // The `KIND_ENTRY` marker makes `main` the package entry, so `compile` links the files instead of
    // treating them as one flat arena.
    inputs.push(rcdzc::cli::entry_artifact("main"));

    Ok(finish_compile(
        &inputs,
        target,
        spans.as_ref(),
        from == Format::Ml,
    ))
}

/// The result of a TEST compile: the component (its boundary laid out from the program's `@test` defs, not
/// its `(export …)` clauses), the diagnostics, and the names of the discovered `@test` defs. Each name is
/// an invocable boundary export in `component`; the browser test-runner calls each and reports pass/fail
/// (a clean return = pass, a trap = fail). `nullary_test_names` is the subset with ZERO parameters — the
/// ones the browser runs today; a parameterized `@test` is a PROPERTY test (or compiles to a `-gen`
/// wrapper that performs `Test.gen`) whose generated-input driving is a follow-up, so it's listed
/// separately in `param_test_names` and the runner defers it rather than mis-reporting it.
#[wasm_bindgen(getter_with_clone)]
pub struct TestCompileResult {
    /// The emitted test component bytes, or `None` if the test compile failed / there were no `@test`s.
    pub component: Option<Vec<u8>>,
    /// Every diagnostic — errors (on failure, incl. "no `@test`") or warnings.
    pub diagnostics: Vec<Diagnostic>,
    /// Names of the discovered NULLARY `@test` defs — the ones the browser runs today (invoke → pass/fail).
    pub nullary_test_names: Vec<String>,
    /// Names of the discovered PARAMETERIZED `@test` defs (property/exhaustive) — deferred by the runner
    /// (they need generated-input / `Test.gen` host-response driving; a follow-up), listed so the UI can
    /// show "N property tests deferred" rather than silently dropping or mis-failing them.
    pub param_test_names: Vec<String>,
}

/// Compile `text` in TEST-LAYOUT mode: the emitted component's boundary is laid out from the program's
/// `@test` NULLARY defs (`rcdzc` `layout::compute_tests`, driven by a `Request::EmitTests` sidecar
/// request) instead of its `(export …)` clauses — so every `@test` crosses as an invocable boundary
/// export. This is what a `cdz test` build requests; the browser test-runner then invokes each `@test`
/// export through the SAME run worker the playground uses (clean return = PASS, trap = FAIL), matching the
/// local `cdz test` contract. Also enumerates the `@test` def names (split nullary vs parameterized, so
/// the runner runs the nullary ones and defers the parameterized/property ones — see `TestCompileResult`).
/// A program with no `@test` declines (an error diagnostic), matching `compute_tests`.
#[wasm_bindgen]
pub fn compile_tests(text: &str, from: &str) -> Result<TestCompileResult, JsError> {
    let from = parse_format(from)?;

    // Parse → canonical binary AST (+ spans for source-ranged diagnostics), exactly like `compile`.
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(msg) => {
            let (from_b, to_b) = ml_parse_error_span(text, from).unwrap_or((0, 0));
            return Ok(TestCompileResult {
                component: None,
                diagnostics: vec![codeless_error(msg, from_b, to_b)],
                nullary_test_names: Vec::new(),
                param_test_names: Vec::new(),
            });
        }
    };

    // Enumerate the `@test` defs from a `Db` built off the SAME AST — names + arity (nullary vs param), so
    // the runner runs the nullary tests and defers the parameterized ones. (Mirrors `cdz test`'s
    // `run_test_file`, which reads `db.test_defs()` → `db.defs[i].name`; a single-snippet browser has no
    // entry-file filter, so every `@test` in the snippet is listed.)
    let (mut nullary_test_names, mut param_test_names) = (Vec::new(), Vec::new());
    if let Some(arenas) = rcdzc::codec::decode(&ast_bytes) {
        let db = rcdzc::db::Db::load(arenas);
        for i in db.test_defs() {
            let def = &db.defs[i];
            // A test is NULLARY (runs today: invoke → pass/fail) only if it has NO params AND its name is
            // not a synthesized generator wrapper. A COMPOUND-param @test (e.g. `p(xs: List Int64)`) is
            // neutralized and hoisted as a synthesized nullary wrapper `p-gen` — which has no params but
            // PERFORMS `Test.gen`, so invoking it as a plain unit test errors. The `-gen` suffix is the
            // stable signal for that wrapper (v-property-testing owns the suffix; a user can't collide).
            // So: param/deferred = (params non-empty) OR (name ends `-gen`). The runtime Test.gen guard in
            // the runner is the authoritative backstop; this keeps the classification honest too.
            if def.params.is_empty() && !def.name.ends_with("-gen") {
                nullary_test_names.push(def.name.clone());
            } else {
                param_test_names.push(def.name.clone());
            }
        }
    }

    // Compile with a `Request::EmitTests` sidecar request driving the emit (targets left empty — the
    // request selects the test-layout emit). The component is produced under the "component" artifact,
    // same as a normal wasm emit.
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast_bytes),
        rcdzc::Artifact::new(
            rcdzc::sidecar::KIND_SIDECAR,
            "drive",
            rcdzc::sidecar::encode(&[rcdzc::Request::EmitTests]),
        ),
    ];
    let out = rcdzc::compile(&inputs, &[]);
    let diagnostics = out
        .diagnostics
        .iter()
        .map(|d| to_js_diag(d, spans.as_ref(), from == Format::Ml))
        .collect();
    let component = out.artifact("component").map(|b| b.to_vec());
    Ok(TestCompileResult {
        component,
        diagnostics,
        nullary_test_names,
        param_test_names,
    })
}

/// One parameterized `@test`'s signature, for the browser property-test DRIVER (`v-property-testing`).
/// A SCALAR-param property test keeps its parameters ON THE EXPORT (the driver generates JS args of each
/// param's type and calls the export — `Int64`→`bigint`, `Bool`→`boolean`, …). A COMPOUND-param test is
/// neutralized into a synthesized nullary `-gen` wrapper that performs `Test.gen` internally, so it has no
/// callable params and is DEFERRED to the driver's phase-2 (host-response) path. This struct tells the
/// driver which is which (`compound`) and, for the scalar case, each param's type (`param_types`).
#[wasm_bindgen(getter_with_clone)]
pub struct ParamTestSignature {
    /// The `@test` def's name (a `-gen` wrapper keeps its `-gen` suffix, matching `compile_tests`'
    /// `param_test_names`).
    pub name: String,
    /// Each parameter's type as a STABLE lowercase enum the driver switches on:
    /// `int8`|`int16`|`int32`|`int64`|`uint8`|`uint16`|`uint32`|`uint64`|`bool`|`float32`|`float64` for the
    /// scalar types the arg-driver generates, or `other` for anything outside that set (an unannotated /
    /// inferred / non-scalar param — the driver skips or defers it). EMPTY for a `-gen` wrapper (no callable
    /// params — see `compound`).
    pub param_types: Vec<String>,
    /// `true` when the test is a synthesized `-gen` wrapper (a COMPOUND param was hoisted to an internal
    /// `Test.gen`) — the driver routes it to the deferred phase-2 path, NOT the scalar arg-driver. `false`
    /// for a scalar-param test whose params are on the export.
    pub compound: bool,
}

/// The stable scalar-type enum string for a parameter's TYPE node, or `"other"` for anything the browser
/// arg-driver can't generate directly (a compound `(List …)`, a nominal newtype, an unannotated/inferred
/// param — anything not a bare scalar type name). Kept in sync with the jco boundary lowering the driver
/// relies on (Int/UInt widths + Bool + Float widths → a JS bigint/boolean/number).
fn scalar_type_enum(ast: &rcdzc::ast::Arenas, ty_node: rcdzc::ast::StructId) -> String {
    let Some(name) = ast.as_name(ty_node) else {
        // A non-atom type node (e.g. `(List Int64)`, `(Tuple …)`) — not a bare scalar.
        return "other".to_string();
    };
    match name {
        "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64"
        | "Bool" | "Float32" | "Float64" => name.to_ascii_lowercase(),
        _ => "other".to_string(),
    }
}

/// The signatures of every PARAMETERIZED `@test` in `text` — the metadata the browser property-test driver
/// needs to generate inputs (see [`ParamTestSignature`]). Mirrors `compile_tests`' param-test enumeration
/// (same `Db::test_defs()` walk, same `-gen`-suffix classification), but additionally reads each scalar
/// param's TYPE so the driver generates from real types instead of guessing by arity. A parse error / no
/// `@test` yields an empty list.
#[wasm_bindgen]
pub fn param_test_signatures(text: &str, from: &str) -> Result<Vec<ParamTestSignature>, JsError> {
    let from = parse_format(from)?;
    let Ok((ast_bytes, _spans)) = parse_spanned(text, from) else {
        return Ok(Vec::new());
    };
    let Some(arenas) = rcdzc::codec::decode(&ast_bytes) else {
        return Ok(Vec::new());
    };
    let db = rcdzc::db::Db::load(arenas);
    let mut out = Vec::new();
    for i in db.test_defs() {
        let def = &db.defs[i];
        let is_gen_wrapper = def.name.ends_with("-gen");
        // Same classification as `compile_tests`: a NULLARY non-wrapper test isn't parameterized — skip it
        // (the runner invokes it directly). We report only property tests: a params-bearing scalar test OR
        // a synthesized `-gen` wrapper.
        if def.params.is_empty() && !is_gen_wrapper {
            continue;
        }
        // A `-gen` wrapper carries no callable params (its compound param was hoisted to an internal
        // `Test.gen`) — deferred phase-2, no scalar param types to report.
        if is_gen_wrapper {
            out.push(ParamTestSignature {
                name: def.name.clone(),
                param_types: Vec::new(),
                compound: true,
            });
            continue;
        }
        // A scalar-param test: read each param's annotated type. A parameter is a bare name atom or an
        // annotated binder `(: name T)` — the two shapes `Db`/`resolve` recognize; the TYPE is the binder's
        // SECOND child (after the name). A bare (unannotated) param has no type node → `other`.
        let mut param_types = Vec::with_capacity(def.params.len());
        let mut any_non_scalar = false;
        for &p in &def.params {
            let enum_str = if let Some(tail) = db.ast.as_form(p, ":") {
                // `(: name TYPE)` → the type node is `tail[1]`.
                match tail.get(1) {
                    Some(&ty_node) => scalar_type_enum(&db.ast, ty_node),
                    None => "other".to_string(),
                }
            } else {
                // A bare (unannotated) param — no annotation to read.
                "other".to_string()
            };
            if enum_str == "other" {
                any_non_scalar = true;
            }
            param_types.push(enum_str);
        }
        out.push(ParamTestSignature {
            name: def.name.clone(),
            param_types,
            // A scalar test whose every param is a generatable scalar → scalar path (compound=false). If
            // any param isn't a scalar the driver can't fully drive it via args — flag it compound so it's
            // routed to the deferred path rather than the driver generating a wrong-typed arg.
            compound: any_non_scalar,
        });
    }
    Ok(out)
}

/// One `@param` site's WIDGET-MANIFEST entry, flattened for JavaScript — the metadata `/cad` (and any
/// parametric host) reads from a compiled model to render a control per param and drive it over the
/// host-response path. Mirrors [`ParamTestSignature`]'s role for property tests. All optional fields are
/// `None` (JS `undefined`) when the `@param`'s config omits them; `type_name` is always present (the
/// B-invariant requires an explicit type). `range_lo`/`range_hi`/`default` are rendered as STRINGS (not
/// numbers) so an exact `Rational` default like `1/4` or a `Qty` survives the boundary — JS parses per widget.
#[wasm_bindgen(getter_with_clone)]
pub struct ParamManifestEntry {
    /// The param name — the accessor member (`Param.<name>`), the manifest key, and the host-bind name.
    pub name: String,
    /// The declared type, rendered (e.g. `Int64`, `Rational`, `(Qty Rational meter)`) — the accessor's
    /// result type the host value must satisfy. Reduced via the same evaluator the annotation path uses, so
    /// it equals what the type checker asserts (falls back to the node's inferred type render if it does not
    /// reduce to a type value, keeping the field total).
    pub type_name: String,
    /// The `(: widget <name>)` config value (e.g. `"slider"`), or `None` if the `@param` declares no widget.
    pub widget: Option<String>,
    /// The low / high bound of a `(: range [<lo> <hi>])` config, each rendered to its literal text, or
    /// `None` when there is no range config.
    pub range_lo: Option<String>,
    pub range_hi: Option<String>,
    /// The `(: default <val>)` config value rendered to its literal text, or `None` when there is no default.
    pub default: Option<String>,
    /// EXACT num/den of a `Rational` range/default, for a fraction-native host (`/cad`'s slider carries a
    /// `{num, den}` and drives `Param.<name>-num`/`-den`). Each is the base-10 text of the numerator /
    /// denominator of the config value folded to a `Core::ConstRational` (normalized, gcd-reduced — the SAME
    /// rational the type checker + the num/den host-response ABI use), so the host reads the pair directly
    /// instead of parsing the literal-text `range_lo`/`default` source. `None` for a non-Rational (e.g.
    /// `Int64`) bound or a non-constant config — the caller reads the literal-text field for those. The
    /// literal-text fields above are ALWAYS kept (a tooltip/label), these are the additive exact-rational
    /// companions the caller reads when `type_name` is a `Rational`.
    pub range_lo_num: Option<String>,
    pub range_lo_den: Option<String>,
    pub range_hi_num: Option<String>,
    pub range_hi_den: Option<String>,
    pub default_num: Option<String>,
    pub default_den: Option<String>,
}

/// Render a value/config node (a `range`/`default` literal, or a small nested form) to compact source-like
/// text for the manifest — an integer/float/string/name atom reads directly; a `[a b]`-style `(list …)` or
/// any other compound renders as a space-joined `(head child…)` s-expression. Total: an unrenderable node
/// falls back to its head/atom spelling so the field stays a definite string (the manifest is advisory data
/// the host parses per widget, not a re-parsed program). Bounded recursion — config values are shallow.
fn render_manifest_node(ast: &rcdzc::ast::Arenas, id: rcdzc::ast::StructId) -> String {
    if let Some(v) = ast.as_int(id) {
        // The integer's exact base-10 text (`IntValue` is a big-endian bignum, no `Display`).
        return v.to_decimal_string();
    }
    if let Some(d) = ast.as_float(id) {
        // A decimal literal has no direct text render on `Decimal`; go through its `f64` bit pattern for a
        // human-readable magnitude (a range/default bound is a display value the host parses, not a
        // re-parsed exact literal — the exact value still crosses via the num/den host path at run time).
        return f64::from_bits(d.to_f64_bits()).to_string();
    }
    if let Some(s) = ast.as_str(id) {
        return s.to_string();
    }
    if let Some(n) = ast.as_name(id) {
        return n.to_string();
    }
    // A compound node (e.g. `(list 0 100)` from `[0 100]`, or a `(Unit.base …)`): render its children
    // space-joined, so a structured default/range value stays legible rather than collapsing to a node id.
    match ast.get(id) {
        rcdzc::ast::Struct::List(children) => {
            let parts: Vec<String> = children
                .iter()
                .map(|&c| render_manifest_node(ast, c))
                .collect();
            format!("({})", parts.join(" "))
        }
        // An atom that matched none of the readers above (should not occur) — an empty, definite fallback.
        rcdzc::ast::Struct::Atom(_) => String::new(),
    }
}

/// The EXACT `(num, den)` of a config value node for a `Rational` bound, as base-10 text — the fraction-
/// native form a `/cad` slider drives (`Param.<name>-num`/`-den`). Folds the node to its `Core` via
/// `lower::core_of` and reads its exact rational value. A `Core::ConstRational(n, d)` (a written fraction
/// `(Rational.of 1 4)`, normalized + gcd-reduced by the compiler — the SAME rational the num/den
/// host-response ABI reconstructs) yields `(n, d)`. A `Core::ConstInt(n)` yields `(n, 1)` — an INTEGER
/// config value (`default: 5`, `range: [2, 20]`) on a Rational @param IS the exact rational n/1; this is the
/// common case, since a Rational bound written as a bare integer folds to `ConstInt` (a lone int literal has
/// no Rational-typed context at the config node), so without this arm the num/den companions never populate
/// for an integer-bound Rational param. Returns `None` for a node that folds to neither (a `Float` bound, a
/// non-constant expression) — the caller reads the literal-text field. Reads from the compiler's own fold,
/// NOT by parsing source, so it is robust to any surface/printer change.
fn rational_num_den(db: &mut rcdzc::db::Db, id: rcdzc::ast::StructId) -> Option<(String, String)> {
    match rcdzc::lower::core_of(db, id) {
        rcdzc::core::Core::ConstRational(n, d) => {
            Some((n.to_decimal_string(), d.to_decimal_string()))
        }
        // An integer config value IS the exact rational n/1 — the common bare-integer bound on a Rational
        // @param (`default: 5`, `range: [2, 20]`), which folds to `ConstInt` at the untyped config node.
        rcdzc::core::Core::ConstInt(n) => Some((n.to_decimal_string(), "1".to_string())),
        _ => None,
    }
}

/// The WIDGET MANIFEST of every `@param` site in `text` — the metadata a parametric host (the operator's
/// single-mode `/cad`) reads from a compiled model to render a slider/control per param and drive it over
/// the host-response path (`Param.<name>` / the num/den pair for a Rational). Rides `param_sidecar::
/// scan_manifest` (the SAME scan the `cdz param-manifest` CLI + `Query::ParamManifest` use), rendering each
/// record's type + config nodes to JS-readable strings. Mirrors [`param_test_signatures`] for the property-
/// test driver. A parse error / no `@param` yields an empty list (a model with no params needs no controls).
#[wasm_bindgen]
pub fn param_manifest(text: &str, from: &str) -> Result<Vec<ParamManifestEntry>, JsError> {
    let from = parse_format(from)?;
    let Ok((ast_bytes, _spans)) = parse_spanned(text, from) else {
        return Ok(Vec::new());
    };
    let Some(arenas) = rcdzc::codec::decode(&ast_bytes) else {
        return Ok(Vec::new());
    };
    let mut db = rcdzc::db::Db::load(arenas);
    // Scan first (an immutable borrow), collecting the records, THEN render types (a mutable borrow of `db`
    // for `typeval_of`'s memoization) — the two borrows must not overlap. Same ordering as the CLI's
    // `param_manifest_text`, so the browser manifest's type equals what the type checker asserts.
    let records = rcdzc::param_sidecar::scan_manifest(&db.ast);
    let mut out = Vec::with_capacity(records.len());
    for rec in records {
        // The declared TYPE-EXPRESSION node reduced to its type VALUE (same as the CLI), falling back to the
        // node's inferred type render so the field is always a definite string.
        // Compute the Ty first (releases the `&mut db` borrow), THEN build the render-time `NameCtx`
        // (an immutable borrow of `db.type_decls`) and render — the two borrows must not overlap.
        let ty = match rcdzc::eval::typeval_of(&mut db, rec.ty) {
            Some(t) => t,
            None => rcdzc::infer::type_of(&mut db, rec.ty),
        };
        let type_name = ty.render_name(&db.name_ctx());
        // Literal-text fields (always present when the config is) — an immutable borrow of the arena.
        let (range_lo, range_hi) = match rec.range {
            Some((lo, hi)) => (
                Some(render_manifest_node(&db.ast, lo)),
                Some(render_manifest_node(&db.ast, hi)),
            ),
            None => (None, None),
        };
        let default = rec.default.map(|d| render_manifest_node(&db.ast, d));
        // Exact num/den companions — ONLY for a `Rational` param (the caller reads these when
        // `type_name === "Rational"`; an Int64/Float param reads the integer/literal-text field instead).
        // Gating on the declared type keeps num/den absent for a non-Rational param even though an integer
        // config folds to `ConstInt` for every param — an Int64 slider's `5` is not a rational 5/1 to drive.
        let is_rational = type_name == "Rational";
        let (range_lo_num, range_lo_den) = match rec
            .range
            .filter(|_| is_rational)
            .and_then(|(lo, _)| rational_num_den(&mut db, lo))
        {
            Some((n, d)) => (Some(n), Some(d)),
            None => (None, None),
        };
        let (range_hi_num, range_hi_den) = match rec
            .range
            .filter(|_| is_rational)
            .and_then(|(_, hi)| rational_num_den(&mut db, hi))
        {
            Some((n, d)) => (Some(n), Some(d)),
            None => (None, None),
        };
        let (default_num, default_den) = match rec
            .default
            .filter(|_| is_rational)
            .and_then(|d| rational_num_den(&mut db, d))
        {
            Some((n, d)) => (Some(n), Some(d)),
            None => (None, None),
        };
        out.push(ParamManifestEntry {
            name: rec.name,
            type_name,
            widget: rec.widget,
            range_lo,
            range_hi,
            default,
            range_lo_num,
            range_lo_den,
            range_hi_num,
            range_hi_den,
            default_num,
            default_den,
        });
    }
    Ok(out)
}

/// The names of every top-level `def` the buffer declares — for the playground REPL's autocomplete.
/// Parses the buffer (surface-aware) and reads each definition's bound name, in source order. A parse
/// error yields an empty list (autocomplete is a nicety; a mid-edit unparseable buffer just offers
/// nothing rather than erroring). Callable functions AND bare value bindings are both included.
#[wasm_bindgen]
pub fn defined_names(buffer: &str, from: &str) -> Result<Vec<String>, JsError> {
    let from = parse_format(from)?;
    let Ok((bytes, _)) = parse_spanned(buffer, from) else {
        return Ok(Vec::new());
    };
    let Some(arenas) = cadenza_syntax::codec::decode(&bytes) else {
        return Ok(Vec::new());
    };
    Ok(cadenza_syntax::repl::defined_names(&arenas))
}

/// Evaluate an EXPRESSION against the reader's BUFFER of definitions — the playground's mini-REPL.
/// Builds one runnable module (every top-level `def`/`type` the buffer declares, then a synthesized
/// `(def (cdz-repl-eval) <expr>)` exported as the sole entry, the buffer's OWN exports dropped),
/// compiles it, and returns the component + diagnostics exactly as [`compile`] does — so the caller
/// runs the result through the SAME run worker, and a REPL result (scalar OR compound) renders like a
/// normal run, in the reader's surface. The reader effectively calls any function their module defines,
/// composing them freely, written in the syntax they're already using.
///
/// Both pieces are parsed at the AST level and re-emitted into one arena — NOT string-spliced — so a
/// string literal containing parentheses (or any surface quirk) can't corrupt the assembly. A parse
/// error in the buffer or the expression comes back as one codeless error [`Diagnostic`] (the caller
/// shows it in the REPL). Compiled as plain `Target::Wasm` (no DWARF): a REPL call is run, not stepped.
///
/// Accepts a bare expression (`(dbl 21)`), and tolerates a buffer that is itself a bare expression
/// rather than defs (nothing to call — the expression stands alone). The buffer may be a `(do …)`
/// block (the guide editor's wrap), a `(module NAME …)` (a hand-written program), or a bare form.
/// `exact` selects the CALCULATOR's forced-rational mode: when true, the expression's bare numeric
/// literals ground to an exact `Rational` (via `assemble_repl_program_exact`'s do-local
/// `(pragma default-fraction Rational)` module — C6), so `1 / 3` is `1/3` with no `R` suffix. The
/// general playground REPL passes `false` (ordinary Int64/Float defaults).
#[wasm_bindgen]
pub fn repl_eval(
    buffer: &str,
    expr: &str,
    from: &str,
    exact: bool,
) -> Result<CompileResult, JsError> {
    let from = parse_format(from)?;

    // A parse failure in either piece → one codeless error diagnostic (uniform with `compile`).
    let repl_parse_err = |msg: String| compile_error(msg, 0, 0);

    // Parse both pieces into their own arenas (surface-aware, spanless — the REPL module is freshly
    // synthesized, so its spans aren't the buffer's and aren't needed for the run).
    let buf_bytes = match parse_spanned(buffer, from) {
        Ok((bytes, _)) => bytes,
        Err(msg) => return Ok(repl_parse_err(msg)),
    };
    let buf_arenas = cadenza_syntax::codec::decode(&buf_bytes)
        .ok_or_else(|| JsError::new("internal: buffer AST failed to decode"))?;
    let expr_bytes = match parse_spanned(expr, from) {
        Ok((bytes, _)) => bytes,
        Err(msg) => return Ok(repl_parse_err(format!("in the expression: {msg}"))),
    };
    let expr_arenas = cadenza_syntax::codec::decode(&expr_bytes)
        .ok_or_else(|| JsError::new("internal: REPL expression AST failed to decode"))?;

    // Assemble the combined program via the SHARED assembler (`cadenza_syntax::repl`) — the same one the
    // native `cdz calc` REPL uses, so the two surfaces never drift in how the buffer's items are
    // unwrapped and the `(def (cdz-repl-eval) <expr>)` entry synthesized + exported.
    let arenas = if exact {
        cadenza_syntax::repl::assemble_repl_program_exact(&buf_arenas, &expr_arenas)
    } else {
        cadenza_syntax::repl::assemble_repl_program(&buf_arenas, &expr_arenas)
    };
    let ast_bytes = cadenza_syntax::codec::encode(&arenas);

    // Compile the synthesized module (plain Wasm — a REPL call is run, not stepped). Diagnostics from a
    // type error in the expression (or the buffer) ride back so the REPL can show them; they anchor to
    // the SYNTHESIZED module's nodes, so no span table is passed (the REPL surfaces the message text,
    // not an editor squiggle).
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[rcdzc::Target::Wasm],
    );
    let diagnostics = out
        .diagnostics
        .iter()
        .map(|d| to_js_diag(d, None, from == Format::Ml))
        .collect();
    let component = out
        .artifact(rcdzc::Target::Wasm.artifact_kind())
        .map(|b| b.to_vec());
    Ok(CompileResult {
        component,
        diagnostics,
    })
}

/// Project a front-end `SpanTable` (+ the source text) into rcdzc's `spans::SpanData` wire form — the
/// `(start, len)` byte range per `StructId`, a module path, and the source text (for DWARF line
/// derivation). MIRRORS rcdzc's format at this driver boundary (the two crates share no code, so the
/// mapping lives wherever both are held — here, and in the `cdz` bin). The module path is a stable
/// `"main"` label (the guide compiles one in-memory buffer; there is no tree-relative file path).
fn span_data_of(
    source: &str,
    spantable: &cadenza_syntax::spans::SpanTable,
) -> rcdzc::spans::SpanData {
    let spans: Vec<(u32, u32)> = (0..spantable.len())
        .map(
            |i| match spantable.get(cadenza_syntax::StructId(i as u32)) {
                Some(sp) => (sp.start as u32, (sp.end - sp.start) as u32),
                None => (0, 0),
            },
        )
        .collect();
    rcdzc::spans::SpanData {
        module_path: "main.cdz".to_string(),
        spans,
        source: source.to_string(),
    }
}

/// Type-check `text` (in `from` surface) and return every well-formedness diagnostic — WITHOUT
/// requiring the program to export anything or emit. This is the as-you-type entry: it reads the
/// surface into an arena + span table, loads a `Db`, and runs the same total fault collection a
/// compile does (`rcdzc::diagnostics`), so a mid-edit buffer or a set of sibling defs gets its real
/// type/shape faults with source ranges. Each diagnostic carries `from`/`to` byte offsets resolved
/// through the span table. A parse failure comes back as one codeless error diagnostic.
///
/// Note: like `compile`, this checks DEFINITION BODIES. A bare top-level expression is not a
/// definition, so the caller wraps a snippet into a module (`(module m (def (main) <expr>) …)`) — the
/// same wrapping the runnable examples already do — before asking for diagnostics.
#[wasm_bindgen]
pub fn diagnostics(text: &str, from: &str) -> Result<Vec<Diagnostic>, JsError> {
    let from = parse_format(from)?;
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(msg) => {
            return Ok(vec![codeless_error(msg, 0, 0)]);
        }
    };
    // Ride the first-class `Diagnostics` sidecar query (the same one `cdz check` runs) — a total fault
    // read that needs no export. Its `KIND_DIAGNOSTICS` result is canonical BINARY AST (operator seq-254:
    // binary AST everywhere), decoded with the shared `rcdzc::decode_diagnostics` into `Diagnostic`
    // structs — no bespoke tab-column parse. We resolve each fault's node id — and its fix's node id — to
    // a byte span here.
    let diag_bytes = run_query_bytes(&ast_bytes, &rcdzc::Query::Diagnostics)?;
    // Single-file: every node id keys directly into the user's span table (a miss defaults to the
    // whole-buffer `(0, 0)` — we never drop a single-file fault).
    let span_of = |n: u32| -> Option<(u32, u32)> {
        Some(
            spans
                .as_ref()
                .and_then(|s| s.get(cadenza_syntax::ast::StructId(n)))
                .map(|s| (s.start as u32, s.end as u32))
                .unwrap_or((0, 0)),
        )
    };
    Ok(diags_to_js(
        &rcdzc::decode_diagnostics(&diag_bytes),
        from == Format::Ml,
        span_of,
    ))
}

/// The browser-facing `fix_kind` string for a [`rcdzc::FixKind`] — the SAME vocabulary the `cdz` LSP
/// emits (`lsp::fix_kind_str`), so the guide's quick-fix affordance keys on one stable set
/// (`replace`/`insert`/`wrap`/`delete`).
fn fix_kind_str(kind: rcdzc::FixKind) -> &'static str {
    match kind {
        rcdzc::FixKind::Replace => "replace",
        rcdzc::FixKind::InsertInto => "insert",
        rcdzc::FixKind::Wrap => "wrap",
        rcdzc::FixKind::Delete => "delete",
    }
}

/// Map the decoded [`rcdzc::Diagnostic`] structs (from the binary-AST `KIND_DIAGNOSTICS` wire, via
/// `rcdzc::decode_diagnostics`) into JS [`Diagnostic`]s. `resolve_span` maps a real node id to its
/// `[from, to)` byte range in the source UNDER EDIT — the ONE thing the single-file and preloaded-package
/// paths differ on (the package path demuxes a GLOBAL id through the link-map to the user file first).
/// Returning `None` means "this fault's node is not in the source under edit" — the fault is DROPPED
/// (over a linked package that is a fault inside a trusted preloaded library, which the reader can't act
/// on in their own buffer; the single-file path's `resolve_span` is total, so it never drops). An
/// UNANCHORED fault (`node == None`) is always kept with a `(0, 0)` range (a whole-program fault, not
/// tied to a node). `is_ml` selects the surface for a `wrap` fix's prefix/suffix split.
fn diags_to_js(
    diags: &[rcdzc::Diagnostic],
    is_ml: bool,
    resolve_span: impl Fn(u32) -> Option<(u32, u32)>,
) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for d in diags {
        // Resolve the primary node. An unanchored (`None`) node stays a whole-buffer `(0, 0)` fault; a
        // real id that `resolve_span` places OUTSIDE the source under edit (a preloaded-library fault) is
        // dropped — `continue` skips it so it never squiggles the user's buffer at a bogus offset.
        let (span_from, span_to) = match d.node {
            Some(n) => match resolve_span(n) {
                Some(span) => span,
                None => continue,
            },
            None => (0, 0),
        };
        // A fix that targets a node outside the source under edit is dropped to a no-fix (the diagnostic
        // is still shown; only its quick-fix, which would splice the wrong file, is suppressed).
        let resolved_fix = d
            .fix
            .as_ref()
            .and_then(|f| resolve_span(f.node).map(|span| (f, span)));
        // A WRAP fix splits into surface-correct `prefix`/`suffix` (never the raw `…`-sentinel
        // replacement, which a JS splice would write literally and corrupt) — the shared reshape+split.
        let (fix_replacement, fix_prefix, fix_suffix, fix_from, fix_to, fix_verified, fix_kind) =
            match resolved_fix {
                Some((f, (ff, ft))) => {
                    let (repl, prefix, suffix) = if matches!(f.kind, rcdzc::FixKind::Wrap) {
                        let (p, s) = rcdzc::wrap_prefix_suffix(&f.replacement, is_ml);
                        (String::new(), p, s)
                    } else {
                        (f.replacement.clone(), String::new(), String::new())
                    };
                    (
                        repl,
                        prefix,
                        suffix,
                        ff,
                        ft,
                        f.verified,
                        fix_kind_str(f.kind).to_string(),
                    )
                }
                None => (
                    String::new(),
                    String::new(),
                    String::new(),
                    0,
                    0,
                    false,
                    String::new(),
                ),
            };
        out.push(Diagnostic {
            error: d.severity != rcdzc::Severity::Warning,
            code: d.code.clone().unwrap_or_default(),
            message: d.message.clone(),
            node: d.node.unwrap_or(u32::MAX),
            from: span_from,
            to: span_to,
            fix_replacement,
            fix_prefix,
            fix_suffix,
            fix_from,
            fix_to,
            fix_verified,
            fix_kind,
        });
    }
    out
}

/// A preloaded-package link set up for an IDE fact query: the compile inputs (user `main` AST + each
/// preloaded library `ast` + a `KIND_ENTRY` marker + a sidecar request) plus the USER model's span
/// table. Produced by [`link_preloaded_query`]; the caller runs `rcdzc::compile`, extracts its query
/// artifact, and maps the result's GLOBAL node ids back to user spans via [`user_span_resolver`].
struct PreloadedQuery {
    inputs: Vec<rcdzc::Artifact>,
    /// The user model's span table (`None` only for a span-less surface — never for ml/sexpr text).
    spans: Option<cadenza_syntax::spans::SpanTable>,
}

/// Why a preloaded-package IDE query could not be set up (before it even links). Each preload-aware
/// entry renders this in its OWN result type (a `Diagnostic` list, an empty token list, …).
enum PreloadSetupError {
    /// The user model itself failed to parse — carry the message + the mistake's source byte range.
    UserParse { message: String, from: u32, to: u32 },
    /// A preloaded LIBRARY module failed to parse — carry its name + the parse message.
    BrokenModule { name: String, message: String },
}

/// Validate + parse a preloaded-package IDE query and build its compile inputs. Shared by every
/// preload-aware IDE entry (`diagnostics_with_preloaded`, `semantic_tokens_with_preloaded`, …): each
/// preloaded source becomes an `ast` artifact NAMED by its module (the target of `import from "<name>"`),
/// the user text becomes the `main` `ast`, a `KIND_ENTRY` marks `main` the entry (so `rcdzc::compile`
/// LINKS the package exactly as `compile_with_preloaded` does), and `query` rides as the sidecar request
/// run over the whole package. Returns `Err(JsError)` for a mismatched-array-length caller bug; the inner
/// `Result` distinguishes a set-up failure ([`PreloadSetupError`]) the caller renders in its own result
/// type from a linked-OK [`PreloadedQuery`]. The `preloaded_names` MUST be non-empty (callers short-circuit
/// the empty case to the plain single-file entry first, keeping that path byte-identical).
fn link_preloaded_query(
    text: &str,
    from: Format,
    preloaded_names: &[String],
    preloaded_sources: &[String],
    preloaded_formats: &[String],
    query: rcdzc::Query,
) -> Result<Result<PreloadedQuery, PreloadSetupError>, JsError> {
    if preloaded_names.len() != preloaded_sources.len()
        || preloaded_names.len() != preloaded_formats.len()
    {
        return Err(JsError::new(
            "preloaded_names/sources/formats must be equal length",
        ));
    }

    // Parse the user model into the `main` AST + span table (a parse failure is surfaced by the caller).
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(msg) => {
            let (from_b, to_b) = ml_parse_error_span(text, from).unwrap_or((0, 0));
            return Ok(Err(PreloadSetupError::UserParse {
                message: msg,
                from: from_b,
                to: to_b,
            }));
        }
    };

    let mut inputs = vec![rcdzc::Artifact::new(
        rcdzc::Artifact::KIND_AST,
        "main",
        ast_bytes,
    )];
    for ((name, source), fmt) in preloaded_names
        .iter()
        .zip(preloaded_sources.iter())
        .zip(preloaded_formats.iter())
    {
        let pf = parse_format(fmt)?;
        match parse_spanned(source, pf) {
            Ok((bytes, _)) => {
                inputs.push(rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, name, bytes));
            }
            Err(msg) => {
                return Ok(Err(PreloadSetupError::BrokenModule {
                    name: name.clone(),
                    message: msg,
                }));
            }
        }
    }
    inputs.push(rcdzc::cli::entry_artifact("main"));
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(query)]),
    ));
    Ok(Ok(PreloadedQuery { inputs, spans }))
}

/// A closure that maps a linked package's GLOBAL node id to the USER model's `[from, to)` byte range,
/// returning `None` for an id OUTSIDE the user buffer (one inside a preloaded library — an IDE overlay
/// the reader can't place in their own text, so it's dropped). Demuxes through the package `link-map`
/// (`decode_link_map` → `FileSpan{path, struct_base, struct_count}`): the `main` file's `[base,
/// base+count)` range rebases to its local id space, which the user span table is keyed by. A compile
/// that emitted no link-map (linking collapsed to one file) treats every id as already local.
fn user_span_resolver(
    out: &rcdzc::CompileOutput,
    spans: Option<cadenza_syntax::spans::SpanTable>,
) -> impl Fn(u32) -> Option<(u32, u32)> {
    let link_map = out
        .artifact(rcdzc::link::KIND_LINK_MAP)
        .map(rcdzc::link::decode_link_map)
        .unwrap_or_default();
    let main_range = link_map
        .iter()
        .find(|fs| fs.path == "main")
        .map(|fs| (fs.struct_base, fs.struct_count));
    move |global: u32| -> Option<(u32, u32)> {
        let local = match main_range {
            Some((base, count)) => {
                if global >= base && global < base + count {
                    global - base
                } else {
                    return None; // a node in a preloaded library — not the user's buffer
                }
            }
            None => global,
        };
        spans
            .as_ref()
            .and_then(|s| s.get(cadenza_syntax::ast::StructId(local)))
            .map(|s| (s.start as u32, s.end as u32))
    }
}

/// EMBED DWARF DEBUG INFO whenever we have a span table (a text surface): push the `spans` artifact into
/// `inputs` and select `Target::WasmDebug`, so the emitted component carries `.debug_line`/`.debug_info`
/// sections (Chrome's DWARF extension steps through the actual Cadenza source). A binary/output-only
/// surface has no spans and gets a plain `Target::Wasm`. The sections are inert + strippable, so this
/// costs nothing at runtime (`DESIGN-debug-info-rcdzc.md`, Mode E). Shared by `compile` /
/// `compile_with_preloaded`, which both DWARF the user `main` model.
fn push_spans_target(
    inputs: &mut Vec<rcdzc::Artifact>,
    text: &str,
    spans: &Option<cadenza_syntax::spans::SpanTable>,
) -> rcdzc::Target {
    match spans {
        Some(span_table) => {
            inputs.push(rcdzc::Artifact::new(
                rcdzc::spans::KIND_SPANS,
                "main",
                rcdzc::spans::encode(&span_data_of(text, span_table)),
            ));
            rcdzc::Target::WasmDebug
        }
        None => rcdzc::Target::Wasm,
    }
}

/// Run the compile and project its output into a [`CompileResult`]: map every diagnostic to its JS form
/// (source-ranged via `spans`) and extract the emitted component. Both `Wasm` and `WasmDebug` produce a
/// `component`-kinded artifact (a debug component is a decorated component, not a new kind), so the
/// artifact lookup is the same either way. The shared compile-epilogue behind `compile` /
/// `compile_with_preloaded`.
fn finish_compile(
    inputs: &[rcdzc::Artifact],
    target: rcdzc::Target,
    spans: Option<&cadenza_syntax::spans::SpanTable>,
    is_ml: bool,
) -> CompileResult {
    let out = rcdzc::compile(inputs, &[target]);
    let diagnostics = out
        .diagnostics
        .iter()
        .map(|d| to_js_diag(d, spans, is_ml))
        .collect();
    let component = out
        .artifact(rcdzc::Target::Wasm.artifact_kind())
        .map(|b| b.to_vec());
    CompileResult {
        component,
        diagnostics,
    }
}

/// One codeless error [`Diagnostic`] with an optional source range — the uniform shape for a parse
/// failure / broken-library / internal-decline reported by the preload-aware diagnostics entry.
fn codeless_error(message: String, from: u32, to: u32) -> Diagnostic {
    Diagnostic {
        error: true,
        code: String::new(),
        message,
        node: u32::MAX,
        from,
        to,
        fix_replacement: String::new(),
        fix_prefix: String::new(),
        fix_suffix: String::new(),
        fix_from: 0,
        fix_to: 0,
        fix_verified: false,
        fix_kind: String::new(),
    }
}

/// Type-check `text` LINKED against preloaded library modules and return diagnostics over the USER
/// text's spans — the preload-aware sibling of [`diagnostics`], mirroring [`compile_with_preloaded`]'s
/// linking so the `/cad` IDE linter resolves a model's `import from "<lib>"` against an ambiently-supplied
/// library instead of false-flagging every imported name unbound. `preloaded_names[i]` is the module a
/// `import from "<name>"` binds to, `preloaded_sources[i]` its source, `preloaded_formats[i]` its surface;
/// the three MUST be equal length.
///
/// Spans map to the USER model (the buffer the reader edits), NOT the preloaded libraries: the user text
/// is the `main` file, the library modules are linked siblings, and Diagnostics runs over the whole
/// package so a cross-file name resolves. Each fault's GLOBAL node id is demuxed through the package
/// link-map back to the `main` file's LOCAL id and looked up in the user span table; a fault landing in a
/// preloaded library (a fault the reader can't act on in their own buffer) is dropped. With no preloaded
/// modules this is byte-identical to [`diagnostics`] (no linkage, flat namespace) — an editor that passes
/// empty arrays behaves exactly as before.
#[wasm_bindgen]
pub fn diagnostics_with_preloaded(
    text: &str,
    from: &str,
    preloaded_names: Vec<String>,
    preloaded_sources: Vec<String>,
    preloaded_formats: Vec<String>,
) -> Result<Vec<Diagnostic>, JsError> {
    let from = parse_format(from)?;

    // No preloaded modules → nothing to link; the flat single-file diagnostics path is byte-identical.
    if preloaded_names.is_empty() {
        return diagnostics(text, from.name());
    }

    let setup = link_preloaded_query(
        text,
        from,
        &preloaded_names,
        &preloaded_sources,
        &preloaded_formats,
        rcdzc::Query::Diagnostics,
    )?;
    let PreloadedQuery { inputs, spans } = match setup {
        Ok(pq) => pq,
        // A user parse failure → one codeless error carrying the mistake's source range (so the editor
        // underlines the syntax error); a broken library → one codeless error naming the module (matching
        // `compile_with_preloaded`), rather than a cascade of "unbound" faults for its every export.
        Err(PreloadSetupError::UserParse { message, from, to }) => {
            return Ok(vec![codeless_error(message, from, to)]);
        }
        Err(PreloadSetupError::BrokenModule { name, message }) => {
            return Ok(vec![codeless_error(
                format!("preloaded module `{name}` failed to parse: {message}"),
                0,
                0,
            )]);
        }
    };

    let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS) else {
        // The diagnostics query itself failed to produce its artifact (an internal decline) — surface it
        // as one codeless fault rather than an empty (falsely-clean) diagnostic set.
        return Ok(vec![codeless_error(
            "diagnostics query produced no result".to_string(),
            0,
            0,
        )]);
    };
    let resolve_span = user_span_resolver(&out, spans);
    Ok(diags_to_js(
        &rcdzc::decode_diagnostics(bytes),
        from == Format::Ml,
        resolve_span,
    ))
}

/// The inferred type at a source byte offset — for a hover tooltip. Finds the innermost user node
/// whose span contains `byte_offset`, reads its solved type from the type column
/// (`infer::type_of` → `Ty::render_name`), and returns that type text plus the node's byte range (so
/// the caller highlights exactly the sub-expression). Returns `None` (via a null-carrying result) when
/// there is no user node at the offset or no meaningful type. Total: never errors on a well-parsed
/// buffer.
#[wasm_bindgen(getter_with_clone)]
pub struct TypeAt {
    pub type_name: String,
    pub from: u32,
    pub to: u32,
}

#[wasm_bindgen]
pub fn type_at(text: &str, from: &str, byte_offset: u32) -> Result<Option<TypeAt>, JsError> {
    let from = parse_format(from)?;
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(_) => return Ok(None), // a buffer that won't parse has no type-at
    };
    let Some(spans) = spans else { return Ok(None) };
    // Resolve the offset to the innermost containing node via the SHARED helper — the SAME
    // offset→node resolution the `cdz type-at` CLI uses (`SpanTable::node_at_offset`), so the browser
    // IDE's "type at cursor" and the CLI agree by construction rather than by two copies of the loop.
    // (The span table only holds user nodes, so any hit is a user node.)
    let off = byte_offset as usize;
    let Some(node) = spans.node_at_offset(off) else {
        return Ok(None);
    };
    let span = spans
        .get(node)
        .expect("node_at_offset returned a spanned node");
    // Ride the `TypeAt` sidecar query (the same one `cdz type-at` runs) — the node id crosses the
    // copy-don't-depend boundary as its raw index (the byte-identical codec keeps the index space
    // aligned). The result is a BINARY-AST hover verdict (`type_at_wire`, seq-254 "binary AST is THE
    // data exchange format"), NOT text — so take the BYTES and DECODE them. (Interpreting the binary as
    // UTF-8 via `run_query_text` was a bug: it either threw "not valid UTF-8" or rendered garbage — the
    // same class as the `export_types`/`KIND_EXPORTS` regression, #6324.) Render the decoded verdict to
    // the display type text the guide editor's hover shows, mirroring `cdz`'s `render_type_at`.
    let bytes = run_query_bytes(&ast_bytes, &rcdzc::Query::TypeAt { node: node.0 })?;
    let type_name = render_type_at_verdict(&rcdzc::sidecar::decode_type_at(&bytes));
    Ok(Some(TypeAt {
        type_name,
        from: span.start as u32,
        to: span.end as u32,
    }))
}

/// Render a decoded [`rcdzc::sidecar::TypeAt`] hover verdict to its display type text — the SAME mapping
/// `cdz`'s `render_type_at` uses (kept in sync; the shared renderer is `cadenza_syntax::render_ty`). A
/// definition renders `name : <scheme>`, a keyword `keyword <kw>`, a bare typed node its rendered type,
/// and an untypeable/non-user node `unknown`. `render_ty_scheme` (not `render_ty`) because an export /
/// definition signature may be polymorphic and gets stable Var-lettering.
fn render_type_at_verdict(v: &rcdzc::sidecar::TypeAt) -> String {
    // Alias so the verdict enum doesn't collide with this crate's wasm-bindgen `TypeAt` return struct.
    use rcdzc::sidecar::TypeAt as Verdict;
    match v {
        Verdict::Def { name, ty } => {
            let t = match ty {
                Some(a) => cadenza_syntax::render_ty::render_ty_scheme(a, a.root),
                None => "unknown".to_string(),
            };
            format!("{name} : {t}")
        }
        Verdict::Keyword(kw) => format!("keyword {kw}"),
        Verdict::Ty(a) => cadenza_syntax::render_ty::render_ty_scheme(a, a.root),
        Verdict::Unknown => "unknown".to_string(),
    }
}

/// The definition a name at a source byte offset refers to — for go-to-definition. Resolves the
/// innermost user node at the offset; if it is a REFERENCE to a binding (`Resolved::Ref`), returns the
/// byte range of the bound value (the `let`/`def`/parameter initializer occurrence). Returns `None`
/// when the offset isn't on a resolvable reference, when it resolves to something without a source
/// span (a prelude/built-in binding), or when it already IS the definition. Total on a well-parsed
/// buffer.
#[wasm_bindgen(getter_with_clone)]
pub struct DefineAt {
    /// Byte range of the definition the reference points to.
    pub from: u32,
    pub to: u32,
    /// Byte range of the reference itself (the token under the cursor), so the caller can confirm the
    /// cursor was on a navigable name.
    pub ref_from: u32,
    pub ref_to: u32,
}

#[wasm_bindgen]
pub fn define_at(text: &str, from: &str, byte_offset: u32) -> Result<Option<DefineAt>, JsError> {
    let from = parse_format(from)?;
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(_) => return Ok(None),
    };
    let Some(spans) = spans else { return Ok(None) };
    let off = byte_offset as usize;
    let Some(node) = spans.node_at_offset(off) else {
        return Ok(None);
    };
    let ref_span = spans
        .get(node)
        .expect("node_at_offset returned a spanned node");
    // Ride the `ResolveOf` sidecar query (the same one `cdz def` runs): it resolves the reference at
    // this node to its defining occurrence's node id (following `Ref`/`Lambda`), or none for a
    // non-navigable token or a span-less binding. The result is a BINARY-AST value (`resolve_wire`,
    // #6152), NOT text — so DECODE the node id (`String::from_utf8` + `parse` on the binary was the same
    // from_utf8-on-binary bug class as `type_at`/`export_types` #6324).
    let bytes = run_query_bytes(&ast_bytes, &rcdzc::Query::ResolveOf { node: node.0 })?;
    let Some(target_id) = rcdzc::sidecar::decode_resolve(&bytes) else {
        return Ok(None); // not a navigable reference
    };
    let target = cadenza_syntax::ast::StructId(target_id);
    // The target must be a USER node with a source span (a prelude binding has none — nothing to jump
    // to in the editor). The span table is keyed in the shared index space (codec keeps ids aligned).
    let Some(def_span) = spans.get(target) else {
        return Ok(None);
    };
    // Don't offer a no-op jump to the same place the cursor already is.
    if def_span.start == ref_span.start && def_span.end == ref_span.end {
        return Ok(None);
    }
    Ok(Some(DefineAt {
        from: def_span.start as u32,
        to: def_span.end as u32,
        ref_from: ref_span.start as u32,
        ref_to: ref_span.end as u32,
    }))
}

/// Every source occurrence that references the same definition as the name at a byte offset — for
/// find-all-references / highlight-occurrences. Finds the name at the cursor, asks the compiler for
/// every use of that name (the `UsesOf` sidecar query — the transpose of the resolution column), and
/// returns the byte range of each use PLUS the cursor's own occurrence. A flat `[from0,to0,from1,to1,…]`
/// so it crosses the wasm-bindgen boundary as one `Uint32Array`. Empty when the cursor isn't on a
/// name, or the name has no references.
#[wasm_bindgen]
pub fn references_at(text: &str, from: &str, byte_offset: u32) -> Result<Vec<u32>, JsError> {
    let from = parse_format(from)?;
    let Ok((ast_bytes, Some(spans))) = parse_spanned(text, from) else {
        return Ok(Vec::new());
    };
    let off = byte_offset as usize;
    let Some(node) = spans.node_at_offset(off) else {
        return Ok(Vec::new());
    };
    let arenas = match rcdzc::codec::decode(&ast_bytes) {
        Some(a) => a,
        None => return Err(JsError::new("internal: re-encoded AST failed to decode")),
    };
    let mut db = rcdzc::db::Db::load(arenas);
    // The name at the cursor. Only a bare-name occurrence has references to find; anything else yields
    // an empty set. (`as_name` returns the source spelling of a name leaf.)
    let Some(name) = db
        .ast
        .as_name(rcdzc::ast::StructId(node.0))
        .map(|s| s.to_string())
    else {
        return Ok(Vec::new());
    };
    // The `UsesOf` sidecar query returns the referencing node ids (declaration sites + the definition
    // excluded), one per line. Ride the first-class query so the browser IDE and `cdz uses` agree.
    let result = rcdzc::sidecar::run_query(&mut db, &rcdzc::Query::UsesOf { name });
    let text_ids = String::from_utf8(result.bytes).unwrap_or_default();
    let mut out: Vec<u32> = Vec::new();
    // Include the occurrence under the cursor itself (a use or the declaration — either way, highlight
    // it), then every use the query found. De-dup by (from,to) so the cursor node isn't doubled.
    let push_span = |id: u32, out: &mut Vec<u32>| {
        if let Some(s) = spans.get(cadenza_syntax::ast::StructId(id)) {
            let (f, t) = (s.start as u32, s.end as u32);
            let mut k = 0;
            while k + 1 < out.len() {
                if out[k] == f && out[k + 1] == t {
                    return;
                }
                k += 2;
            }
            out.push(f);
            out.push(t);
        }
    };
    push_span(node.0, &mut out);
    for line in text_ids.lines() {
        if let Ok(id) = line.trim().parse::<u32>() {
            push_span(id, &mut out);
        }
    }
    Ok(out)
}

/// The exported names of a program paired with their solved types, as `name<TAB>type` lines (one per
/// export; empty when the program has no exports or doesn't parse).
///
/// The browser RUN path calls a scalar export and stringifies the JS return value, but jco lowers a
/// whole-number `Float64`/`Float32` to a JS integer-valued `number` — so `String(5)` drops the `.0` and
/// a float prints indistinguishably from an `Int64`. The value type alone can't disambiguate (sized
/// ints and `Qty.value` are `number` too), only the STATIC result type can. This exposes that type (via
/// the existing `Exports` sidecar query — the same one `cdz`'s module-interface read uses) so the runner
/// can format a `Float*`-typed scalar with a forced decimal and leave `Int*`/`Qty`/`Bool` alone. The
/// node-id column the query also emits is dropped here — the run path only needs name→type.
#[wasm_bindgen]
pub fn export_types(text: &str, from: &str) -> Result<String, JsError> {
    let from = parse_format(from)?;
    // Parse to AST bytes (spans unused — this is a whole-program query, not a cursor query). A buffer
    // that won't parse simply has no known exports → empty string.
    let ast_bytes = match parse_spanned(text, from) {
        Ok((bytes, _spans)) => bytes,
        Err(_) => return Ok(String::new()),
    };
    // `Query::Exports` yields the `KIND_EXPORTS` BINARY-AST payload (`exports_wire`), NOT text: each entry
    // carries the export NAME + its FULL structured type payload as a sub-AST arena (operator seq-307: full
    // type AST, no render-name string on the wire — the CONSUMER renders a display name from the decoded
    // structure, exports_wire.rs). So take the bytes (NOT `run_query_text`, which would `from_utf8` the
    // binary → a fatal "not valid UTF-8" throw on a complex type, or a silent empty parse on a simple one —
    // the guide-examples render regression), decode them, and render each type via the SHARED
    // `cadenza_syntax::render_ty` renderer (byte-parity with `Ty::render_name`/`Scheme::render_scheme`) to
    // reconstruct the documented `name<TAB>type` text contract the JS consumers parse.
    let bytes = run_query_bytes(&ast_bytes, &rcdzc::Query::Exports)?;
    let mut out = String::new();
    for entry in rcdzc::sidecar::decode_exports(&bytes) {
        out.push_str(&entry.name);
        out.push('\t');
        // An export naming a def with a solved scheme carries its type payload; render it as a SCHEME
        // (a generalized sig's vars get stable letters, monomorphic types render exactly as the name).
        // An export with no def / unsolved type carries no payload → the type column is empty (the old
        // "unknown" behavior — the consumer treats a missing type as unknown).
        if let Some(ty) = &entry.ty {
            out.push_str(&cadenza_syntax::render_ty::render_ty_scheme(ty, ty.root));
        }
        out.push('\n');
    }
    Ok(out)
}

/// Re-render one program from one surface to another — the guide's global syntax toggle.
///
/// Because every surface is a lossless projection of the same binary AST, converting `text` from
/// `from` to `to` never changes the program; it only re-prints it. `to` may be an output-only view
/// (`debug`/`flat`) for "show the raw AST" affordances.
#[wasm_bindgen]
pub fn render_syntax(text: &str, from: &str, to: &str) -> Result<String, JsError> {
    let from = parse_format(from)?;
    let to = parse_format(to)?;
    let bytes = convert::convert(text.as_bytes(), from, to).map_err(|e| {
        JsError::new(&format!(
            "convert {} -> {}: {}",
            from.name(),
            to.name(),
            e.0
        ))
    })?;
    String::from_utf8(bytes).map_err(|_| JsError::new("rendered output was not valid UTF-8"))
}

/// Like [`render_syntax`], but renders the ML target for human DISPLAY (the spec's "typed-result-to-
/// text" surface): a `Rational` prints bare (`1/4`), a quantity in its concise `<value> <unit>` form,
/// and an outer result type annotation is dropped. Used by the calculator to render a result readably;
/// the playground keeps the canonical, re-readable [`render_syntax`]. For a non-ML target this is
/// identical to `render_syntax` (display is an ML-printer concern).
#[wasm_bindgen]
pub fn render_syntax_display(text: &str, from: &str, to: &str) -> Result<String, JsError> {
    let from = parse_format(from)?;
    let to = parse_format(to)?;
    // Read to the canonical arena, then write with display enabled (ML) or plainly (any other target).
    let arenas = convert::read(text.as_bytes(), from)
        .map_err(|e| JsError::new(&format!("read {}: {}", from.name(), e.0)))?;
    let opts = convert::Options {
        display: true,
        ..Default::default()
    };
    let bytes = convert::write_with(&arenas, to, opts)
        .map_err(|e| JsError::new(&format!("write {}: {}", to.name(), e.0)))?;
    String::from_utf8(bytes).map_err(|_| JsError::new("rendered output was not valid UTF-8"))
}

/// Decode canonical value-form bytes (the `list<u8>` an emitted compound program returns from its
/// `encode` accessor) into their `(: value type)` display text.
///
/// A compound result crosses the component boundary as opaque runtime handles; the emitted program
/// walks it into the deterministic value form, and the run worker decodes those bytes here — the same
/// `binary AST -> s-expression` path the reference runner (`cdz-run`) uses to render a result.
#[wasm_bindgen]
pub fn render_value(bytes: &[u8]) -> Result<String, JsError> {
    let text = convert::convert(bytes, Format::Binary, Format::Sexpr)
        .map_err(|e| JsError::new(&format!("decode value form: {}", e.0)))?;
    String::from_utf8(text).map_err(|_| JsError::new("decoded value was not valid UTF-8"))
}

/// Render an EMBEDDED canonical binary AST to a text SURFACE — the guide's `(cdz …)` inline-Cadenza tag
/// renders its base64-decoded binary-AST subtree per-surface for the auto-toggle, WITHOUT re-parsing text
/// (binary-AST is THE exchange format; one canonical render). `to` is the surface name (`ml`/`sexpr`/…);
/// `kind` is the fragment grammatical role the sibling tags carry (`expr` for `(cdz …)`, `type` for
/// `(cdz-type …)`, `pattern` for `(cdz-pat …)`) — see [`convert::FragmentKind`]. Generalizes
/// [`render_value`] (which is binary→sexpr only) to any surface + fragment kind.
#[wasm_bindgen]
pub fn render_binary(bytes: &[u8], to: &str, kind: &str) -> Result<String, JsError> {
    let to = parse_format(to)?;
    let kind = convert::FragmentKind::parse(kind).ok_or_else(|| {
        JsError::new(&format!(
            "unknown fragment kind: {kind} (expected expr|type|pattern)"
        ))
    })?;
    convert::render_binary(bytes, to, kind, Default::default())
        .map_err(|e| JsError::new(&format!("render binary -> {}: {}", to.name(), e.0)))
}

/// Emit the program as Rust SOURCE — the compiler's second backend. Cadenza is target-neutral above
/// the backend seam, so the same typed core lowers to a self-contained `.rs` module (one `pub fn` per
/// export). `is_async` selects the ASYNC, gas-metered calling convention (every emitted function is an
/// `async fn` threading an `env`, so a long computation yields rather than blocking) instead of the
/// plain synchronous form. Returns the Rust text, or the first error's message (a program that
/// declines emits no source). Lets the playground show "what this compiles to" beyond wasm.
#[wasm_bindgen]
pub fn emit_rust(text: &str, from: &str, is_async: bool) -> Result<String, JsError> {
    let from = parse_format(from)?;
    let (ast_bytes, _spans) = parse_spanned(text, from).map_err(|m| JsError::new(&m))?;
    let target = if is_async {
        rcdzc::Target::RustAsync
    } else {
        rcdzc::Target::Rust
    };
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[target],
    );
    match out.artifact(target.artifact_kind()) {
        Some(bytes) => String::from_utf8(bytes.to_vec())
            .map_err(|_| JsError::new("Rust output was not valid UTF-8")),
        None => {
            let msg = out
                .diagnostics
                .iter()
                .find(|d| d.severity == rcdzc::Severity::Error)
                .map(|d| match &d.code {
                    Some(c) => format!("{c}: {}", d.message),
                    None => d.message.clone(),
                })
                .unwrap_or_else(|| "this program does not emit Rust".to_string());
            Ok(format!("// declined:\n// {msg}"))
        }
    }
}

/// Emit the program as lowered-optimized CADENZA SOURCE — the compiler's third backend (`Target::Cadenza`),
/// which lowers the OPTIMIZED core IR back to Cadenza surface AST (AFTER type-resolution / const-fold /
/// optimization). Unlike the wat/rust views (which are text), this target emits the canonical BINARY AST,
/// so we RENDER it to a text surface for display: `syntax` is `"sexpr"` or `"ml"` (the view's toggle). Lets
/// the playground show what lowering + the optimizer did to the program — the same source, in its own
/// language, post-optimization. Returns the rendered text, or a `; declined: …` note (a program the Cadenza
/// backend does not yet lower emits nothing). Mirrors [`emit_rust`] + the binary→text render [`render_value`] uses.
#[wasm_bindgen]
pub fn emit_cadenza(text: &str, from: &str, syntax: &str) -> Result<String, JsError> {
    let from = parse_format(from)?;
    let to = match parse_format(syntax)? {
        f @ (Format::Sexpr | Format::Ml) => f,
        other => {
            return Err(JsError::new(&format!(
                "emit_cadenza display syntax must be \"sexpr\" or \"ml\", not {other:?}"
            )));
        }
    };
    let (ast_bytes, _spans) = parse_spanned(text, from).map_err(|m| JsError::new(&m))?;
    let target = rcdzc::Target::Cadenza;
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[target],
    );
    match out.artifact(target.artifact_kind()) {
        // `Target::Cadenza` emits the BINARY AST; render it to the requested text surface for display.
        Some(bytes) => {
            let rendered = convert::convert(bytes, Format::Binary, to)
                .map_err(|e| JsError::new(&format!("render cadenza output: {}", e.0)))?;
            String::from_utf8(rendered)
                .map_err(|_| JsError::new("cadenza output was not valid UTF-8"))
        }
        None => {
            let msg = out
                .diagnostics
                .iter()
                .find(|d| d.severity == rcdzc::Severity::Error)
                .map(|d| match &d.code {
                    Some(c) => format!("{c}: {}", d.message),
                    None => d.message.clone(),
                })
                .unwrap_or_else(|| "this program does not emit Cadenza".to_string());
            Ok(format!("; declined:\n; {msg}"))
        }
    }
}

/// The program's embedded CORE MODULE bytes — for the playground's "WAT" view. Compiles `text` to a
/// PLAIN `Target::Wasm` component (NO `spans` artifact, so NO DWARF `.debug_*` sections at all — the
/// debug info is only wanted in the browser debugger, not the human-readable WAT), then unwraps the
/// component down to the core wasm module it embeds. The caller prints THOSE bytes with `wasm-tools
/// print`, so the WAT view shows just the executed module (`(module …)`) rather than the component-
/// model wrapper (`(component (core module …) …)`) — the shape a reader actually wants to read.
///
/// Returns `None` if the program declines (no component to unwrap) or if the component carries no core
/// module (it never doesn't, but the extraction is total). A parse error surfaces as a `JsError`.
#[wasm_bindgen]
pub fn core_module(text: &str, from: &str) -> Result<Option<Vec<u8>>, JsError> {
    let from = parse_format(from)?;
    // No span table here on purpose: plain `Target::Wasm`, so the emitted component embeds a lean core
    // module with none of the DWARF custom sections `compile()` adds for the debugger.
    let (ast_bytes, _spans) = parse_spanned(text, from).map_err(|m| JsError::new(&m))?;
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[rcdzc::Target::Wasm],
    );
    let Some(component) = out.artifact(rcdzc::Target::Wasm.artifact_kind()) else {
        return Ok(None); // the program declined — nothing to unwrap
    };
    Ok(program_core_module(component))
}

/// Unwrap a WebAssembly component to the PROGRAM's embedded core module bytes. Walks the component's
/// top-level sections and returns the LAST core-module section's (`COMP_SEC_CORE_MODULE`) payload — the
/// last, because the resource-escape shape emits the standalone `t-dtor` module FIRST (it must
/// instantiate before the resource type) and the program's own module last; a scalar/plain component
/// embeds exactly one, so "last" is right for both. The nested re-export component (a distinct section
/// id, `COMP_SEC_COMPONENT`) is skipped by id, so its inner core modules never leak out. Returns `None`
/// if the bytes are too short or carry no core module.
///
/// A component section is `<id:u8> <size:uleb128> <payload:size>`, after the 8-byte component preamble
/// (magic + layer version) — the same framing rcdzc's `envelope` emits. Section ids are read from the
/// GENERATED `wasm_abi` table, not hand-typed.
fn program_core_module(component: &[u8]) -> Option<Vec<u8>> {
    use rcdzc::backend::wasm::wasm_abi;
    let mut p = 8usize; // skip the component preamble (magic + layer version)
    let mut last_core: Option<Vec<u8>> = None;
    while p < component.len() {
        let id = component[p];
        p += 1;
        // Read the uleb128 section length.
        let mut len: usize = 0;
        let mut shift: u32 = 0;
        loop {
            let byte = *component.get(p)?;
            p += 1;
            len |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let end = p.checked_add(len)?;
        if end > component.len() {
            return None; // truncated section — bail rather than read past the end
        }
        if id == wasm_abi::COMP_SEC_CORE_MODULE {
            last_core = Some(component[p..end].to_vec());
        }
        p = end;
    }
    last_core
}

/// The content-address (BLAKE3, lowercase hex) of the value-heap runtime this compiler emits imports
/// against — the tree-unified content address (operator 2026-08-08: BLAKE3, same digest as `Hash::of`).
///
/// A compound-returning program imports `cadenza:runtime/heap@0.0.0+<hash>`; the guide must compose
/// the runtime whose hash equals this. Exposing it lets the JS side assert it bundled the right
/// runtime `.wasm` rather than hard-coding the hex in two places.
#[wasm_bindgen]
pub fn required_runtime_hash() -> String {
    rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH.to_string()
}

/// One SEMANTIC token — a source byte range plus the ROLE the compiler classified it as (`type`,
/// `constructor`, `function`, `param`, `variable`, `effect`, `label`, `keyword`, `number`, `string`,
/// `char`, `bytes`, `symbol`, `literal`, `unbound`). The editor maps `kind` to a colour. Byte offsets
/// (UTF-8), resolved through the span table in Rust — the JS only converts byte↔UTF-16.
#[wasm_bindgen(getter_with_clone)]
pub struct SemanticToken {
    pub from: u32,
    pub to: u32,
    pub kind: String,
}

/// SEMANTIC SYNTAX HIGHLIGHTING for `text` — every token CLASSIFIED by the role it plays, so the editor
/// can colour a name by what it MEANS (a type vs a constructor vs a local vs a call vs an unbound typo)
/// rather than by its spelling. Rides the `Highlight` sidecar query (the same one `cdz highlight` runs):
/// the compiler classifies each user leaf off the resolved column + the meta channels a value carries,
/// so a token's colour equals what a compile determines — never a second lexical guess. Each classified
/// node id is mapped to its `[from, to)` byte range here through the span table (canonicalized for both
/// surfaces by `parse_spanned`, so the ranges are correct in ML as well as s-expr).
///
/// Total: a buffer that doesn't parse (or a surface with no span table) yields the empty list, so an
/// editor simply keeps its cheap lexical colours until the next well-parsed edit — the compiler overlay
/// REFINES the lexical pass, it does not replace it.
#[wasm_bindgen]
pub fn semantic_tokens(text: &str, from: &str) -> Result<Vec<SemanticToken>, JsError> {
    let from = parse_format(from)?;
    let Ok((ast_bytes, Some(spans))) = parse_spanned(text, from) else {
        return Ok(Vec::new()); // unparseable / span-less — leave it to the lexical fallback
    };
    // Ride the `Highlight` query — a `(node-id, kind)` pair per classified leaf, ascending id order,
    // on the canonical binary-AST wire (ZERO string parsing).
    let hl_bytes = run_query_bytes(&ast_bytes, &rcdzc::Query::Highlight)?;
    // Single-file: a node id keys directly into the user span table (a span-less node is dropped — it
    // should not happen for a user leaf).
    let resolve_span = |n: u32| {
        spans
            .get(cadenza_syntax::ast::StructId(n))
            .map(|s| (s.start as u32, s.end as u32))
    };
    Ok(highlight_tokens_to_semantic(
        &rcdzc::sidecar::decode_highlight(&hl_bytes),
        resolve_span,
    ))
}

/// Turn the `(node-id, kind)` pairs the `Highlight` sidecar query emits into JS [`SemanticToken`]s.
/// `resolve_span` maps a real node id to its `[from, to)` byte range in the source UNDER EDIT — the ONE
/// thing the single-file and preloaded-package paths differ on (the package path demuxes a GLOBAL id
/// through the link-map to the user file first). A node `resolve_span` places OUTSIDE the source under
/// edit (a token in a preloaded library) returns `None` and is DROPPED — the editor only colours its own
/// buffer.
fn highlight_tokens_to_semantic(
    tokens: &[(u32, String)],
    resolve_span: impl Fn(u32) -> Option<(u32, u32)>,
) -> Vec<SemanticToken> {
    let mut out = Vec::new();
    for (node, kind) in tokens {
        if let Some((from, to)) = resolve_span(*node) {
            out.push(SemanticToken {
                from,
                to,
                kind: kind.clone(),
            });
        }
    }
    out
}

/// SEMANTIC SYNTAX HIGHLIGHTING for `text` LINKED against preloaded library modules — the preload-aware
/// sibling of [`semantic_tokens`], mirroring [`diagnostics_with_preloaded`]'s linking so the `/cad` editor
/// can CLASSIFY (and thus colour) a model's names that resolve to an ambiently-supplied library
/// (`Solid`/`v3r`/`lower` from the preloaded CAD lib) instead of leaving them uncoloured/`unbound`.
/// `preloaded_names[i]`/`preloaded_sources[i]`/`preloaded_formats[i]` are the module name / source /
/// surface, equal length.
///
/// Tokens are over the USER model only: the whole package is classified (so a name resolving into a
/// preloaded library is classified as the `function`/`type`/`constructor` it truly is, not `unbound`),
/// but each token's GLOBAL node id is demuxed through the package link-map back to the `main` file and a
/// token landing in a preloaded library is dropped (the editor colours only its own buffer). With no
/// preloaded modules this is byte-identical to [`semantic_tokens`] (flat namespace). Total: an unparseable
/// user buffer or a broken preloaded library yields the empty list (the editor keeps its lexical
/// fallback), matching `semantic_tokens`'s "refine, don't replace" contract — a set-up failure is not
/// surfaced as a token (highlighting degrades silently; the linter is where a fault shows).
#[wasm_bindgen]
pub fn semantic_tokens_with_preloaded(
    text: &str,
    from: &str,
    preloaded_names: Vec<String>,
    preloaded_sources: Vec<String>,
    preloaded_formats: Vec<String>,
) -> Result<Vec<SemanticToken>, JsError> {
    let from = parse_format(from)?;

    // No preloaded modules → the flat single-file highlight path, byte-identical.
    if preloaded_names.is_empty() {
        return semantic_tokens(text, from.name());
    }

    let setup = link_preloaded_query(
        text,
        from,
        &preloaded_names,
        &preloaded_sources,
        &preloaded_formats,
        rcdzc::Query::Highlight,
    )?;
    // A set-up failure (unparseable user buffer / broken library) yields NO tokens — highlighting is an
    // overlay that REFINES the editor's lexical colours, so it degrades to the fallback rather than
    // surfacing an error here (the linter — `diagnostics_with_preloaded` — is where a fault shows).
    let PreloadedQuery { inputs, spans } = match setup {
        Ok(pq) => pq,
        Err(_) => return Ok(Vec::new()),
    };

    let out = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let Some(bytes) = out.artifact(rcdzc::sidecar::KIND_HIGHLIGHT) else {
        return Ok(Vec::new()); // no highlight artifact — fall back to lexical colours
    };
    // Decode the `(node-id, kind)` pairs from the canonical binary-AST wire (ZERO string parsing).
    let tokens = rcdzc::sidecar::decode_highlight(bytes);
    let resolve_span = user_span_resolver(&out, spans);
    Ok(highlight_tokens_to_semantic(&tokens, resolve_span))
}

/// The compilation DISPOSITION of the definition under the cursor — what the compiler DID with it — for
/// a hover tooltip. `disposition` is one of `inlined` / `specialized` / `emitted` / `transformed→COPY` /
/// `unreferenced` (a `+`-joined set when more than one applies); `name` is the definition's name;
/// `instances` lists each concrete monomorphization (only for `specialized`), each an `arg, arg, …`
/// string (a runtime param `name: TYPE`, an erased compile-time param `const name = VALUE`). `from`/`to`
/// are the def-name's byte range, so the caller can anchor the tooltip to the name. The reverse of "one
/// source def, one function": the browser shows how each definition is actually compiled.
#[wasm_bindgen(getter_with_clone)]
pub struct Disposition {
    pub name: String,
    pub disposition: String,
    pub instances: Vec<String>,
    pub from: u32,
    pub to: u32,
}

/// The compilation disposition of the definition whose NAME is at a source byte offset — the "how was
/// this compiled?" hover companion of `type_at`. Resolves the offset to the innermost user node, reads
/// that node's source text as a definition NAME, and rides the `Instantiations` sidecar query (the same
/// one `cdz instantiations` runs): it forces whole-program monomorphization, then reports the def's
/// disposition plus, if it is specialized, every concrete instantiation. `None` when the offset is not on
/// a name that denotes a definition (an operator, a literal, whitespace) — the tooltip simply doesn't
/// show. Total on a well-parsed buffer.
#[wasm_bindgen]
pub fn disposition(
    text: &str,
    from: &str,
    byte_offset: u32,
) -> Result<Option<Disposition>, JsError> {
    let from = parse_format(from)?;
    let (ast_bytes, spans) = match parse_spanned(text, from) {
        Ok(pair) => pair,
        Err(_) => return Ok(None), // a buffer that won't parse has no disposition
    };
    let Some(spans) = spans else { return Ok(None) };
    // The innermost user node at the cursor, and its source text — the candidate definition NAME. The
    // query is BY NAME (unlike `type_at`, which is by node id), so we read the hovered name off the source
    // through its span rather than passing the node id.
    let off = byte_offset as usize;
    let Some(node) = spans.node_at_offset(off) else {
        return Ok(None);
    };
    let span = spans
        .get(node)
        .expect("node_at_offset returned a spanned node");
    let name = &text[span.start..span.end];
    // Only a bare identifier can name a definition — skip a token that is obviously not a name (a paren,
    // an operator, a literal), so a hover in dead space yields no tooltip rather than an empty query.
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        return Ok(None);
    }
    // Ride the `Instantiations` query — a BINARY-AST report (`instantiations_wire`), NOT text. DECODE it
    // (the old `run_query_text` + TAB-split was the same from_utf8-on-binary bug class as type_at /
    // export_types #6324). `known == false` = the hovered token names no definition → no tooltip.
    let bytes = run_query_bytes(
        &ast_bytes,
        &rcdzc::Query::Instantiations {
            name: name.to_string(),
        },
    )?;
    let Some(report) = rcdzc::sidecar::decode_instantiations(&bytes) else {
        return Ok(None); // artifact absent / malformed → no tooltip
    };
    if !report.known {
        return Ok(None); // an unknown name has no disposition
    }
    // Present the disposition set readably (joined by `+`, as `cdz instantiations` renders it — a def may
    // carry a combination like `transformed→copy`). A known def always carries at least one.
    let disposition = report.dispositions.join("+");
    if disposition.is_empty() {
        return Ok(None);
    }
    // Anchor the tooltip to the DEFINITION's name occurrence (which may differ from the hovered use), so
    // the range is stable whether the reader hovers the def or a call site; fall back to the hovered span.
    let def_span = report
        .name_node
        .and_then(|n| spans.get(cadenza_syntax::ast::StructId(n)))
        .unwrap_or(span);
    // Each instance's per-argument descriptors rendered `a, b, c` (dropping the unstable synthesized spec
    // name + node id the report also carries) — the same readable form the old `;`-joined row produced.
    let instances: Vec<String> = report.instances.iter().map(|i| i.args.join(", ")).collect();
    Ok(Some(Disposition {
        name: name.to_string(),
        disposition,
        instances,
        from: def_span.start as u32,
        to: def_span.end as u32,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MULTI-form s-expr guide snippet (several top-level forms) must PARSE and its span table must be
    /// keyed by the CANONICAL node ids the compiler reports. `read_spanned` errors on trailing input, so
    /// before the fix a multi-form program failed to parse at all; and even parsed, the synthetic `(do …)`
    /// wrap reorders ids under canonicalization, so an un-remapped table maps a diagnostic to a
    /// NEIGHBOUR's span (the `cdz` M24 corruption). This pins both: it parses, and the `helper` def
    /// name's node maps back to the `helper` bytes — not a neighbour.
    #[test]
    fn multi_form_sexpr_parses_and_spans_are_canonical() {
        let src = "(def (helper x) (+ x 1)) (def (main) (helper 5)) (export main)";
        let (ast_bytes, spans) = parse_spanned(src, Format::Sexpr).expect("multi-form parses");
        assert!(!ast_bytes.is_empty(), "produced AST bytes");
        let spans = spans.expect("s-expr carries spans");
        // Every recorded span must slice its own text without panicking, and at least one span must be
        // exactly the `helper` def name — proving the table is keyed by ids that still line up with the
        // source after canonicalization (a shifted table would map some id onto `(f x y)`-style bytes).
        let mut saw_helper_name = false;
        for i in 0..(ast_bytes.len() as u32) {
            if let Some(span) = spans.get(cadenza_syntax::ast::StructId(i))
                && span.end <= src.len()
                && &src[span.start..span.end] == "helper"
            {
                saw_helper_name = true;
                break;
            }
        }
        assert!(
            saw_helper_name,
            "a node's span must cover exactly `helper` — the canonical span mapping holds"
        );
    }

    #[test]
    fn export_types_renders_display_name_text_not_raw_binary() {
        // Regression guard (guide-examples render red 410->399/10 + a live-app bug): `Query::Exports` now
        // emits the KIND_EXPORTS BINARY payload (exports_wire), so `export_types` must DECODE it and render
        // each type via the shared type-name renderer to restore the documented `name<TAB>type` TEXT
        // contract. The bug was calling `run_query_text` (String::from_utf8 of the binary) -> Class B: a
        // whole-Float export's row vanished (silent), Class A: a complex-type module threw "not valid UTF-8".
        // Class B: a whole Float64 export must yield a `main<TAB>Float…` row (the scalar .0-recovery input).
        let out = export_types("(do (def (main) 3000.0) (export main))", "sexpr")
            .expect("export_types returns text, not a UTF-8 error");
        let main_row = out
            .lines()
            .find(|l| l.starts_with("main\t"))
            .expect("a `main<TAB>type` row");
        assert!(
            main_row.contains("Float"),
            "main's whole-float type renders as a Float name (Class B .0-recovery input): {main_row:?}"
        );
        // Class A: a complex recursive/compound-returning module must NOT throw invalid-UTF-8, and main's
        // Int result type must render — the exact shape (recursive Iter sum over a Tuple payload) that threw.
        let iter = "(do \
            (type Iter (Nil unit) (Cons (Tuple Int64 Iter))) \
            (def (from-list xs) (match xs (#list() (Nil unit)) (#list(h .. t) (Cons #tuple(h (from-list t)))))) \
            (def (ifold it acc f) (match it ((Nil _) acc) ((Cons c) (ifold (. c 1) (f acc (. c 0)) f)))) \
            (def (main) (ifold (from-list #list(1 2 3)) 0 (fn (a x) (+ a x)))) \
            (export main))";
        let out2 = export_types(iter, "sexpr")
            .expect("a complex-type module does NOT throw invalid-UTF-8 (Class A regression)");
        let main_row2 = out2
            .lines()
            .find(|l| l.starts_with("main\t"))
            .expect("a `main<TAB>type` row for the iterator module");
        assert!(
            main_row2.contains("Int"),
            "main's Int result type renders: {main_row2:?}"
        );
        // Every non-empty row is the documented tab-delimited `name<TAB>type` the JS consumers parse.
        assert!(
            out2.lines().all(|l| l.is_empty() || l.contains('\t')),
            "each row is `name<TAB>type` text: {out2:?}"
        );
    }

    #[test]
    fn type_at_renders_the_hover_type_not_raw_binary() {
        // Regression guard (same bug class as export_types #6324): `Query::TypeAt` emits the KIND_TYPE_AT
        // BINARY hover verdict (`type_at_wire`), so `type_at` must DECODE it + render (mirroring `cdz`'s
        // `render_type_at`), NOT `String::from_utf8` the binary. The bug: Class A threw "not valid UTF-8"
        // on the browser editor hover; Class B rendered garbage. `type_at` is LIVE-wired in the guide
        // editor (client.ts -> worker.ts), so this was a real user-facing hover bug, not just a gate.
        let src = "(do (def (main) 3000.0) (export main))";
        // A bare typed node (the whole-`Float64` literal) → the `Ty` verdict → rendered type name.
        let lit_off = src.find("3000.0").expect("float literal present") as u32;
        let hit = type_at(src, "sexpr", lit_off)
            .expect("type_at returns a verdict, not a UTF-8 error (Class A regression)")
            .expect("a user node at the float literal offset");
        assert!(
            hit.type_name.contains("Float"),
            "the whole-float literal's hover type renders as a Float name (not raw binary / not empty): {:?}",
            hit.type_name
        );
        // A definition name → the `Def` verdict → `name : <scheme>` (exercises the Def arm's render).
        let name_off = (src.find("(main)").expect("main def present") + 1) as u32;
        let def_hit = type_at(src, "sexpr", name_off)
            .expect("type_at on a def name returns a verdict, not a UTF-8 error")
            .expect("a user node at the def name");
        assert!(
            def_hit.type_name.contains("main"),
            "a def hover renders `name : type`: {:?}",
            def_hit.type_name
        );
    }

    #[test]
    fn define_at_resolves_a_reference_not_raw_binary() {
        // Regression guard (same bug class as type_at / export_types #6324): `Query::ResolveOf` emits the
        // KIND_RESOLVE target node id as BINARY AST (`resolve_wire`, #6152), so `define_at` must
        // `decode_resolve` it, NOT `String::from_utf8` + `parse::<u32>()` (which threw / mis-parsed on the
        // binary). `define_at` backs the guide editor's go-to-definition.
        let src = "(do (def (foo) 1) (def (main) foo) (export main))";
        // The `foo` REFERENCE (in `main`'s body) resolves to the `foo` DEFINITION (a distinct earlier span).
        let ref_off = src.rfind("foo").expect("the `foo` reference") as u32;
        let hit = define_at(src, "sexpr", ref_off)
            .expect("define_at returns a resolution, not a UTF-8 error (Class A regression)")
            .expect("the `foo` reference resolves to its definition");
        assert!(
            (hit.from, hit.to) != (hit.ref_from, hit.ref_to),
            "the resolved def span is distinct from the reference span (decode_resolve gave a real target): \
             def [{},{}) ref [{},{})",
            hit.from,
            hit.to,
            hit.ref_from,
            hit.ref_to
        );
        // The reference span covers the cursor offset (sanity: we hovered the ref token).
        assert!(
            hit.ref_from <= ref_off && ref_off < hit.ref_to,
            "the reference span [{},{}) covers the hovered offset {ref_off}",
            hit.ref_from,
            hit.ref_to
        );
    }

    #[test]
    fn ml_nested_ctor_in_type_def_block_has_no_syntax_error_through_the_browser_entrypoints() {
        // Regression for a reported native-vs-browser divergence (v-guide-infra): a multi-line ML sum-
        // type-def block with a NESTED multi-arg constructor application —
        // `Solidr.Differencer(Solidr.Cuber(V3r(r(4), r(4), r(4))), Solidr.Spherer(Rational.of(5, 2)))` —
        // was reported to fail in the browser with "unexpected ')' at byte N" (which is the S-EXPR
        // reader's error, sexpr.rs:484). This pins that the WASM browser entrypoints handle it via the ML
        // reader with NO syntax error: `parse_spanned`/`diagnostics`/`compile` at `from="ml"` never route
        // ML source through the s-expr reader. (cadenza-syntax's `parser::read_ml` correctness is pinned
        // separately in parser.rs; THIS pins the cdz-wasm compile layer the guide actually calls.) A
        // future change that made an ML path re-read through the s-expr surface would fail here.
        let src = "type Vec3r = | V3r(Rational, Rational, Rational)\n\
                   type Solidr =\n\
                   \x20\x20| Cuber(Vec3r)\n\
                   \x20\x20| Spherer(Rational)\n\
                   \x20\x20| Differencer(Solidr, Solidr)\n\
                   def r(n: Int64) = Rational.of(n, 1)\n\
                   def main() =\n\
                   \x20\x20Solidr.Differencer(\n\
                   \x20\x20\x20\x20Solidr.Cuber(V3r(r(4), r(4), r(4))),\n\
                   \x20\x20\x20\x20Solidr.Spherer(Rational.of(5, 2)))\n";
        // parse_spanned (the shared front-end leg of every compile/diagnostics call) succeeds.
        let (bytes, _spans) =
            parse_spanned(src, Format::Ml).expect("ML parses through parse_spanned");
        assert!(!bytes.is_empty(), "produced a non-empty AST");
        // diagnostics: NO syntax/parse error — only the expected unused-`main` warning (the snippet
        // exports nothing). Crucially, NONE mentions the s-expr reader's "unexpected ')'".
        let ds = diagnostics(src, "ml").expect("diagnostics runs");
        for d in &ds {
            assert!(
                !d.message.contains("unexpected ')'") && !d.message.contains("trailing input"),
                "no s-expr-reader syntax error should appear on the ML path, got: {}",
                d.message
            );
            // The only diagnostic on this well-formed (if unexported) program is the unused-def warning.
            assert!(
                !d.error || d.code == "CDZ0306",
                "unexpected hard error on a syntactically-valid ML program: [{}] {}",
                d.code,
                d.message
            );
        }
    }

    /// The byte offset of the FIRST occurrence of `needle` in `src` — a hover target for `disposition`.
    fn offset_of(src: &str, needle: &str) -> u32 {
        src.find(needle).expect("needle present") as u32
    }

    #[test]
    fn disposition_reports_how_a_definition_was_compiled() {
        // A non-recursive fn is INLINED; a recursive generic is SPECIALIZED (its instances listed); an
        // exported entry is EMITTED. Hovering each def name reports its disposition — the browser IDE
        // answering the same "what did the compiler do" question `cdz instantiations` does on the CLI.
        let src = "(def (ident v) v) \
                   (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
                   (def (main (: a Int64)) (+ (ident a) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\"))))) \
                   (export main)";
        // `ident` — inlined, no instances.
        let d = disposition(src, "sexpr", offset_of(src, "ident"))
            .expect("query ok")
            .expect("ident is a definition");
        assert_eq!(d.name, "ident");
        assert_eq!(d.disposition, "inlined");
        assert!(d.instances.is_empty(), "an inlined def has no instances");
        // `loopn` — specialized at Int64 and String; two instances.
        let d = disposition(src, "sexpr", offset_of(src, "loopn"))
            .expect("query ok")
            .expect("loopn is a definition");
        assert_eq!(d.disposition, "specialized");
        let mut inst = d.instances.clone();
        inst.sort();
        assert_eq!(inst, vec!["n: Int64, x: Int64", "n: Int64, x: String"]);
        // `main` — emitted (an export).
        let d = disposition(src, "sexpr", offset_of(src, "main"))
            .expect("query ok")
            .expect("main is a definition");
        assert_eq!(d.disposition, "emitted");
        // A hover that is NOT on a definition name (a literal) yields no tooltip.
        assert!(
            disposition(src, "sexpr", offset_of(src, "3"))
                .expect("query ok")
                .is_none(),
            "a literal is not a definition"
        );
    }

    #[test]
    fn compile_with_preloaded_links_a_user_import_against_a_supplied_module() {
        // The `/cad` seam: the buffer holds ONLY the user's model, which `import`s a name from a PRELOADED
        // library module it never had to paste in. Here `lib` exports `answer`; the model imports and uses
        // it. With the module preloaded, the program links + compiles to a component (no "unbound name").
        let lib = "def answer() = 42\nexport { answer }";
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let out = compile_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec![lib.to_string()],
            vec!["ml".to_string()],
        )
        .expect("compile_with_preloaded runs");
        let errors: Vec<_> = out.diagnostics.iter().filter(|d| d.error).collect();
        assert!(
            errors.is_empty(),
            "a model importing a preloaded module compiles cleanly, got errors: {:?}",
            errors.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        assert!(
            out.component.is_some(),
            "linking the preloaded module produced a component"
        );
    }

    #[test]
    fn compile_with_preloaded_without_the_module_leaves_the_import_unbound() {
        // Control: the SAME model with NO preloaded module fails to resolve the import — proving the pass
        // above succeeds BECAUSE the module was linked in, not because the import was ignored.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let out = compile_with_preloaded(model, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("compile_with_preloaded runs with no preloads");
        assert!(
            out.component.is_none() || out.diagnostics.iter().any(|d| d.error),
            "an unsatisfied import without its module does not silently produce a good component"
        );
    }

    #[test]
    fn compile_with_preloaded_empty_matches_plain_compile() {
        // No preloaded modules → byte-identical to `compile` (the flat single-file path, no linkage).
        let src = "def main() = 1\nexport { main }";
        let a = compile(src, "ml").expect("compile");
        let b = compile_with_preloaded(src, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("compile_with_preloaded");
        assert_eq!(
            a.component, b.component,
            "empty-preload compile equals plain compile"
        );
    }

    // NOTE: the mismatched-array-length guard (returns a `JsError`) is not unit-tested here — a `JsError`
    // cannot be constructed on a non-wasm host target (wasm-bindgen panics), so the Err path is only
    // exercisable in a wasm environment. The length check itself is a plain `if` above the JS boundary.

    #[test]
    fn compile_with_preloaded_names_a_broken_library_module() {
        // A preloaded module that fails to parse → a codeless error diagnostic NAMING the module, so the
        // user sees which library broke rather than a mysterious unbound-name cascade in their own model.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let out = compile_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec!["def answer( = ".to_string()], // malformed
            vec!["ml".to_string()],
        )
        .expect("compile_with_preloaded runs");
        assert!(
            out.component.is_none()
                && out
                    .diagnostics
                    .iter()
                    .any(|d| d.error && d.message.contains("preloaded module `lib`")),
            "a broken preloaded module is reported by name, got: {:?}",
            out.diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn diagnostics_with_preloaded_resolves_a_preloaded_import_clean() {
        // The `/cad` IDE-linter seam: the editor buffer holds ONLY the user's model, which imports names
        // from a PRELOADED library. The preload-aware linter links the library, so the imported names
        // resolve and the buffer shows NO error squiggles (the plain, non-preload `diagnostics` would
        // false-flag the import "not modeled" + the names unbound — see the control below).
        let lib = "def answer() = 42\nexport { answer }";
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let diags = diagnostics_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec![lib.to_string()],
            vec!["ml".to_string()],
        )
        .expect("diagnostics_with_preloaded runs");
        let errors: Vec<_> = diags.iter().filter(|d| d.error).collect();
        assert!(
            errors.is_empty(),
            "a model importing a preloaded module lints clean, got errors: {:?}",
            errors.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn diagnostics_without_the_preloaded_module_flags_the_import() {
        // Control: the SAME model with NO preloaded module DOES flag the unresolved import — proving the
        // clean lint above is BECAUSE the module was linked in, not because the import was ignored.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let diags = diagnostics_with_preloaded(model, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("diagnostics_with_preloaded runs with no preloads");
        assert!(
            diags.iter().any(|d| d.error),
            "an unsatisfied import without its module produces an error diagnostic"
        );
    }

    #[test]
    fn diagnostics_with_preloaded_empty_matches_plain_diagnostics() {
        // No preloaded modules → byte-identical to plain `diagnostics` (the flat single-file path). A
        // buffer with a real fault (unbound `nope`) yields the SAME diagnostic set either way.
        let src = "def main() = nope\nexport { main }";
        let a = diagnostics(src, "ml").expect("diagnostics");
        let b = diagnostics_with_preloaded(src, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("diagnostics_with_preloaded");
        assert_eq!(a.len(), b.len(), "same fault count");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (x.code.as_str(), x.from, x.to),
                (y.code.as_str(), y.from, y.to)
            );
            assert_eq!(x.message, y.message);
        }
    }

    #[test]
    fn diagnostics_with_preloaded_maps_a_user_fault_to_the_user_span() {
        // A REAL fault in the user model (an unbound `bogus`, alongside a correctly-imported preloaded
        // name) must still be reported — mapped to the user buffer's OWN span, not dropped as a library
        // fault and not offset by the linked library's node ids.
        let lib = "def answer() = 42\nexport { answer }";
        let model =
            "import { answer } from \"lib\"\ndef main() = answer() + bogus\nexport { main }";
        let diags = diagnostics_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec![lib.to_string()],
            vec!["ml".to_string()],
        )
        .expect("diagnostics_with_preloaded runs");
        let unbound: Vec<_> = diags
            .iter()
            .filter(|d| d.error && d.message.contains("bogus"))
            .collect();
        assert_eq!(
            unbound.len(),
            1,
            "the user's own unbound name is reported exactly once, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
        // Its span points INTO the user model at the `bogus` occurrence (a non-zero range within the
        // model text) — not at (0,0) and not somewhere in the linked library's id space.
        let d = unbound[0];
        let at = model.find("bogus").expect("bogus in model") as u32;
        assert_eq!(
            (d.from, d.to),
            (at, at + "bogus".len() as u32),
            "the fault maps to the `bogus` occurrence in the USER model"
        );
    }

    #[test]
    fn diagnostics_with_preloaded_names_a_broken_library_module() {
        // A preloaded module that fails to parse → a codeless error diagnostic NAMING the module (mirrors
        // `compile_with_preloaded`), so the reader sees which library broke rather than an unbound cascade.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let diags = diagnostics_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec!["def answer( = ".to_string()], // malformed
            vec!["ml".to_string()],
        )
        .expect("diagnostics_with_preloaded runs");
        assert!(
            diags
                .iter()
                .any(|d| d.error && d.message.contains("preloaded module `lib`")),
            "a broken preloaded module is reported by name, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_tokens_with_preloaded_classifies_a_preloaded_call() {
        // A name resolving into a PRELOADED library is classified as the `function` it truly is — so the
        // /cad editor colours `answer()` as a call, not leaving it uncoloured/unbound. Without the linked
        // library it would classify as `unbound` (the control below).
        let lib = "def answer() = 42\nexport { answer }";
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let toks = semantic_tokens_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec![lib.to_string()],
            vec!["ml".to_string()],
        )
        .expect("semantic_tokens_with_preloaded runs");
        // Every token maps into the USER model (no token past the model's length, none in the library's
        // id space) — the demux keeps highlighting on the reader's own buffer.
        assert!(
            toks.iter().all(|t| (t.to as usize) <= model.len()),
            "every token maps into the user model, got: {:?}",
            toks.iter()
                .map(|t| (t.from, t.to, &t.kind))
                .collect::<Vec<_>>()
        );
        // The `answer` call occurrence in the model is classified (a non-`unbound` kind) — proving the
        // preloaded library resolved the name.
        let call_at = model.rfind("answer").expect("answer call in model") as u32;
        let tok = toks.iter().find(|t| t.from == call_at);
        assert!(
            tok.is_some_and(|t| t.kind != "unbound"),
            "the preloaded call `answer` is classified (not unbound), got: {:?}",
            toks.iter()
                .map(|t| (t.from, t.to, &t.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn semantic_tokens_without_the_module_leaves_the_call_unbound() {
        // Control: the SAME model with NO preloaded module classifies the call as `unbound` — proving the
        // classification above is BECAUSE the library was linked in, not incidental.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let toks = semantic_tokens_with_preloaded(model, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("semantic_tokens_with_preloaded runs with no preloads");
        // With no preload this is the plain single-file path; whatever it classifies `answer` as, it is
        // NOT the resolved `function` the linked version yields — assert it is unbound or absent.
        let call_at = model.rfind("answer").expect("answer call in model") as u32;
        let tok = toks.iter().find(|t| t.from == call_at);
        assert!(
            tok.is_none_or(|t| t.kind == "unbound"),
            "without the module the call is unbound (or unclassified), got: {:?}",
            tok.map(|t| &t.kind)
        );
    }

    #[test]
    fn semantic_tokens_with_preloaded_empty_matches_plain() {
        // No preloaded modules → byte-identical to plain `semantic_tokens` (the flat single-file path).
        let src = "def main() = 1\nexport { main }";
        let a = semantic_tokens(src, "ml").expect("semantic_tokens");
        let b = semantic_tokens_with_preloaded(src, "ml", Vec::new(), Vec::new(), Vec::new())
            .expect("semantic_tokens_with_preloaded");
        assert_eq!(a.len(), b.len(), "same token count");
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(
                (x.from, x.to, x.kind.as_str()),
                (y.from, y.to, y.kind.as_str())
            );
        }
    }

    #[test]
    fn semantic_tokens_with_preloaded_broken_library_degrades_to_empty() {
        // A broken preloaded library → NO tokens (highlighting is a refine-don't-replace overlay, so it
        // falls back to the editor's lexical colours rather than surfacing an error as a token). The
        // LINTER (`diagnostics_with_preloaded`) is where the broken library shows up.
        let model = "import { answer } from \"lib\"\ndef main() = answer()\nexport { main }";
        let toks = semantic_tokens_with_preloaded(
            model,
            "ml",
            vec!["lib".to_string()],
            vec!["def answer( = ".to_string()], // malformed
            vec!["ml".to_string()],
        )
        .expect("semantic_tokens_with_preloaded runs");
        assert!(
            toks.is_empty(),
            "a broken preloaded library degrades highlighting to the lexical fallback (no tokens)"
        );
    }

    #[test]
    fn param_manifest_reports_each_at_param_sites_metadata() {
        // The `/cad` single-mode enabler: `param_manifest` reads every `@param` site from a compiled model
        // into JS-readable {name, type_name, widget, range, default} so the browser renders a control per
        // param. A scalar param reports its declared type + widget; a Rational (heap-typed) param reports
        // `Rational` (the num/den desugar is a compile detail the manifest abstracts over). A model with no
        // @param yields an empty list.
        // NOTE: `range` is spelled `(list 0 100)` — the `[0 100]` bracket sugar is ML-surface-only; the
        // s-expr reader reads `[0` / `100]` as two atoms, so a `.sexp`/s-expr fixture must use `(list …)`
        // (the canonical arena node `[lo hi]` desugars to on the ML side).
        let src = "(do \
                     (pragma param (param (: widget slider) (: range (list 0 100)) (: default 5)) (: width Int64)) \
                     (pragma param (param (: widget slider)) (: rate Rational)) \
                     (def (main) (host (Param) (+ (Param.width) (Rational.value (Param.rate))))) \
                     (export main))";
        let entries = param_manifest(src, "sexpr").expect("param_manifest runs");
        assert_eq!(entries.len(), 2, "two @param sites → two manifest entries");

        let width = entries
            .iter()
            .find(|e| e.name == "width")
            .expect("the `width` @param is in the manifest");
        assert_eq!(width.type_name, "Int64", "declared type is reported");
        assert_eq!(
            width.widget.as_deref(),
            Some("slider"),
            "widget is reported"
        );
        assert_eq!(
            width.range_lo.as_deref(),
            Some("0"),
            "range low bound is rendered to its literal text"
        );
        assert_eq!(width.range_hi.as_deref(), Some("100"), "range high bound");
        assert_eq!(
            width.default.as_deref(),
            Some("5"),
            "default is rendered to its literal text"
        );
        // An Int64 bound is NOT a Rational → the exact num/den companions stay None (the caller reads the
        // integer-string fields for an Int64 @param).
        assert!(
            width.range_lo_num.is_none()
                && width.range_lo_den.is_none()
                && width.default_num.is_none()
                && width.default_den.is_none(),
            "an Int64 @param's num/den companions are None — the caller reads the integer strings"
        );

        let rate = entries
            .iter()
            .find(|e| e.name == "rate")
            .expect("the `rate` @param is in the manifest");
        assert_eq!(
            rate.type_name, "Rational",
            "a heap-typed Rational @param reports its declared type (num/den desugar is abstracted)"
        );
        // No range/default config on `rate` → those fields are None (JS undefined).
        assert!(
            rate.range_lo.is_none() && rate.range_hi.is_none() && rate.default.is_none(),
            "a @param with no range/default config reports None for them"
        );

        // A model with NO @param yields an empty manifest (no controls to render).
        assert!(
            param_manifest("(do (def (main) 42) (export main))", "sexpr")
                .expect("runs")
                .is_empty(),
            "no @param sites → empty manifest"
        );
    }

    #[test]
    fn param_manifest_reports_exact_num_den_for_a_rational_default() {
        // The num/den fast-follow (v-guide-infra's fraction-native `/cad` slider): a `Rational` @param whose
        // `default` (or range bound) is a constant fraction crosses as EXACT num/den — the `{num, den}` the
        // slider drives over `Param.<name>-num`/`-den` — NOT a lossy number or a source-text string the host
        // would have to parse. `(Rational.of 1 4)` folds to `Core::ConstRational(1, 4)`, so `default_num`/
        // `default_den` are `"1"`/`"4"`; the literal-text `default` is kept for a tooltip.
        let src = "(do \
                     (pragma param (param (: widget slider) (: default (Rational.of 1 4))) (: frac Rational)) \
                     (def (main) (host (Param) (Param.frac))) \
                     (export main))";
        let entries = param_manifest(src, "sexpr").expect("param_manifest runs");
        let frac = entries
            .iter()
            .find(|e| e.name == "frac")
            .expect("the `frac` @param is in the manifest");
        assert_eq!(frac.type_name, "Rational", "declared type is Rational");
        assert_eq!(
            frac.default_num.as_deref(),
            Some("1"),
            "the exact numerator of the 1/4 default"
        );
        assert_eq!(
            frac.default_den.as_deref(),
            Some("4"),
            "the exact denominator of the 1/4 default"
        );
        // The literal-text field is still present (a tooltip/label) — kept alongside the exact pair.
        assert!(
            frac.default.is_some(),
            "the literal-text default is kept alongside the exact num/den"
        );
        // A reducible fraction is normalized by the compiler's fold (gcd-reduce): `(Rational.of 2 8)` = 1/4.
        let src2 = "(do \
                      (pragma param (param (: widget slider) (: default (Rational.of 2 8))) (: frac Rational)) \
                      (def (main) (host (Param) (Param.frac))) \
                      (export main))";
        let e2 = param_manifest(src2, "sexpr").expect("runs");
        let frac2 = e2.iter().find(|e| e.name == "frac").expect("frac present");
        assert_eq!(
            (frac2.default_num.as_deref(), frac2.default_den.as_deref()),
            (Some("1"), Some("4")),
            "2/8 is gcd-reduced to 1/4 — the manifest reports the compiler's normalized rational"
        );
    }

    #[test]
    fn param_manifest_reports_num_den_for_an_integer_bound_on_a_rational_param() {
        // v-guide-infra bug: a Rational @param whose default/range is written as a BARE INTEGER (`default: 5`,
        // `range: [2, 20]`) reported `default_num`/`range_lo_num` = undefined, because a lone int literal
        // folds to `Core::ConstInt` (no Rational-typed context at the config node), and rational_num_den only
        // matched `ConstRational`. But an integer IS the exact rational n/1 — the common case for a Rational
        // slider's bounds. rational_num_den now maps `ConstInt(n)` → (n, 1), so the exact-slider path
        // populates for integer bounds too, not only written fractions.
        let src = "(do \
                     (pragma param (param (: widget slider) (: range (list 2 20)) (: default 5)) (: thickness Rational)) \
                     (def (main) (host (Param) (Param.thickness))) \
                     (export main))";
        let entries = param_manifest(src, "sexpr").expect("param_manifest runs");
        let t = entries
            .iter()
            .find(|e| e.name == "thickness")
            .expect("the `thickness` @param is in the manifest");
        assert_eq!(t.type_name, "Rational", "declared type is Rational");
        // default 5 → the exact rational 5/1 (was undefined before the ConstInt arm).
        assert_eq!(
            (t.default_num.as_deref(), t.default_den.as_deref()),
            (Some("5"), Some("1")),
            "an integer default `5` on a Rational param is the exact rational 5/1"
        );
        // range [2, 20] → 2/1 and 20/1.
        assert_eq!(
            (t.range_lo_num.as_deref(), t.range_lo_den.as_deref()),
            (Some("2"), Some("1")),
            "an integer range-low bound `2` is the exact rational 2/1"
        );
        assert_eq!(
            (t.range_hi_num.as_deref(), t.range_hi_den.as_deref()),
            (Some("20"), Some("1")),
            "an integer range-high bound `20` is the exact rational 20/1"
        );
        // The literal-text fields are still present alongside.
        assert_eq!(t.default.as_deref(), Some("5"), "literal-text default kept");
    }

    /// The playground's "Cadenza" Compiled sub-view is backed by [`emit_cadenza`], which lowers the
    /// program through `Target::Cadenza` and RENDERS the resulting binary AST to the requested text
    /// surface. This pins the browser-facing contract the guide relies on:
    ///
    ///   - a well-formed, lowerable program yields a NON-declined rendering (not `; declined:` / `; error`);
    ///   - the `syntax` toggle is actually threaded — the `"sexpr"` and `"ml"` renderings of the SAME
    ///     program differ in surface;
    ///   - the input `from` surface is honored — an `"ml"`-source program lowers to the identical rendering.
    ///
    /// A regression that made the backend decline a basic scalar program or dropped the surface toggle
    /// would fail here — protecting the guide feature across the fleet. (The ERROR paths cannot be
    /// exercised natively: `JsError::new` is a wasm-bindgen import that panics off-wasm, so this pins only
    /// the Ok-returning success surface, which is what the guide actually renders.)
    #[test]
    fn emit_cadenza_lowers_a_basic_program_and_honors_the_surface_toggle() {
        // A scalar-returning program with a parameter (so it does not fully const-fold away) — squarely
        // inside the cadenza backend's comprehensive scalar-construction coverage. The `match` arms report
        // an unexpected `Err` without formatting the (native-unformattable) `JsError`.
        let src = "(def (main (: x Int64)) (+ x 1)) (export main)";
        let sexpr = match emit_cadenza(src, "sexpr", "sexpr") {
            Ok(s) => s,
            Err(_) => panic!("emit_cadenza returned Err (JsError) on the sexpr surface"),
        };
        assert!(!sexpr.is_empty(), "produced a non-empty rendering");
        assert!(
            !sexpr.starts_with("; declined") && !sexpr.starts_with("; error"),
            "a basic scalar program must LOWER, not decline: {sexpr}"
        );
        // The exported entry survives lowering + re-render — the rendering is real Cadenza source, not a
        // note. (`(+ x 1)` does not const-fold: `x` is a parameter.)
        assert!(
            sexpr.contains("main"),
            "the exported `main` def survives the round-trip: {sexpr}"
        );

        // The `syntax` toggle re-renders the SAME lowered AST in the ML surface — it must differ from the
        // s-expr rendering, proving the arg is threaded to `convert` rather than ignored.
        let ml = match emit_cadenza(src, "sexpr", "ml") {
            Ok(s) => s,
            Err(_) => panic!("emit_cadenza returned Err (JsError) on the ml render surface"),
        };
        assert!(
            !ml.starts_with("; declined") && !ml.starts_with("; error"),
            "the ML rendering must also lower: {ml}"
        );
        assert_ne!(
            sexpr, ml,
            "the sexpr and ml renderings of the same program must differ in surface"
        );

        // The input `from` surface is honored: the same program written in ML lowers just the same (the
        // s-expr reader never gets in the way of an ML-source program).
        let ml_src = "def main(x: Int64) = x + 1\nexport { main }";
        let from_ml = match emit_cadenza(ml_src, "ml", "sexpr") {
            Ok(s) => s,
            Err(_) => panic!("emit_cadenza returned Err (JsError) on the ml INPUT surface"),
        };
        assert!(
            !from_ml.starts_with("; declined") && !from_ml.starts_with("; error"),
            "the ML-sourced program must lower too: {from_ml}"
        );
        // An identical program via either input surface lowers to the identical s-expr rendering — the
        // front-end surface is fully normalized away before the cadenza backend.
        assert_eq!(
            sexpr, from_ml,
            "the same program lowers identically regardless of input surface"
        );
    }

    #[test]
    fn parse_spanned_rejects_an_over_limit_source_before_parsing() {
        // The untrusted-ingestion size guard: a source past CDZ_WASM_MAX_SOURCE_BYTES is a clean error
        // BEFORE parsing (bounds the O(input) arena — the DoS backstop that lets the reader drop its
        // depth cap). The content need not be valid: the guard fires on byte length alone.
        let too_big = "a".repeat(CDZ_WASM_MAX_SOURCE_BYTES + 1);
        let err = parse_spanned(&too_big, Format::Sexpr)
            .expect_err("an over-limit source must be rejected before parsing");
        assert!(
            err.contains("exceeds the maximum size"),
            "expected a size-limit message, got: {err}"
        );
        // The ML surface funnels through the same guard.
        let err_ml = parse_spanned(&too_big, Format::Ml)
            .expect_err("the ML surface funnels through the same size guard");
        assert!(err_ml.contains("exceeds the maximum size"), "got: {err_ml}");
    }

    #[test]
    fn parse_spanned_accepts_a_legit_deep_source_under_the_limit() {
        // The guard must not OVER-reject: a genuinely deep (but under the reader's nesting cap and well
        // under the byte limit) source parses fine. 500 levels is a few KB — far below 1 MiB — and under
        // MAX_NESTING_DEPTH (1024), so the iterative reader parses it without the size guard tripping.
        let n = 500usize;
        let deep = format!("{}1{}", "(+ ".repeat(n), " 1)".repeat(n));
        assert!(
            deep.len() < CDZ_WASM_MAX_SOURCE_BYTES,
            "the deep source must be under the byte limit for this test to be meaningful"
        );
        let (ast_bytes, _) =
            parse_spanned(&deep, Format::Sexpr).expect("a legit under-limit deep source parses");
        assert!(!ast_bytes.is_empty(), "produced AST bytes");
    }

    #[test]
    fn compile_with_preloaded_rejects_an_over_aggregate_input() {
        // The aggregate guard (refinement R1): each preloaded source is individually under the per-source
        // cap, but their SUM exceeds CDZ_WASM_MAX_TOTAL_BYTES, so the many-sources DoS is rejected BEFORE
        // any parsing — surfaced as a codeless diagnostic (not a JS exception). Nine 1 MiB modules = 9 MiB
        // aggregate, past the 8 MiB total cap.
        let module = "a".repeat(CDZ_WASM_MAX_SOURCE_BYTES); // exactly the per-source limit (passes per-source)
        let count = (CDZ_WASM_MAX_TOTAL_BYTES / CDZ_WASM_MAX_SOURCE_BYTES) + 1; // sum just over the total cap
        let names: Vec<String> = (0..count).map(|i| format!("m{i}")).collect();
        let sources: Vec<String> = (0..count).map(|_| module.clone()).collect();
        let formats: Vec<String> = (0..count).map(|_| "sexpr".to_string()).collect();
        let result = compile_with_preloaded("(export main)", "sexpr", names, sources, formats)
            .expect("an over-aggregate input returns a diagnostic, not a JsError");
        assert!(
            result.component.is_none(),
            "an over-aggregate input must not produce a component"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("aggregate source size exceeds")),
            "expected an aggregate-size diagnostic, got: {:?}",
            result
                .diagnostics
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
    }
}
