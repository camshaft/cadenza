//! The `cdz-platform.{sexpr,ml}.parse` contract's CANONICAL value codec — the parser targets' request/response
//! as strongly-typed Cadenza VALUES (not the old bare-list wire). Operator directive 2026-09-11: "use strong
//! contracts in all of those places" — a build guest emits its response as the canonical encoding of the
//! contract's OUTPUT type, so the Cadenza compile-route handler reads it back with `Value.decode : Option(
//! ParseResult)` (and builds the request with `Value.encode` of a `ParseRequest`). No bare-list wire, no Ast
//! surgery — the contract type IS the wire.
//!
//! The types (see `cdz-platform/contracts/userspace/{sexpr,ml}-parse.cdz`):
//!   ParseRequest = | Parse(String)
//!   Diagnostic   = | Diagnostic(Record(message: String, byteOffset: UInt32, len: UInt32))
//!   ParseResult  = | Parsed(Record(ast: Bytes, diagnostics: List(Diagnostic)))
//!
//! Encoding = the canonical value forms `Value.encode`/`Value.decode` speak, produced via the shared
//! `cadenza-value` toolkit (the same codec the http gateway round-trips through Cadenza): a constructor
//! application `(Parsed <record>)`; a record `#record((= field value)…)` with fields in ascending NAME order;
//! a `List` compound; `String`/`Bytes`/`UInt32` leaves; the whole value wrapped in a root ascription
//! `(: value ParseResult)`. Decoding is TOTAL (malformed -> `None`/empty), ascription-tolerant.

use cadenza_value::{self as value, ValueBuilder};

/// One parse diagnostic: a human message + the byte span in the SOURCE it concerns (`byte_offset`..+`len`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseDiag {
    pub message: String,
    pub byte_offset: u32,
    pub len: u32,
}

/// Decode a canonical `ParseRequest` value into its source string — `Parse(String)`. `None` if the payload is
/// not a well-formed `Parse(<Str>)` value (malformed / wrong ctor / wrong payload shape).
#[must_use]
pub fn decode_parse_request(bytes: &[u8]) -> Option<String> {
    let arenas = value::decode(bytes)?;
    let root = arenas.root;
    if value::read_ctor(&arenas, root)? != "Parse" {
        return None;
    }
    let payload = value::ctor_payload(&arenas, root)?;
    value::read_str(&arenas, *payload.first()?)
}

/// Encode a canonical `ParseRequest.Parse(source)` value — the request a caller builds to `run` a parser guest.
/// Round-trips with [`decode_parse_request`].
#[must_use]
pub fn encode_parse_request(source: &str) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let s = value::str_leaf(&mut b, source);
    let parse = value::bare_ctor(&mut b, "Parse", vec![s]);
    value::finish(b, parse, "ParseRequest").to_vec()
}

/// Encode a canonical `ParseResult.Parsed(Record(ast, diagnostics))` value — the parser guest's response.
/// Round-trips with [`decode_parse_result`], and reads back on the Cadenza side as `Value.decode : Option(
/// ParseResult)`.
#[must_use]
pub fn encode_parse_result(ast: &[u8], diagnostics: &[ParseDiag]) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let ast_leaf = value::bytes_leaf(&mut b, ast);
    let diag_values: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            let msg = value::str_leaf(&mut b, &d.message);
            let off = value::uint_leaf(&mut b, u64::from(d.byte_offset));
            let len = value::uint_leaf(&mut b, u64::from(d.len));
            let rec = value::record(
                &mut b,
                vec![("message", msg), ("byteOffset", off), ("len", len)],
            );
            value::bare_ctor(&mut b, "Diagnostic", vec![rec])
        })
        .collect();
    let diags = value::list_value(&mut b, diag_values);
    let rec = value::record(&mut b, vec![("ast", ast_leaf), ("diagnostics", diags)]);
    let parsed = value::bare_ctor(&mut b, "Parsed", vec![rec]);
    value::finish(b, parsed, "ParseResult").to_vec()
}

/// Decode a canonical `ParseResult` value back into `(ast bytes, diagnostics)` — the inverse of
/// [`encode_parse_result`]. TOTAL: a malformed / wrong-shape payload degrades to `(empty, empty)`.
#[must_use]
pub fn decode_parse_result(bytes: &[u8]) -> (Vec<u8>, Vec<ParseDiag>) {
    let Some(arenas) = value::decode(bytes) else {
        return (Vec::new(), Vec::new());
    };
    let root = arenas.root;
    // Expect `Parsed(<record>)`; anything else degrades to empty.
    if value::read_ctor(&arenas, root) != Some("Parsed") {
        return (Vec::new(), Vec::new());
    }
    let Some(payload) = value::ctor_payload(&arenas, root) else {
        return (Vec::new(), Vec::new());
    };
    let Some(&rec) = payload.first() else {
        return (Vec::new(), Vec::new());
    };
    let ast = value::record_field(&arenas, rec, "ast")
        .and_then(|f| value::read_bytes(&arenas, f))
        .map(|b| b.to_vec())
        .unwrap_or_default();
    let diagnostics = value::record_field(&arenas, rec, "diagnostics")
        .and_then(|f| value::read_list(&arenas, f))
        .map(|elems| {
            elems
                .iter()
                .filter_map(|&e| decode_diag(&arenas, e))
                .collect()
        })
        .unwrap_or_default();
    (ast, diagnostics)
}

/// Decode one `Diagnostic(Record(message, byteOffset, len))` value.
fn decode_diag(arenas: &value::Arenas, id: value::ValueId) -> Option<ParseDiag> {
    if value::read_ctor(arenas, id)? != "Diagnostic" {
        return None;
    }
    let rec = *value::ctor_payload(arenas, id)?.first()?;
    Some(ParseDiag {
        message: value::read_str(arenas, value::record_field(arenas, rec, "message")?)?,
        byte_offset: u32::try_from(value::read_uint(
            arenas,
            value::record_field(arenas, rec, "byteOffset")?,
        )?)
        .ok()?,
        len: u32::try_from(value::read_uint(
            arenas,
            value::record_field(arenas, rec, "len")?,
        )?)
        .ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_request_round_trips() {
        let src = "(do (def (main) 42) (export main))";
        assert_eq!(
            decode_parse_request(&encode_parse_request(src)).as_deref(),
            Some(src)
        );
        // A wrong-ctor / garbage payload decodes to None (total).
        assert_eq!(decode_parse_request(b"not a value"), None);
    }

    #[test]
    fn parse_result_round_trips_ast_plus_diagnostics() {
        // A recovered parse: a well-formed AST tree PLUS two byte-span diagnostics round-trips exactly through
        // the canonical value form.
        let ast = vec![0x00, 0xde, 0xad, 0xff, 0x01];
        let diags = vec![
            ParseDiag {
                message: "expected `)`".into(),
                byte_offset: 12,
                len: 1,
            },
            ParseDiag {
                message: "trailing input at byte 40".into(),
                byte_offset: 40,
                len: 0,
            },
        ];
        let (ast2, diags2) = decode_parse_result(&encode_parse_result(&ast, &diags));
        assert_eq!(ast2, ast);
        assert_eq!(diags2, diags);
    }

    #[test]
    fn clean_and_garbage_degrade() {
        let ast = vec![1, 2, 3];
        let (a, d) = decode_parse_result(&encode_parse_result(&ast, &[]));
        assert_eq!(a, ast);
        assert!(d.is_empty());
        assert_eq!(
            decode_parse_result(b"not a value form"),
            (Vec::new(), Vec::new())
        );
    }
}
