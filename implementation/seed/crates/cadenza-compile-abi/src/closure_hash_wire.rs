//! The `KIND_CLOSURE_HASH` wire — the Option-C shared-closure CONTENT-HASH (`u64`) a composed
//! `cdz test <dir>` build emits, carried as canonical BINARY AST (`cadenza_ast::codec`), the SAME wire
//! every compile-boundary artifact speaks (operator P0, seq-284: "Binary AST everywhere" — no bespoke
//! hex-string format). The producer (`rcdzc`'s `Query::ClosureHash` + the `EmitTestsComposed` /
//! `EmitTestsConsumerOnly` emit paths) calls [`encode_closure_hash`]; the consumer (`cdz`'s
//! `precompile_group`, which uses the hash as a content-addressed provider-cache key) calls
//! [`decode_closure_hash`]. ONE shared codec, so neither side hand-rolls a hex parse.
//!
//! Shape: the root is a single `Ast.Int` LEAF carrying the `u64` fold value — the full structured scalar,
//! not a rendered hex string. A consumer that wants the historical `{:016x}` cache-key filename formats the
//! decoded `u64` itself (so on-disk cache files stay byte-stable), but it decodes the WIRE via the codec,
//! doing zero string parsing. TOTAL on decode: a malformed / wrong-shape / out-of-range payload yields
//! `None` (the consumer treats it as "no shared-closure hash" — the decline path), never a crash.

use cadenza_ast::ast::{Builder, IntValue, Leaf, Radix, Struct};

/// Encode the closure content-hash as the `KIND_CLOSURE_HASH` artifact bytes — canonical binary AST
/// (see module docs): a root `Ast.Int` leaf holding the `u64` fold. Round-trips with
/// [`decode_closure_hash`].
pub fn encode_closure_hash(hash: u64) -> Vec<u8> {
    let mut b = Builder::new();
    let root = b.atom_leaf(Leaf::Int {
        value: IntValue::from_u128(u128::from(hash)),
        radix: Radix::Dec,
    });
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_CLOSURE_HASH` bytes back into the `u64` content-hash — the inverse of
/// [`encode_closure_hash`], read via the shared `cadenza_ast::codec`. TOTAL: a non-AST payload, a
/// non-`Int` root, or a value outside `u64` range yields `None`.
pub fn decode_closure_hash(bytes: &[u8]) -> Option<u64> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::Atom(_) = a.get(a.root) else {
        return None;
    };
    let v = a.as_int(a.root)?.to_u128()?;
    u64::try_from(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST closure-hash round-trips exactly (operator P0 seq-284: no bespoke hex text): zero, a
    // mid-range value, and `u64::MAX` (top bit set — the case a signed `from_i64` would have lost) all
    // survive encode→decode. This is the drift guard the rcdzc producer + cdz consumer rely on so the
    // content-addressed cache keys the same on both sides.
    #[test]
    fn closure_hash_binary_ast_round_trips() {
        for h in [0u64, 1, 0xdead_beef, 0x0123_4567_89ab_cdef, u64::MAX] {
            assert_eq!(decode_closure_hash(&encode_closure_hash(h)), Some(h));
        }
        // A non-AST / garbage payload decodes to None (total, graceful-degrade — never panics).
        assert_eq!(decode_closure_hash(b"not a binary-ast tree"), None);
    }
}
