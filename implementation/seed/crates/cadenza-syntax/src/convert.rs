//! Format conversion between the three surfaces of one program: the canonical **binary** encoding,
//! the **s-expression** text, and the **ML** text. All three are projections of the same
//! [`Arenas`]; converting is `read into arenas` then `write from arenas`. Pure (no I/O) — the CLI
//! bin does the file/stdin/stdout plumbing.
//!
//! This crate IS the reader/printer surface the compiler exposes — text-to-canonical-binary (a
//! `read`) and canonical-binary-to-text (a `print`) — so the knowledge of a value's textual form lives
//! here, not in a host; and the text a printer produces is the value's canonical text (a structurally-
//! equal value prints identical text):
//!
//= spec/capabilities/self-hosting-surface.md#the-reader-printer-and-display-are-compiler-exposed-surfaces
//# The reader, printer, and display conversion MUST be surfaces the compiler exposes — text-to-canonical-binary, canonical-binary-to-text, and typed-result-to-text — rather than logic any host embeds, so that the knowledge of a value's textual form lives in the compiler and a host stays value-agnostic (host-interface-binding.md §The Host Formats Nothing).
//!
//= spec/capabilities/self-hosting-surface.md#the-reader-printer-and-display-are-compiler-exposed-surfaces
//# The text form the printer produces for a value MUST be the value's canonical text form, so that two runs producing structurally-equal values print identical text.

use crate::ast::Arenas;
use crate::{codec, parser, sexpr};

/// A surface format a program can be read from or written to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    /// The canonical hand-rolled binary encoding.
    Binary,
    /// The s-expression text surface.
    Sexpr,
    /// The keyword-based ML text surface.
    Ml,
    /// The markdown surface: a literate `(document …)`. Reads any CommonMark to a document arena and
    /// prints a document arena back to CommonMark; a fenced `cdz`/`ml`/`sexp` block embeds its program
    /// as a real arena subtree. A document is data, not a program (the compiler never sees one).
    Markdown,
    /// The JSON surface: a faithful data document (`(json-object …)`/`(json-array …)`/`(json-null)`
    /// plus scalar leaves). Reads any JSON to a value arena and prints a value arena back to JSON. Like
    /// a markdown document, it is data, not a program; it preserves duplicate/non-identifier keys, key
    /// order, heterogeneous arrays, and exact numbers rather than coercing to native `record`/`list`.
    Json,
    /// The TOML surface: a source-faithful config document (`(toml-document …)`). Reads any TOML to a
    /// decor-in-arena value document (comments, whitespace, and each scalar's raw spelling stored as
    /// nodes) and prints it back BYTE-EXACT for an unmutated doc. Data, not a program; interconverts
    /// with JSON.
    Toml,
    /// The Cedar surface: an authorization-policy document (`(cedar-policyset …)`, mirroring Cedar's
    /// `pst`). Reads Cedar policy text to a structured arena and prints it back. Data, not a program
    /// (no authorization engine); its point is that policies become structurally editable by Cadenza's
    /// tools. Arena-idempotent (comments/formatting not preserved by the underlying pst).
    Cedar,
    /// A readable debug view of the arena structure as an indented TREE — OUTPUT ONLY (not a
    /// re-readable surface). Shows the raw shape the compiler sees, for inspecting a binary AST.
    Debug,
    /// A FLAT dump of the two arenas (leaf pool + structure vector + root) — OUTPUT ONLY. Shows the
    /// storage layout directly: leaf interning and the post-order structure order the codec writes.
    Flat,
}

impl Format {
    /// Parse a format name (`binary`/`bin`, `sexpr`/`sexp`, `ml`, `markdown`/`md`, `json`, `toml`,
    /// `cedar`, `debug`, `flat`). Case-insensitive.
    pub fn parse(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "binary" | "bin" => Some(Format::Binary),
            "sexpr" | "sexp" | "s" => Some(Format::Sexpr),
            "ml" => Some(Format::Ml),
            "markdown" | "md" => Some(Format::Markdown),
            "json" => Some(Format::Json),
            "toml" => Some(Format::Toml),
            "cedar" => Some(Format::Cedar),
            "debug" => Some(Format::Debug),
            "flat" => Some(Format::Flat),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Binary => "binary",
            Format::Sexpr => "sexpr",
            Format::Ml => "ml",
            Format::Markdown => "markdown",
            Format::Json => "json",
            Format::Toml => "toml",
            Format::Cedar => "cedar",
            Format::Debug => "debug",
            Format::Flat => "flat",
        }
    }

    /// Infer the surface format from a file path's extension: `.cdz`/`.ml` → ML, `.sexp`/`.sexpr` →
    /// s-expr, `.bin`/`.cdzb` → binary, `.md`/`.markdown` → markdown, `.json` → JSON, `.toml` → TOML,
    /// `.cedar` → Cedar. The output-only `debug`/`flat` views have no extension. `None` if the path has
    /// no recognized extension (the caller then requires an explicit format).
    pub fn from_extension(path: &str) -> Option<Format> {
        let ext = std::path::Path::new(path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase();
        match ext.as_str() {
            "cdz" | "ml" => Some(Format::Ml),
            "sexp" | "sexpr" => Some(Format::Sexpr),
            "bin" | "cdzb" => Some(Format::Binary),
            "md" | "markdown" => Some(Format::Markdown),
            "json" => Some(Format::Json),
            "toml" => Some(Format::Toml),
            "cedar" => Some(Format::Cedar),
            _ => None,
        }
    }
}

/// A conversion failure, with a human-readable message.
#[derive(Debug)]
pub struct ConvertError(pub String);

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ConvertError {}

/// Rewrite an `at byte N` in a reader error message to `at LINE:COL`, using `src` to map the offset —
/// so a multi-line parse error points at a place a user/editor can navigate to (`(module …\n  )))` →
/// "at 4:3", not "at byte 40"). The reader bakes the byte offset into its error string (it has no
/// line:col at the recursive-descent site); the callers that HOLD the source (this module, `cdz`'s
/// loader) map it here. Handles a byte number that is NOT at the very end — the JSON reader writes
/// `invalid literal at byte 6 (expected `null`)`, with text after the offset — by rewriting just the
/// `at byte <digits>` run and keeping the trailing text. A message with no `at byte N` (e.g.
/// "unterminated list", "unexpected end of input") is returned unchanged.
pub fn locate_byte_in_message(msg: &str, src: &str) -> String {
    const MARKER: &str = " at byte ";
    // The LAST marker is the position one (an earlier "byte" would be prose); rewrite its digit run.
    let Some(marker_at) = msg.rfind(MARKER) else {
        return msg.to_string();
    };
    let after = marker_at + MARKER.len();
    // The digit run immediately after the marker — may be followed by trailing text (` (expected …)`).
    let digits_end = after
        + msg[after..]
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(msg.len() - after);
    let Ok(byte) = msg[after..digits_end].parse::<usize>() else {
        return msg.to_string(); // no integer after the marker — leave untouched
    };
    let (line, col) = crate::query::driver::line_col(src, byte);
    format!(
        "{} at {line}:{col}{}",
        &msg[..marker_at],
        &msg[digits_end..]
    )
}

/// Read `input` (bytes) in `from` format into arenas.
///
/// Text formats decode `input` as UTF-8 first; the binary format uses the bytes directly. A text
/// parse that recovers errors is reported as a failure (with the first error's message), because a
/// convert is meant to be faithful — not to silently emit a patched-up tree.
pub fn read(input: &[u8], from: Format) -> Result<Arenas, ConvertError> {
    match from {
        Format::Binary => {
            codec::decode(input).ok_or_else(|| ConvertError("invalid binary encoding".into()))
        }
        Format::Sexpr => {
            let text = utf8(input)?;
            sexpr::read(text).map_err(|e| {
                ConvertError(format!(
                    "s-expr parse error: {}",
                    locate_byte_in_message(&e.0, text)
                ))
            })
        }
        Format::Ml => {
            let text = utf8(input)?;
            let parsed = parser::read_ml(text);
            if let Some(err) = parsed.errors.first() {
                // Render the position as `line:col`, not a raw byte offset an editor/user can't place —
                // the same shape `cdz check` gives an ML parse error (the source is in hand here, so the
                // mapping is cheap and there is no reason to leak the byte number).
                let (line, col) = crate::query::driver::line_col(text, err.span.start);
                return Err(ConvertError(format!(
                    "ML parse error at {line}:{col}: {}",
                    err.message
                )));
            }
            Ok(parsed.arenas)
        }
        Format::Markdown => {
            // CommonMark parsing is total (it never fails), so unlike the code surfaces there is no
            // error to surface — a document always reads to a `(document …)` arena.
            let text = utf8(input)?;
            Ok(crate::markdown::read(text))
        }
        Format::Json => {
            // JSON, unlike CommonMark, can fail — a malformed document is a clean error, mapped from
            // the reader's `at byte N` to `at line:col` like the s-expr surface.
            let text = utf8(input)?;
            crate::json::read(text).map_err(|e| {
                ConvertError(format!(
                    "JSON parse error: {}",
                    locate_byte_in_message(&e.0, text)
                ))
            })
        }
        Format::Toml => {
            // TOML can fail too — surface the parse error with its byte offset mapped to line:col.
            let text = utf8(input)?;
            crate::toml_surface::read(text).map_err(|e| {
                ConvertError(format!(
                    "TOML parse error: {}",
                    locate_byte_in_message(&e.0, text)
                ))
            })
        }
        #[cfg(feature = "cedar")]
        Format::Cedar => {
            // Cedar can fail; its error is a multi-line `ParseErrors` with a source excerpt (not an
            // `at byte N`), so the reader already reduced it to a headline — surface it as-is.
            let text = utf8(input)?;
            crate::cedar::read(text)
                .map_err(|e| ConvertError(format!("Cedar parse error: {}", e.0)))
        }
        // Lean build (no `cedar` feature): the Cedar surface isn't compiled — a clean error, not a panic.
        #[cfg(not(feature = "cedar"))]
        Format::Cedar => Err(ConvertError(
            "the `cedar` surface is not compiled in this build (enable the `cedar` feature)".into(),
        )),
        // `debug` is an output-only view — there is no reader from it back to arenas.
        // `debug`/`flat` are output-only views — there is no reader from them back to arenas.
        Format::Debug => Err(ConvertError(
            "`debug` is an output-only format, not an input".into(),
        )),
        Format::Flat => Err(ConvertError(
            "`flat` is an output-only format, not an input".into(),
        )),
    }
}

/// Output options for a `write`.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Target line width for the ML pretty-printer.
    pub width: usize,
    /// Render for human DISPLAY rather than for re-reading (ML target only). When set, a value's ML
    /// text drops the round-trip ceremony — a `Rational` prints bare (`1/4`), a quantity in its
    /// concise `<value> <unit>` surface, and an outer result type annotation is stripped — the spec's
    /// "typed-result-to-text" display conversion (self-hosting-surface.md). `false` (the default) is
    /// the canonical, re-readable printer that `cdz convert` and the round-trip gate rely on.
    pub display: bool,
    /// Render the s-expression output STRUCTURALLY (Sexpr target only): comment nodes print as ordinary
    /// `(comment "text" form)` / `(comment-after "text" form)` lists rather than being collapsed back to
    /// `;` line-comments. This is the `render_sexpr` form (DESIGN-parser-test-corpus.md §2) — the parse-
    /// tree golden the `spec/syntax/` corpus compares against, where a comment is part of the tree, not
    /// droppable `;` trivia. `false` (the default) keeps the fmt-idempotent `;` rendering. No effect on a
    /// non-Sexpr target.
    pub structural: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            width: crate::printer::DEFAULT_WIDTH,
            display: false,
            structural: false,
        }
    }
}

/// Write `arenas` in `to` format to bytes, with default options.
pub fn write(arenas: &Arenas, to: Format) -> Result<Vec<u8>, ConvertError> {
    write_with(arenas, to, Options::default())
}

/// Write `arenas` in `to` format to bytes, with explicit options (the ML width).
pub fn write_with(arenas: &Arenas, to: Format, opts: Options) -> Result<Vec<u8>, ConvertError> {
    match to {
        Format::Binary => Ok(codec::encode(arenas)),
        // The STRUCTURAL s-expr render (`--structural`): comment nodes as ordinary `(comment …)` lists,
        // the `spec/syntax/` parse-tree golden form (DESIGN-parser-test-corpus.md §2). Same layout as the
        // default pretty print — only comment handling differs.
        Format::Sexpr if opts.structural => {
            Ok(sexpr::render_sexpr_width(arenas, opts.width).into_bytes())
        }
        // The s-expr surface pretty-prints across lines (breaking a form only when it overflows
        // `width`), the same width knob the ML printer uses — a single-line dump is unreadable for
        // anything but the smallest forms.
        Format::Sexpr => Ok(sexpr::print_pretty_width(arenas, opts.width).into_bytes()),
        Format::Ml if opts.display => {
            Ok(crate::printer::print_display(arenas, opts.width).into_bytes())
        }
        Format::Ml => Ok(crate::printer::print(arenas, opts.width).into_bytes()),
        // A `(document …)` arena prints back to CommonMark; a NON-document root (a bare program handed
        // to `--to markdown`) is wrapped in a single ```cdz fence over its ML rendering (see
        // `markdown::print`), so `--to markdown` stays total.
        Format::Markdown => Ok(crate::markdown::print(arenas, opts.width).into_bytes()),
        // A JSON value arena prints back to JSON; a NON-JSON root (a bare program handed to `--to json`)
        // becomes a single JSON string over its ML rendering (see `json::print`), so `--to json` stays
        // total.
        Format::Json => {
            Ok(crate::json::print(arenas, opts.width, crate::printer::print).into_bytes())
        }
        // A `(toml-document …)` arena prints back BYTE-EXACT (unmutated); a NON-TOML root becomes a
        // single `program = "<ml>"` key (see `toml_surface::print`), so `--to toml` stays total.
        Format::Toml => {
            Ok(crate::toml_surface::print(arenas, opts.width, crate::printer::print).into_bytes())
        }
        // A `(cedar-policyset …)` arena prints back to Cedar policy text (rebuilding a pst); a NON-Cedar
        // root becomes a `//`-comment block over its ML rendering (see `cedar::print`), so `--to cedar`
        // stays total.
        #[cfg(feature = "cedar")]
        Format::Cedar => {
            Ok(crate::cedar::print(arenas, opts.width, crate::printer::print).into_bytes())
        }
        // Lean build (no `cedar` feature): the Cedar surface isn't compiled — a clean error, not a panic.
        #[cfg(not(feature = "cedar"))]
        Format::Cedar => Err(ConvertError(
            "the `cedar` surface is not compiled in this build (enable the `cedar` feature)".into(),
        )),
        Format::Debug => Ok(crate::debug::print(arenas).into_bytes()),
        Format::Flat => Ok(crate::debug::print_flat(arenas).into_bytes()),
    }
}

/// Convert `input` from `from` to `to` in one step, with default options.
pub fn convert(input: &[u8], from: Format, to: Format) -> Result<Vec<u8>, ConvertError> {
    convert_with(input, from, to, Options::default())
}

/// Convert `input` from `from` to `to`, with explicit options (the ML width).
pub fn convert_with(
    input: &[u8],
    from: Format,
    to: Format,
    opts: Options,
) -> Result<Vec<u8>, ConvertError> {
    let arenas = read(input, from)?;
    write_with(&arenas, to, opts)
}

fn utf8(input: &[u8]) -> Result<&str, ConvertError> {
    std::str::from_utf8(input).map_err(|_| ConvertError("input is not valid UTF-8".into()))
}

/// The grammatical KIND of an embedded Cadenza FRAGMENT, supplied by the author (the guide's `(cdz …)` /
/// `(cdz-type …)` / `(cdz-pat …)` sibling tags) and threaded to [`render_binary`]. Cadenza's reader parses
/// a fragment UNIFORMLY to faithful binary-AST — a `(Tuple Int64 Iter)` is the SAME application-shaped AST
/// whether the author means it as a value or a type — so the grammatical role is a RENDER-time property
/// (positional in a full program), and a STANDALONE fragment must be TOLD which surface role to render in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentKind {
    /// An expression fragment (the default + by far the most frequent): value / application / member /
    /// list-or-record with rest. Rendered by the ordinary canonical printer.
    Expr,
    /// A type fragment. VERIFIED (`type_fragments_render_idiomatically_kind_independent`): the canonical
    /// printer ALREADY renders a type fragment idiomatically — the ML type surface falls out of the AST
    /// SHAPE (arrow `(-> A B)` → `A -> B`, type application `(Tuple/Option/List/… args)` → `Name(args)`),
    /// so the render is kind-INDEPENDENT and no separate type-render path is needed. `Tuple(Int64, Iter)`
    /// IS the idiomatic ML tuple-type surface (NOT a ctor-application mis-render). `Type` is carried for
    /// tag semantics / future-proofing, not because rendering currently branches on it.
    Type,
    /// A pattern fragment. A backable pattern (a list/record WITH a rest) renders idiomatically via the
    /// canonical printer, kind-independent (v-syntax verified the round-trip). A bare STANDALONE spread
    /// cannot be backed (it has no meaning outside its enclosing construction) and stays static.
    Pattern,
}

impl FragmentKind {
    /// Parse the kind name carried on the tag→codegen→render wire (`expr` / `type` / `pattern`).
    pub fn parse(name: &str) -> Option<FragmentKind> {
        match name {
            "expr" => Some(FragmentKind::Expr),
            "type" => Some(FragmentKind::Type),
            "pattern" | "pat" => Some(FragmentKind::Pattern),
            _ => None,
        }
    }
}

/// Render a canonical binary-AST document to a text SURFACE string — the binary→surface direction the
/// guide's `(cdz …)` inline tag (rendering an EMBEDDED AST per-surface for the auto-toggle) and the general
/// AST-to-string refactor tool need. Decodes the binary AST, then prints it via the canonical per-surface
/// printer (binary-AST is THE exchange format — one canonical render, NO text re-parse). `surface` is a
/// text surface (`Sexpr`/`Ml`/…); `kind` selects the FRAGMENT render mode (see [`FragmentKind`]).
///
/// Rendering is currently KIND-INDEPENDENT: the canonical printer renders every BACKABLE fragment
/// idiomatically from its AST shape — `Expr` (value/application/member/list-record-with-rest), `Type` (the
/// ML type surface `A -> B` / `Name(args)` falls out of the same shape, VERIFIED
/// `type_fragments_render_idiomatically_kind_independent`), and a backable `Pattern` (list/record with
/// rest). The `kind` is threaded so the interface (and its `cdz-wasm` binding) is stable and carries the
/// tag's semantic role — and is the seam a HYPOTHETICAL future construct that renders differently per kind
/// would branch at — but no such branch is needed today. (A bare standalone spread cannot be backed at all.)
pub fn render_binary(
    bytes: &[u8],
    surface: Format,
    kind: FragmentKind,
    opts: Options,
) -> Result<String, ConvertError> {
    let arenas = read(bytes, Format::Binary)?;
    // Rendering is kind-independent — the idiomatic per-kind surface falls out of the AST shape via the
    // canonical printer (see the doc + `type_fragments_render_idiomatically_kind_independent`). `kind` is
    // the seam a future per-kind divergence would branch at; none is needed today.
    let _ = kind;
    let out = write_with(&arenas, surface, opts)?;
    String::from_utf8(out)
        .map_err(|e| ConvertError(format!("surface render is not valid UTF-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_binary_decodes_and_prints_the_canonical_surface() {
        // render_binary is the binary→surface direction (guide `(cdz …)` tag + AST-to-string refactor tool):
        // decode the canonical binary AST, then print via the canonical per-surface printer — byte-identical
        // to `write_with` on the decoded arena, and NO text re-parse. Pin it for both surfaces + the kinds.
        let arenas = crate::sexpr::read("(module m (def (main) (: 42 Int64)) (export main))")
            .expect("sexpr parses");
        let bytes = crate::codec::encode(&arenas);
        let decoded = read(&bytes, Format::Binary).expect("decodes the binary AST");

        // Sexpr surface == the canonical sexpr pretty printer on the decoded arena.
        let expect_sexpr =
            String::from_utf8(write_with(&decoded, Format::Sexpr, Options::default()).unwrap())
                .unwrap();
        assert_eq!(
            render_binary(
                &bytes,
                Format::Sexpr,
                FragmentKind::Expr,
                Options::default()
            )
            .unwrap(),
            expect_sexpr,
            "render_binary(Sexpr, Expr) is the canonical sexpr render of the decoded AST"
        );
        // ML surface == the canonical ML printer on the decoded arena.
        let expect_ml =
            String::from_utf8(write_with(&decoded, Format::Ml, Options::default()).unwrap())
                .unwrap();
        assert_eq!(
            render_binary(&bytes, Format::Ml, FragmentKind::Expr, Options::default()).unwrap(),
            expect_ml,
            "render_binary(Ml, Expr) is the canonical ML render of the decoded AST"
        );
        // INCREMENT-1: Type/Pattern render faithfully via the same canonical printer (no panic; idiomatic
        // type/pattern surface is the follow-up increment).
        assert_eq!(
            render_binary(
                &bytes,
                Format::Sexpr,
                FragmentKind::Type,
                Options::default()
            )
            .unwrap(),
            expect_sexpr,
            "increment-1: Type renders faithfully via the canonical printer"
        );
        assert_eq!(
            render_binary(
                &bytes,
                Format::Ml,
                FragmentKind::Pattern,
                Options::default()
            )
            .unwrap(),
            expect_ml,
            "increment-1: Pattern renders faithfully via the canonical printer"
        );
        // The tag→codegen→render kind wire spelling.
        assert_eq!(FragmentKind::parse("expr"), Some(FragmentKind::Expr));
        assert_eq!(FragmentKind::parse("type"), Some(FragmentKind::Type));
        assert_eq!(FragmentKind::parse("pattern"), Some(FragmentKind::Pattern));
        assert_eq!(FragmentKind::parse("pat"), Some(FragmentKind::Pattern));
        assert_eq!(FragmentKind::parse("nope"), None);
    }

    #[test]
    fn type_fragments_render_idiomatically_kind_independent() {
        // DESIGN VERIFICATION (kind=type increment): does a TYPE fragment need a SEPARATE render path, or
        // does the canonical printer already render it idiomatically? The ML type surfaces (arrow `A -> B`,
        // type application `Name(args)`) fall out of the AST SHAPE unconditionally (printer.rs:506/1011 arrow,
        // :4893 tuple), so render_binary(Type) should equal render_binary(Expr) AND be the idiomatic type
        // surface — i.e. kind=type is a no-op over the canonical printer, not new render work.
        let cases = [
            ("(-> Int64 Iter)", "Int64 -> Iter"), // arrow type -> infix (printer.rs:506/1011)
            ("(Tuple Int64 Iter)", "Tuple(Int64, Iter)"), // tuple type -> Name(args) (printer.rs:4893)
            ("(Option Int64)", "Option(Int64)"),          // generic type application
            ("(List Int64)", "List(Int64)"),
            ("(Set Int64)", "Set(Int64)"),
            ("(Map Int64 Bool)", "Map(Int64, Bool)"),
            // Curried arrow (right-assoc) + a nested compound arg — the type surface still falls out of shape.
            ("(-> Int64 (-> Bool Iter))", "Int64 -> Bool -> Iter"),
            (
                "(Tuple (Option Int64) (List Bool))",
                "Tuple(Option(Int64), List(Bool))",
            ),
        ];
        for (sexp, expect) in cases {
            let arenas = crate::sexpr::read(sexp).expect("fragment AST");
            let bytes = crate::codec::encode(&arenas);
            let as_type =
                render_binary(&bytes, Format::Ml, FragmentKind::Type, Options::default()).unwrap();
            let as_expr =
                render_binary(&bytes, Format::Ml, FragmentKind::Expr, Options::default()).unwrap();
            assert_eq!(
                as_type, expect,
                "{sexp} renders as the idiomatic ML type surface"
            );
            assert_eq!(
                as_type, as_expr,
                "the ML type surface is kind-independent (falls out of the AST shape) for {sexp}"
            );
        }
    }

    #[test]
    fn render_binary_renders_a_standalone_value_annotation_doc() {
        // The rust value-doc convergence (op-seq-283/#7295) and cdz-run's `value_codec` both emit a
        // SELF-DESCRIBING `(: value <type>)` doc — a bare top-level colon-annotation, NOT module-wrapped —
        // and the gate harness renders it through THIS fn (`render_binary(bytes,'sexpr','expr')`), the same
        // path cdz-run uses. This pins that render_binary handles that annotation-doc shape and produces the
        // CANONICAL value surface, so a future printer/codec change can't silently break the convergence
        // contract. The load-bearing property: a VALUE tuple renders as the idiomatic `(tuple …)` / `(…, …)`
        // form, NOT the bespoke `#tuple` the old type-driven rust renderer produced (that divergence is
        // exactly what routing through render_binary closes).
        let cases = [
            // (sexpr value-annotation doc, canonical Sexpr surface, canonical ML surface)
            ("(: 42 Int64)", "(: 42 Int64)", "42 : Int64"),
            (
                "(: (tuple 1 2) (Tuple Int64 Int64))",
                "(: (tuple 1 2) (Tuple Int64 Int64))",
                "(1, 2) : Tuple(Int64, Int64)",
            ),
        ];
        for (sexp, expect_sexpr, expect_ml) in cases {
            let arenas = crate::sexpr::read(sexp).expect("value-annotation doc parses");
            let bytes = crate::codec::encode(&arenas);
            assert_eq!(
                render_binary(
                    &bytes,
                    Format::Sexpr,
                    FragmentKind::Expr,
                    Options::default()
                )
                .unwrap(),
                expect_sexpr,
                "{sexp}: render_binary(Sexpr, Expr) is the canonical value-doc s-expr (round-trips)"
            );
            assert_eq!(
                render_binary(&bytes, Format::Ml, FragmentKind::Expr, Options::default()).unwrap(),
                expect_ml,
                "{sexp}: render_binary(Ml, Expr) is the canonical ML value surface"
            );
            // A value doc is kind=expr; the canonical printer is kind-independent (the value surface falls
            // out of the AST shape), so Type/Pattern render identically — pin that so the kind seam stays inert.
            let as_expr = render_binary(
                &bytes,
                Format::Sexpr,
                FragmentKind::Expr,
                Options::default(),
            )
            .unwrap();
            for k in [FragmentKind::Type, FragmentKind::Pattern] {
                assert_eq!(
                    render_binary(&bytes, Format::Sexpr, k, Options::default()).unwrap(),
                    as_expr,
                    "{sexp}: value-doc render is kind-independent (kind seam inert over the canonical printer)"
                );
            }
        }
    }

    #[test]
    fn render_binary_renders_non_finite_floats_inside_compounds() {
        // REGRESSION GUARD for the render HALF of breaker's wasm-boundary bug (2026-09-01): a non-finite
        // float (nan / ±inf) INSIDE a compound value renders SILENTLY WRONG on the wasm boundary (the whole
        // compound collapses to `#list()`). Root cause is the wasm ENCODE (cdz-runtime value_codec declines a
        // non-finite `float_leaf` and has no FloatNan/FloatInf doc leaf) — NOT this render path. This test
        // pins that the render half (codec decode + the canonical printer) is CORRECT + ready: a doc carrying
        // `Leaf::FloatNan` / `Leaf::FloatInf` inside a `Ctor`-headed compound renders the idiomatic value form
        // with the word-forms `nan`/`inf`/`-inf`, byte-for-byte matching the rust renderer (breaker's matrix:
        // `#list(nan)` / `#tuple(nan 7)`). The reader never produces these leaves (source `nan` is a Name), so
        // the doc is built directly via `Builder` — exactly the shape value_codec/`Ast.encode` emits once the
        // encode gap is fixed. Guards a future printer/codec change from silently dropping the word-forms.
        use crate::ast::{Builder, CompoundCtor, IntValue, Leaf, Radix};
        let render = |build: &dyn Fn(&mut Builder) -> crate::ast::StructId| {
            let mut b = Builder::new();
            let root = build(&mut b);
            let a = b.finish(root);
            let bytes = crate::codec::encode(&a);
            let sexpr = render_binary(
                &bytes,
                Format::Sexpr,
                FragmentKind::Expr,
                Options::default(),
            )
            .unwrap();
            let ml =
                render_binary(&bytes, Format::Ml, FragmentKind::Expr, Options::default()).unwrap();
            (sexpr, ml)
        };
        // (list nan) — a NaN inside a list.
        let (sx, ml) = render(&|b| {
            let n = b.atom_leaf(Leaf::FloatNan);
            b.compound(CompoundCtor::List, &[n])
        });
        assert_eq!((sx.as_str(), ml.as_str()), ("#list(nan)", "[nan]"));
        // (tuple nan 7) — a NaN alongside a finite Int in a tuple; the whole tuple survives.
        let (sx, ml) = render(&|b| {
            let n = b.atom_leaf(Leaf::FloatNan);
            let i = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(7),
                radix: Radix::Dec,
            });
            b.compound(CompoundCtor::Tuple, &[n, i])
        });
        assert_eq!((sx.as_str(), ml.as_str()), ("#tuple(nan 7)", "(nan, 7)"));
        // (list -inf inf) — both infinity signs inside a list.
        let (sx, ml) = render(&|b| {
            let ni = b.atom_leaf(Leaf::FloatInf { negative: true });
            let pi = b.atom_leaf(Leaf::FloatInf { negative: false });
            b.compound(CompoundCtor::List, &[ni, pi])
        });
        assert_eq!(
            (sx.as_str(), ml.as_str()),
            ("#list(-inf inf)", "[-inf, inf]")
        );
    }

    #[test]
    fn pattern_fragments_render_idiomatically_kind_independent() {
        // DESIGN VERIFICATION + GATE for kind=Pattern (mirrors `type_fragments_render_idiomatically_kind_independent`):
        // the module doc-comment claims a BACKABLE pattern (a list/record WITH a rest) renders idiomatically via
        // the canonical printer, kind-independent — but the render_binary suite only pinned Pattern on a MODULE
        // doc ("renders faithfully"), never a real pattern FRAGMENT's idiomatic surface. This completes the
        // kind-coverage matrix (expr + type each have a dedicated idiomatic-surface pin; pattern did not). It
        // pins that a list/record-with-rest fragment renders the idiomatic pattern surface (`[h, .. t]` /
        // `{ a = x, .. rest }`) AND that Pattern == Expr (the surface falls out of the AST shape, so the `kind`
        // seam is inert). The rest marker is the canonical wrapped `(.. v)` node. (A bare STANDALONE spread is
        // not backable and is out of scope — this covers the backable list/record-with-rest forms.)
        let cases = [
            // (canonical sexpr fragment, canonical Sexpr surface (round-trips), idiomatic ML pattern surface)
            ("#list(h (.. t))", "#list(h (.. t))", "[h, .. t]"),
            (
                "#list(a b (.. rest))",
                "#list(a b (.. rest))",
                "[a, b, .. rest]",
            ),
            (
                "#record((= a x) (.. rest))",
                "#record((= a x) (.. rest))",
                "{ a = x, .. rest }",
            ),
        ];
        for (sexp, expect_sexpr, expect_ml) in cases {
            let arenas = crate::sexpr::read(sexp).expect("pattern fragment parses");
            let bytes = crate::codec::encode(&arenas);
            let as_pat_sexpr = render_binary(
                &bytes,
                Format::Sexpr,
                FragmentKind::Pattern,
                Options::default(),
            )
            .unwrap();
            let as_pat_ml = render_binary(
                &bytes,
                Format::Ml,
                FragmentKind::Pattern,
                Options::default(),
            )
            .unwrap();
            let as_expr_ml =
                render_binary(&bytes, Format::Ml, FragmentKind::Expr, Options::default()).unwrap();
            assert_eq!(
                as_pat_sexpr, expect_sexpr,
                "{sexp}: render_binary(Sexpr, Pattern) is the canonical pattern s-expr (round-trips)"
            );
            assert_eq!(
                as_pat_ml, expect_ml,
                "{sexp}: render_binary(Ml, Pattern) is the idiomatic ML pattern surface"
            );
            assert_eq!(
                as_pat_ml, as_expr_ml,
                "{sexp}: the ML pattern surface is kind-independent (falls out of the AST shape)"
            );
        }
    }

    #[test]
    fn locate_byte_in_message_maps_a_trailing_byte_offset_to_line_col() {
        // A multi-line s-expr parse error's trailing `at byte N` becomes `at line:col`.
        let src = "(module m\n  (def (main)\n    (+ 1 2)))\n  )))";
        let out = locate_byte_in_message("unexpected ')' at byte 40", src);
        assert_eq!(out, "unexpected ')' at 4:3", "byte 40 is line 4, col 3");
        // A message with no `at byte N` tail is returned unchanged.
        assert_eq!(
            locate_byte_in_message("unterminated list", src),
            "unterminated list"
        );
        // A trailing non-integer after `at byte ` (shouldn't happen, but be robust) is untouched.
        assert_eq!(
            locate_byte_in_message("weird at byte xyz", src),
            "weird at byte xyz"
        );
        // A byte offset NOT at the end — the JSON reader's `invalid literal at byte N (expected …)` —
        // maps the offset and KEEPS the trailing text (byte 6 = line 1, col 7 in `{"a": nul}`).
        let json_src = "{\"a\": nul}";
        assert_eq!(
            locate_byte_in_message("invalid literal at byte 6 (expected `null`)", json_src),
            "invalid literal at 1:7 (expected `null`)"
        );
    }

    #[test]
    fn locate_byte_in_message_edge_cases() {
        let src = "ab\ncde\nf"; // line starts: 0, 3, 7; len 8
        // Offset 0 is line 1, col 1.
        assert_eq!(locate_byte_in_message("oops at byte 0", src), "oops at 1:1");
        // An offset PAST end-of-input is clamped to the last position (no panic, no out-of-bounds) —
        // `line_col` does `byte.min(src.len())`. byte 8 == len → line 3, col 2 (after `f`).
        assert_eq!(
            locate_byte_in_message("truncated at byte 999", src),
            "truncated at 3:2",
            "a past-EOF offset clamps to the end rather than panicking"
        );
        // "last marker wins": an earlier " at byte " inside PROSE is left alone; only the final
        // position marker's digits are rewritten. (byte 3 = line 2, col 1.)
        assert_eq!(
            locate_byte_in_message("expected a byte literal at byte 3", src),
            "expected a byte literal at 2:1",
            "an earlier 'byte' word in prose is not the position marker"
        );
        // The marker with no digits at all (a bare `at byte ` then non-digit) is untouched — the parse
        // of an empty digit run fails, so the message is returned verbatim.
        assert_eq!(
            locate_byte_in_message("dangling at byte ", src),
            "dangling at byte "
        );
    }

    #[test]
    fn a_multi_line_sexpr_parse_error_reports_line_col() {
        // End-to-end through `read`: trailing input on line 4 reports 4:3, not a byte offset.
        let err = read("(module m)\n\n\n  )))".as_bytes(), Format::Sexpr).unwrap_err();
        assert!(
            err.0.contains(" at 4:") && !err.0.contains("byte"),
            "expected a line:col position, no raw byte; got {}",
            err.0
        );
    }

    #[test]
    fn an_ml_parse_error_renders_line_col_not_a_byte_offset() {
        // A `read(.., Ml)` parse error names the position as `line:col` (matching `cdz check`), not a
        // raw byte offset a user/editor can't place. A second-line error reports line 2.
        let err = read("let x = 1\n  @bad".as_bytes(), Format::Ml).unwrap_err();
        assert!(
            err.0.contains("at 2:") && !err.0.contains("byte"),
            "expected a line:col position, no raw byte offset; got {}",
            err.0
        );
    }

    #[test]
    fn format_names() {
        assert_eq!(Format::parse("bin"), Some(Format::Binary));
        assert_eq!(Format::parse("SEXPR"), Some(Format::Sexpr));
        assert_eq!(Format::parse("ml"), Some(Format::Ml));
        assert_eq!(Format::parse("JSON"), Some(Format::Json));
        assert_eq!(Format::parse("toml"), Some(Format::Toml));
        assert_eq!(Format::parse("cedar"), Some(Format::Cedar));
        assert_eq!(Format::parse("nope"), None);
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(Format::from_extension("prog.cdz"), Some(Format::Ml));
        assert_eq!(Format::from_extension("prog.ml"), Some(Format::Ml));
        assert_eq!(Format::from_extension("a/b/c.sexp"), Some(Format::Sexpr));
        assert_eq!(Format::from_extension("prog.sexpr"), Some(Format::Sexpr));
        assert_eq!(Format::from_extension("prog.bin"), Some(Format::Binary));
        assert_eq!(Format::from_extension("data.json"), Some(Format::Json));
        assert_eq!(Format::from_extension("Cargo.toml"), Some(Format::Toml));
        assert_eq!(Format::from_extension("policy.cedar"), Some(Format::Cedar));
        assert_eq!(Format::from_extension("PROG.CDZ"), Some(Format::Ml)); // case-insensitive
        assert_eq!(Format::from_extension("prog"), None); // no extension
        assert_eq!(Format::from_extension("prog.txt"), None); // unknown extension
    }

    // Every `Format` variant, tied to the variant COUNT at compile time (`ALL_FORMATS.len() ==
    // FORMAT_COUNT`, anchored by the exhaustive `format_ordinal` match below). Adding a variant to the
    // enum fails to compile until it is given an ordinal here AND listed in this array — so the
    // round-trip sweep below can never silently skip a new variant. (Same guard shape as token.rs's
    // ALL_KEYWORDS/KEYWORD_COUNT.)
    const ALL_FORMATS: &[Format] = &[
        Format::Binary,
        Format::Sexpr,
        Format::Ml,
        Format::Markdown,
        Format::Json,
        Format::Toml,
        Format::Cedar,
        Format::Debug,
        Format::Flat,
    ];

    const fn format_ordinal(f: Format) -> usize {
        match f {
            Format::Binary => 0,
            Format::Sexpr => 1,
            Format::Ml => 2,
            Format::Markdown => 3,
            Format::Json => 4,
            Format::Toml => 5,
            Format::Cedar => 6,
            Format::Debug => 7,
            Format::Flat => 8,
        }
    }
    const FORMAT_COUNT: usize = format_ordinal(Format::Flat) + 1;
    const _: () = assert!(
        ALL_FORMATS.len() == FORMAT_COUNT,
        "ALL_FORMATS must list every Format variant exactly once"
    );

    #[test]
    fn name_and_parse_round_trip_for_every_format_variant() {
        // The CLI's `--from`/`--to` consistency rests on `parse(name(f)) == Some(f)` for EVERY variant:
        // whatever canonical name a format prints as, re-parsing that name must recover the same format.
        // The point-wise `format_names` test samples a few and OMITS markdown/debug/flat — a new variant
        // (or one whose canonical name lacks a matching `parse` arm) would slip through. Drive from
        // ALL_FORMATS (the whole variant set) so none is skipped.
        for &f in ALL_FORMATS {
            assert_eq!(
                Format::parse(f.name()),
                Some(f),
                "{f:?}: parse(name) must round-trip (name = {:?})",
                f.name()
            );
        }
        // `name()` is INJECTIVE — every variant prints a distinct canonical name (else the round-trip
        // above would be ambiguous and `--to <name>` could not name every format).
        let mut names: Vec<&str> = ALL_FORMATS.iter().map(|f| f.name()).collect();
        names.sort_unstable();
        let distinct = names.len();
        names.dedup();
        assert_eq!(
            names.len(),
            distinct,
            "Format::name must be injective across variants"
        );
    }

    #[test]
    fn from_extension_is_a_left_inverse_and_only_debug_flat_are_output_only() {
        // Extension inference must AGREE with the format's own name: every non-output-only variant has at
        // least one extension mapping back to it, and every extension that resolves lands on the RIGHT
        // variant. The two OUTPUT-ONLY views (Debug, Flat) — per `from_extension`'s doc — have no
        // extension, on purpose (you can't read a debug dump back). This pins the extension table against
        // the variant set so a new readable surface can't be added without an extension (silently making
        // `cdz foo.newext` an "unknown extension" error) and a new output-only one is deliberate.
        let output_only = [Format::Debug, Format::Flat];
        // Known extensions and where each must land — the inverse view of `from_extension`.
        let exts: &[(&str, Format)] = &[
            ("cdz", Format::Ml),
            ("ml", Format::Ml),
            ("sexp", Format::Sexpr),
            ("sexpr", Format::Sexpr),
            ("bin", Format::Binary),
            ("cdzb", Format::Binary),
            ("md", Format::Markdown),
            ("markdown", Format::Markdown),
            ("json", Format::Json),
            ("toml", Format::Toml),
            ("cedar", Format::Cedar),
        ];
        // Each listed extension resolves to exactly its stated variant (via a real path `x.<ext>`).
        for &(ext, want) in exts {
            assert_eq!(
                Format::from_extension(&format!("x.{ext}")),
                Some(want),
                ".{ext} must infer {want:?}"
            );
        }
        // Coverage matches the output-only classification exactly: a variant has an extension IFF it is
        // not output-only.
        for &f in ALL_FORMATS {
            let has_ext = exts.iter().any(|&(_, v)| v == f);
            let is_output_only = output_only.contains(&f);
            assert_eq!(
                has_ext, !is_output_only,
                "{f:?}: has-extension ({has_ext}) must equal not-output-only ({})",
                !is_output_only
            );
        }
    }

    #[test]
    fn json_to_binary_to_json_round_trips() {
        // JSON reads to a value arena, encodes to canonical binary, and re-reads to the same tree.
        let src = "{\"a\": [1, 2, {\"b\": null}], \"c\": true}";
        let bin = convert(src.as_bytes(), Format::Json, Format::Binary).unwrap();
        let back = convert(&bin, Format::Binary, Format::Json).unwrap();
        // Re-encoding the reprinted JSON gives identical canonical bytes (arena-idempotent).
        let again = convert(&back, Format::Json, Format::Binary).unwrap();
        assert_eq!(bin, again);
    }

    #[test]
    fn json_parse_error_reports_line_col() {
        // A malformed JSON document is refused, with the position mapped to line:col.
        let err = convert(b"{\n  \"a\": ,\n}", Format::Json, Format::Sexpr).unwrap_err();
        assert!(
            err.0.contains("JSON parse error") && !err.0.contains("byte"),
            "expected a JSON parse error with line:col, got {}",
            err.0
        );
    }

    #[test]
    fn toml_to_binary_to_toml_is_byte_exact() {
        // A TOML doc reads to a decor-in-arena document, encodes to canonical binary, and prints back
        // BYTE-EXACT (the stronger TOML contract) through the binary form.
        let src = "# cfg\n[server]\nhost = \"127.0.0.1\"\nports = [8000, 8001]\n";
        let bin = convert(src.as_bytes(), Format::Toml, Format::Binary).unwrap();
        let back = convert(&bin, Format::Binary, Format::Toml).unwrap();
        assert_eq!(String::from_utf8(back).unwrap(), src);
    }

    #[test]
    fn toml_to_json_is_total() {
        // `--to json` over a TOML arena is TOTAL (never errors) — but note it does NOT translate the
        // DATA: each surface prints only its own vocabulary, so the JSON printer takes its non-JSON
        // fallback and emits the TOML tree as a JSON string. We assert only that the pipe runs and
        // yields well-formed JSON (a real cross-format data transform would be separate).
        let out = convert(b"a = 1\nb = \"x\"\n", Format::Toml, Format::Json).unwrap();
        let json = String::from_utf8(out).unwrap();
        assert!(
            convert(json.as_bytes(), Format::Json, Format::Binary).is_ok(),
            "got {json}"
        );
    }

    #[test]
    fn toml_parse_error_reports_line_col() {
        let err = convert(b"a = 1\na = 2\n", Format::Toml, Format::Sexpr).unwrap_err();
        assert!(
            err.0.contains("TOML parse error"),
            "expected a TOML parse error, got {}",
            err.0
        );
    }

    #[cfg(feature = "cedar")]
    #[test]
    fn cedar_to_binary_to_cedar_round_trips() {
        // A Cedar policy reads to a `(cedar-policyset …)` arena, encodes to canonical binary, and
        // re-reads to the same tree (arena-idempotent through the binary form).
        let src = "permit (principal in Group::\"admins\", action == Action::\"read\", resource) when { resource.public == true };";
        let bin = convert(src.as_bytes(), Format::Cedar, Format::Binary).unwrap();
        let back = convert(&bin, Format::Binary, Format::Cedar).unwrap();
        let again = convert(&back, Format::Cedar, Format::Binary).unwrap();
        assert_eq!(bin, again);
    }

    #[cfg(feature = "cedar")]
    #[test]
    fn cedar_parse_error_is_surfaced() {
        let err = convert(
            b"allow (principal, action, resource);",
            Format::Cedar,
            Format::Sexpr,
        )
        .unwrap_err();
        assert!(
            err.0.contains("Cedar parse error"),
            "expected a Cedar parse error, got {}",
            err.0
        );
    }

    #[test]
    fn sexpr_to_binary_to_sexpr() {
        let src = "(let ((p (record (x 1) (y 2)))) (. p x))";
        let bin = convert(src.as_bytes(), Format::Sexpr, Format::Binary).unwrap();
        let back = convert(&bin, Format::Binary, Format::Sexpr).unwrap();
        // binary is canonical; re-printing the sexpr is stable
        let again = convert(&back, Format::Sexpr, Format::Binary).unwrap();
        assert_eq!(bin, again);
    }

    // `ml_to_sexpr` (ml `f(a, b)` → `(f a b)`) MIGRATED to the spec/syntax corpus (inc-6): the ML
    // parse-direction it asserted is pinned language-neutrally by `spec/syntax/ml/04-call` (input
    // `f(a, b)` → tree.sexp `(f a b)`), graded by the per-case nix check + the self-consistency test.
    // Deleted here so the behavior lives in ONE neutral place, not a Rust-only assertion.

    #[test]
    fn ml_to_binary_roundtrips_via_sexpr() {
        let bin = convert(b"1 + 2 * 3", Format::Ml, Format::Binary).unwrap();
        let sexpr = convert(&bin, Format::Binary, Format::Sexpr).unwrap();
        assert_eq!(String::from_utf8(sexpr).unwrap(), "(+ 1 (* 2 3))");
    }

    #[test]
    fn list_and_tuple_literals_use_a_native_ctor_head_and_round_trip_both_directions() {
        // A `[…]`/`(a,b)` literal desugars to a native COMPOUND-CTOR head — `#list(…)` / `#tuple(…)`, a
        // `Leaf::Ctor` recognized by kind identity (M2 native-compound-data migration) — NOT a bare
        // `(list …)` name and NOT the legacy STRING primitive `("list" …)`. The distinct leaf kind is
        // unshadowable by construction (it cannot collide with a rebound `list`/`tuple` name or a `#"list"`
        // symbol), so a literal always builds the compound (see parser.rs `ctor_head`). This pins that the
        // native ctor head is the CANONICAL s-expr form AND that the ML↔s-expr round-trip is SOUND both
        // ways — a regression that emitted a bare-name head (re-introducing the shadowing hole) OR that
        // failed to re-read the native head back to `[…]`/`(a,b)` would break this. The LEGACY string-head
        // input still re-reads until the M3 reader drop (dual-read window), pinned below.
        let ml_to_sx = |src: &[u8]| {
            String::from_utf8(convert(src, Format::Ml, Format::Sexpr).unwrap()).unwrap()
        };
        let sx_to_ml = |src: &[u8]| {
            String::from_utf8(convert(src, Format::Sexpr, Format::Ml).unwrap()).unwrap()
        };
        // ML literal → the NATIVE-ctor-head s-expr canonical form.
        assert_eq!(ml_to_sx(b"[1, 2]"), "#list(1 2)");
        assert_eq!(ml_to_sx(b"(1, 2)"), "#tuple(1 2)");
        // The nested list-of-tuple case (v-notebook's chart/table cell) — native heads throughout.
        assert_eq!(
            ml_to_sx(b"[(1, 2), (3, 4)]"),
            "#list(#tuple(1 2) #tuple(3 4))"
        );
        // The native-head s-expr re-reads + prints BACK to the ML literal sugar — round-trip is sound.
        assert_eq!(sx_to_ml(b"#list(1 2)"), "[1, 2]");
        assert_eq!(sx_to_ml(b"#tuple(1 2)"), "(1, 2)");
        assert_eq!(
            sx_to_ml(b"#list(#tuple(1 2) #tuple(3 4))"),
            "[(1, 2), (3, 4)]"
        );
        // The LEGACY string-head surface still re-reads to the same ML sugar (dual-read until M3).
        assert_eq!(sx_to_ml(b"(\"list\" 1 2)"), "[1, 2]");
        assert_eq!(sx_to_ml(b"(\"tuple\" 1 2)"), "(1, 2)");
        // ARENA-IDEMPOTENT through the binary codec: ML → binary → sexpr is stable across a second pass.
        let bin = convert(b"[(1, 2), (3, 4)]", Format::Ml, Format::Binary).unwrap();
        let sx = convert(&bin, Format::Binary, Format::Sexpr).unwrap();
        let bin2 = convert(
            &convert(&bin, Format::Binary, Format::Ml).unwrap(),
            Format::Ml,
            Format::Binary,
        )
        .unwrap();
        assert_eq!(
            sx,
            convert(&bin2, Format::Binary, Format::Sexpr).unwrap(),
            "list-of-tuple is arena-idempotent through the codec (no round-trip corruption)"
        );
    }

    #[test]
    fn an_embedded_syntax_region_survives_the_whole_code_surface_matrix() {
        // The operator's tool-transparency promise for first-class embedded syntaxes: because a
        // `json{ … }` region lands as ORDINARY arena nodes — `(embedded #json <json-subtree>)` — every
        // code-surface converter and the binary codec handle it for FREE, with NO embedded-syntax-aware
        // code anywhere downstream of the parser. Prove it end-to-end: an ML program carrying an embedded
        // JSON region survives ML → binary → sexpr → binary → ML and comes back structurally identical.
        // A codec that dropped the grammar tag, or an ML/sexpr printer that choked on the `(embedded …)`
        // node, would break this — so it pins that the node is fully first-class across the matrix.
        let src = br#"def config() = json{ {"port": 8080, "hosts": ["a", "b"]} }"#;
        let bin = convert(src, Format::Ml, Format::Binary).expect("ml → binary");
        let sexpr = convert(&bin, Format::Binary, Format::Sexpr).expect("binary → sexpr");
        // The grammar tag + the embedded subtree both survive into the s-expr projection.
        let sx = String::from_utf8(sexpr.clone()).unwrap();
        assert!(
            sx.contains("embedded") && sx.contains("#\"json\""),
            "the s-expr projection keeps the embedded node + its grammar tag: {sx}"
        );
        // Round the binary back to ML and re-read: structurally identical to the original parse.
        let ml = convert(&bin, Format::Binary, Format::Ml).expect("binary → ml");
        let bin2 = convert(&ml, Format::Ml, Format::Binary).expect("ml → binary again");
        assert_eq!(
            bin, bin2,
            "an embedded-syntax program is byte-stable across ml ⇄ binary (canonical)"
        );
        // And the sexpr side round-trips too (sexpr → binary → sexpr identical).
        let bin_from_sx = convert(&sexpr, Format::Sexpr, Format::Binary).expect("sexpr → binary");
        assert_eq!(
            bin, bin_from_sx,
            "the embedded node is codec-stable through the sexpr surface too"
        );
    }

    #[test]
    fn ml_width_option_controls_wrapping() {
        let src = b"(outer (inner aaaa bbbb) (inner cccc dddd))";
        // wide: one line
        let wide = convert_with(
            src,
            Format::Sexpr,
            Format::Ml,
            Options {
                width: 100,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(wide).unwrap(),
            "outer(inner(aaaa, bbbb), inner(cccc, dddd))"
        );
        // narrow: breaks
        let narrow = convert_with(
            src,
            Format::Sexpr,
            Format::Ml,
            Options {
                width: 20,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(String::from_utf8(narrow).unwrap().contains('\n'));
    }

    #[test]
    fn structural_sexpr_option_expands_comment_nodes() {
        // The `structural` Options flag (the `cdz convert --to sexpr --structural` surface) renders comment
        // wrappers as ordinary `(comment …)` lists — the parse-tree golden form the `spec/syntax/` corpus
        // uses (DESIGN-parser-test-corpus.md §2) — while the DEFAULT keeps the fmt-idempotent `;` rendering.
        let src = b"; a header\n(def (f) 1)";
        let structural = |input: &[u8]| {
            String::from_utf8(
                convert_with(
                    input,
                    Format::Sexpr,
                    Format::Sexpr,
                    Options {
                        structural: true,
                        ..Options::default()
                    },
                )
                .unwrap(),
            )
            .unwrap()
        };
        let out = structural(src);
        // Structural: an explicit `(comment …)` list, NO `;` trivia.
        assert!(
            out.contains("(comment"),
            "structural emits a comment list: {out:?}"
        );
        assert!(!out.contains(';'), "structural drops `;` trivia: {out:?}");
        // The default (non-structural) still collapses to `;`.
        let default =
            String::from_utf8(convert(src, Format::Sexpr, Format::Sexpr).unwrap()).unwrap();
        assert!(default.contains(';'), "default keeps `;`: {default:?}");
        // Both re-read to the SAME arena (structural output is round-trippable).
        let a = read(src, Format::Sexpr).unwrap();
        assert!(
            a.structurally_eq(&read(out.as_bytes(), Format::Sexpr).unwrap()),
            "structural render re-reads to the identical arena"
        );
    }

    #[test]
    fn sexpr_to_ml() {
        // The full conversion matrix is now closed: sexpr -> ml works.
        let out = convert(b"(+ 1 (* 2 3))", Format::Sexpr, Format::Ml).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "1 + 2 * 3");
    }

    #[test]
    fn binary_to_ml() {
        let bin = convert(b"(let ((x 1)) x)", Format::Sexpr, Format::Binary).unwrap();
        let ml = convert(&bin, Format::Binary, Format::Ml).unwrap();
        assert_eq!(String::from_utf8(ml).unwrap(), "let x = 1 in\nx");
    }

    #[test]
    fn invalid_binary_is_error() {
        assert!(convert(b"garbage", Format::Binary, Format::Sexpr).is_err());
    }

    #[test]
    fn bad_utf8_is_error() {
        assert!(read(&[0xff, 0xfe], Format::Sexpr).is_err());
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible generation without a dependency (mirrors
    /// the unit-test PRNGs in `codec.rs`/`lexer.rs`/`canon.rs`).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Generate a random s-expr program string (bounded by `depth`) — a mix of atoms, infix, calls,
    /// `let`, `if`, and list/tuple literals. Enough shape to exercise every code-surface conversion leg
    /// (the ML printer's operator/precedence handling, the s-expr reader, the binary codec).
    fn gen_prog(rng: &mut Rng, depth: usize) -> String {
        let names = ["a", "b", "x", "y", "f", "+", "g"];
        if depth == 0 || rng.below(3) == 0 {
            return match rng.below(4) {
                0 => names[rng.below(names.len())].to_string(),
                1 => rng.below(50).to_string(),
                2 => "true".to_string(),
                _ => "x".to_string(),
            };
        }
        let sub = |rng: &mut Rng| gen_prog(rng, depth - 1);
        match rng.below(6) {
            0 => format!("(+ {} {})", sub(rng), sub(rng)),
            1 => format!("(f {} {})", sub(rng), sub(rng)),
            2 => format!("(if {} {} {})", sub(rng), sub(rng), sub(rng)),
            3 => format!("(let ((x {}) (y {})) {})", sub(rng), sub(rng), sub(rng)),
            4 => format!("#list({} {})", sub(rng), sub(rng)),
            _ => format!("#tuple({} {})", sub(rng), sub(rng)),
        }
    }

    #[test]
    fn code_surface_conversion_matrix_is_transitively_round_trip_consistent() {
        // The public `convert::convert` matrix — the entry `cdz convert` and cdz-wasm ride — must be
        // TRANSITIVELY round-trip-consistent across all three code surfaces: chaining every conversion
        // leg (sexpr → binary → ml → binary → sexpr) preserves the program. `xtask roundtrip` sweeps
        // binary→ONE surface→binary per corpus record; it never chains the cross-surface ML leg in one
        // pass, so a printer/reader asymmetry that only shows when the ML text is re-read as the SOURCE
        // of the next conversion isn't caught there. Here we drive the whole chain through the in-process
        // `convert` API over random programs and assert the canonical binary is invariant end-to-end.
        let mut rng = Rng(0x0bad_c0de_dead_beef);
        for _ in 0..4000 {
            let depth = 1 + rng.below(4);
            let src = format!("(def (main) {})", gen_prog(&mut rng, depth));
            // The canonical binary of the source (the fixed point every leg must return to).
            let bin0 = convert(src.as_bytes(), Format::Sexpr, Format::Binary)
                .expect("generated s-expr → binary");
            // Chain across surfaces: binary → ml → binary → sexpr → binary.
            let ml = convert(&bin0, Format::Binary, Format::Ml).expect("binary → ml");
            let bin_via_ml = convert(&ml, Format::Ml, Format::Binary).expect("ml → binary");
            assert_eq!(
                bin0,
                bin_via_ml,
                "binary→ml→binary changed the program for {src}\n  ml: {}",
                String::from_utf8_lossy(&ml)
            );
            let sexpr =
                convert(&bin_via_ml, Format::Binary, Format::Sexpr).expect("binary → sexpr");
            let bin_via_sexpr =
                convert(&sexpr, Format::Sexpr, Format::Binary).expect("sexpr → binary");
            assert_eq!(
                bin0, bin_via_sexpr,
                "binary→sexpr→binary changed the program for {src}"
            );
        }
    }

    #[test]
    fn convert_is_total_over_the_whole_from_to_format_matrix() {
        // `convert(input, from, to)` must NEVER PANIC for any (from, to) pair — it either produces bytes
        // or returns a clean `ConvertError`. Individual pairs are tested piecemeal; this pins the WHOLE
        // matrix at once, including cross data↔code pairs (`Cedar → Ml`, `Json → Toml`, `Ml → Cedar`) and
        // every OUTPUT-ONLY target (`Debug`/`Flat`, which `write` handles) — the combinations `cdz convert
        // --from X --to Y` exposes but no single test enumerates. For each READABLE source we take a valid
        // sample; `Debug`/`Flat` are output-only (rejected as a SOURCE), so they are only used as targets.
        // `mut` only used when the `cedar` feature pushes the Cedar row below; a lean build leaves it unpushed.
        #[cfg_attr(not(feature = "cedar"), allow(unused_mut))]
        let mut readable: Vec<(Format, &[u8])> = vec![
            (Format::Binary, b""), // filled below with a real binary sample
            (Format::Sexpr, b"(def (main) (+ 1 2))"),
            (Format::Ml, b"def main() = 1 + 2"),
            (Format::Markdown, b"# Title\n\ntext with `code`\n"),
            (Format::Json, b"{\"a\": [1, 2, null], \"b\": \"x\"}"),
            (Format::Toml, b"a = 1\nb = \"x\"\n"),
        ];
        // Cedar as a readable SOURCE only when the `cedar` surface is compiled — a lean build's `Format::Cedar`
        // read is a clean "not compiled" error (not a valid source), so the read-ok sanity assert below would
        // (correctly) fail; Cedar stays in `all_targets` regardless since the totality contract there accepts
        // the clean-error arm.
        #[cfg(feature = "cedar")]
        readable.push((Format::Cedar, b"permit(principal, action, resource);"));
        // A real binary sample (the codec form of a small program) for the Binary source row.
        let bin_sample = convert(b"(def (main) 42)", Format::Sexpr, Format::Binary).unwrap();
        let all_targets = [
            Format::Binary,
            Format::Sexpr,
            Format::Ml,
            Format::Markdown,
            Format::Json,
            Format::Toml,
            Format::Cedar,
            Format::Debug,
            Format::Flat,
        ];
        for &(from, sample) in &readable {
            let input: &[u8] = if from == Format::Binary {
                &bin_sample
            } else {
                sample
            };
            // The source must itself read cleanly (sanity: the samples are valid).
            assert!(read(input, from).is_ok(), "sample for {from:?} should read");
            for &to in &all_targets {
                // The whole point: this call must RETURN (Ok or Err), never panic. A cross data↔code
                // pair may legitimately Err (e.g. a JSON value has no ML program form), and that's fine —
                // totality, not success, is the contract. `catch_unwind` would mask the panic we're
                // guarding against, so we simply call it: a panic fails the test at the unwind.
                let _ = convert(input, from, to);
            }
        }
    }
}
