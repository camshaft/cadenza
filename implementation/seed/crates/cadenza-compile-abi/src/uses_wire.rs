//! The `KIND_USES` wire — the node ids of every occurrence that USES a name (the `UsesOf` query's
//! answer), as canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary
//! artifact speaks (operator P0 seq-284/307-308: "Binary AST everywhere" — no bespoke TAB/newline
//! text format). The producer (`rcdzc`'s `run_query` `UsesOf` arm) calls [`encode_uses`]; the
//! consumers (`cdz`'s `uses`/references/incoming-calls handlers in `main.rs`/`lsp.rs`, which map each
//! id to a source span) call [`decode_uses`]. ONE shared codec, so neither side hand-rolls a parser —
//! the consumer does ZERO string-splitting.
//!
//! Shape: a root `Ast.List` of `Ast.Int` node-id leaves, in ascending id order (the deterministic
//! order the columns model requires — a query answer is a function of the program, not of traversal
//! order). TOTAL on decode: a malformed / wrong-shape entry is skipped (never a crash), and a
//! non-tree byte string decodes to the empty list.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct};

/// Encode the `UsesOf` node ids as the `KIND_USES` artifact bytes — canonical binary AST (see module
/// docs). Round-trips with [`decode_uses`].
pub fn encode_uses(ids: &[u32]) -> Vec<u8> {
    let mut b = Builder::new();
    let leaves: Vec<_> = ids
        .iter()
        .map(|&id| {
            b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(i64::from(id)),
                radix: Radix::Dec,
            })
        })
        .collect();
    let root = b.list(leaves);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_USES` bytes back into the reference node ids — the inverse of [`encode_uses`],
/// read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape entry is skipped, and a
/// non-tree byte string yields the empty list.
pub fn decode_uses(bytes: &[u8]) -> Vec<u32> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(ids) = a.get(a.root).clone() else {
        return Vec::new();
    };
    ids.iter()
        .filter_map(|&id| u32::try_from(a.as_int(id)?.to_i64()?).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST uses list round-trips exactly (operator P0 seq-284: no bespoke TAB/newline): a
    // multi-reference list, a single reference, and the empty list all survive encode→decode. This is
    // the drift guard the rcdzc producer + cdz consumers rely on.
    #[test]
    fn uses_binary_ast_round_trips() {
        let ids = vec![3u32, 17, 42, 100];
        assert_eq!(decode_uses(&encode_uses(&ids)), ids);
        assert_eq!(decode_uses(&encode_uses(&[7])), vec![7]);
        assert!(decode_uses(&encode_uses(&[])).is_empty());
        assert!(decode_uses(b"not a binary-ast tree").is_empty());
    }
}
