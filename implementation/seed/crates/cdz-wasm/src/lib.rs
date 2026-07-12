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

fn to_js_diag(d: &rcdzc::Diagnostic) -> Diagnostic {
    Diagnostic {
        error: d.severity == rcdzc::Severity::Error,
        code: d.code.clone().unwrap_or_default(),
        message: d.message.clone(),
        node: d.node.unwrap_or(u32::MAX),
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

    // Text surface -> canonical binary AST. A parse failure becomes a codeless error diagnostic.
    let ast_bytes = match convert::convert(text.as_bytes(), from, Format::Binary) {
        Ok(bytes) => bytes,
        Err(e) => {
            return Ok(CompileResult {
                component: None,
                diagnostics: vec![Diagnostic {
                    error: true,
                    code: String::new(),
                    message: e.0,
                    node: u32::MAX,
                }],
            });
        }
    };

    // Binary AST -> WebAssembly component. `compile_component` returns the first error diagnostic on
    // failure; use the full `compile` entry so warnings ride alongside a successful component too.
    let out = rcdzc::compile(
        &[rcdzc::Artifact::new(
            rcdzc::Artifact::KIND_AST,
            "main",
            ast_bytes,
        )],
        &[rcdzc::Target::Wasm],
    );
    let diagnostics = out.diagnostics.iter().map(to_js_diag).collect();
    let component = out
        .artifact(rcdzc::Target::Wasm.artifact_kind())
        .map(|b| b.to_vec());
    Ok(CompileResult {
        component,
        diagnostics,
    })
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
    let bytes = convert::convert(text.as_bytes(), from, to)
        .map_err(|e| JsError::new(&format!("convert {} -> {}: {}", from.name(), to.name(), e.0)))?;
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
