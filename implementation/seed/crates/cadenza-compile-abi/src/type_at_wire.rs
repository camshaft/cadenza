//! The `KIND_TYPE_AT` RESULT wire — the `cdz type-at FILE OFFSET` / editor-hover answer as canonical
//! BINARY AST (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator
//! seq-254/284/307: "Binary AST is THE data exchange format. No exceptions." + "I want the full type
//! ast!"). The producer (`rcdzc::sidecar::run_query`'s `Query::TypeAt`) calls [`encode_type_at`]; the
//! consumer (`cdz type-at`, the LSP hover) calls [`decode_type_at`]. ONE shared codec, so neither side
//! hand-rolls a parser (this replaces the bespoke free-text hover wire).
//!
//! A hover is a PRESENTATION answer with render-distinct cases; the wire carries the STRUCTURED components
//! (a type as its FULL `encode_ty_payload` sub-AST, never a `render_name` string), and the consumer renders
//! the display text from the decoded structure (a def → `name : <type>`, a bare type → `<type>`, a keyword
//! → `keyword <kw>`, else `unknown`):
//! - `(type-at (def <Str name> <ty-opt>))` — the node identifies a DEFINITION; `<ty-opt>` is the def's
//!   signature payload (`(list [])` = unsolved → the consumer shows `name : unknown`, `(list [payload])` =
//!   the structured type). The consumer renders `name : <type>`.
//! - `(type-at (keyword <Str kw>))` — a grammar keyword atom (`if`/`let`/`def`/…): syntax, no type. The
//!   consumer renders `keyword <kw>`.
//! - `(type-at (ty <ty-payload>))` — a bare typed node; the consumer renders `<type>` from the payload.
//! - `(type-at (unknown))` — an untypeable / non-user / poison node; the consumer renders `unknown`.
//!
//! TOTAL on decode: a malformed / unknown-tag value decodes to [`TypeAt::Unknown`] — a defined benign
//! hover, never a crash (safe for editor hover on incomplete programs).

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};

/// The hover verdict for a node — the render-distinct cases of the `TypeAt` query.
#[derive(Clone, Debug)]
pub enum TypeAt {
    /// The node identifies a definition — its name + signature type payload (`None` = unsolved).
    Def { name: String, ty: Option<Arenas> },
    /// A grammar keyword atom (`if`/`let`/`def`/…) — syntax, no type.
    Keyword(String),
    /// A bare typed node — its solved type as the `encode_ty_payload` sub-AST.
    Ty(Arenas),
    /// An untypeable / non-user / poison node — no defined type.
    Unknown,
}

/// Encode a hover verdict as the `KIND_TYPE_AT` artifact bytes — ONE canonical binary AST value (see module
/// docs). Round-trips with [`decode_type_at`].
pub fn encode_type_at(info: &TypeAt) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("type-at");
    let case = match info {
        TypeAt::Def { name, ty } => {
            let tag = b.name("def");
            let nm = b.atom_leaf(Leaf::Str(name.as_str().into()));
            let ty_opt = match ty {
                None => b.list(vec![]),
                Some(a) => {
                    let payload = copy_from(&mut b, a, a.root);
                    b.list(vec![payload])
                }
            };
            b.list(vec![tag, nm, ty_opt])
        }
        TypeAt::Keyword(kw) => {
            let tag = b.name("keyword");
            let k = b.atom_leaf(Leaf::Str(kw.as_str().into()));
            b.list(vec![tag, k])
        }
        TypeAt::Ty(a) => {
            let tag = b.name("ty");
            let payload = copy_from(&mut b, a, a.root);
            b.list(vec![tag, payload])
        }
        TypeAt::Unknown => {
            let tag = b.name("unknown");
            b.list(vec![tag])
        }
    };
    let root = b.list(vec![head, case]);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_TYPE_AT` bytes back into the hover verdict — the inverse of [`encode_type_at`], read
/// via the shared `cadenza_ast::codec`. A `Def`/`Ty` payload is a fresh standalone arena rooted at the
/// type payload subtree (the consumer renders it directly). TOTAL: a malformed / unknown-tag value decodes
/// to [`TypeAt::Unknown`], never a crash.
pub fn decode_type_at(bytes: &[u8]) -> TypeAt {
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return TypeAt::Unknown;
    };
    let Some(tail) = a.as_form(a.root, "type-at") else {
        return TypeAt::Unknown;
    };
    let Some(&case) = tail.first() else {
        return TypeAt::Unknown;
    };
    match a.head_name(case) {
        Some("def") => {
            let Some(inner) = a.as_form(case, "def") else {
                return TypeAt::Unknown;
            };
            let Some(name) = inner.first().and_then(|&s| a.as_str(s)) else {
                return TypeAt::Unknown;
            };
            let ty = inner
                .get(1)
                .and_then(|&opt| opt_child(&a, opt))
                .map(|payload| extract(&a, payload));
            TypeAt::Def {
                name: name.to_string(),
                ty,
            }
        }
        Some("keyword") => match a.as_form(case, "keyword").and_then(|t| t.first().copied()) {
            Some(s) => match a.as_str(s) {
                Some(kw) => TypeAt::Keyword(kw.to_string()),
                None => TypeAt::Unknown,
            },
            None => TypeAt::Unknown,
        },
        Some("ty") => match a.as_form(case, "ty").and_then(|t| t.first().copied()) {
            Some(payload) => TypeAt::Ty(extract(&a, payload)),
            None => TypeAt::Unknown,
        },
        _ => TypeAt::Unknown,
    }
}

/// Read the AST-native Option `(list [])` = None / `(list [v])` = Some — the single child of a one-element
/// list, else `None`.
fn opt_child(a: &Arenas, id: StructId) -> Option<StructId> {
    match a.get(id) {
        Struct::List(kids) if kids.len() == 1 => Some(kids[0]),
        _ => None,
    }
}

/// Extract the subtree rooted at `payload` of `a` into a fresh standalone arena rooted at it.
fn extract(a: &Arenas, payload: StructId) -> Arenas {
    let mut b = Builder::new();
    let root = copy_from(&mut b, a, payload);
    b.finish(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int64_ty() -> Arenas {
        let mut b = Builder::new();
        let head = b.name("Int");
        let w = b.atom_leaf(Leaf::Int {
            value: cadenza_ast::ast::IntValue::from_i64(64),
            radix: cadenza_ast::ast::Radix::Dec,
        });
        let root = b.list(vec![head, w]);
        b.finish(root)
    }

    #[test]
    fn type_at_def_with_type_round_trips() {
        let v = TypeAt::Def {
            name: "answer".into(),
            ty: Some(int64_ty()),
        };
        match decode_type_at(&encode_type_at(&v)) {
            TypeAt::Def { name, ty } => {
                assert_eq!(name, "answer");
                assert!(ty.unwrap().structurally_eq(&int64_ty()));
            }
            other => panic!("expected Def, got {other:?}"),
        }
    }

    #[test]
    fn type_at_def_unsolved_round_trips() {
        let v = TypeAt::Def {
            name: "f".into(),
            ty: None,
        };
        match decode_type_at(&encode_type_at(&v)) {
            TypeAt::Def { name, ty } => {
                assert_eq!(name, "f");
                assert!(ty.is_none());
            }
            other => panic!("expected Def(None), got {other:?}"),
        }
    }

    #[test]
    fn type_at_keyword_round_trips() {
        match decode_type_at(&encode_type_at(&TypeAt::Keyword("if".into()))) {
            TypeAt::Keyword(kw) => assert_eq!(kw, "if"),
            other => panic!("expected Keyword, got {other:?}"),
        }
    }

    #[test]
    fn type_at_ty_round_trips() {
        match decode_type_at(&encode_type_at(&TypeAt::Ty(int64_ty()))) {
            TypeAt::Ty(a) => assert!(a.structurally_eq(&int64_ty())),
            other => panic!("expected Ty, got {other:?}"),
        }
    }

    #[test]
    fn type_at_unknown_and_garbage_are_total() {
        assert!(matches!(
            decode_type_at(&encode_type_at(&TypeAt::Unknown)),
            TypeAt::Unknown
        ));
        assert!(matches!(
            decode_type_at(b"not a binary-ast tree"),
            TypeAt::Unknown
        ));
        assert!(matches!(decode_type_at(&[]), TypeAt::Unknown));
    }
}
