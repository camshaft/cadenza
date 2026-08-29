//! SUFFIXED-LITERAL NORMALIZATION — rewrite a `Leaf::Suffixed { value, kind }` atom (a type-suffixed
//! numeric literal `100N` / `0.5R`) into the annotation form a suffix denotes: `(: <body> BigInt|Rational)`.
//!
//! The reader DESUGARS a suffixed atom to `(: <leaf> BigInt|Rational)` (a suffix IS a terse annotation), so
//! rcdzc's compiler is designed to NEVER see a bare `Leaf::Suffixed` — every downstream pass types the
//! literal through the ordinary annotation path (`0N` → `(: 0 BigInt)` → BigInt-typed). The ast-consolidation
//! (#5158) made rcdzc consume `cadenza-ast`'s shared codec, whose `decode` PRESERVES the `Leaf::Suffixed`
//! kind (rcdzc's former in-crate codec decoded `KIND_SUFFIXED` straight to `Int`/`Float`+annotation, so a
//! `Suffixed` leaf never reached resolve). This pass restores that invariant: a single in-place rewrite at
//! load — BEFORE resolve — turning any surviving `Suffixed` leaf into the `(: <body> <Type>)` annotation, so
//! its type is fixed by the annotation exactly as a reader-desugared suffix is. Modelled on
//! [`crate::tagged_template::expand`] (scan originals, overwrite each node's structure entry in place so its
//! `StructId`/span is preserved, blank the duplicate appended root). `resolve.rs`'s `Suffixed → Poison` arm
//! stays as a belt-and-suspenders guard for a truly-stray `Suffixed` that escaped normalization.

use crate::ast::{Arenas, Leaf, Struct, StructId, SuffixBody, SuffixKind};
use crate::prelude::{push_atom, push_list};

/// Rewrite every `Leaf::Suffixed` atom into the `(: <body> <Type>)` annotation a suffix denotes.
pub fn normalize(ast: &mut Arenas) {
    // FAST BAIL: no `Suffixed` leaf anywhere → nothing to rewrite (the overwhelming common case, since the
    // reader desugars suffixes and only a non-desugaring codec source ever carries the leaf). A cheap
    // O(leaves) prescan, mirroring the sibling load-time desugar passes' fast-bails.
    if !ast
        .leaves
        .iter()
        .any(|l| matches!(l, Leaf::Suffixed { .. }))
    {
        return;
    }
    // Only ORIGINAL nodes can be a source `Suffixed` atom; the rewrite APPENDS, so bound the scan.
    let original_len = ast.structure.len() as u32;
    let mut plans: Vec<(StructId, StructId)> = Vec::new();
    for i in 0..original_len {
        let id = StructId(i);
        if let Some(replacement) = rewrite_of(ast, id) {
            plans.push((id, replacement));
        }
    }
    for (node, replacement) in plans {
        // Overwrite the `Suffixed` atom with a COPY of the annotation List, so the node's own `StructId`
        // (and span) is preserved. Blank the now-duplicate appended root (leaving it would out-rank the copy
        // as the shared children's parent — the orphan hazard `tagged_template::expand` also guards).
        let entry = ast.get(replacement).clone();
        ast.structure[node.0 as usize] = entry;
        ast.structure[replacement.0 as usize] = Struct::List(Vec::new());
    }
}

/// If `node` is an `Atom(Leaf::Suffixed { value, kind })`, build and return the annotation `(: <body>
/// <Type>)` (a fresh appended node): `<body>` is the bare `Int`/`Float` leaf the suffix decorated, `<Type>`
/// is `BigInt` (suffix `N`) or `Rational` (suffix `R`). Else `None`.
fn rewrite_of(ast: &mut Arenas, node: StructId) -> Option<StructId> {
    let leaf = match ast.get(node) {
        Struct::Atom(l) => ast.leaf(*l).clone(),
        _ => return None,
    };
    let Leaf::Suffixed { value, kind } = leaf else {
        return None;
    };
    // The body literal: exactly the bare `Int`/`Float` leaf the suffix decorated (radix preserved).
    let body_leaf = match value {
        SuffixBody::Int { value, radix } => Leaf::Int { value, radix },
        SuffixBody::Float(d) => Leaf::Float(d),
    };
    let body = push_atom(ast, body_leaf);
    // `N` → `BigInt`, `R` → `Rational`.
    let type_name = match kind {
        SuffixKind::BigInt => "BigInt",
        SuffixKind::Rational => "Rational",
    };
    let colon = push_atom(ast, Leaf::Name(":".into()));
    let ty = push_atom(ast, Leaf::Name(type_name.into()));
    Some(push_list(ast, vec![colon, body, ty]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Builder, Decimal, IntValue, Radix};

    /// A `Suffixed` leaf normalizes to the `(: <body> <Type>)` annotation a suffix denotes: `N`→BigInt over
    /// an integer body, `R`→Rational over a float body — exactly what the reader desugars a suffix to.
    #[test]
    fn suffixed_normalizes_to_the_colon_annotation() {
        for (leaf, want_body_is_int, want_type) in [
            (
                Leaf::Suffixed {
                    value: SuffixBody::Int {
                        value: IntValue::from_i64(100),
                        radix: Radix::Dec,
                    },
                    kind: SuffixKind::BigInt,
                },
                true,
                "BigInt",
            ),
            (
                Leaf::Suffixed {
                    value: SuffixBody::Float(Decimal::from_f64(0.5).unwrap()),
                    kind: SuffixKind::Rational,
                },
                false,
                "Rational",
            ),
        ] {
            let mut b = Builder::new();
            let atom = b.atom_leaf(leaf);
            let root = b.list(vec![atom]);
            let mut a = b.finish(root);
            normalize(&mut a);

            // The `Suffixed` atom is now a `(: <body> <Type>)` triple, StructId preserved.
            let kids = match a.get(atom) {
                Struct::List(kids) => kids.clone(),
                _ => panic!("Suffixed atom should have become a `(:` List"),
            };
            assert_eq!(kids.len(), 3, "annotation is a triple");
            assert_eq!(
                a.as_name(kids[0]),
                Some(":"),
                "head is the `:` annotation marker"
            );
            assert_eq!(
                want_type == "BigInt",
                a.as_int(kids[1]).is_some(),
                "int body iff BigInt"
            );
            assert_eq!(want_body_is_int, a.as_int(kids[1]).is_some());
            assert!(want_body_is_int == a.as_float(kids[1]).is_none());
            assert_eq!(a.as_name(kids[2]), Some(want_type), "annotated type name");
        }
    }

    /// No `Suffixed` leaf → the fast-bail leaves the arena untouched.
    #[test]
    fn no_suffixed_leaf_is_a_noop() {
        let mut b = Builder::new();
        let n = b.name("x");
        let root = b.list(vec![n]);
        let mut a = b.finish(root);
        let before = a.structure.len();
        normalize(&mut a);
        assert_eq!(
            a.structure.len(),
            before,
            "no Suffixed → no rewrite, no appended nodes"
        );
    }
}
