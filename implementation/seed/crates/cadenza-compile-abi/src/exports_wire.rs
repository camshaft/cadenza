//! The `KIND_EXPORTS` RESULT wire — the module's exported interface as canonical BINARY AST
//! (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator seq-254/284/307:
//! "Binary AST is THE data exchange format. No exceptions." + "I want the full type ast!"). The producer
//! (`rcdzc::sidecar::run_query`'s `Query::Exports`) calls [`encode_exports`]; the consumers (`cdz exports`,
//! `cdz clean`'s output-stem read, the LSP completion detail) call [`decode_exports`]. ONE shared codec,
//! so neither side hand-rolls a parser (this replaces the bespoke `name\ttype\tdef-node` TAB text wire).
//!
//! Each export carries the FULL structured resolved-type sub-AST (`encode_ty_payload`), NOT a
//! `Ty::render_name` string — the consumer renders a display NAME from the decoded structure (via the
//! shared cadenza-syntax type-name renderer), so the boundary stays structural and no render-name string
//! crosses it.
//!
//! Shape: a root `(exports <export>…)` list, one `(export <Str name> <ty-opt> <node-opt>)` form per
//! export, in export (declaration) order.
//! - `<ty-opt>` is the AST-native Option idiom (as `diagnostics_wire` uses): `(list [])` = the export's
//!   type did not resolve (rendered as "unknown"), `(list [ty-payload])` = the resolved type sub-AST
//!   grafted directly (a `(-> …)`/`(Sum …)`/scalar payload).
//! - `<node-opt>` = `(list [])` when the export names no definition, `(list [Int node-id])` = the def's
//!   NAME occurrence (the sig's first child) a consumer maps to `file:line:col` for go-to.
//!
//! TOTAL on decode: a malformed / wrong-shape form is skipped, never a crash.

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// One exported item — its name, its resolved type (as a standalone arena rooted at the `encode_ty_payload`
/// sub-AST, or `None` when the type did not resolve), and its def NAME-occurrence node id (or `None` when
/// the export names no definition).
#[derive(Clone, Debug)]
pub struct ExportEntry {
    pub name: String,
    pub ty: Option<Arenas>,
    pub node: Option<u32>,
}

/// Encode the module's export interface as the `KIND_EXPORTS` artifact bytes — ONE canonical binary AST
/// value (see module docs). Each entry's `ty` arena (when present) is rooted at that export's type payload
/// sub-AST; its root subtree is grafted verbatim. Order is preserved. Round-trips with [`decode_exports`].
pub fn encode_exports(entries: &[ExportEntry]) -> Vec<u8> {
    let mut b = Builder::new();
    let mut forms: Vec<StructId> = Vec::with_capacity(entries.len());
    for e in entries {
        let head = b.name("export");
        let name_node = b.atom_leaf(Leaf::Str(e.name.as_str().into()));
        let ty_opt = match &e.ty {
            None => b.list(vec![]),
            Some(ty_arena) => {
                let payload = copy_from(&mut b, ty_arena, ty_arena.root);
                b.list(vec![payload])
            }
        };
        let node_opt = match e.node {
            None => b.list(vec![]),
            Some(n) => {
                let int = b.atom_leaf(Leaf::Int {
                    value: IntValue::from_i64(i64::from(n)),
                    radix: Radix::Dec,
                });
                b.list(vec![int])
            }
        };
        forms.push(b.list(vec![head, name_node, ty_opt, node_opt]));
    }
    let ex_head = b.name("exports");
    let mut children = Vec::with_capacity(forms.len() + 1);
    children.push(ex_head);
    children.extend(forms);
    let root = b.list(children);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_EXPORTS` bytes back into the export interface — the inverse of [`encode_exports`], read
/// via the shared `cadenza_ast::codec`. Each `ty` (when present) is a fresh standalone arena rooted at that
/// export's type payload subtree (so a consumer renders it directly). TOTAL: a malformed / wrong-shape form
/// is skipped rather than failing the whole decode.
pub fn decode_exports(bytes: &[u8]) -> Vec<ExportEntry> {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return Vec::new();
    };
    let Some(forms) = a.as_form(a.root, "exports") else {
        return Vec::new();
    };
    forms
        .to_vec()
        .iter()
        .filter_map(|&f| decode_one(&a, f))
        .collect()
}

fn decode_one(a: &Arenas, form: StructId) -> Option<ExportEntry> {
    let tail = a.as_form(form, "export")?;
    let name = a.as_str(*tail.first()?)?.to_string();
    // ty-opt: an empty list = None, a one-element list = the grafted payload subtree.
    let ty = opt_child(a, *tail.get(1)?).map(|payload| {
        let mut b = Builder::new();
        let root = copy_from(&mut b, a, payload);
        b.finish(root)
    });
    // node-opt: an empty list = None, a one-element list = the Int node id.
    let node = opt_child(a, *tail.get(2)?)
        .and_then(|n| a.as_int(n))
        .and_then(|iv| iv.to_i64())
        .and_then(|v| u32::try_from(v).ok());
    Some(ExportEntry { name, ty, node })
}

/// Read the AST-native Option `(list [])` = None / `(list [v])` = Some — returning the single child of a
/// one-element list, else `None`. A wrong shape (a non-list, or a list with ≠1 child) reads as None.
fn opt_child(a: &Arenas, id: StructId) -> Option<StructId> {
    match a.get(id) {
        Struct::List(kids) if kids.len() == 1 => Some(kids[0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A standalone `(Int 64)` type payload arena, as the sidecar extracts for a resolved scalar type.
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

    #[test]
    fn exports_binary_ast_round_trips() {
        let entries = vec![
            ExportEntry {
                name: "answer".into(),
                ty: Some(int64_ty()),
                node: Some(7),
            },
            // An export whose type did not resolve AND names no def — both optionals None.
            ExportEntry {
                name: "mystery".into(),
                ty: None,
                node: None,
            },
        ];
        let decoded = decode_exports(&encode_exports(&entries));
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].name, "answer");
        assert_eq!(decoded[0].node, Some(7));
        let t = decoded[0].ty.as_ref().expect("resolved type");
        assert!(t.structurally_eq(&int64_ty()));
        assert_eq!(decoded[1].name, "mystery");
        assert!(decoded[1].ty.is_none());
        assert_eq!(decoded[1].node, None);
    }

    #[test]
    fn exports_empty_round_trips() {
        assert!(decode_exports(&encode_exports(&[])).is_empty());
    }

    #[test]
    fn exports_total_on_garbage() {
        assert!(decode_exports(b"not a binary-ast tree").is_empty());
        assert!(decode_exports(&[]).is_empty());
    }

    #[test]
    fn exports_skips_a_malformed_form() {
        // A root (exports …) with a wrong-headed form in the middle keeps only the well-formed exports.
        let mut b = Builder::new();
        let ex_head = b.name("exports");
        let good_head = b.name("export");
        let good_name = b.atom_leaf(Leaf::Str("f".into()));
        let ty_none = b.list(vec![]);
        let node_none = b.list(vec![]);
        let good = b.list(vec![good_head, good_name, ty_none, node_none]);
        let bad_head = b.name("nonsense");
        let bad = b.list(vec![bad_head]);
        let root = b.list(vec![ex_head, good, bad]);
        let bytes = cadenza_ast::codec::encode(&b.finish(root));
        let decoded = decode_exports(&bytes);
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].name, "f");
    }
}
