//! The `cdz-platform.{sexpr,ml}.parse` contract's CANONICAL value codec — the parser targets' request/response
//! as strongly-typed Cadenza VALUES (not the old bare-list wire). Operator directive 2026-09-11: "use strong
//! contracts in all of those places" — a build guest emits its response as the canonical encoding of the
//! contract's OUTPUT type, so the Cadenza compile-route handler reads it back with `Value.decode : Option(
//! ParseResult)` (and builds the request with `Value.encode` of a `ParseRequest`). No bare-list wire, no Ast
//! surgery — the contract type IS the wire.
//!
//! The types (see `cdz-platform/contracts/userspace/{sexpr,ml}-parse.cdz`):
//!   ParseRequest = | Parse(String)
//!   ParseDiagnostic   = | ParseDiagnostic(Record(message: String, byteOffset: UInt32, len: UInt32))
//!   ParseResult  = | Parsed(Record(ast: Bytes, diagnostics: List(ParseDiagnostic)))
//!
//! Encoding = the canonical value forms `Value.encode`/`Value.decode` speak, produced via the shared
//! `cadenza-value` toolkit (the same codec the http gateway round-trips through Cadenza). CRITICAL: every one
//! of these types is a SINGLE-CONSTRUCTOR SINGLE-PAYLOAD sum — a NOMINAL NEWTYPE — which rcdzc's value form
//! ERASES: the constructor is ELIDED and the inner value is ASCRIBED with the newtype's type name
//! (`rcdzc/src/lower/value_form.rs` `Ty::Nominal` -> `Named(TypeName, shape_of(inner))`; confirmed against the
//! now-green gateway Header fix, v-gateway-rewrite 2026-09-11). So:
//!   `ParseRequest.Parse(src)`   encodes as  `(: "src" ParseRequest)`      (Parse elided)
//!   `ParseDiagnostic.ParseDiagnostic(rec)` encodes as `(: #record ParseDiagnostic)`      (ParseDiagnostic elided)
//!   `ParseResult.Parsed(rec)`   encodes as  `(: #record ParseResult)`     (Parsed elided)
//! — NOT `(Parse …)`/`(Parsed …)`. Records are `#record((= field value)…)`, fields ascending NAME order;
//! `String`/`Bytes`/`UInt32` leaves; `List` compound. Decoding is TOTAL (malformed -> `None`/empty),
//! ascription-tolerant (the readers peel the `(: … Type)` frame).

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
    // The `Parse` ctor is ELIDED (Nominal newtype erased); the value IS the ascribed source string.
    value::read_str(&arenas, arenas.root)
}

/// Encode a canonical `ParseRequest.Parse(source)` value — the request a caller builds to `run` a parser guest.
/// The `Parse` ctor is elided (Nominal newtype), so it is the source string ascribed `(: "src" ParseRequest)`.
/// Round-trips with [`decode_parse_request`].
#[must_use]
pub fn encode_parse_request(source: &str) -> Vec<u8> {
    let mut b = ValueBuilder::new();
    let s = value::str_leaf(&mut b, source);
    value::finish(b, s, "ParseRequest").to_vec()
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
            // The `ParseDiagnostic` ctor is ELIDED (Nominal newtype) — the ascribed bare record `(: #record ParseDiagnostic)`.
            value::ascribe(&mut b, rec, "ParseDiagnostic")
        })
        .collect();
    let diags = value::list_value(&mut b, diag_values);
    let rec = value::record(&mut b, vec![("ast", ast_leaf), ("diagnostics", diags)]);
    // The `Parsed` ctor is ELIDED (Nominal newtype) — the ascribed bare record `(: #record ParseResult)`.
    value::finish(b, rec, "ParseResult").to_vec()
}

/// Decode a canonical `ParseResult` value back into `(ast bytes, diagnostics)` — the inverse of
/// [`encode_parse_result`]. TOTAL: a malformed / wrong-shape payload degrades to `(empty, empty)`.
#[must_use]
pub fn decode_parse_result(bytes: &[u8]) -> (Vec<u8>, Vec<ParseDiag>) {
    let Some(arenas) = value::decode(bytes) else {
        return (Vec::new(), Vec::new());
    };
    // The `Parsed` ctor is elided (Nominal newtype); the root is the ascribed record itself (readers peel the
    // `(: … ParseResult)` frame). A wrong-shape payload degrades to empty.
    let rec = arenas.root;
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

/// Decode one `ParseDiagnostic` value — the `ParseDiagnostic` ctor is elided (Nominal newtype), so `id` is the ascribed
/// record directly (`record_field` peels the `(: … ParseDiagnostic)` frame).
fn decode_diag(arenas: &value::Arenas, id: value::ValueId) -> Option<ParseDiag> {
    let rec = id;
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
