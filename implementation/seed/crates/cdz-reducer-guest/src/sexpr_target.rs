//! The `sexpr.parse` reducer target — s-expression source bytes -> canonical AST bytes + diagnostics. Pure
//! `bytes -> bytes`: the body a `pure-reducer-world` guest's `on-message` wraps (the returned envelope is the
//! `close` reason). sexpr reading is STRICT (not error-recovering): a read failure yields NO ast + a single
//! diagnostic carrying the reader's message (the caller checks `diagnostics` before using `ast`).

use crate::parse_result_wire::{encode_parse_result, ParseDiag};

/// Handle a sexpr parse request: the request payload is the raw UTF-8 source bytes. Returns the
/// `{ast, diagnostics}` envelope. Non-UTF-8 source and a hard read error are surfaced as a diagnostic with
/// empty `ast` (never a trap).
pub fn handle(source: &[u8]) -> Vec<u8> {
    let text = match core::str::from_utf8(source) {
        Ok(t) => t,
        Err(_) => return one_error("source is not valid UTF-8"),
    };
    match cadenza_syntax::sexpr::read(text) {
        Ok(arenas) => encode_parse_result(&cadenza_ast::codec::encode(&arenas), &[]),
        Err(e) => one_error(&e.0),
    }
}

/// An empty-ast envelope carrying one whole-source diagnostic (sexpr read errors are not byte-located
/// beyond the reader's own message, which itself names the offending byte, e.g. "trailing input at byte N").
fn one_error(message: &str) -> Vec<u8> {
    encode_parse_result(
        &[],
        &[ParseDiag {
            message: message.to_string(),
            byte_offset: 0,
            len: 0,
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_result_wire::decode_parse_result;

    #[test]
    fn parses_a_valid_sexpr_to_ast() {
        // Happy path: valid s-expr source -> a non-empty canonical AST, no diagnostics. The AST decodes back
        // to the same arenas the reader produced (round-trip through the envelope).
        let src = b"(do (def (main) 42) (export main))";
        let (ast, diags) = decode_parse_result(&handle(src));
        assert!(
            diags.is_empty(),
            "clean parse has no diagnostics: {diags:?}"
        );
        assert!(!ast.is_empty(), "a valid program yields AST bytes");
        // The envelope's ast IS the canonical encoding of the parsed arenas.
        let expected = cadenza_ast::codec::encode(
            &cadenza_syntax::sexpr::read("(do (def (main) 42) (export main))").unwrap(),
        );
        assert_eq!(ast, expected);
    }

    #[test]
    fn a_read_error_yields_a_diagnostic_and_no_ast() {
        // Unbalanced parens: a hard read error -> empty ast + one diagnostic (sexpr is strict, not recovering).
        let (ast, diags) = decode_parse_result(&handle(b"(do (def"));
        assert!(ast.is_empty(), "a failed read yields no ast");
        assert_eq!(diags.len(), 1, "one read-error diagnostic");
        assert!(!diags[0].message.is_empty());
    }

    #[test]
    fn non_utf8_is_a_diagnostic_not_a_trap() {
        let (ast, diags) = decode_parse_result(&handle(&[0xff, 0xfe]));
        assert!(ast.is_empty());
        assert_eq!(diags.len(), 1);
    }
}
