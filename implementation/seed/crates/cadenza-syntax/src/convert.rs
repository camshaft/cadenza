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
}

impl Format {
    /// Parse a format name (`binary`/`bin`, `sexpr`/`sexp`, `ml`). Case-insensitive.
    pub fn parse(name: &str) -> Option<Format> {
        match name.to_ascii_lowercase().as_str() {
            "binary" | "bin" => Some(Format::Binary),
            "sexpr" | "sexp" | "s" => Some(Format::Sexpr),
            "ml" => Some(Format::Ml),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Binary => "binary",
            Format::Sexpr => "sexpr",
            Format::Ml => "ml",
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
    }
}

/// Write `arenas` in `to` format to bytes.
pub fn write(arenas: &Arenas, to: Format) -> Result<Vec<u8>, ConvertError> {
    match to {
        Format::Binary => Ok(codec::encode(arenas)),
        Format::Sexpr => Ok(sexpr::print(arenas).into_bytes()),
        Format::Ml => {
            // The ML printer is not yet implemented; until it lands, `--to ml` is unavailable.
            Err(ConvertError(
                "ML output is not implemented yet (the ML printer is pending)".into(),
            ))
        }
    }
}

/// Convert `input` from `from` to `to` in one step.
pub fn convert(input: &[u8], from: Format, to: Format) -> Result<Vec<u8>, ConvertError> {
    let arenas = read(input, from)?;
    write(&arenas, to)
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
        // ML input works even though ML OUTPUT is pending.
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
    fn ml_output_is_pending_not_a_panic() {
        let err = convert(b"42", Format::Sexpr, Format::Ml).unwrap_err();
        assert!(err.0.contains("not implemented"));
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
