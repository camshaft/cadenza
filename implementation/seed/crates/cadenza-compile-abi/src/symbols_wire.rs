//! The `KIND_SYMBOLS` wire — the DOCUMENT OUTLINE: every top-level declaration classified by kind (the
//! `Symbols` query's answer), one `(name, kind, name-node-id)` record per declaration. Canonical BINARY
//! AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator P0
//! seq-284/307-308: "Binary AST everywhere" — no bespoke TAB/newline text format). The producer
//! (`rcdzc`'s `run_query` `Symbols` arm) calls [`encode_symbols`]; the consumers (`cdz`'s `symbols`
//! CLI + the LSP documentSymbols / completion / package-symbol-set handlers in `main.rs`/`lsp.rs`) call
//! [`decode_symbols`]. ONE shared codec, so neither side hand-rolls a parser — the consumer does ZERO
//! string-splitting.
//!
//! Shape: a root `Ast.List` of per-declaration `(list [Str name, Str kind, Int name-node-id])` forms,
//! grouped defs → types → effects → modules in declaration order (a deterministic function of the
//! program). The `kind` is the `SymbolKind::as_str` token (`value`/`function`/`type`/`effect`/`module`),
//! a classification enum carried as a `Str` LEAF — NOT a `Ty` render-name (the outline classifies a
//! DECLARATION, it carries no type), the same string-leaf-in-a-codec pattern as `result_types_wire`. The
//! consumer maps the token to its editor symbol kind — a match, not a parse. TOTAL on decode: a
//! malformed / wrong-shape entry is skipped; a non-tree byte string yields the empty list.

use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// Encode the `Symbols` outline declarations as the `KIND_SYMBOLS` artifact bytes — canonical binary AST
/// (see module docs). Round-trips with [`decode_symbols`].
pub fn encode_symbols(entries: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = entries
        .iter()
        .map(|&(name, kind, node)| {
            let n = b.atom_leaf(Leaf::Str(name.into()));
            let k = b.atom_leaf(Leaf::Str(kind.into()));
            let id = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(i64::from(node)),
                radix: Radix::Dec,
            });
            b.list(vec![n, k, id])
        })
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_SYMBOLS` bytes back into the `(name, kind, name-node-id)` records — the inverse of
/// [`encode_symbols`], read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape entry
/// is skipped; a non-tree byte string yields the empty list.
pub fn decode_symbols(bytes: &[u8]) -> Vec<(String, String, u32)> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms.iter().filter_map(|&f| decode_one(&a, f)).collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(String, String, u32)> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    let name = a.as_str(*c.first()?)?.to_string();
    let kind = a.as_str(*c.get(1)?)?.to_string();
    let node = u32::try_from(a.as_int(*c.get(2)?)?.to_i64()?).ok()?;
    Some((name, kind, node))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST symbols outline round-trips exactly (operator P0 seq-284: no bespoke TAB/newline): a
    // multi-declaration outline spanning several kinds and the empty list both survive encode→decode.
    // This is the drift guard the rcdzc producer + cdz consumers rely on.
    #[test]
    fn symbols_binary_ast_round_trips() {
        let entries = vec![
            ("main", "function", 3u32),
            ("pi", "value", 7),
            ("Color", "type", 12),
            ("Ask", "effect", 20),
            ("Geo", "module", 25),
        ];
        let want: Vec<(String, String, u32)> = entries
            .iter()
            .map(|&(n, k, id)| (n.to_string(), k.to_string(), id))
            .collect();
        assert_eq!(decode_symbols(&encode_symbols(&entries)), want);
        assert!(decode_symbols(&encode_symbols(&[])).is_empty());
        assert!(decode_symbols(b"not a binary-ast tree").is_empty());
    }
}
