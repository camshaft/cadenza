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
use crate::unify::{Fresh, Subst};

/// The solved type of the node at `id`, filling the column on demand (memoized). Works backward:
/// reads the resolved form and, for a compound node, its children's types.
pub fn type_of(db: &mut Db, id: StructId) -> Ty {
    if let Slot::Filled(t) = db.types.get(id) {
        return t.clone();
    }
    let t = compute(db, id);
    tracing::trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "solved type");
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
        // A record that IS a type — a ground-type record (`Bool`), a built integer module, or any
        // record carrying a `(meta t)` — is a type VALUE, so its type is `Type`. Otherwise a plain
        // data record's type is the record of its fields' types (each a lazy `type_of`).
        Resolved::Record { fields } => {
            if crate::eval::typeval_of(db, id).is_some() {
                Ty::Type
            } else {
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, value) in fields {
                    let t = type_of(db, value);
                    field_tys.insert(label, t);
                }
                Ty::Record(field_tys)
            }
        }
        // Member access — the field's type is the type of the field's VALUE, found by reducing the
        // operand to a record and projecting (the one projection, via the evaluator, so it works off a
        // literal record, a `let`-bound one, OR a module a type constructor built like `(Int 64)`). A
        // non-record operand or an absent field has no field type — typed `Any` here so it does not
        // cascade; the actual fault (CDZ0201) is reported by `type_errors`.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => type_of(db, value),
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
        Resolved::Prim(_) => Ty::Any,
        // Application — the ONE generic rule: read the head's TYPE (its `(meta t)` scheme), instantiate
        // it with fresh variables, and unify each argument's type into the curried parameter
        // positions; the result is the instantiated return type. This is HM application — the SAME
        // rule for every operator (and every function later), with NO per-operator arm. A type
        // constructor's application yields a type value, typed `Any` at the value level. `apply_type`
        // returns the result type (unification FAULTS are surfaced separately by `type_errors`).
        Resolved::Apply { head, args } => apply_type(db, head, &args),
        // A type annotation `(: expr T)`: the node's type is the annotation type `T`, with `expr`'s
        // type UNIFIED into it — so a deferred width in `expr` (a bare literal) is GROUNDED by `T`
        // (`(: 5 Int64)` types as `Int64`), and a genuine conflict (`(: true Int64)`) is a fault the
        // `type_errors` side reports (here we return `T`, the asserted type, so the value column stays
        // definite). If `T` is not a type expression this stage reduces, fall back to `expr`'s type.
        Resolved::Annot { expr, ty_expr } => match crate::eval::typeval_of(db, ty_expr) {
            Some(annot_ty) => {
                let expr_ty = type_of(db, expr);
                let mut subst = Subst::new();
                let _ = crate::unify::unify(&mut subst, &annot_ty, &expr_ty);
                subst.apply(&annot_ty)
            }
            None => type_of(db, expr),
        },
        // The type of an un-typeable node: compatible with everything, so it cannot cascade.
        Resolved::Poison(_) => Ty::Any,
        // A lambda parameter used as a value — a formal whose type is being solved. Milestone A types
        // it `Any` (it never reaches a runtime slot yet — a lambda's runtime form is deferred); the
        // full HM rule gives it a fresh variable per its binder in Milestone B.
        Resolved::Param { .. } => Ty::Any,
        // A TYPE value is a value, so it has a type — `Type` (the type of types). A bare type value
        // (a `(typeval …)` node, OR a value the evaluator reduces to a type) types as `Type`; this is
        // what makes a type first-class (it can be passed, returned, checked). A compile-time lambda
        // is not a scalar value → `Any`.
        Resolved::TypeVal(_) => Ty::Type,
        Resolved::Lambda { .. } => Ty::Any,
    }
}

/// The result type of applying `head` to `args` — the ONE generic application rule. Read the head's
/// type as a [`Scheme`] (its `(meta t)`, a type-lambda reduced by the evaluator), instantiate it with
/// fresh variables, and unify each argument's type into the curried parameter positions; the result
/// is the instantiated return type after substitution. This types an operator (`+ : ∀a. (Int a) →
/// (Int a) → (Int a)`) and a user function alike, with no per-operator logic. On any failure — a
/// head with no type, a non-function head, an arity/unify mismatch — return `Any` so the value column
/// stays total; the actual FAULT is reported by `type_errors`.
fn apply_type(db: &mut Db, head: StructId, args: &[StructId]) -> Ty {
    // A LAMBDA head β-reduces; the application's type is the reduced body's type. The reduction runs
    // under the recursion guard (keyed by the lambda body), so a recursive call declines to `Any`
    // rather than diverging — matching lowering. (For C's corpus every function call folds this way;
    // a scheme-based typing of a lambda head arrives with a def's inferred scheme.)
    if crate::eval::lambda_body(db, head).is_some() {
        return match db.enter_reduction() {
            Some(mut guard) => {
                let g = guard.db();
                match crate::eval::apply_lambda(g, head, args) {
                    Ok(Some(reduced)) => type_of(g, reduced),
                    _ => Ty::Any,
                }
            }
            None => Ty::Any, // recursive — the fault is reported by the lowering/poison path.
        };
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No `(meta t)` (e.g. a bare type constructor whose result is a type value) → `Any`.
        None => return Ty::Any,
    };
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        // Peel one curried parameter: `cur` must be a function type; unify the arg into its parameter.
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                let at = type_of(db, arg);
                // A unify failure here is a real type fault; ignore it for the VALUE (reported by
                // `type_errors`) and continue with the declared result so the shape stays sane.
                let _ = crate::unify::unify(&mut subst, &param, &at);
                cur = *result;
            }
            // Applied to more args than it takes — not a function; the fault is reported elsewhere.
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
}

/// Check an application for type faults — the ONE rule's fault side. Instantiate the head's scheme and
/// unify each argument into its curried parameter; a unify failure is the conflicting-use type error.
/// A head with no `(meta t)` scheme (a type constructor, or a not-yet-typed value) is not checked here.
fn check_application(db: &mut Db, head: StructId, args: &[StructId], out: &mut Vec<Reject>) {
    // A LAMBDA head β-reduces; its faults live in the reduced body, checked when that body is
    // collected (it is reached from this node's core). So the scheme-based check below is only for a
    // non-lambda head (an operator).
    if matches!(crate::eval::apply_lambda(db, head, args), Ok(Some(_))) {
        return;
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        None => return,
    };
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                let at = type_of(db, arg);
                if let Err(reject) = crate::unify::unify(&mut subst, &param, &at) {
                    out.push(reject);
                }
                cur = *result;
            }
            // Applying a non-function (too many args) — a shape fault reported as a type mismatch.
            other => {
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!("cannot apply a value of type {}", other.render_name()),
                ));
                return;
            }
        }
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

/// Collect faults at and under `id`, stamping each with its origin node. The recursive `collect_node`
/// pushes this node's own faults and recurses into children (each child stamped by its OWN frame
/// first); afterwards we stamp every fault THIS frame added that is still unanchored with `id` — so a
/// fault ends up anchored to the innermost node whose frame produced it, with no per-`push` threading.
fn collect(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    let before = out.len();
    collect_node(db, id, out);
    for reject in &mut out[before..] {
        reject.set_origin_if_absent(id);
    }
}

fn collect_node(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
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
            // Project via the evaluator (reduces refs / a ctor-built module), so a missing field on a
            // built module is caught too. A poison operand reports its OWN fault (via the descent
            // below), so we don't add a redundant "not a record" for it.
            let operand_is_poison =
                matches!(resolved_of(db, operand), Resolved::Poison(_));
            match crate::eval::member_value(db, operand, &key) {
                crate::eval::Member::Field(_) => {}
                crate::eval::Member::NoField => out.push(Reject::coded(
                    Code::Malformed,
                    format!("record has no field `{}`", key.name),
                )),
                crate::eval::Member::NotRecord if !operand_is_poison => out.push(Reject::coded(
                    Code::Malformed,
                    format!(
                        "member access requires a record, found {}",
                        type_of(db, operand).render_name()
                    ),
                )),
                crate::eval::Member::NotRecord => {}
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
        // Application faults, by the ONE rule: instantiate the head's `(meta t)` scheme and unify each
        // argument's type into the curried parameter positions. A unify FAILURE is the conflicting-use
        // type error (a non-integer where an integer is required, two operands of different widths —
        // `numeric-model.md` no silent promotion). One check for every operator; no arith-specific
        // logic. A head with no `(meta t)` (a type constructor / not-yet-typed value) is not checked
        // here — its own fault, if any, surfaces via the head descent.
        Resolved::Apply { head, args } => {
            check_application(db, head, &args, out);
            // Descend into the HEAD (an unbound head like `frobnicate` is a scope error caught here)
            // and each operand for their own faults.
            collect(db, head, out);
            for &arg in &args {
                collect(db, arg, out);
            }
        }
        // A type annotation `(: expr T)`: UNIFY the asserted type `T` against `expr`'s type. A failure
        // is the conflicting-use type error (`(: true Int64)` — Bool asserted as Int64 → CDZ0203); a
        // success grounds a deferred width harmlessly. If `T` is not a type this stage reduces, decline
        // the CHECK (no false reject) — the expr still type-checks on its own via the descent below.
        Resolved::Annot { expr, ty_expr } => {
            if let Some(annot_ty) = crate::eval::typeval_of(db, ty_expr) {
                let expr_ty = type_of(db, expr);
                let mut subst = Subst::new();
                if let Err(reject) = crate::unify::unify(&mut subst, &annot_ty, &expr_ty) {
                    out.push(Reject::coded(
                        Code::TypeMismatch,
                        format!(
                            "annotation type {} does not match value type {}",
                            annot_ty.render_name(),
                            expr_ty.render_name()
                        ),
                    ));
                    let _ = reject; // the annotation's own message supersedes the raw unify message.
                }
            }
            collect(db, expr, out);
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
        Resolved::Prim(_)
        | Resolved::Ref { .. }
        | Resolved::Param { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Unit
        | Resolved::TypeVal(_)
        | Resolved::Lambda { .. } => {}
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
