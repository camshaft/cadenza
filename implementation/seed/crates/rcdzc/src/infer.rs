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
use crate::ty::{Scheme, Ty};
use crate::unify::{Fresh, Subst};
use tracing::trace;

/// The solved type of the node at `id`, filling the column on demand (memoized). Works backward:
/// reads the resolved form and, for a compound node, its children's types.
pub fn type_of(db: &mut Db, id: StructId) -> Ty {
    if let Slot::Filled(t) = db.types.get(id) {
        trace!(target: "rcdzc::infer", node = id.0, "memo hit");
        return t.clone();
    }
    let t = compute(db, id);
    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "solved type");
    // Do NOT memoize a provisional `Any`: a node typed `Any` here may be a recursive-def parameter (or
    // a reference to one) whose CONNECTED solve (A2) has not run yet — caching `Any` would freeze that
    // stale answer even after the solve fills the real type. `Any` is compatible with everything and
    // costs only a recompute, so leaving it unmemoized is safe and lets the solved type win on a later
    // demand. Every DEFINITE type is memoized as before (the solve-once discipline holds for real types).
    if !matches!(t, Ty::Any) {
        db.types.fill(id, t.clone());
    }
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
        // A match's type is the JOIN of its arm bodies (like an `if` over N branches) — every arm must
        // produce the same type, and a branch that fixed a deferred width contributes it. The
        // arms-agree and exhaustiveness CHECKS are `type_errors`' job; this fills the value column.
        Resolved::Match { scrutinee, arms } => {
            let _scrut = type_of(db, scrutinee);
            let mut ty = Ty::Any;
            for (_, body) in &arms {
                let bt = type_of(db, *body);
                ty = ty.join(&bt);
            }
            ty
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
        // A lambda/def parameter used as a value — a formal. If its binder is ANNOTATED (`(: a T)`),
        // its type is that annotation `T` — so the body type-checks against a definite parameter type
        // (`(: a Bool)` used as an integer operand is caught). Otherwise, for the parameter of a
        // RECURSIVE def (which cannot inline, so its type must be inferred rather than flowing from a
        // call site), the CONNECTED def-body solve (`solve_recursive_params`, A2) infers it from its
        // uses; a still-`None` result means either a non-recursive param (typed `Any` — it inlines at
        // its call site, where the argument's type flows in via the fold) or an unconstrained one.
        Resolved::Param { binder } => param_annot_ty(db, binder)
            .or_else(|| solved_param_ty(db, binder))
            .unwrap_or(Ty::Any),
        // A TYPE value is a value, so it has a type — `Type` (the type of types). A bare type value
        // (a `(typeval …)` node, OR a value the evaluator reduces to a type) types as `Type`; this is
        // what makes a type first-class (it can be passed, returned, checked). A compile-time lambda
        // is not a scalar value → `Any`.
        Resolved::TypeVal(_) => Ty::Type,
        Resolved::Lambda { .. } => Ty::Any,
    }
}

/// The declared type of an annotated parameter whose NAME occurrence is `binder`, if any. A parameter
/// is annotated when its name sits in a `(: name T)` binder (the name's parent is that form); the type
/// is `T` reduced to a `Ty` by the evaluator (`typeval_of`). `None` for a bare (unannotated) parameter
/// or an unreducible annotation type — in which case the parameter's type is left open (`Any`).
fn param_annot_ty(db: &mut Db, binder: StructId) -> Option<Ty> {
    let parent = db.parent_of(binder)?;
    let tail = db.ast.as_form(parent, ":")?;
    // The binder must be the NAME position (first) of the `(: name T)`, not the type position.
    if tail.first().copied() != Some(binder) {
        return None;
    }
    let ty_expr = *tail.get(1)?;
    crate::eval::typeval_of(db, ty_expr)
}

/// The inferred type of an unannotated RECURSIVE-def parameter whose name occurrence is `binder`, or
/// `None` if `binder` is not such a parameter (a non-recursive def's param inlines at its call site and
/// stays `Any`; an annotated param is handled by `param_annot_ty`). Locates `binder`'s def, and — if
/// that def is recursive — runs the connected parameter solve (memoized), returning `binder`'s solved
/// type. This is the ONE place a parameter's type is INFERRED from its uses rather than read off an
/// annotation or a call-site argument (ANF step 2 / A2).
fn solved_param_ty(db: &mut Db, binder: StructId) -> Option<Ty> {
    if let Some(t) = db.param_types.get(&binder) {
        return Some(t.clone());
    }
    let def = def_of_param(db, binder)?;
    // Only a RECURSIVE def needs this: a non-recursive def's call always inlines (β-reduces), so its
    // param's type flows from the concrete argument and never reaches here as `Any`. Solving a
    // non-recursive param would be redundant work and could mask the inlining path — so gate on it.
    let body = db.defs[def].body?;
    if !crate::eval::is_recursive(db, body) {
        return None;
    }
    solve_recursive_params(db, def);
    db.param_types.get(&binder).cloned()
}

/// The `db.defs` index whose signature declares the parameter name-occurrence `binder`, or `None` if
/// `binder` is not a top-level def parameter (e.g. a `fn` lambda parameter). Walks up from the binder to
/// the `(def (NAME param…) body)` it sits in.
fn def_of_param(db: &mut Db, binder: StructId) -> Option<usize> {
    // A param occurrence is either bare (`binder`'s parent is the signature list) or the name of a
    // `(: name T)` binder (parent is the `:` form, whose parent is the signature). Find the signature
    // list, then the def whose `sig_occ` is it.
    let parent = db.parent_of(binder)?;
    let sig = if db.ast.as_form(parent, ":").is_some() {
        db.parent_of(parent)?
    } else {
        parent
    };
    db.defs.iter().position(|d| d.sig_occ == sig)
}

/// Solve the parameter types of a RECURSIVE def by a single connected, threaded-`Subst` unification
/// over its body — the one place inference is a connected solve rather than a per-node column read
/// (ANF step 2 / A2). Fills `db.param_types` for EVERY parameter of the def at once.
///
/// The method: give each parameter a fresh type variable and bind it in a local env; walk the body
/// collecting constraints on those variables — where a parameter (or an expression over it) is used as
/// an operand of a built-in operation, unify its variable with the operand type the operation's SCHEME
/// requires; where a self-call passes an argument in a parameter position, unify the argument's type
/// with that parameter's variable (the fixpoint — a recursive call constrains the very signature being
/// solved). Then ground each variable: a solved variable becomes its concrete type; an UNCONSTRAINED
/// variable (no use pinned it) grounds to the default integer only if a use marked it numeric, else it
/// is left `Any` (the parameter is genuinely unconstrained — the export/select layer then declines,
/// asking for an annotation, rather than the compiler inventing a type). Order-independent because
/// unification is: the constraints commute, so demand order cannot change the solution.
fn solve_recursive_params(db: &mut Db, def: usize) {
    // Re-entry backstop: if this def's solve is already on the stack, do not recompute (a demand landing
    // mid-solve reads the provisional/absent entry). The local-env walk below does not call `type_of`
    // on a param, so this is defensive.
    if db.solving_params.contains(&def) {
        return;
    }
    db.solving_params.insert(def);

    let sig_params = db.defs[def].params.clone();
    let mut fresh = Fresh::new();
    // Each parameter's name occurrence → its fresh type variable. An ANNOTATED param uses its
    // annotation type as a fixed constraint (not a fresh var), so a mixed signature still solves.
    let mut env: std::collections::HashMap<StructId, Ty> = std::collections::HashMap::new();
    let mut param_binders: Vec<(StructId, Ty)> = Vec::new();
    for p in &sig_params {
        let binder = match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *p,
        };
        let var = param_annot_ty(db, binder).unwrap_or_else(|| Ty::Var(fresh.var()));
        env.insert(binder, var.clone());
        param_binders.push((binder, var));
    }

    let mut subst = Subst::new();
    if let Some(body) = db.defs[def].body {
        collect_param_constraints(db, body, &env, def, &mut subst, &mut fresh);
    }

    // Ground each parameter: apply the substitution, then default a still-unsolved NUMERIC variable to
    // the signed-64 integer (a bare literal's default) and leave anything else `Any` (unconstrained).
    for (binder, var) in param_binders {
        let solved = subst.apply(&var);
        let grounded = ground_param(solved);
        trace!(target: "rcdzc::infer", def, binder = binder.0, ty = %grounded.render_name(), "A2: solved recursive param");
        db.param_types.insert(binder, grounded);
    }

    db.solving_params.remove(&def);
}

/// Ground a solved parameter type: a still-unsolved variable that a numeric use constrained becomes the
/// default integer (`Int64`), matching a bare literal's defaulting; a fully-unconstrained variable stays
/// `Any` (the parameter is genuinely undetermined — the boundary layer declines rather than guess).
fn ground_param(ty: Ty) -> Ty {
    match ty {
        // A variable no constraint pinned: leave it `Any` — the caller (layout/select) declines an
        // ambiguous parameter rather than inventing a width.
        Ty::Var(_) => Ty::Any,
        // A deferred-width integer (a numeric use pinned it integer but not the width) grounds to the
        // default width, exactly as a bare literal does.
        Ty::Int(_) => Ty::int64(),
        other => other,
    }
}

/// Walk the resolved body collecting constraints on the parameter variables in `env`, extending
/// `subst`. For each application of a built-in operation, the operation's SCHEME fixes what type each
/// argument must have; unifying the argument's type into the scheme's parameter positions constrains
/// any parameter variable that argument mentions. A self/mutual call to a def in the same recursive
/// group unifies each argument against the CALLEE's parameter variable (read from `env` when the callee
/// is THIS def; a cross-def callee's params are solved by its own pass — its scheme is read there). The
/// walk descends every sub-expression that runs.
fn collect_param_constraints(
    db: &mut Db,
    node: StructId,
    env: &std::collections::HashMap<StructId, Ty>,
    def: usize,
    subst: &mut Subst,
    fresh: &mut Fresh,
) {
    match resolved_of(db, node) {
        Resolved::Apply { head, args } => {
            // An operator (a prim with a `(meta t)` scheme): instantiate it and unify each argument's
            // type into the curried parameter positions. This is what pins `n` to an integer in `(= n
            // 0)` / `(+ n …)` and to a Bool in a boolean op.
            if let Some(scheme) = crate::eval::scheme_of(db, head, fresh) {
                let mut cur = crate::unify::instantiate(&scheme, fresh);
                for &arg in &args {
                    let applied = subst.apply(&cur);
                    if let Ty::Fn(param, result) = applied {
                        let at = arg_ty_in_env(db, arg, env, subst);
                        let _ = crate::unify::unify(subst, &param, &at);
                        cur = *result;
                    } else {
                        break;
                    }
                }
            } else if let Some(callee) = callee_def_index_for_infer(db, head) {
                // A call to a user def. If it is THIS def (self-recursion), unify each argument against
                // this def's own parameter variable — the fixpoint. (A cross-def callee in a mutual
                // group is handled by its own solve; here we still constrain the argument sub-terms.)
                if callee == def {
                    // The parameters in signature order (env is unordered) — arguments match positionally.
                    let ordered = ordered_param_binders(db, def);
                    for (i, &arg) in args.iter().enumerate() {
                        if let Some(&binder) = ordered.get(i)
                            && let Some(pvar) = env.get(&binder)
                        {
                            let at = arg_ty_in_env(db, arg, env, subst);
                            let _ = crate::unify::unify(subst, pvar, &at);
                        }
                    }
                }
            }
            // Descend into the head (a computed head) and every argument for THEIR own constraints.
            if matches!(resolved_of(db, head), Resolved::Apply { .. }) {
                collect_param_constraints(db, head, env, def, subst, fresh);
            }
            for arg in args {
                collect_param_constraints(db, arg, env, def, subst, fresh);
            }
        }
        Resolved::If { cond, then_, else_ } => {
            // The condition must be Bool — constrain it (a bare-param condition `(if n …)` pins n Bool).
            let ct = arg_ty_in_env(db, cond, env, subst);
            let _ = crate::unify::unify(subst, &ct, &Ty::Bool);
            collect_param_constraints(db, cond, env, def, subst, fresh);
            collect_param_constraints(db, then_, env, def, subst, fresh);
            collect_param_constraints(db, else_, env, def, subst, fresh);
        }
        Resolved::Match { scrutinee, arms } => {
            // Each LITERAL pattern constrains the scrutinee's type: matching `n` against the integer
            // literal `0` (or `true`) means `n` has that literal's type. So unify the scrutinee's type
            // with each non-wildcard pattern's type — this is what pins a `match`-based recursive
            // parameter (`(match n (0 …) (_ …))` → `n : Int64`). Then descend into scrutinee + bodies.
            let st = arg_ty_in_env(db, scrutinee, env, subst);
            for (pat, _) in &arms {
                if let Some(pt) = literal_pattern_ty(db, *pat) {
                    let _ = crate::unify::unify(subst, &st, &pt);
                }
            }
            collect_param_constraints(db, scrutinee, env, def, subst, fresh);
            for (_, body) in &arms {
                collect_param_constraints(db, *body, env, def, subst, fresh);
            }
        }
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_param_constraints(db, value, env, def, subst, fresh);
            }
            collect_param_constraints(db, body, env, def, subst, fresh);
        }
        Resolved::Annot { expr, .. } => collect_param_constraints(db, expr, env, def, subst, fresh),
        Resolved::Member { operand, .. } => {
            collect_param_constraints(db, operand, env, def, subst, fresh)
        }
        Resolved::Record { fields } => {
            for value in fields.values() {
                collect_param_constraints(db, *value, env, def, subst, fresh);
            }
        }
        // Leaves and references contribute no sub-constraints (a bare param ref's constraint comes from
        // the operation that consumes it, handled at the enclosing Apply).
        _ => {}
    }
}

/// The type a LITERAL match pattern implies for the scrutinee, or `None` for a non-literal pattern (the
/// wildcard `_`, or a binder — which constrains nothing). An integer-literal pattern implies a deferred
/// integer (grounds like a bare literal); a boolean-literal pattern implies `Bool`. Used to constrain a
/// scrutinee from its arms (both in the parameter solve and — later — exhaustiveness).
fn literal_pattern_ty(db: &mut Db, pat: StructId) -> Option<Ty> {
    match resolved_of(db, pat) {
        Resolved::Int(_) => Some(Ty::int()),
        Resolved::Bool(_) => Some(Ty::Bool),
        _ => None,
    }
}

/// The type of an argument occurrence within the parameter-solve env: a reference to a parameter being
/// solved is its variable (applied through `subst`); anything else is its ordinary solved `type_of`
/// (a literal is a deferred int, a nested op is its result type, a cross-def call its scheme result).
fn arg_ty_in_env(
    db: &mut Db,
    arg: StructId,
    env: &std::collections::HashMap<StructId, Ty>,
    subst: &Subst,
) -> Ty {
    // A reference to a parameter being solved → its variable. A body param reference resolves to
    // `Ref { value: <param binder> }` (or a bare `Param { binder }`).
    match resolved_of(db, arg) {
        Resolved::Ref { value } => {
            if let Some(var) = env.get(&value) {
                return subst.apply(var);
            }
        }
        Resolved::Param { binder } => {
            if let Some(var) = env.get(&binder) {
                return subst.apply(var);
            }
        }
        _ => {}
    }
    // Not a parameter reference — its ordinary type. (Reads the type column; a nested op over a param
    // returns the op's result type, which the enclosing unify relates to the param var separately.)
    type_of(db, arg)
}

/// The parameter NAME occurrences of def `def` in signature order (the order arguments match). Mirrors
/// the peeling `solve_recursive_params` does, but returns them ordered for positional self-call unify.
fn ordered_param_binders(db: &Db, def: usize) -> Vec<StructId> {
    db.defs[def]
        .params
        .iter()
        .map(
            |p| match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => *p,
            },
        )
        .collect()
}

/// The generalized SIGNATURE of top-level definition `def` as a [`Scheme`] — its type as a value, so a
/// CALL can be typed by instantiating this scheme rather than by β-reducing the body. Curried: a def
/// `(f a b)` whose params solve to `A`,`B` and whose body solves to `R` has scheme `A -> B -> R` (a
/// nullary def is just `R`). Memoized on `db.def_schemes` keyed by the def index (a pure function of
/// the fixed def structure).
///
/// **A1 scope (see `implementation/DESIGN-anf-step2-runtime-functions.md`).** This computes the scheme
/// ONLY for a def whose signature is already fully DETERMINED without a connected def-body solve —
/// every parameter has a definite machine type (from its annotation) and the body types to a definite
/// type reading those params. That covers an exported/annotated function, whose scheme here AGREES
/// with what β-reduction produces at a call (cross-checked in tests). It returns `None` — deferring to
/// the existing β-reduction typing — when a parameter is undetermined (`Any`, an unannotated param
/// needing inference) OR the body's type is `Any` (a recursive self-call, which needs the fixpoint A2
/// adds). So this is purely additive: no call yet reads it, and the fallback path is unchanged.
pub fn def_scheme(db: &mut Db, def: usize) -> Option<Scheme> {
    if let Some(cached) = db.def_schemes.get(&def) {
        return cached.clone();
    }
    // RE-ENTRY GUARD. Computing a recursive def's scheme demands `type_of` of its body, whose self-call
    // demands this def's scheme AGAIN before the memo is filled. Seed a `None` sentinel FIRST, so the
    // re-entrant call reads `None` (a self-call types as `Any`, absorbed by the base case — the same
    // behavior as the β-reduction recursion guard) rather than looping forever. The real scheme
    // overwrites the sentinel once the body solve completes.
    db.def_schemes.insert(def, None);
    let scheme = compute_def_scheme(db, def);
    db.def_schemes.insert(def, scheme.clone());
    scheme
}

fn compute_def_scheme(db: &mut Db, def: usize) -> Option<Scheme> {
    let body = db.defs[def].body?;
    let sig_params = db.defs[def].params.clone();
    // Each parameter's type — read `type_of` on its NAME occurrence (the annotation type for an
    // annotated param, `Any` for an unannotated one). An `Any` parameter is UNDETERMINED here: it
    // needs the connected def-body solve A2 adds, so decline the scheme and let the caller β-reduce.
    let mut param_tys: Vec<Ty> = Vec::new();
    for p in &sig_params {
        let binder = match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *p,
        };
        let ty = type_of(db, binder);
        if matches!(ty, Ty::Any) {
            trace!(target: "rcdzc::infer", def, "def_scheme: an undetermined param → defer to β-reduction (A2)");
            return None;
        }
        param_tys.push(ty);
    }
    // The result type is the body's solved type. `Any` here means the body could not be typed without
    // reducing a self-call (recursion) — defer to the fixpoint A2 adds.
    let result = type_of(db, body);
    if matches!(result, Ty::Any) {
        trace!(target: "rcdzc::infer", def, "def_scheme: undetermined result (recursive?) → defer (A2)");
        return None;
    }
    // Curry: `p_0 -> p_1 -> … -> result`. A nullary def is just `result`.
    let mut ty = result;
    for pt in param_tys.into_iter().rev() {
        ty = Ty::Fn(Box::new(pt), Box::new(ty));
    }
    // A1 schemes are MONOMORPHIC (every param has a concrete annotated type; nothing is quantified).
    // Real let-generalization — quantifying a free variable a parameter's type still carries — arrives
    // with the connected solve (A2), which is where a polymorphic def signature becomes meaningful.
    trace!(target: "rcdzc::infer", def, scheme = %ty.render_name(), "def_scheme: determined monomorphic signature");
    Some(Scheme::mono(ty))
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
        trace!(target: "rcdzc::infer", head = head.0, args = args.len(), "apply: β-reduce lambda head for its type");
        let reduced_ty = match db.enter_reduction() {
            Some(mut guard) => {
                let g = guard.db();
                match crate::eval::apply_lambda(g, head, args) {
                    Ok(Some(reduced)) => Some(type_of(g, reduced)),
                    _ => None, // recursive / partial — β-reduction can't type it; try the scheme below.
                }
            }
            None => None, // depth limit (recursive) — try the scheme below.
        };
        if let Some(t) = reduced_ty {
            return t;
        }
        // β-reduction couldn't type it (a RECURSIVE callee). Type the call by the callee's DEF SCHEME
        // instead — an annotated recursive def has a determined signature (`def_scheme`), so applying
        // it to the args yields the real result type (Int64 for `(sum-to 3)`) rather than `Any`. This
        // is what lets a recursive call's RESULT flow to a machine type (a wasm return valtype). A
        // callee with no determined scheme (unannotated, needs the connected solve) stays `Any`.
        if let Some(callee) = callee_def_index_for_infer(db, head)
            && let Some(scheme) = def_scheme(db, callee)
        {
            trace!(target: "rcdzc::infer", head = head.0, callee, "apply: recursive call typed by def_scheme");
            return apply_scheme_to_args(db, &scheme, args);
        }
        return Ty::Any; // recursive with an undetermined signature — fault reported elsewhere.
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No `(meta t)` (e.g. a bare type constructor whose result is a type value) → `Any`.
        None => {
            trace!(target: "rcdzc::infer", head = head.0, "apply: head has no (meta t) scheme → Any");
            return Ty::Any;
        }
    };
    trace!(target: "rcdzc::infer", head = head.0, scheme = %scheme.ty.render_name(), args = args.len(), "apply: instantiate head scheme");
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

/// Apply a def SCHEME to `args`, returning the result type — instantiate it with fresh variables, then
/// peel one curried parameter per argument (unifying the arg's type into the parameter). The result is
/// the instantiated return type after substitution. Used to type a RECURSIVE call by its callee's
/// signature (which β-reduction can't type). Mirrors the operator-scheme application in `apply_type`.
fn apply_scheme_to_args(db: &mut Db, scheme: &Scheme, args: &[StructId]) -> Ty {
    let mut fresh = Fresh::new();
    let mut cur = crate::unify::instantiate(scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                let at = type_of(db, arg);
                let _ = crate::unify::unify(&mut subst, &param, &at);
                cur = *result;
            }
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
}

/// The `db.defs` index of the top-level def an application head names, if any — for typing a recursive
/// call by its scheme. Follows a `Ref` to a `Lambda` whose body matches a def's body occurrence. (The
/// infer-side sibling of `lower::callee_def_index`; kept here so infer does not depend on lower.)
fn callee_def_index_for_infer(db: &mut Db, head: StructId) -> Option<usize> {
    let body = match crate::resolve::resolved_of(db, head) {
        crate::resolved::Resolved::Lambda { body, .. } => body,
        crate::resolved::Resolved::Ref { value } => return callee_def_index_for_infer(db, value),
        _ => return None,
    };
    db.defs.iter().position(|d| d.body == Some(body))
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
                    trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "apply: argument conflicts with parameter (type fault)");
                    out.push(reject);
                }
                cur = *result;
            }
            // Applying a non-function (too many args) — a shape fault reported as a type mismatch.
            other => {
                trace!(target: "rcdzc::infer", head = head.0, ty = %other.render_name(), "apply: applied a non-function (type fault)");
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
    trace!(target: "rcdzc::infer", node = id.0, faults = out.len(), "type check complete");
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
                trace!(target: "rcdzc::infer", node = id.0, cond_ty = %cond_ty.render_name(), "fault: if condition not Bool (CDZ0203)");
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!("if condition must be Bool, found {}", cond_ty.render_name()),
                ));
            }
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            if !then_ty.agrees_with(&else_ty) {
                trace!(target: "rcdzc::infer", node = id.0, then_ty = %then_ty.render_name(), else_ty = %else_ty.render_name(), "fault: if branches differ (CDZ0203)");
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
            let operand_is_poison = matches!(resolved_of(db, operand), Resolved::Poison(_));
            match crate::eval::member_value(db, operand, &key) {
                crate::eval::Member::Field(_) => {}
                crate::eval::Member::NoField => {
                    trace!(target: "rcdzc::infer", node = id.0, key = %key.name, "fault: record has no such field (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!("record has no field `{}`", key.name),
                    ))
                }
                crate::eval::Member::NotRecord if !operand_is_poison => {
                    trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: member access on a non-record (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!(
                            "member access requires a record, found {}",
                            type_of(db, operand).render_name()
                        ),
                    ))
                }
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
        // A match: every arm body must AGREE in type with the first (like an `if`'s branches), and the
        // scrutinee + bodies are checked for their own faults. (Exhaustiveness — a scalar match with no
        // covering arm — is a lowering decision for now; the arms-agree check is the type fault here.)
        Resolved::Match { scrutinee, arms } => {
            if let Some((_, first_body)) = arms.first() {
                let first_ty = type_of(db, *first_body);
                for (_, body) in arms.iter().skip(1) {
                    let bt = type_of(db, *body);
                    if !first_ty.agrees_with(&bt) {
                        out.push(Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "match arms differ: {} vs {}",
                                first_ty.render_name(),
                                bt.render_name()
                            ),
                        ));
                    }
                }
            }
            collect(db, scrutinee, out);
            for (_, body) in &arms {
                collect(db, *body, out);
            }
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
                // A bare integer LITERAL annotated with an integer type is a GROUNDING, not a
                // unification: the literal has no intrinsic signedness/width to conflict, so the
                // annotation fixes its type (`(: 200 UInt8)` : UInt8) subject only to a RANGE CHECK —
                // a literal outside the width is rejected (CDZ0302), never truncated. (This is
                // "Annotations Constrain": the annotation determines the literal's type rather than
                // clashing with the signed-64 default a bare literal would otherwise take.)
                if let (Resolved::Int(v), Ty::Int(it)) = (resolved_of(db, expr), &annot_ty) {
                    if let crate::ty::Width::Fixed(w) = it.width
                        && !v.fits_width(it.ground_signed(), w)
                    {
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(), "fault: literal does not fit annotated width (CDZ0302)");
                        out.push(Reject::coded(
                            Code::IntOutOfRange,
                            format!(
                                "integer literal does not fit the annotated type {}",
                                annot_ty.render_name()
                            ),
                        ));
                    }
                } else {
                    // A non-literal value has a real type that must AGREE with the annotation — unify,
                    // and report a genuine conflict (`(: true Int64)`) as CDZ0203.
                    let expr_ty = type_of(db, expr);
                    let mut subst = Subst::new();
                    if crate::unify::unify(&mut subst, &annot_ty, &expr_ty).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(), expr_ty = %expr_ty.render_name(), "fault: annotation type mismatch (CDZ0203)");
                        out.push(Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "annotation type {} does not match value type {}",
                                annot_ty.render_name(),
                                expr_ty.render_name()
                            ),
                        ));
                    }
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
                trace!(target: "rcdzc::infer", node = id.0, "fault: unbound name reported (CDZ0101)");
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

    // ── def_scheme (ANF step 2, sub-increment A1): a def's signature as a value ────────────────────
    //
    // These pin the FOUNDATION only: a fully-determined (annotated) def's scheme is its curried
    // signature and it AGREES with what β-reduction produces at a call; an undetermined (unannotated /
    // recursive) def declines the scheme (defers to β-reduction). No caller reads it yet.

    use crate::testkit::parse;

    /// The def index of `name` in a parsed program.
    fn def_of(db: &Db, name: &str) -> usize {
        db.def_by_name(name).expect("def present")
    }

    #[test]
    fn def_scheme_of_an_annotated_function_is_its_curried_signature() {
        // (def (add (: a Int64) (: b Int64)) (+ a b)) → Int64 -> Int64 -> Int64.
        let ast = parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "add");
        let scheme = def_scheme(&mut db, d).expect("determined scheme");
        let expected = Ty::Fn(
            Box::new(Ty::int64()),
            Box::new(Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()))),
        );
        assert!(
            scheme.ty.agrees_with(&expected),
            "add scheme {} != Int64->Int64->Int64",
            scheme.ty.render_name()
        );
    }

    #[test]
    fn def_scheme_agrees_with_beta_reduction_at_a_call() {
        // The scheme's RESULT (after applying both args) must equal the type β-reduction gives the
        // call `(add 20 22)` — the cross-check that the scheme is a faithful stand-in for inlining.
        let ast = parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) (add 20 22)) (export main))",
        );
        let mut db = Db::load(ast);
        // β-reduction path: type of the call node in main's body.
        let main_body = db.defs[def_of(&db, "main")].body.expect("main body");
        let call_ty = type_of(&mut db, main_body);
        // scheme path: peel the two Fn arrows.
        let d = def_of(&db, "add");
        let scheme = def_scheme(&mut db, d).expect("determined scheme");
        let result = match scheme.ty {
            Ty::Fn(_, r) => match *r {
                Ty::Fn(_, r2) => *r2,
                other => other,
            },
            other => other,
        };
        assert!(
            call_ty.agrees_with(&result),
            "β-reduced call type {} disagrees with scheme result {}",
            call_ty.render_name(),
            result.render_name()
        );
    }

    #[test]
    fn def_scheme_of_a_nullary_def_is_its_body_type() {
        let ast = parse("(module m (def (main) 42) (export main))");
        let mut db = Db::load(ast);
        let d = def_of(&db, "main");
        let scheme = def_scheme(&mut db, d).expect("nullary scheme");
        assert!(scheme.ty.agrees_with(&Ty::int64()));
    }

    #[test]
    fn def_scheme_declines_an_unannotated_parameter() {
        // `(def (id x) x)` — `x` is unannotated (`Any`), so its signature needs the connected solve
        // (A2). A1 declines the scheme and defers to β-reduction.
        let ast = parse("(module m (def (id x) x) (def (main) (id 5)) (export main))");
        let mut db = Db::load(ast);
        let d = def_of(&db, "id");
        assert!(
            def_scheme(&mut db, d).is_none(),
            "an unannotated param must defer to β-reduction (A2 territory)"
        );
    }

    #[test]
    fn def_scheme_of_an_annotated_recursive_def_is_determined_by_absorption() {
        // KEY FINDING (shrinks A2): an ANNOTATED recursive def types WITHOUT an explicit fixpoint. The
        // self-call `(sum-to …)` returns `Any` (the recursion guard in `apply_type`), and `Any` is
        // ABSORBED by unification/join with the concrete parts — the base case `0` and `(+ n …)` pin
        // the result to Int64. So `def_scheme(sum-to)` = Int64 -> Int64 already, and this is
        // order-independent (the self-call is always `Any` regardless of visit order; the concrete
        // branch determines the type). An explicit recursion fixpoint (A2) is only needed when NO
        // concrete part pins the result — which, for terminating monomorphic recursion, cannot happen
        // (there must be a base case, and it pins the type).
        let ast = parse(
            "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "sum-to");
        let scheme = def_scheme(&mut db, d).expect("annotated recursive def types via absorption");
        let expected = Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()));
        assert!(
            scheme.ty.agrees_with(&expected),
            "sum-to scheme {} != Int64->Int64",
            scheme.ty.render_name()
        );
    }

    #[test]
    fn def_scheme_of_an_unannotated_recursive_def_is_solved_by_a2() {
        // The ACTUAL corpus `sum-to` has an UNANNOTATED param `(def (sum-to n) …)`. A2's connected solve
        // (`solve_recursive_params`) infers `n : Int64` from its uses (`(= n 0)`, `(+ n …)`), so the def
        // now HAS a scheme `Int64 -> Int64` — where before A2 it declined. (The mechanism the recursive
        // corpus rides.)
        let ast = parse(
            "(module m (def (sum-to n) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "sum-to");
        let scheme = def_scheme(&mut db, d).expect("A2 solves the unannotated recursive signature");
        let expected = Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()));
        assert!(
            scheme.ty.agrees_with(&expected),
            "sum-to scheme {} != Int64->Int64",
            scheme.ty.render_name()
        );
    }

    #[test]
    fn recursive_param_solve_is_order_independent() {
        // The NON-NEGOTIABLE property (build-order Stage 2 "done when"; the coarse-kind post-mortem): a
        // recursive def's parameter type is the SAME regardless of which node's type is demanded first.
        // Solve `sum-to`'s param via two different first-demands and assert they agree.
        let src = "(module m (def (sum-to n) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)) (export main))";

        // Order A: demand the def's scheme first (drives the solve from the signature).
        let mut db_a = Db::load(parse(src));
        let da = def_of(&db_a, "sum-to");
        let sa = def_scheme(&mut db_a, da).expect("scheme A");

        // Order B: demand `main`'s body type first (drives the solve from the call site), THEN the scheme.
        let mut db_b = Db::load(parse(src));
        let main_b = db_b.defs[def_of(&db_b, "main")].body.expect("main body");
        let _ = type_of(&mut db_b, main_b);
        let db_b_def = def_of(&db_b, "sum-to");
        let sb = def_scheme(&mut db_b, db_b_def).expect("scheme B");

        assert_eq!(
            sa.ty.render_name(),
            sb.ty.render_name(),
            "recursive param solve must be order-independent"
        );
    }
}
