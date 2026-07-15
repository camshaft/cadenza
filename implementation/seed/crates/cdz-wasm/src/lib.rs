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

/// Decode `ast_bytes` into a `Db` and run one sidecar `Query`, returning its result artifact's bytes
/// as UTF-8 text. This is the ONE path every IDE fact read goes through — the browser IDE speaks the
/// same sidecar query vocabulary as the `cdz` CLI (`cdz type-at`/`def`/`check`/`uses`), so a fact the
/// editor shows equals what the CLI would answer, by construction. (The span table stays a front-end
/// concern: the consumer maps a query's node ids to source ranges, per the query-engine contract.)
fn run_query_text(ast_bytes: &[u8], query: &rcdzc::Query) -> Result<String, JsError> {
    let arenas = rcdzc::codec::decode(ast_bytes)
        .ok_or_else(|| JsError::new("internal: re-encoded AST failed to decode"))?;
    let mut db = rcdzc::db::Db::load(arenas);
    let result = rcdzc::sidecar::run_query(&mut db, query);
    String::from_utf8(result.bytes).map_err(|_| JsError::new("query result was not valid UTF-8"))
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
            return Ok(CompileResult {
                component: None,
                diagnostics: vec![Diagnostic {
                    error: true,
                    code: String::new(),
                    message: msg,
                    node: u32::MAX,
                    from: from_b,
                    to: to_b,
                    fix_replacement: String::new(),
                    fix_prefix: String::new(),
                    fix_suffix: String::new(),
                    fix_from: 0,
                    fix_to: 0,
                    fix_verified: false,
                    fix_kind: String::new(),
                }],
            });
        }
    };

    // Binary AST -> WebAssembly component. Use the full `compile` entry so warnings ride alongside a
    // successful component too.
    //
    // EMBED DWARF DEBUG INFO whenever we have a span table (a text surface — the guide's case): pass
    // the `spans` artifact and request `WasmDebug`, so the emitted component carries `.debug_line` /
    // `.debug_info` sections. Chrome's "C/C++ DevTools Support (DWARF)" extension then steps through the
    // ACTUAL Cadenza source and prints scalar arguments — the whole point of running the guide's
    // programs in the browser debugger. The sections are inert (they change no executed byte) and
    // strippable, so this costs nothing at runtime; a binary/output-only surface has no spans and falls
    // back to a plain component (`DESIGN-debug-info-rcdzc.md`, Mode E).
    let mut inputs = vec![rcdzc::Artifact::new(
        rcdzc::Artifact::KIND_AST,
        "main",
        ast_bytes,
    )];
    let target = match &spans {
        Some(span_table) => {
            inputs.push(rcdzc::Artifact::new(
                rcdzc::spans::KIND_SPANS,
                "main",
                rcdzc::spans::encode(&span_data_of(text, span_table)),
            ));
            rcdzc::Target::WasmDebug
        }
        None => rcdzc::Target::Wasm,
    };
    let out = rcdzc::compile(&inputs, &[target]);
    let diagnostics = out
        .diagnostics
        .iter()
        .map(|d| to_js_diag(d, spans.as_ref(), from == Format::Ml))
        .collect();
    // Both `Wasm` and `WasmDebug` produce a `component`-kinded artifact (a debug component is a
    // decorated component, not a new kind), so the artifact lookup is the same either way.
    let component = out
        .artifact(rcdzc::Target::Wasm.artifact_kind())
        .map(|b| b.to_vec());
    Ok(CompileResult {
        component,
        diagnostics,
    })
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
    let repl_parse_err = |msg: String| CompileResult {
        component: None,
        diagnostics: vec![Diagnostic {
            error: true,
            code: String::new(),
            message: msg,
            node: u32::MAX,
            from: 0,
            to: 0,
            fix_replacement: String::new(),
            fix_prefix: String::new(),
            fix_suffix: String::new(),
            fix_from: 0,
            fix_to: 0,
            fix_verified: false,
            fix_kind: String::new(),
        }],
    };

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
            return Ok(vec![Diagnostic {
                error: true,
                code: String::new(),
                message: msg,
                node: u32::MAX,
                from: 0,
                to: 0,
                fix_replacement: String::new(),
                fix_prefix: String::new(),
                fix_suffix: String::new(),
                fix_from: 0,
                fix_to: 0,
                fix_verified: false,
                fix_kind: String::new(),
            }]);
        }
    };
    // Ride the first-class `Diagnostics` sidecar query (the same one `cdz check` runs) — a total fault
    // read that needs no export. Its result is one fault per line, TAB-separated:
    // `severity  code  node  fix-kind  fix-node  fix-replacement  fix-verified  message` (each of code /
    // node / the four fix columns is `-` when absent). We resolve each fault's node id — and its fix's
    // node id — to a byte span here.
    let text_out = run_query_text(&ast_bytes, &rcdzc::Query::Diagnostics)?;
    let span_of = |field: &str| -> (u32, u32) {
        field
            .parse::<u32>()
            .ok()
            .and_then(|n| spans.as_ref()?.get(cadenza_syntax::ast::StructId(n)))
            .map(|s| (s.start as u32, s.end as u32))
            .unwrap_or((0, 0))
    };
    let mut out = Vec::new();
    for line in text_out.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(8, '\t');
        let severity = parts.next().unwrap_or("error");
        let code = parts.next().unwrap_or("-");
        let node_field = parts.next().unwrap_or("-");
        let fix_kind = parts.next().unwrap_or("-");
        let fix_node_field = parts.next().unwrap_or("-");
        let fix_replacement = parts.next().unwrap_or("-");
        let fix_verified = parts.next().unwrap_or("-");
        let message = parts.next().unwrap_or("").to_string();
        let node = node_field.parse::<u32>().ok();
        let (span_from, span_to) = span_of(node_field);
        let has_fix = fix_node_field != "-";
        let (fix_from, fix_to) = if has_fix {
            span_of(fix_node_field)
        } else {
            (0, 0)
        };
        // A WRAP fix splits into surface-correct `prefix`/`suffix` (never the raw `…`-sentinel
        // replacement, which a JS splice would write literally and corrupt) — the shared reshape+split.
        let (fix_replacement, fix_prefix, fix_suffix) = if has_fix && fix_kind == "wrap" {
            let (p, s) = rcdzc::wrap_prefix_suffix(fix_replacement, from == Format::Ml);
            (String::new(), p, s)
        } else if has_fix {
            (fix_replacement.to_string(), String::new(), String::new())
        } else {
            (String::new(), String::new(), String::new())
        };
        out.push(Diagnostic {
            error: severity != "warning",
            code: if code == "-" {
                String::new()
            } else {
                code.to_string()
            },
            message,
            node: node.unwrap_or(u32::MAX),
            from: span_from,
            to: span_to,
            fix_replacement,
            fix_prefix,
            fix_suffix,
            fix_from,
            fix_to,
            fix_verified: fix_verified == "verified",
            fix_kind: if has_fix {
                fix_kind.to_string()
            } else {
                String::new()
            },
        });
    }
    Ok(out)
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
    // aligned). The result is the rendered type text (or "unknown" for a non-user/unsolved node).
    let name = run_query_text(&ast_bytes, &rcdzc::Query::TypeAt { node: node.0 })?;
    Ok(Some(TypeAt {
        type_name: name,
        from: span.start as u32,
        to: span.end as u32,
    }))
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
    // this node to its defining occurrence's node id (following `Ref`/`Lambda`), or an empty result for
    // a non-navigable token or a span-less binding. One node id, or empty.
    let text_out = run_query_text(&ast_bytes, &rcdzc::Query::ResolveOf { node: node.0 })?;
    let Some(target_id) = text_out
        .lines()
        .next()
        .and_then(|l| l.trim().parse::<u32>().ok())
    else {
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
    let text_out = run_query_text(&ast_bytes, &rcdzc::Query::Exports)?;
    // `Exports` yields `name<TAB>type<TAB>def-name-node-id` per line; keep just `name<TAB>type`.
    let mut out = String::new();
    for line in text_out.lines() {
        let mut cols = line.split('\t');
        if let (Some(name), Some(ty)) = (cols.next(), cols.next()) {
            out.push_str(name);
            out.push('\t');
            out.push_str(ty);
            out.push('\n');
        }
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

/// The content-address (SHA-256, hex) of the value-heap runtime this compiler emits imports against.
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
    // Ride the `Highlight` query — one `node-id<TAB>kind` line per classified leaf, ascending id order.
    let text_out = run_query_text(&ast_bytes, &rcdzc::Query::Highlight)?;
    let mut out = Vec::new();
    for line in text_out.lines() {
        let mut cols = line.splitn(2, '\t');
        let (Some(node), Some(kind)) = (cols.next(), cols.next()) else {
            continue;
        };
        // Map the node id to a byte span; skip a span-less node (should not happen for a user leaf).
        if let Some(span) = node
            .parse::<u32>()
            .ok()
            .and_then(|n| spans.get(cadenza_syntax::ast::StructId(n)))
        {
            out.push(SemanticToken {
                from: span.start as u32,
                to: span.end as u32,
                kind: kind.to_string(),
            });
        }
    }
    Ok(out)
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
    // Ride the `Instantiations` query — TAB-tagged lines: one `disp<TAB>name-node<TAB>DISPOSITION`, then
    // (for a specialized def) `inst<TAB>spec-name<TAB>name-node<TAB>arg;arg;…` per instance. An UNKNOWN
    // name (the hovered token names no definition) yields the empty string → no tooltip.
    let text_out = run_query_text(
        &ast_bytes,
        &rcdzc::Query::Instantiations {
            name: name.to_string(),
        },
    )?;
    let mut disposition = String::new();
    let mut def_span = span;
    let mut instances: Vec<String> = Vec::new();
    for line in text_out.lines() {
        let mut cols = line.split('\t');
        match cols.next() {
            Some("disp") => {
                let (Some(node_str), Some(disp)) = (cols.next(), cols.next()) else {
                    continue;
                };
                disposition = disp.to_string();
                // Anchor the tooltip to the DEFINITION's name occurrence (which may differ from the
                // hovered use), so the range is stable whether the reader hovers the def or a call site.
                if let Some(s) = node_str
                    .parse::<u32>()
                    .ok()
                    .and_then(|n| spans.get(cadenza_syntax::ast::StructId(n)))
                {
                    def_span = s;
                }
            }
            Some("inst") => {
                // `inst<TAB>spec-name<TAB>name-node<TAB>arg;arg;…` — render the `;`-joined args as a
                // readable `a, b, c`, dropping the unstable synthesized spec name + the node id.
                let (_spec, _node, arglist) = match (cols.next(), cols.next(), cols.next()) {
                    (Some(s), Some(n), Some(a)) => (s, n, a),
                    _ => continue,
                };
                let pretty = arglist
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                instances.push(pretty);
            }
            _ => continue,
        }
    }
    // No `disp` line ⇒ the hovered token names no definition (an unknown name) ⇒ no tooltip.
    if disposition.is_empty() {
        return Ok(None);
    }
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
}
