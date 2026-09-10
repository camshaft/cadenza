//! The `ml.parse` reducer target — ML surface source bytes -> canonical AST bytes + diagnostics. Pure
//! `bytes -> bytes`: the body a `pure-reducer-world` guest's `on-message` wraps (the returned envelope is the
//! `close` reason). The ML reader is ERROR-RECOVERING (`parser::read_ml` -> `Parsed { arenas, errors }`): the
//! arenas are ALWAYS a well-formed tree (recovery substitutes `Name` placeholders), so `ast` is non-empty
//! even when `diagnostics` is non-empty — a caller gets a best-effort AST AND the problems, each byte-located.

use crate::parse_result_wire::{encode_parse_result, ParseDiag};

/// Handle an ml parse request: the request payload is the raw UTF-8 source bytes. Returns the
/// `{ast, diagnostics}` envelope — a best-effort AST plus every recovered parse error (byte-span located).
/// Non-UTF-8 source is surfaced as a diagnostic with empty `ast` (never a trap).
pub fn handle(source: &[u8]) -> Vec<u8> {
    let text = match core::str::from_utf8(source) {
        Ok(t) => t,
        Err(_) => {
            return encode_parse_result(
                &[],
                &[ParseDiag {
                    message: "source is not valid UTF-8".to_string(),
                    byte_offset: 0,
                    len: 0,
                }],
            );
        }
    };
    let parsed = cadenza_syntax::parser::read_ml(text);
    let ast = cadenza_ast::codec::encode(&parsed.arenas);
    let diagnostics: Vec<ParseDiag> = parsed
        .errors
        .iter()
        .map(|e| ParseDiag {
            message: e.message.clone(),
            byte_offset: e.span.start as u32,
            len: (e.span.end.saturating_sub(e.span.start)) as u32,
        })
        .collect();
    encode_parse_result(&ast, &diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_result_wire::decode_parse_result;

    #[test]
    fn parses_valid_ml_to_ast() {
        // Happy path: valid ML source -> a well-formed AST, no diagnostics.
        let (ast, diags) = decode_parse_result(&handle(b"def main() -> Int64 = 42"));
        assert!(
            diags.is_empty(),
            "clean parse has no diagnostics: {diags:?}"
        );
        assert!(!ast.is_empty(), "a valid program yields AST bytes");
    }

    #[test]
    fn error_recovery_yields_ast_plus_byte_located_diagnostics() {
        // ML is error-recovering: even a malformed program yields a well-formed AST (placeholders) AND
        // byte-located diagnostics — the arenas are always valid (parser::Parsed invariant).
        let (ast, diags) = decode_parse_result(&handle(b"def main() -> Int64 = "));
        assert!(
            !ast.is_empty(),
            "error recovery still yields a well-formed AST"
        );
        assert!(
            !diags.is_empty(),
            "a malformed program surfaces recovered diagnostics"
        );
    }

    #[test]
    fn non_utf8_is_a_diagnostic_not_a_trap() {
        let (ast, diags) = decode_parse_result(&handle(&[0xff, 0xfe]));
        assert!(ast.is_empty());
        assert_eq!(diags.len(), 1);
    }
}
