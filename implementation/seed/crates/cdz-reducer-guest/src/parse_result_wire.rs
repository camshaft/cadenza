//! The parser-target RESPONSE wire — `{ast, diagnostics}` as ONE canonical binary-AST payload, the OUTPUT
//! type of the `cdz-platform.<syntax>.parse` contracts. A parser guest is a pure `run` guest: source bytes
//! in (`message.payload`), this envelope out (the `close` reason). SUPERSEDES the drafted
//! `cadenza-syntax/wit/syntax.wit` `parsed` record (brief) — the operator wants a run-callable reducer GUEST,
//! not an import-library world; this is that record carried as the guest's close reason.
//!
//! Distinct from the compile envelope (`cadenza_compile_abi::compile_output_wire`): a PARSE diagnostic is
//! BYTE-SPAN located (`message` + `byte-offset` + `len`) — there is no AST node to point at when a parse
//! fails/recovers — whereas a compile `Diagnostic` is AST-NODE located. So parsers carry their own span model
//! (mirrors `parser::ParseError { span, message }` + `sexpr::ReadError`), not the node-based compile one.
//!
//! Shape: a root `(list [ast-Bytes, diagnostics])`; `diagnostics` is a `(list [diag-form, …])`, each
//! `(list [message-Str, byte-offset-Int, len-Int])`. `ast` empty ⟺ a hard read failure with no tree
//! (sexpr); the ML path is error-recovering so `ast` is always a well-formed tree even with diagnostics.
//! TOTAL on decode (malformed -> empty), mirroring the diagnostics/compile wires.

use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};
use std::sync::Arc;

/// One parse diagnostic: a human message + the byte span in the SOURCE it concerns (`byte_offset`..+`len`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseDiag {
    pub message: String,
    pub byte_offset: u32,
    pub len: u32,
}

/// Encode a parse result (`ast` bytes + parse diagnostics) as the response payload — canonical binary AST.
/// Round-trips with [`decode_parse_result`].
pub fn encode_parse_result(ast: &[u8], diagnostics: &[ParseDiag]) -> Vec<u8> {
    let mut b = Builder::new();
    let ast_leaf = b.atom_leaf(Leaf::Bytes(Arc::from(ast)));
    let diag_forms: Vec<StructId> = diagnostics
        .iter()
        .map(|d| {
            let msg = b.atom_leaf(Leaf::Str(d.message.as_str().into()));
            let off = int_leaf(&mut b, d.byte_offset);
            let len = int_leaf(&mut b, d.len);
            b.list(vec![msg, off, len])
        })
        .collect();
    let diags = b.list(diag_forms);
    let root = b.list(vec![ast_leaf, diags]);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the response payload back into `(ast bytes, diagnostics)` — the inverse of
/// [`encode_parse_result`]. TOTAL: a malformed / wrong-shape payload degrades to `(empty, empty)`.
pub fn decode_parse_result(bytes: &[u8]) -> (Vec<u8>, Vec<ParseDiag>) {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return (Vec::new(), Vec::new());
    };
    let Struct::List(top) = a.get(a.root).clone() else {
        return (Vec::new(), Vec::new());
    };
    let ast = top
        .first()
        .and_then(|&id| as_bytes(&a, id))
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    let diagnostics = match top.get(1).map(|&id| a.get(id).clone()) {
        Some(Struct::List(forms)) => forms.iter().filter_map(|&f| decode_diag(&a, f)).collect(),
        _ => Vec::new(),
    };
    (ast, diagnostics)
}

fn decode_diag(a: &Arenas, form: StructId) -> Option<ParseDiag> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    Some(ParseDiag {
        message: a.as_str(*c.first()?)?.to_string(),
        byte_offset: u32::try_from(a.as_int(*c.get(1)?)?.to_i64()?).ok()?,
        len: u32::try_from(a.as_int(*c.get(2)?)?.to_i64()?).ok()?,
    })
}

fn int_leaf(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

fn as_bytes(a: &Arenas, id: StructId) -> Option<&[u8]> {
    match a.get(id) {
        Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Bytes(b) => Some(b),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_result_round_trips_ast_plus_diagnostics() {
        // A recovered parse: a well-formed AST tree PLUS two byte-span diagnostics round-trips exactly.
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
        // A clean parse (ast, no diagnostics) round-trips; garbage decodes to empty (total, never panics).
        let ast = vec![1, 2, 3];
        let (a, d) = decode_parse_result(&encode_parse_result(&ast, &[]));
        assert_eq!(a, ast);
        assert!(d.is_empty());
        assert_eq!(
            decode_parse_result(b"not a binary-ast tree"),
            (Vec::new(), Vec::new())
        );
    }
}
