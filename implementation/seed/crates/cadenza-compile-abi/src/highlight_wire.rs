//! The `KIND_HIGHLIGHT` wire — every user LEAF classified by the syntactic role it plays (the
//! `Highlight` query's answer, for semantic syntax highlighting): a `(node-id, kind)` pair per leaf, in
//! ascending node-id order. Canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every
//! compile-boundary artifact speaks (operator P0 seq-284/307-308: "Binary AST everywhere" — no bespoke
//! TAB/newline text format). The producer (`rcdzc`'s `run_query` `Highlight` arm) calls
//! [`encode_highlight`]; the consumers (`cdz`'s `highlight` CLI + LSP semanticTokens in `main.rs`/
//! `lsp.rs`, and `cdz-wasm`'s browser semantic tokens) call [`decode_highlight`]. ONE shared codec, so
//! neither side hand-rolls a parser — the consumer does ZERO string-splitting.
//!
//! Shape: a root `Ast.List` of per-leaf `(list [Int node-id, Str kind])` forms. The `kind` is the
//! syntactic-role token (`HighlightKind::as_str` — `keyword`/`type`/`constructor`/`function`/`param`/
//! `variable`/…), a stable kebab-case classification enum carried as a `Str` LEAF — NOT a type
//! render-name (highlighting classifies a leaf's ROLE, it carries no `Ty`), the same string-leaf-in-a-
//! codec pattern as `result_types_wire`'s discriminator. The consumer maps the token to its editor
//! semantic-token index (`highlight_kind_to_token_index`) — a match, not a parse. TOTAL on decode: a
//! malformed / wrong-shape entry is skipped; a non-tree byte string yields the empty list.

use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// Encode the `Highlight` classified leaves as the `KIND_HIGHLIGHT` artifact bytes — canonical binary
/// AST (see module docs). Round-trips with [`decode_highlight`].
pub fn encode_highlight(tokens: &[(u32, &str)]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = tokens
        .iter()
        .map(|&(node, kind)| {
            let n = b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(i64::from(node)),
                radix: Radix::Dec,
            });
            let k = b.atom_leaf(Leaf::Str(kind.into()));
            b.list(vec![n, k])
        })
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_HIGHLIGHT` bytes back into the `(node-id, kind)` pairs — the inverse of
/// [`encode_highlight`], read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape
/// entry is skipped; a non-tree byte string yields the empty list.
pub fn decode_highlight(bytes: &[u8]) -> Vec<(u32, String)> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms.iter().filter_map(|&f| decode_one(&a, f)).collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(u32, String)> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    let node = u32::try_from(a.as_int(*c.first()?)?.to_i64()?).ok()?;
    let kind = a.as_str(*c.get(1)?)?.to_string();
    Some((node, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST highlight list round-trips exactly (operator P0 seq-284: no bespoke TAB/newline): a
    // multi-token list carrying several role kinds and the empty list both survive encode→decode. This
    // is the drift guard the rcdzc producer + cdz/cdz-wasm consumers rely on.
    #[test]
    fn highlight_binary_ast_round_trips() {
        let tokens = vec![(3u32, "keyword"), (7, "function"), (12, "param")];
        let want: Vec<(u32, String)> = tokens.iter().map(|&(n, k)| (n, k.to_string())).collect();
        assert_eq!(decode_highlight(&encode_highlight(&tokens)), want);
        assert!(decode_highlight(&encode_highlight(&[])).is_empty());
        assert!(decode_highlight(b"not a binary-ast tree").is_empty());
    }
}
