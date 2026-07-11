//! `infer` — the query that fills the type column: for a node's `StructId`, its solved [`Ty`].
//!
//! One concern: type determination. [`type_of`] solves one node's type, reading its resolved form
//! (via [`crate::resolve::resolved_of`], which fills the resolved column on demand) and, for a
//! compound node, its children's types (each a lazy `type_of`). It memoizes into `db.types` and is
//! the ONLY module that fills it (`reference-compiler.md` §Types Are Solved Once And Read
//! Downstream). Asking one node's type solves only the nodes that answer reaches — the query stays
//! demand-driven.
//!
//! Stage 0 is monomorphic: a literal's type is a signed integer of DEFERRED width (numeric-literal
//! polymorphism — inference or the backend grounds it), a boolean is `Bool`, an `if` is the join of
//! its branches, and a poison is `Ty::Any` (compatible with everything, so a "no" never cascades
//! into a spurious mismatch). Real Hindley-Milner slots in behind this same `type_of` seam later.
//!
//! [`type_errors`] is a SEPARATE query — a read over the (demand-filled) type column that reports
//! type-agreement faults (an `if` whose condition is not `Bool`, or whose branches disagree). Keeping
//! the fault check apart from the value fill is what lets filling a type never reject and a later
//! rewrite preserve both the value and its checks (`reference-compiler.md` §A Meaning-Preserving
//! Rewrite Preserves Value And Checks).

use crate::arena::Slot;
use crate::ast::StructId;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::resolve::resolved_of;
use crate::resolved::Resolved;
use crate::ty::Ty;

/// The solved type of the node at `id`, filling the column on demand (memoized). Works backward:
/// reads the resolved form and, for a compound node, its children's types.
pub fn type_of(db: &mut Db, id: StructId) -> Ty {
    if let Slot::Filled(t) = db.types.get(id) {
        return t.clone();
    }
    let t = compute(db, id);
    db.types.fill(id, t.clone());
    t
}

/// Solve one node's type. A poison is typed `Any` (compatible with everything) so a "no" never
/// induces a spurious mismatch upward. An integer literal is typed with a DEFERRED width, which
/// inference (or, failing that, the backend) grounds later.
fn compute(db: &mut Db, id: StructId) -> Ty {
    match resolved_of(db, id) {
        // A bare integer literal is polymorphic in its width until something fixes it.
        Resolved::Int(_) => Ty::int(),
        Resolved::Bool(_) => Ty::Bool,
        Resolved::Unit => Ty::Unit,
        // A name IS its bound value's type — follow the ref (a lazy `type_of` on the value occurrence).
        Resolved::Ref { value } => type_of(db, value),
        // A `let`'s type is its body's type (the bindings are compile-time structure that folds away).
        Resolved::Let { body, .. } => type_of(db, body),
        // A record's type is the record of its fields' types — each a lazy `type_of` on the value occ.
        Resolved::Record { fields } => {
            let mut field_tys = std::collections::BTreeMap::new();
            for (label, value) in fields {
                let t = type_of(db, value);
                field_tys.insert(label, t);
            }
            Ty::Record(field_tys)
        }
        // Member access projects the field's type from the operand's record type. A non-record operand
        // or an absent field has no field type — typed `Any` here so it does not cascade; the actual
        // fault (CDZ0201) is reported by `type_errors`.
        Resolved::Member { operand, key } => match type_of(db, operand) {
            Ty::Record(fields) => fields.get(&key).cloned().unwrap_or(Ty::Any),
            _ => Ty::Any,
        },
        Resolved::If { cond, then_, else_ } => {
            // Reading the children's types is the backward demand: each is a lazy `type_of`.
            let _cond_ty = type_of(db, cond);
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            // The if's type is the join of its branches — `Any` yields the other, and a branch that
            // fixed a deferred integer width contributes it. The cond-is-Bool and branches-agree
            // CHECKS are `type_errors`' job; this fills the value column.
            then_ty.join(&else_ty)
        }
        // A bare built-in operation value standing alone has no scalar type yet (it is not a runtime
        // value until functions/closures exist). Typed `Any`; applying it is what has a type.
        Resolved::Intrinsic(_) => Ty::Any,
        // An arithmetic application: the operation is generic over the integer type, so its result is
        // the integer type its operands share. Type = the JOIN of the operand types (a deferred/var
        // width unifies with a fixed one; two fixed widths that differ are a mismatch reported by
        // `type_errors`). A non-integer operand yields `Any` here (the fault is reported there).
        Resolved::Apply { op, args } => match crate::resolve::intrinsic_of(db, op) {
            Some(_) => {
                let mut result = Ty::int(); // signed, deferred width — the generic operand type.
                for &arg in &args {
                    let at = type_of(db, arg);
                    result = if matches!(at, Ty::Int(_) | Ty::Var(_) | Ty::Any) {
                        result.join(&at)
                    } else {
                        // A non-integer operand: the application is ill-typed; surface `Any` here so
                        // the whole program does not cascade, `type_errors` reports the mismatch.
                        Ty::Any
                    };
                }
                result
            }
            None => Ty::Any,
        },
        // The type of an un-typeable node: compatible with everything, so it cannot cascade.
        Resolved::Poison(_) => Ty::Any,
    }
}

/// The type-agreement faults reachable from `id` — a query READ over the demand-filled type column,
/// separate from the value it holds. An `if` whose condition is not `Bool`, or whose branches do not
/// agree, is a coded mismatch. Descends into children for their own faults.
pub fn type_errors(db: &mut Db, id: StructId) -> Vec<Reject> {
    let mut out = Vec::new();
    collect(db, id, &mut out);
    out
}

fn collect(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    match resolved_of(db, id) {
        Resolved::If { cond, then_, else_ } => {
            let cond_ty = type_of(db, cond);
            if !cond_ty.agrees_with(&Ty::Bool) {
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!("if condition must be Bool, found {}", cond_ty.render_name()),
                ));
            }
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            if !then_ty.agrees_with(&else_ty) {
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "if branches differ: {} vs {}",
                        then_ty.render_name(),
                        else_ty.render_name()
                    ),
                ));
            }
            collect(db, cond, out);
            collect(db, then_, out);
            collect(db, else_, out);
        }
        // Member access: the operand must be a record, and it must have the named field. Both faults
        // are CDZ0201 (`core-semantics.md` §Member Access Projects A Record Field — projecting a field
        // of a non-record, or an absent field, has no defined result). The user record is CLOSED (a
        // missing field rejects) — distinct from an open built-in module (a later stage).
        Resolved::Member { operand, key } => {
            match type_of(db, operand) {
                Ty::Record(fields) => {
                    if !fields.contains_key(&key) {
                        out.push(Reject::coded(
                            Code::Malformed,
                            format!("record has no field `{}`", key.name),
                        ));
                    }
                }
                // `Any` is a poison operand — its own fault is already reported; don't double-report.
                Ty::Any => {}
                other => out.push(Reject::coded(
                    Code::Malformed,
                    format!(
                        "member access requires a record, found {}",
                        other.render_name()
                    ),
                )),
            }
            collect(db, operand, out);
        }
        // Descend into the new binding/aggregate forms for their own faults.
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect(db, value, out);
            }
            collect(db, body, out);
        }
        Resolved::Record { fields } => {
            for (_, value) in fields {
                collect(db, value, out);
            }
        }
        // An arithmetic application: each operand must be an integer, and their widths/signedness must
        // agree. A non-integer operand or a width/signedness mismatch is the conflicting-use type
        // error CDZ0301 (`numeric-model.md` — mixing two numeric types without an explicit conversion
        // does not silently promote). Descend into the operands for their own faults.
        Resolved::Apply { op, args } => {
            if crate::resolve::intrinsic_of(db, op).is_some() {
                let mut prev: Option<Ty> = None;
                for &arg in &args {
                    let at = type_of(db, arg);
                    match &at {
                        Ty::Int(_) | Ty::Var(_) | Ty::Any => {}
                        other => out.push(Reject::coded(
                            Code::TypeMismatch,
                            format!("arithmetic requires an integer, found {}", other.render_name()),
                        )),
                    }
                    // Operands must agree with each other (no silent promotion between numeric types).
                    if let Some(p) = &prev {
                        if !p.agrees_with(&at) {
                            out.push(Reject::coded(
                                Code::NumericMismatch,
                                format!(
                                    "operands of different numeric types: {} vs {}",
                                    p.render_name(),
                                    at.render_name()
                                ),
                            ));
                        }
                    }
                    prev = Some(at);
                }
            }
            for &arg in &args {
                collect(db, arg, out);
            }
        }
        // A scope-error poison (an unbound name) is UNCONDITIONAL well-formedness — report it here,
        // where the walk descends into EVERY position (including an `if`'s branches), so an unbound
        // name in an untaken branch or an uncalled definition is still rejected (`core-semantics.md`
        // §Binding Is Lexical — not gated on reachability). A DECLINE or a compile-provable TRAP
        // poison is NOT reported here: a decline is not a well-formedness fault, and a trap is
        // reachability-gated (the trap-poison walk in `compile` handles it, skipping untaken branches).
        Resolved::Poison(r) => {
            if r.code == Some(Code::Unbound) {
                out.push(r);
            }
        }
        // A ref's target-node fault is reported when that node is collected on its own. A bare
        // intrinsic value and the atomic leaves have no sub-faults.
        Resolved::Intrinsic(_)
        | Resolved::Ref { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Unit => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Slot;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn type_of_a_literal_is_a_deferred_int_rendering_as_int64() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        // The type of the literal node is a column read — the Stage-0 done-criterion. A bare literal
        // has a DEFERRED width (nothing fixed it), which agrees with `Int64` and renders as `Int64`.
        let t = type_of(&mut db, body);
        assert_eq!(t, Ty::int());
        assert!(t.agrees_with(&Ty::int64()));
        assert_eq!(t.render_name(), "Int64");
    }

    #[test]
    fn type_of_an_if_is_the_branch_type_and_it_checks_clean() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        // `(if false 1 2)` : the branches are integers, so the if is an integer.
        assert!(type_of(&mut db, if_node).agrees_with(&Ty::int64()));
        // A well-typed if reports no faults.
        assert!(type_errors(&mut db, if_node).is_empty());
    }

    #[test]
    fn asking_a_type_does_not_fill_the_core_column() {
        // Laziness across modules: solving a type must not have lowered anything.
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let _ = type_of(&mut db, body);
        assert!(
            matches!(db.core.get(body), Slot::Absent),
            "core filled without demand"
        );
    }
}
