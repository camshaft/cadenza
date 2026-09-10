//! The `sexpr.parse` reducer target — a canonical `ParseRequest` value -> a canonical `ParseResult` value.
//! Pure `bytes -> bytes`: the body a `pure-reducer-world` guest's `on-message` wraps (the returned envelope is
//! the `close` reason). The request payload is a `ParseRequest.Parse(source)` value (decoded to the source
//! string); sexpr reading is STRICT (not error-recovering), so a read failure yields NO ast + a single
//! diagnostic carrying the reader's message. Strong contracts (operator 2026-09-11): request + response are
//! canonical typed values, not a raw byte blob — the Cadenza compile-route handler composes them via
//! `Value.encode`/`Value.decode`.

use crate::parse_contract::{decode_parse_request, encode_parse_result, ParseDiag};

/// Handle a sexpr parse request: decode the `ParseRequest` value to its source string, parse it, and return
/// the `ParseResult` value. A malformed request or a hard read error is surfaced as a diagnostic with empty
/// `ast` (never a trap).
pub fn handle(request: &[u8]) -> Vec<u8> {
    let Some(source) = decode_parse_request(request) else {
        return one_error("payload is not a valid ParseRequest value");
    };
    match cadenza_syntax::sexpr::read(&source) {
        Ok(arenas) => encode_parse_result(&cadenza_ast::codec::encode(&arenas), &[]),
        Err(e) => one_error(&e.0),
    }
}

/// An empty-ast `ParseResult` carrying one whole-source diagnostic (sexpr read errors are not byte-located
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
    use crate::parse_contract::{decode_parse_result, encode_parse_request};

    #[test]
    fn parses_a_valid_sexpr_to_ast() {
        // Happy path: a ParseRequest carrying valid s-expr source -> a non-empty canonical AST, no diagnostics.
        let request = encode_parse_request("(do (def (main) 42) (export main))");
        let (ast, diags) = decode_parse_result(&handle(&request));
        assert!(
            diags.is_empty(),
            "clean parse has no diagnostics: {diags:?}"
        );
        assert!(!ast.is_empty(), "a valid program yields AST bytes");
        // The response's ast IS the canonical encoding of the parsed arenas.
        let expected = cadenza_ast::codec::encode(
            &cadenza_syntax::sexpr::read("(do (def (main) 42) (export main))").unwrap(),
        );
        assert_eq!(ast, expected);
    }

    #[test]
    fn a_read_error_yields_a_diagnostic_and_no_ast() {
        // Unbalanced parens: a hard read error -> empty ast + one diagnostic (sexpr is strict, not recovering).
        let (ast, diags) = decode_parse_result(&handle(&encode_parse_request("(do (def")));
        assert!(ast.is_empty(), "a failed read yields no ast");
        assert_eq!(diags.len(), 1, "one read-error diagnostic");
        assert!(!diags[0].message.is_empty());
    }

    #[test]
    fn a_malformed_request_is_a_diagnostic_not_a_trap() {
        let (ast, diags) = decode_parse_result(&handle(b"not a ParseRequest value"));
        assert!(ast.is_empty());
        assert_eq!(diags.len(), 1);
    }
}
