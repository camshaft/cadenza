//! The `KIND_TYPE_INFO` RESULT wire — the `cdz type NAME` answer as canonical BINARY AST
//! (`cadenza_ast::codec`), the SAME wire every compile-boundary artifact speaks (operator seq-254/284/307:
//! "Binary AST is THE data exchange format. No exceptions." + "I want the full type ast!"). The producer
//! (`rcdzc::sidecar::run_query`'s `Query::TypeOf`) calls [`encode_type_info`]; the consumer (`cdz type`)
//! calls [`decode_type_info`]. ONE shared codec, so neither side hand-rolls a parser (this replaces the
//! bespoke free-text wire the consumer had to string-match with `is_no_such_definition`).
//!
//! `cdz type` is a TOTAL query with THREE structurally-distinct verdicts — the consumer needs the
//! distinction (a typo exits FAILURE; a real/unsolved type exits SUCCESS) WITHOUT string-matching the
//! message, so the wire is a tagged sum, not a string:
//! - `(type-info (found <ty-payload>))` — the def's resolved (generalized) type as the FULL structured Ty
//!   sub-AST (`encode_ty_payload`), NOT a `render_name` string; the consumer renders a display name from
//!   the decoded structure. → SUCCESS.
//! - `(type-info (unknown))` — the def EXISTS but its type could not be solved (an ambiguous unannotated
//!   parameter); the consumer prints "unknown". → SUCCESS.
//! - `(type-info (no-def <Str message>))` — the name refers to NO definition (a typo); the `message` is the
//!   producer's total "no such definition `X`" verdict (with an optional did-you-mean suggestion) carried
//!   as a leaf — a diagnostic MESSAGE, printed verbatim (like `diagnostics_wire`'s message field), never
//!   re-parsed. → FAILURE.
//!
//! TOTAL on decode: a malformed / unknown-tag value decodes to `NoDef("")` (a defined empty verdict), so a
//! consumer never panics on a skewed wire (it degrades to a benign "no such definition" with no message).

use crate::graft::copy_from;
use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct};

/// The `cdz type` verdict — the three structurally-distinct answers of the total `TypeOf` query.
#[derive(Clone, Debug)]
pub enum TypeInfo {
    /// The def's resolved type, as a standalone arena rooted at the `encode_ty_payload` sub-AST.
    Found(Arenas),
    /// The def exists but its type could not be solved (rendered "unknown").
    Unknown,
    /// No such definition — the total verdict message (incl. any did-you-mean), printed verbatim.
    NoDef(String),
}

/// Encode a `cdz type` verdict as the `KIND_TYPE_INFO` artifact bytes — ONE canonical binary AST value
/// (see module docs). Round-trips with [`decode_type_info`].
pub fn encode_type_info(info: &TypeInfo) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("type-info");
    let case = match info {
        TypeInfo::Found(ty_arena) => {
            let tag = b.name("found");
            let payload = copy_from(&mut b, ty_arena, ty_arena.root);
            b.list(vec![tag, payload])
        }
        TypeInfo::Unknown => {
            let tag = b.name("unknown");
            b.list(vec![tag])
        }
        TypeInfo::NoDef(msg) => {
            let tag = b.name("no-def");
            let m = b.atom_leaf(Leaf::Str(msg.as_str().into()));
            b.list(vec![tag, m])
        }
    };
    let root = b.list(vec![head, case]);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Decode the `KIND_TYPE_INFO` bytes back into the verdict — the inverse of [`encode_type_info`], read via
/// the shared `cadenza_ast::codec`. A `Found` carries a fresh standalone arena rooted at the type payload
/// subtree (the consumer renders it directly). TOTAL: a malformed / unknown-tag value decodes to
/// `NoDef(String::new())` — a defined benign verdict, never a crash.
pub fn decode_type_info(bytes: &[u8]) -> TypeInfo {
    let fallback = || TypeInfo::NoDef(String::new());
    let Some(a) = cadenza_ast::codec::decode(bytes) else {
        return fallback();
    };
    let Some(tail) = a.as_form(a.root, "type-info") else {
        return fallback();
    };
    let Some(&case) = tail.first() else {
        return fallback();
    };
    match a.head_name(case) {
        Some("found") => {
            let Struct::List(kids) = a.get(case) else {
                return fallback();
            };
            let Some(&payload) = kids.get(1) else {
                return fallback();
            };
            let mut b = Builder::new();
            let root = copy_from(&mut b, &a, payload);
            TypeInfo::Found(b.finish(root))
        }
        Some("unknown") => TypeInfo::Unknown,
        Some("no-def") => {
            let msg = a
                .as_form(case, "no-def")
                .and_then(|t| t.first().copied())
                .and_then(|s| a.as_str(s))
                .unwrap_or("")
                .to_string();
            TypeInfo::NoDef(msg)
        }
        _ => fallback(),
    }
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
    fn type_info_found_round_trips() {
        match decode_type_info(&encode_type_info(&TypeInfo::Found(int64_ty()))) {
            TypeInfo::Found(a) => assert!(a.structurally_eq(&int64_ty())),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn type_info_unknown_round_trips() {
        assert!(matches!(
            decode_type_info(&encode_type_info(&TypeInfo::Unknown)),
            TypeInfo::Unknown
        ));
    }

    #[test]
    fn type_info_no_def_round_trips() {
        let msg = "no such definition `foo` — did you mean `for`?".to_string();
        match decode_type_info(&encode_type_info(&TypeInfo::NoDef(msg.clone()))) {
            TypeInfo::NoDef(m) => assert_eq!(m, msg),
            other => panic!("expected NoDef, got {other:?}"),
        }
    }

    #[test]
    fn type_info_total_on_garbage() {
        // A non-AST / garbage / wrong-tag payload decodes to the benign empty NoDef, never a panic.
        assert!(matches!(
            decode_type_info(b"not a binary-ast tree"),
            TypeInfo::NoDef(m) if m.is_empty()
        ));
        assert!(matches!(decode_type_info(&[]), TypeInfo::NoDef(_)));
    }
}
