//! The `KIND_RESOLVE` wire — the defining occurrence a reference resolves to (the `ResolveOf` query's
//! answer): a SINGLE node id, or none when the queried node is not a navigable reference. Canonical
//! BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator P0
//! seq-284/307-308: "Binary AST everywhere" — no bespoke text format). The producer (`rcdzc`'s
//! `run_query` `ResolveOf` arm) calls [`encode_resolve`]; the consumers (`cdz`'s go-to-definition in
//! `main.rs` + `lsp.rs`, and the shadowing guards) call [`decode_resolve`]. ONE shared codec, so neither
//! side hand-rolls a parser — the consumer does ZERO string-splitting.
//!
//! Shape: a root `Ast.List` of zero or one `Ast.Int` node-id leaves — a one-element list carries the
//! resolved target, the empty list carries "not a navigable reference" (the total, defined "no answer").
//! TOTAL on decode: a malformed / wrong-shape tree yields `None`.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct};

/// Encode the `ResolveOf` target as the `KIND_RESOLVE` artifact bytes — canonical binary AST (see module
/// docs). `Some(id)` → a one-element list; `None` → the empty list. Round-trips with [`decode_resolve`].
pub fn encode_resolve(target: Option<u32>) -> Vec<u8> {
    let mut b = Builder::new();
    let leaves: Vec<_> = target
        .into_iter()
        .map(|id| {
            b.atom_leaf(Leaf::Int {
                value: IntValue::from_i64(i64::from(id)),
                radix: Radix::Dec,
            })
        })
        .collect();
    let root = b.list(leaves);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_RESOLVE` bytes back into the optional target node id — the inverse of
/// [`encode_resolve`], read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape tree
/// yields `None`.
pub fn decode_resolve(bytes: &[u8]) -> Option<u32> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(ids) = a.get(a.root).clone() else {
        return None;
    };
    u32::try_from(a.as_int(*ids.first()?)?.to_i64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST resolve answer round-trips exactly (operator P0 seq-284: no bespoke text): a
    // resolved target and the "not a navigable reference" empty answer both survive encode→decode. This
    // is the drift guard the rcdzc producer + cdz consumers rely on.
    #[test]
    fn resolve_binary_ast_round_trips() {
        assert_eq!(decode_resolve(&encode_resolve(Some(42))), Some(42));
        assert_eq!(decode_resolve(&encode_resolve(Some(0))), Some(0));
        assert_eq!(decode_resolve(&encode_resolve(None)), None);
        assert_eq!(decode_resolve(b"not a binary-ast tree"), None);
    }
}
