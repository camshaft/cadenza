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
        Format::Cedar => {
            // Cedar can fail; its error is a multi-line `ParseErrors` with a source excerpt (not an
            // `at byte N`), so the reader already reduced it to a headline — surface it as-is.
            let text = utf8(input)?;
            crate::cedar::read(text)
                .map_err(|e| ConvertError(format!("Cedar parse error: {}", e.0)))
        }
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
}

impl Default for Options {
    fn default() -> Options {
        Options {
            width: crate::printer::DEFAULT_WIDTH,
            display: false,
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
        Format::Json => Ok(crate::json::print(arenas, opts.width).into_bytes()),
        // A `(toml-document …)` arena prints back BYTE-EXACT (unmutated); a NON-TOML root becomes a
        // single `program = "<ml>"` key (see `toml_surface::print`), so `--to toml` stays total.
        Format::Toml => Ok(crate::toml_surface::print(arenas, opts.width).into_bytes()),
        // A `(cedar-policyset …)` arena prints back to Cedar policy text (rebuilding a pst); a NON-Cedar
        // root becomes a `//`-comment block over its ML rendering (see `cedar::print`), so `--to cedar`
        // stays total.
        Format::Cedar => Ok(crate::cedar::print(arenas, opts.width).into_bytes()),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn ml_to_sexpr() {
        let out = convert(b"f(a, b)", Format::Ml, Format::Sexpr).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "(f a b)");
    }

    #[test]
    fn ml_to_binary_roundtrips_via_sexpr() {
        let bin = convert(b"1 + 2 * 3", Format::Ml, Format::Binary).unwrap();
        let sexpr = convert(&bin, Format::Binary, Format::Sexpr).unwrap();
        assert_eq!(String::from_utf8(sexpr).unwrap(), "(+ 1 (* 2 3))");
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
}
