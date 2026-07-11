//! Format conversion between the three surfaces of one program: the canonical **binary** encoding,
//! the **s-expression** text, and the **ML** text. All three are projections of the same
//! [`Arenas`]; converting is `read into arenas` then `write from arenas`. Pure (no I/O) — the CLI
//! bin does the file/stdin/stdout plumbing.

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
    /// A readable debug view of the arena structure as an indented TREE — OUTPUT ONLY (not a
    /// re-readable surface). Shows the raw shape the compiler sees, for inspecting a binary AST.
    Debug,
    /// A FLAT dump of the two arenas (leaf pool + structure vector + root) — OUTPUT ONLY. Shows the
    /// storage layout directly: leaf interning and the post-order structure order the codec writes.
    Flat,
}

impl Format {
    /// Parse a format name (`binary`/`bin`, `sexpr`/`sexp`, `ml`, `debug`, `flat`). Case-insensitive.
    pub fn parse(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "binary" | "bin" => Some(Format::Binary),
            "sexpr" | "sexp" | "s" => Some(Format::Sexpr),
            "ml" => Some(Format::Ml),
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
            Format::Debug => "debug",
            Format::Flat => "flat",
        }
    }

    /// Infer the surface format from a file path's extension: `.cdz`/`.ml` → ML, `.sexp`/`.sexpr` →
    /// s-expr, `.bin`/`.cdzb` → binary. The output-only `debug`/`flat` views have no extension.
    /// `None` if the path has no recognized extension (the caller then requires an explicit format).
    pub fn from_extension(path: &str) -> Option<Format> {
        let ext = std::path::Path::new(path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase();
        match ext.as_str() {
            "cdz" | "ml" => Some(Format::Ml),
            "sexp" | "sexpr" => Some(Format::Sexpr),
            "bin" | "cdzb" => Some(Format::Binary),
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
            sexpr::read(text).map_err(|e| ConvertError(format!("s-expr parse error: {}", e.0)))
        }
        Format::Ml => {
            let text = utf8(input)?;
            let parsed = parser::read_ml(text);
            if let Some(err) = parsed.errors.first() {
                return Err(ConvertError(format!(
                    "ML parse error at byte {}: {}",
                    err.span.start, err.message
                )));
            }
            Ok(parsed.arenas)
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

/// Output options. Currently just the ML target line width.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Target line width for the ML pretty-printer.
    pub width: usize,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            width: crate::printer::DEFAULT_WIDTH,
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
        Format::Sexpr => Ok(sexpr::print(arenas).into_bytes()),
        Format::Ml => Ok(crate::printer::print(arenas, opts.width).into_bytes()),
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
    fn format_names() {
        assert_eq!(Format::parse("bin"), Some(Format::Binary));
        assert_eq!(Format::parse("SEXPR"), Some(Format::Sexpr));
        assert_eq!(Format::parse("ml"), Some(Format::Ml));
        assert_eq!(Format::parse("nope"), None);
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(Format::from_extension("prog.cdz"), Some(Format::Ml));
        assert_eq!(Format::from_extension("prog.ml"), Some(Format::Ml));
        assert_eq!(Format::from_extension("a/b/c.sexp"), Some(Format::Sexpr));
        assert_eq!(Format::from_extension("prog.sexpr"), Some(Format::Sexpr));
        assert_eq!(Format::from_extension("prog.bin"), Some(Format::Binary));
        assert_eq!(Format::from_extension("PROG.CDZ"), Some(Format::Ml)); // case-insensitive
        assert_eq!(Format::from_extension("prog"), None); // no extension
        assert_eq!(Format::from_extension("prog.txt"), None); // unknown extension
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
        let wide = convert_with(src, Format::Sexpr, Format::Ml, Options { width: 100 }).unwrap();
        assert_eq!(
            String::from_utf8(wide).unwrap(),
            "outer(inner(aaaa, bbbb), inner(cccc, dddd))"
        );
        // narrow: breaks
        let narrow = convert_with(src, Format::Sexpr, Format::Ml, Options { width: 20 }).unwrap();
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
        assert_eq!(String::from_utf8(ml).unwrap(), "let x = 1;\nx");
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
