//! The `KIND_RESULT_TYPES` map wire — each boundary export's COMPILED result type-name, as canonical
//! BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator P0,
//! seq-284: "Binary AST everywhere" — no bespoke TAB type-string format). The producer (`rcdzc`'s
//! `compile`, emitting both the standalone artifact + the `cdz-result-type` component custom section)
//! calls [`encode_result_types`]; the consumer (`cdz-run`, which byte-scans the section to disambiguate a
//! WIT-erased leaf at render — a `list<u8>` as `Bytes` vs `List UInt8`, a `string` as a `Symbol`) calls
//! [`decode_result_types`]. ONE shared codec, so neither side hand-rolls a parser.
//!
//! Shape: a root `Ast.List` of per-export `(list [Str name, Str render-name])` forms — the same
//! string-leaf-in-a-codec-map pattern as `link_map` (path/base/count) and `diagnostics_wire`. The
//! render-name is the compiler's `Ty::render_name` discriminator (`Bytes`, `Symbol`, `(-> …)`, …) the
//! runtime matches on; carrying it as a `Str` LEAF in the canonical AST retires the bespoke tab text
//! WITHOUT the runtime needing to reconstruct a structured type. TOTAL on decode: a malformed / wrong-shape
//! entry is skipped (the runtime falls back to the WIT-erased default render), never a crash.

use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};

/// Encode the export→result-type-name map as the `KIND_RESULT_TYPES` artifact / `cdz-result-type` section
/// bytes — canonical binary AST (see module docs). Round-trips with [`decode_result_types`].
pub fn encode_result_types(entries: &[(String, String)]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = entries
        .iter()
        .map(|(name, render_name)| {
            let n = b.atom_leaf(Leaf::Str(name.as_str().into()));
            let t = b.atom_leaf(Leaf::Str(render_name.as_str().into()));
            b.list(vec![n, t])
        })
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_RESULT_TYPES` bytes back into the export→result-type-name pairs — the inverse of
/// [`encode_result_types`], read via the shared `cadenza_ast::codec`. TOTAL: a malformed / wrong-shape
/// entry is skipped.
pub fn decode_result_types(bytes: &[u8]) -> Vec<(String, String)> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Struct::List(forms) = a.get(a.root).clone() else {
        return Vec::new();
    };
    forms.iter().filter_map(|&f| decode_one(&a, f)).collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(String, String)> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    let name = a.as_str(*c.first()?)?.to_string();
    let render_name = a.as_str(*c.get(1)?)?.to_string();
    Some((name, render_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST result-types map round-trips exactly (operator P0 seq-284: no bespoke tab): a Bytes
    // discriminator, a Symbol (string), a closure render-name (with spaces/arrows), and the empty map all
    // survive encode→decode. This is the drift guard the rcdzc producer + cdz-run consumer rely on.
    #[test]
    fn result_types_binary_ast_round_trips() {
        let entries = vec![
            ("g".to_string(), "Bytes".to_string()),
            ("greet".to_string(), "Symbol".to_string()),
            ("make-adder".to_string(), "(-> Int64 Int64)".to_string()),
        ];
        assert_eq!(decode_result_types(&encode_result_types(&entries)), entries);
        assert!(decode_result_types(&encode_result_types(&[])).is_empty());
        assert!(decode_result_types(b"not a binary-ast tree").is_empty());
    }
}
