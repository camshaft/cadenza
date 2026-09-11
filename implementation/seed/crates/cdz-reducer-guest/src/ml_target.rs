//! The `ml.parse` reducer target — a canonical `ParseRequest` value -> a canonical `ParseResult` value. Pure
//! `bytes -> bytes`: the body a `pure-reducer-world` guest's `on-message` wraps (the returned envelope is the
//! `close` reason). The request payload is a `ParseRequest.Parse(source)` value. The ML reader is
//! ERROR-RECOVERING (`parser::read_ml` -> `Parsed { arenas, errors }`): the arenas are ALWAYS a well-formed
//! tree (recovery substitutes `Name` placeholders), so `ast` is non-empty even when `diagnostics` is non-empty
//! — a caller gets a best-effort AST AND the problems, each byte-located. Strong contracts (operator
//! 2026-09-11): request + response are canonical typed values the Cadenza handler composes via
//! `Value.encode`/`Value.decode`.

use crate::parse_contract::{decode_parse_request, encode_parse_result, ParseDiag};

/// Handle an ml parse request: decode the `ParseRequest` value to its source string, parse it (error-
/// recovering), and return the `ParseResult` value — a best-effort AST plus every recovered parse error
/// (byte-span located). A malformed request is surfaced as a diagnostic with empty `ast` (never a trap).
pub fn handle(request: &[u8]) -> Vec<u8> {
    let Some(source) = decode_parse_request(request) else {
        return encode_parse_result(
            &[],
            &[ParseDiag {
                message: "payload is not a valid ParseRequest value".to_string(),
                byte_offset: 0,
                len: 0,
            }],
        );
    };
    let parsed = cadenza_syntax::parser::read_ml(&source);
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
    use crate::parse_contract::{decode_parse_result, encode_parse_request};

    #[test]
    fn parses_valid_ml_to_ast() {
        // Happy path: a ParseRequest carrying valid ML source -> a well-formed AST, no diagnostics.
        let (ast, diags) =
            decode_parse_result(&handle(&encode_parse_request("def main() -> Int64 = 42")));
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
        let (ast, diags) =
            decode_parse_result(&handle(&encode_parse_request("def main() -> Int64 = ")));
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
    fn a_malformed_request_is_a_diagnostic_not_a_trap() {
        let (ast, diags) = decode_parse_result(&handle(b"not a ParseRequest value"));
        assert!(ast.is_empty());
        assert_eq!(diags.len(), 1);
    }
}
