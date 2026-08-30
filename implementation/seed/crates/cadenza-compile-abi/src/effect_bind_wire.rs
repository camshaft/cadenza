//! The `effect-bind` INPUT-artifact wire — the compile-request effect→peer-interface REBIND/UNBIND map
//! (`link::KIND_EFFECT_BIND`) that OVERRIDES a program's in-source `(bind …)` defaults (U3). Carried as
//! canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks
//! (operator P0, seq-284: "Binary AST everywhere" — no bespoke `Effect=iface` newline text). A driver
//! builds it with [`encode`]; the compiler (`rcdzc::compile`) reads it with [`decode`] and applies each
//! entry — rebinding an effect to a peer interface, or UNBINDING it (empty interface) so it escapes to the
//! host — doing zero string splitting.
//!
//! Shape: a root `Ast.List` of per-entry `(list [Str effect, Str interface])` forms — the same
//! string-leaf-in-a-codec-map pattern as `link_map` / `result_types_wire`. An entry whose `interface` is
//! the EMPTY string is an UNBIND (the old `Effect=` line); a non-empty interface is a REBIND (the compiler
//! validates it as a component-interface name). TOTAL on decode: a malformed / wrong-shape payload yields
//! `None` (the caller declines), never a panic.

use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};

/// Encode the effect→interface rebind/unbind map as the `effect-bind` artifact bytes — canonical binary AST
/// (see module docs). An empty `interface` string encodes an UNBIND. Round-trips with [`decode`].
pub fn encode(entries: &[(String, String)]) -> Vec<u8> {
    let mut b = Builder::new();
    let forms: Vec<StructId> = entries
        .iter()
        .map(|(effect, iface)| {
            let e = b.atom_leaf(Leaf::Str(effect.as_str().into()));
            let i = b.atom_leaf(Leaf::Str(iface.as_str().into()));
            b.list(vec![e, i])
        })
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `effect-bind` bytes back into the effect→interface pairs — the inverse of [`encode`], read via
/// the shared `cadenza_ast::codec`. An empty `interface` is an UNBIND. TOTAL: a non-AST / wrong-shape payload
/// yields `None`.
pub fn decode(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(forms) = a.get(a.root).clone() else {
        return None;
    };
    forms.iter().map(|&f| decode_one(&a, f)).collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<(String, String)> {
    let Struct::List(c) = a.get(form) else {
        return None;
    };
    if c.len() != 2 {
        return None;
    }
    let effect = a.as_str(c[0])?.to_string();
    let iface = a.as_str(c[1])?.to_string();
    Some((effect, iface))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The binary-AST effect-bind map round-trips exactly (operator P0 seq-284: no bespoke `Effect=iface`
    // text): a REBIND (non-empty interface) and an UNBIND (empty interface) both survive encode→decode, plus
    // the empty map and a garbage payload (→ None).
    #[test]
    fn effect_bind_binary_ast_round_trips() {
        let entries = vec![
            ("Math".to_string(), "cadenza:mathv2/api".to_string()),
            ("Log".to_string(), String::new()), // an UNBIND (empty interface)
        ];
        assert_eq!(decode(&encode(&entries)), Some(entries));
        assert_eq!(decode(&encode(&[])), Some(Vec::new()));
        assert_eq!(decode(b"not a binary-ast tree"), None);
    }
}
