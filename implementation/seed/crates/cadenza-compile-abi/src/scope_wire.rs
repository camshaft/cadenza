//! The `KIND_SCOPE` RESULT wire — the bindings visible at a node (the "what's in scope here" query) as
//! canonical BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks
//! (operator seq-254/284/307: "Binary AST is THE data exchange format. No exceptions." + "I want the full
//! type ast!"). The producer (`rcdzc::sidecar::run_query`'s `Query::ScopeAt`) calls [`encode_scope`]; the
//! consumer (`cdz scope`) calls [`decode_scope`]. ONE shared codec, so neither side hand-rolls a parser
//! (this replaces the bespoke `name\ttype\tbinder-node` TAB text wire).
//!
//! Each binding carries the FULL structured type sub-AST (`encode_ty_payload` of the binder's
//! `infer::type_of`), NOT a `Ty::render_name` string — the consumer renders a display NAME from the decoded
//! structure (via the shared cadenza-syntax type-name renderer), so the boundary stays structural.
//!
//! Shape: a root `(scope <binding>…)` list, one `(binding <Str name> <ty-payload> <Int binder-node>)` form
//! per visible binding, INNERMOST-first (the order the scope walk yields — a deterministic function of the
//! program). `<ty-payload>` is always present (`type_of` is total — an untypeable binder still yields a
//! defined type payload); `<binder-node>` is the binder's node id, mapped by the consumer to `file:line:col`.
//! TOTAL on decode: a malformed / wrong-shape form is skipped, never a crash.

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, StructId};

/// One in-scope binding — its name, its type (a standalone arena rooted at the `encode_ty_payload`
/// sub-AST), and its binder node id (mapped to a source span by the consumer).
#[derive(Clone, Debug)]
pub struct ScopeBinding {
    pub name: String,
    pub ty: Arenas,
    pub node: u32,
}

/// Encode the in-scope bindings as the `KIND_SCOPE` artifact bytes — ONE canonical binary AST value (see
/// module docs). Each binding's `ty` arena is rooted at that binder's type payload sub-AST; its root
/// subtree is grafted verbatim. Order (innermost-first) is preserved. Round-trips with [`decode_scope`].
pub fn encode_scope(bindings: &[ScopeBinding]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms: Vec<StructId> = Vec::with_capacity(bindings.len());
    for bind in bindings {
        let head = b.name("binding");
        let name_node = b.atom_leaf(Leaf::Str(bind.name.as_str().into()));
        let payload = copy_from(&mut b, &bind.ty, bind.ty.root);
        let node = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(i64::from(bind.node)),
            radix: Radix::Dec,
        });
        forms.push(b.list(vec![head, name_node, payload, node]));
    }
    let head = b.name("scope");
    let mut children = Vec::with_capacity(forms.len() + 1);
    children.push(head);
    children.extend(forms);
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_SCOPE` bytes back into the in-scope bindings — the inverse of [`encode_scope`], read
/// via the shared `cadenza_ast::codec`. Each `ty` is a fresh standalone arena rooted at that binder's type
/// payload subtree (so a consumer renders it directly). TOTAL: a malformed / wrong-shape form is skipped.
pub fn decode_scope(bytes: &[u8]) -> Vec<ScopeBinding> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Some(forms) = a.as_form(a.root, "scope") else {
        return Vec::new();
    };
    forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, f))
        .collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<ScopeBinding> {
    let tail = a.as_form(form, "binding")?;
    let name = a.as_str(*tail.first()?)?.to_string();
    let payload = *tail.get(1)?;
    let node = u32::try_from(a.as_int(*tail.get(2)?)?.to_i64()?).ok()?;
    let mut b = Builder::new();
    let root = copy_from(&mut b, a, payload);
    Some(ScopeBinding {
        name,
        ty: b.finish(root),
        node,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standalone `(Int 64)` type payload arena.
    fn int64_ty() -> Arenas {
        let mut b = Builder::new();
        let head = b.name("Int");
        let w = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(64),
            radix: Radix::Dec,
        });
        let root = b.list(vec![head, w]);
        b.finish(root)
    }

    /// A standalone `(-> a b)` type payload arena.
    fn arrow_ty() -> Arenas {
        let mut b = Builder::new();
        let head = b.name("->");
        let a = b.name("a");
        let bb = b.name("b");
        let root = b.list(vec![head, a, bb]);
        b.finish(root)
    }

    #[test]
    fn scope_binary_ast_round_trips() {
        let bindings = vec![
            ScopeBinding {
                name: "x".into(),
                ty: int64_ty(),
                node: 12,
            },
            ScopeBinding {
                name: "f".into(),
                ty: arrow_ty(),
                node: 3,
            },
        ];
        let decoded = decode_scope(&encode_scope(&bindings));
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].name, "x");
        assert_eq!(decoded[0].node, 12);
        assert!(decoded[0].ty.structurally_eq(&int64_ty()));
        assert_eq!(decoded[1].name, "f");
        assert_eq!(decoded[1].node, 3);
        assert!(decoded[1].ty.structurally_eq(&arrow_ty()));
    }

    #[test]
    fn scope_empty_round_trips() {
        assert!(decode_scope(&encode_scope(&[])).is_empty());
    }

    #[test]
    fn scope_total_on_garbage() {
        assert!(decode_scope(b"not a binary-ast tree").is_empty());
        assert!(decode_scope(&[]).is_empty());
    }

    #[test]
    fn scope_skips_a_malformed_form() {
        let mut b = Builder::new();
        let head = b.name("scope");
        let bh = b.name("binding");
        let nm = b.atom_leaf(Leaf::Str("x".into()));
        let ih = b.name("Int");
        let iw = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(64),
            radix: Radix::Dec,
        });
        let ty = b.list(vec![ih, iw]);
        let node = b.atom_leaf(Leaf::Int {
            value: IntValue::from_i64(1),
            radix: Radix::Dec,
        });
        let good = b.list(vec![bh, nm, ty, node]);
        let nonsense = b.name("nonsense");
        let bad = b.list(vec![nonsense]);
        let root = b.list(vec![head, good, bad]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let decoded = decode_scope(&bytes);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].name, "x");
    }
}
