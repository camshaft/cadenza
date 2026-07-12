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
) -> Diagnostic {
    let node = d.node.unwrap_or(u32::MAX);
    let (from, to) = d
        .node
        .and_then(|n| spans?.get(cadenza_syntax::ast::StructId(n)))
        .map(|s| (s.start as u32, s.end as u32))
        .unwrap_or((0, 0));
    Diagnostic {
        error: d.severity == rcdzc::Severity::Error,
        code: d.code.clone().unwrap_or_default(),
        message: d.message.clone(),
        node,
        from,
        to,
    }
}

/// Parse `text` in `surface` into rcdzc's binary AST bytes AND the front-end span table (node id →
/// UTF-8 byte range) built from the SAME parse. Because a fresh parse is already in canonical normal
/// form (`canon.rs` — the readers build structure in the codec's canonical order), encoding to bytes
/// and decoding into rcdzc's arena preserves the user node ids, so the span table indexes rcdzc's
/// diagnostics/type nodes directly. Only the s-expression and ML surfaces have a reader; a binary or
/// output-only (`debug`/`flat`) `from` has no source spans, so `spans` is `None` there.
fn parse_spanned(
    text: &str,
    from: Format,
) -> Result<(Vec<u8>, Option<cadenza_syntax::spans::SpanTable>), String> {
    match from {
        Format::Sexpr => {
            let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(text).map_err(|e| e.0)?;
            Ok((cadenza_syntax::codec::encode(&arenas), Some(spans)))
        }
        Format::Ml => {
            let parsed = cadenza_syntax::parser::read_ml(text);
            if let Some(err) = parsed.errors.first() {
                return Err(format!(
                    "ML parse error at byte {}: {}",
                    err.span.start, err.message
                ));
            }
            Ok((
                cadenza_syntax::codec::encode(&parsed.arenas),
                Some(parsed.spans),
            ))
        }
        // No reader (or output-only) — fall back to the format-agnostic byte conversion, no spans.
        _ => convert::convert(text.as_bytes(), from, Format::Binary)
            .map(|b| (b, None))
            .map_err(|e| e.0),
    }
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
            return Ok(CompileResult {
                component: None,
                diagnostics: vec![Diagnostic {
                    error: true,
                    code: String::new(),
                    message: msg,
                    node: u32::MAX,
                    from: 0,
                    to: 0,
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
        .map(|d| to_js_diag(d, spans.as_ref()))
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
            }]);
        }
    };
    let arenas = match rcdzc::codec::decode(&ast_bytes) {
        Some(a) => a,
        None => return Err(JsError::new("internal: re-encoded AST failed to decode")),
    };
    let mut db = rcdzc::db::Db::load(arenas);
    let diags = rcdzc::diagnostics(&mut db);
    Ok(diags
        .iter()
        .map(|d| to_js_diag(d, spans.as_ref()))
        .collect())
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
    let arenas = match rcdzc::codec::decode(&ast_bytes) {
        Some(a) => a,
        None => return Err(JsError::new("internal: re-encoded AST failed to decode")),
    };
    let mut db = rcdzc::db::Db::load(arenas);
    // The node id crosses the copy-don't-depend boundary as its raw index (`cadenza_syntax` and
    // `rcdzc` each have their own `StructId`, but the byte-identical codec keeps the index space
    // aligned — the same invariant `type-at` relies on).
    let ty = rcdzc::infer::type_of(&mut db, rcdzc::ast::StructId(node.0));
    let name = ty.render_name();
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
    let arenas = match rcdzc::codec::decode(&ast_bytes) {
        Some(a) => a,
        None => return Err(JsError::new("internal: re-encoded AST failed to decode")),
    };
    let mut db = rcdzc::db::Db::load(arenas);
    // A name reference resolves to the occurrence it denotes:
    //   - `Ref { value }`  — a nullary def's body, a `let` initializer, a parameter binder, a sum-type
    //                        or prelude binding (prelude ones have no span, filtered below);
    //   - `Lambda { body }` — a def WITH parameters (a function). Its body is where the definition is.
    // That occurrence's source span IS the definition to jump to.
    let target = match rcdzc::resolve::resolved_of(&mut db, rcdzc::ast::StructId(node.0)) {
        rcdzc::resolved::Resolved::Ref { value } => value,
        rcdzc::resolved::Resolved::Lambda { body, .. } => body,
        _ => return Ok(None), // not a navigable reference (a literal, a prim, an unbound name, …)
    };
    // The target must be a USER node with a source span (a prelude binding has none — nothing to jump
    // to in the editor). The span table is keyed in the shared index space (codec keeps ids aligned).
    let Some(def_span) = spans.get(cadenza_syntax::ast::StructId(target.0)) else {
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

/// The content-address (SHA-256, hex) of the value-heap runtime this compiler emits imports against.
///
/// A compound-returning program imports `cadenza:runtime/heap@0.0.0+<hash>`; the guide must compose
/// the runtime whose hash equals this. Exposing it lets the JS side assert it bundled the right
/// runtime `.wasm` rather than hard-coding the hex in two places.
#[wasm_bindgen]
pub fn required_runtime_hash() -> String {
    rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH.to_string()
}
