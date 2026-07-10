//! `resolve` — the query that fills the resolved column: for a node's `StructId`, what it denotes.
//!
//! One concern: meaning-of-a-name and shape-of-a-form. [`resolved_of`] classifies the AST occurrence
//! at one id and records its [`Resolved`] form, leaving children as ids for their own demand. It is a
//! per-node backward query that memoizes into `db.resolved`; it does not recurse and it touches no
//! other column. This is the ONLY module that fills `db.resolved`
//! (`reference-compiler.md` §One Pass Owns One Concern).
//!
//! It is TOTAL and produces a "no" as a value: an unrecognized head, a malformed form, or an
//! unmodeled literal becomes a [`Resolved::Poison`] carrying its [`Reject`] (a decline for
//! not-yet-built, a coded reject for ill-formed), never an abort
//! (`reference-compiler.md` §A "No" Is A First-Class Value Produced Where The Decision Is Made).
//!
//! Stage-0 expression grammar: an integer / boolean literal, `()` as unit, and `(if COND THEN ELSE)`.

use crate::arena::Slot;
use crate::ast::{Leaf, Struct, StructId};
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::resolved::Resolved;

/// The resolved form of the node at `id`, filling the column on demand (memoized). The query the
/// resolved-form request answers, and the upstream read `infer`/`lower` perform.
pub fn resolved_of(db: &mut Db, id: StructId) -> Resolved {
    if let Slot::Filled(r) = db.resolved.get(id) {
        return r.clone();
    }
    let r = compute(db, id);
    db.resolved.fill(id, r.clone());
    r
}

/// Classify one AST occurrence into its resolved form. Reads only the AST; does not recurse into
/// children (they resolve on their own demand). A "no" is produced as a `Poison` value.
fn compute(db: &Db, id: StructId) -> Resolved {
    match db.ast.get(id) {
        Struct::Atom(leaf_id) => match db.ast.leaf(*leaf_id).clone() {
            // The literal's EXACT value flows through; its machine width is decided at select.
            Leaf::Int { value, .. } => Resolved::Int(value),
            Leaf::Bool(b) => Resolved::Bool(b),
            Leaf::Name(n) => {
                // Stage 0 has no user value bindings in expression position, and unit is a form.
                Resolved::Poison(Reject::decline(format!("unbound name `{n}`")))
            }
            Leaf::Str(_) => Resolved::Poison(Reject::decline("string literals not yet supported")),
            Leaf::Float(_) => Resolved::Poison(Reject::decline("float literals not yet supported")),
        },
        Struct::List(children) => {
            // `()` — the empty list — is unit.
            if children.is_empty() {
                return Resolved::Unit;
            }
            // `(if COND THEN ELSE)`.
            if let Some(tail) = db.ast.as_form(id, "if") {
                if tail.len() != 3 {
                    return Resolved::Poison(Reject::coded(
                        Code::Malformed,
                        "if takes exactly 3 operands",
                    ));
                }
                return Resolved::If { cond: tail[0], then_: tail[1], else_: tail[2] };
            }
            let head = db.ast.head_name(id).unwrap_or("<non-name head>").to_string();
            Resolved::Poison(Reject::decline(format!("unsupported form `{head}`")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::IntValue;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn resolves_a_literal() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        assert_eq!(resolved_of(&mut db, body), Resolved::Int(IntValue::from_i64(42)));
    }

    #[test]
    fn resolves_an_if_leaving_children_as_ids() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        // The `if` resolves to a node referencing its children by id — not recursing into them.
        match resolved_of(&mut db, if_node) {
            Resolved::If { cond, then_, else_ } => {
                // The children resolve on their own demand.
                assert_eq!(resolved_of(&mut db, cond), Resolved::Bool(false));
                assert_eq!(resolved_of(&mut db, then_), Resolved::Int(IntValue::from_i64(1)));
                assert_eq!(resolved_of(&mut db, else_), Resolved::Int(IntValue::from_i64(2)));
            }
            other => panic!("expected If, got {other:?}"),
        }
    }
}
