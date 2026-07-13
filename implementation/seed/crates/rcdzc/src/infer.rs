//! `infer` — the query that fills the type column: for a node's `StructId`, its solved [`Ty`].
//!
//! One concern: type determination. [`type_of`] solves one node's type, reading its resolved form
//! (via [`crate::resolve::resolved_of`], which fills the resolved column on demand) and, for a
//! compound node, its children's types (each a lazy `type_of`). It memoizes into `db.types` and is
//! the ONLY module that fills it (`reference-compiler.md` §Types Are Solved Once And Read
//! Downstream). Asking one node's type solves only the nodes that answer reaches — the query stays
//! demand-driven.
//!
//! A literal's type is a signed integer of DEFERRED width and sign (numeric-literal polymorphism —
//! inference or the backend grounds it), a boolean is `Bool`, an `if` is the join of its branches, and
//! a poison is `Ty::Any` (compatible with everything, so a "no" never cascades into a spurious
//! mismatch). Polymorphism is real Hindley-Milner: an operator's scheme is instantiated with fresh
//! variables and its operands UNIFIED (via [`crate::unify`]) — a generic operation, a recursive def's
//! parameters ([`solve_recursive_params`]), and an annotation constraint all solve through the one
//! unify seam, not a per-node coarse rule.
//!
//! [`type_errors`] is a SEPARATE query — a read over the (demand-filled) type column that reports
//! type-agreement faults (an `if` whose condition is not `Bool`, or whose branches disagree). Keeping
//! the fault check apart from the value fill is what lets filling a type never reject and a later
//! rewrite preserve both the value and its checks (`reference-compiler.md` §A Meaning-Preserving
//! Rewrite Preserves Value And Checks).

use crate::arena::Slot;
use crate::ast::StructId;
use crate::db::Db;
use crate::diag::{Code, Fix, Reject};
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
    // Recursive-descent DEPTH GUARD (shared with `collect` and `core_of`). `compute` re-enters
    // `type_of` for sub-expressions, so pathologically deep input would recurse until the native stack
    // overflows and the process ABORTS. Past the limit, type as `Any` (unknown, compatible with
    // everything) — the fault walk's own guard reports the resource-limit decline, and a compiler must
    // never crash on well-formed input. Not memoized (like the provisional `Any` below): a node typed
    // at a shallower depth still gets its real type. See `db::DESCENT_DEPTH_LIMIT`.
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::infer", node = id.0, "type depth limit hit → Any (resource limit)");
        return Ty::Any;
    }
    db.descent_depth += 1;
    let t = compute(db, id);
    db.descent_depth -= 1;
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
        Resolved::Str(_) => Ty::String,
        Resolved::Bytes(_) => Ty::Bytes,
        // A char literal (`#\a`) is the monomorphic `Ty::Char`.
        Resolved::Char(_) => Ty::Char,
        // A `(bin …)` in value position CONSTRUCTS a byte sequence → `Ty::Bytes`.
        Resolved::Bin { .. } => Ty::Bytes,
        // A `bin` PATTERN binder: an integer segment decodes an `Int`, a `bytes` segment a `Bytes`.
        Resolved::BinField {
            segs, seg_index, ..
        } => match segs.get(seg_index).map(|s| &s.kind) {
            Some(crate::resolved::SegKind::Int { .. } | crate::resolved::SegKind::Bits { .. }) => {
                Ty::int()
            }
            Some(crate::resolved::SegKind::Bytes { .. }) => Ty::Bytes,
            None => Ty::Any,
        },
        // A MAP PATTERN binder: a VALUE binder (`key = Some`) has the map's VALUE type; the REST binder
        // (`key = None`) has the whole MAP type (the scrutinee minus the named keys — same `Map<K,V>`).
        // Both read off the scrutinee's solved `Ty::Map(k, v)`.
        Resolved::MapField { scrutinee, key, .. } => match type_of(db, scrutinee) {
            Ty::Map(k, v) => {
                if key.is_some() {
                    (*v).clone() // a value binder holds the value type
                } else {
                    Ty::Map(k, v) // the rest binder holds the map type
                }
            }
            _ => Ty::Any,
        },
        // A float literal's width is DEFERRED — it grounds to `Float64` unless an annotation or a float
        // operator's signature fixes it (`(: 3.5 Float32)`), mirroring a bare integer literal's width.
        Resolved::Float(_) => Ty::float(),
        Resolved::Unit => Ty::Unit,
        // A name IS its bound value's type — follow the ref (a lazy `type_of` on the value occurrence).
        Resolved::Ref { value } => type_of(db, value),
        // A `let`'s type is its body's type (the bindings are compile-time structure that folds away).
        Resolved::Let { body, .. } => type_of(db, body),
        // A VARIANT CONSTRUCTOR record carrying `(meta variant)` is a sum value/constructor, not a plain
        // data record. Its type is the constructor's `(meta t)`: for a NULLARY variant that is the sum
        // itself (a bare `None` is a VALUE of the sum — `Ty::Sum`), for a PAYLOAD variant it is the
        // curried arrow `(-> P Sum)` (a function value, applied to construct). Reading `(meta t)` as the
        // scheme and taking its type is the same path an operator value would take. This case comes
        // BEFORE the type-value check so a nullary variant is not misread as `Ty::Type`.
        Resolved::Record { .. } if crate::eval::variant_disc_of(db, id).is_some() => {
            let mut fresh = crate::unify::Fresh::new();
            match crate::eval::scheme_of(db, id, &mut fresh) {
                Some(scheme) => scheme.ty,
                None => Ty::Any,
            }
        }
        // An OPERATOR record used as a bare VALUE (not as an application head) — it carries a `(meta
        // apply)` primitive AND a `(meta t)` scheme, so its type is that scheme instantiated. This is what
        // types `Map.empty` (a nullary value operator, `∀k v. (Map k v)`) when it flows into an argument
        // position (`(Map.insert Map.empty …)`), and a bare operator passed as a HOF argument (its arrow
        // type). An APPLIED operator never reaches here — `apply_type` reads its scheme directly off the
        // head — so this only fires for a bare use. Checked before the type-value branch (an op record is
        // not a type-value: it has a `(meta apply)`, so `typeval_of` declines it) and the plain-record
        // branch (which would wrongly type it as `(record (apply …) (t …))`).
        Resolved::Record { .. }
            if crate::eval::meta_apply_of(db, id).is_some()
                && crate::eval::variant_disc_of(db, id).is_none() =>
        {
            let mut fresh = crate::unify::Fresh::new();
            match crate::eval::scheme_of(db, id, &mut fresh) {
                Some(scheme) => crate::unify::instantiate(&scheme, &mut fresh),
                None => Ty::Any,
            }
        }
        // A record that IS a type — a ground-type record (`Bool`), a built integer module, or any
        // record carrying a `(meta t)` — is a type VALUE, so its type is `Type`. Otherwise a plain
        // data record's type is the record of its fields' types (each a lazy `type_of`).
        Resolved::Record { fields } => {
            if crate::eval::typeval_of(db, id).is_some() {
                Ty::Type
            } else {
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    let t = type_of(db, value);
                    field_tys.insert(label.clone(), t);
                }
                Ty::Record(std::sync::Arc::new(field_tys))
            }
        }
        // Member access — the field's type is the type of the field's VALUE, found by reducing the
        // operand to a record and projecting (the one projection, via the evaluator, so it works off a
        // literal record, a `let`-bound one, OR a module a type constructor built like `(Int 64)`). A
        // non-record operand or an absent field has no field type — typed `Any` here so it does not
        // cascade; the actual fault (CDZ0201) is reported by `type_errors`.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => type_of(db, value),
            // The operand does not reduce to a compile-time-visible record (a call result, an `if`
            // selection), but its TYPE may be a record carrying the field — a RUNTIME record. Its
            // field type is that field's type in the record type (the runtime read's result type),
            // mirroring how a tuple projection reads its element type off `Ty::Tuple`.
            _ => match type_of(db, operand) {
                Ty::Record(fields) => fields.get(&key).cloned().unwrap_or(Ty::Any),
                _ => Ty::Any,
            },
        },
        // A tuple's type is the tuple of its elements' types, in position order (each a lazy `type_of`).
        // Arity + element types ARE the type.
        Resolved::Tuple { elems } => Ty::Tuple(elems.iter().map(|&e| type_of(db, e)).collect()),
        // A list's type is `List <elem>` where `<elem>` is the JOIN of the element types (like a match's
        // arm-join — every element shares one type; a deferred/`Any` element yields the others). An empty
        // list is `List Any` (a deferred element, solved by unification against its use). The homogeneity
        // CHECK (a mixed list is CDZ0203) is `type_errors`' job; this fills the value column.
        Resolved::List { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                elem_ty = elem_ty.join(&type_of(db, e));
            }
            Ty::List(Box::new(elem_ty))
        }
        // A map literal's type is `Map <key> <value>` where `<key>` is the JOIN of the entry key types
        // and `<value>` the JOIN of the entry value types (each homogeneous — a mixed-key or mixed-value
        // map is CDZ0201, the CHECK is `type_errors`' job; this fills the value column). An empty `(map)`
        // is `Map Any Any` (deferred, solved by unification against a use). A map's KEY SET is NOT part
        // of its type — only the key TYPE is (`Map<K,V>`).
        Resolved::Map { entries } => {
            let mut key_ty = Ty::Any;
            let mut val_ty = Ty::Any;
            for &(k, v) in entries.iter() {
                key_ty = key_ty.join(&type_of(db, k));
                val_ty = val_ty.join(&type_of(db, v));
            }
            Ty::Map(Box::new(key_ty), Box::new(val_ty))
        }
        // (Arc<[Ty]> collects directly from the element iterator — a refcounted immutable slice.)
        // A tuple projection's type is the operand tuple's element type AT `index`. An operand that is
        // not a tuple, or an index outside its arity, has no element type — typed `Any` here so it does
        // not cascade; the actual fault (CDZ0201) is reported by `type_errors`.
        Resolved::Proj { operand, index } => match type_of(db, operand) {
            Ty::Tuple(elems) => elems.get(index).cloned().unwrap_or(Ty::Any),
            _ => Ty::Any,
        },
        // A sum-variant pattern's payload binder — its type is the variant's PAYLOAD type AT THE
        // SCRUTINEE'S INSTANTIATION. The ctor `(. Sum V)`'s scheme is `∀a. a → Option a`; instantiating
        // it gives `?0 → Option ?0`, and unifying the RESULT `Option ?0` against the scrutinee's solved
        // type `Option Int64` solves `?0 = Int64` — so `(match (s : Option Int64) ((Some x) …))` types
        // `x` as `Int64`, not a free var. For a MONOMORPHIC sum the scheme has no vars, so the payload is
        // read directly. `Any` if the head is not a single-payload variant (a fault the match reports).
        Resolved::SumPayload {
            scrutinee,
            steps,
            heads,
        } => {
            // Walk the scrutinee's solved type down the access PATH. A `Payload` step descends into a
            // variant's payload: the next sub-value's type is that variant's payload AT THE CURRENT
            // instantiation (`payload_ty_at_instantiation` unifies the head's `(-> payload Sum)` result
            // against the current type). `heads` supplies the variant head at each `Payload` step, in
            // order (a queue — one head per Payload step). An `Elem(i)` step descends into a tuple element
            // (a variant whose payload is a tuple, destructured by a nested `(tuple …)` pattern): the next
            // type is the tuple's i-th element. A nested `(Some (Some y))` on `Option (Option Int64)`
            // walks two Payload steps; `(Exp.Add (tuple a b))` walks `[Payload, Elem(0/1)]`.
            let mut cur = type_of(db, scrutinee);
            let mut heads = heads.iter();
            for step in steps.iter() {
                cur = match step {
                    crate::core::PathStep::Payload => {
                        let Some(&head) = heads.next() else {
                            return Ty::Any; // malformed path (fewer heads than Payload steps)
                        };
                        match payload_ty_at_instantiation(db, head, &cur) {
                            Some(t) => t,
                            None => return Ty::Any,
                        }
                    }
                    crate::core::PathStep::Elem(i) => match &cur {
                        Ty::Tuple(elems) => match elems.get(*i) {
                            Some(t) => t.clone(),
                            None => return Ty::Any,
                        },
                        // A list-pattern element binder — every element of a `List T` has type `T`
                        // (homogeneous), regardless of the index.
                        Ty::List(elem) => (**elem).clone(),
                        _ => return Ty::Any,
                    },
                };
            }
            cur
        }
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
        // A boolean connective is a Bool. Reading the operands' types is the backward demand; the
        // operands-are-Bool CHECK is `type_errors`' job (this fills the value column).
        Resolved::And { lhs, rhs, .. } => {
            let _l = type_of(db, lhs);
            let _r = type_of(db, rhs);
            Ty::Bool
        }
        Resolved::Not { operand } => {
            let _o = type_of(db, operand);
            Ty::Bool
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
        // EFFECT CONTROL FORMS. A `handle` evaluates to the value its FOLDED body produces (each perform
        // resolved to its arm's resume value, state threaded away), so its type is the type of that
        // rewritten body — reduce the handler and type the result. This is what lets a state-threading
        // body whose surface uses a nested `(do …)` (which resolve does not model as an expression, so
        // the ORIGINAL body types as `Any`) still get a definite type from its reduced form. If the fold
        // declines (a case the tail path cannot serve), fall back to the original body's type — harmless,
        // since lowering will decline it anyway.
        Resolved::Handle { init, arms, body } => {
            match crate::effects::reduce_handle(db, init, &arms, body) {
                Some(rewritten) => type_of(db, rewritten),
                None => type_of(db, body),
            }
        }
        // A `host` evaluates to its body's value (E2 handles the boundary). A `resume`'s value is handed
        // back at the perform site; outside the tail-rewrite it has no independent type, so `Any`.
        Resolved::Host { body, .. } => type_of(db, body),
        Resolved::Resume { .. } => Ty::Any,
        // A lambda/def parameter used as a value — a formal. If its binder is ANNOTATED (`(: a T)`),
        // its type is that annotation `T` — so the body type-checks against a definite parameter type
        // (`(: a Bool)` used as an integer operand is caught). Otherwise, for the parameter of a
        // RECURSIVE def (which cannot inline, so its type must be inferred rather than flowing from a
        // call site), the CONNECTED def-body solve (`solve_recursive_params`, A2) infers it from its
        // uses; a still-`None` result means either a non-recursive param (typed `Any` — it inlines at
        // its call site, where the argument's type flows in via the fold) or an unconstrained one.
        Resolved::Param { binder } => param_annot_ty(db, binder)
            .or_else(|| crate::effects::handle_arm_param_ty(db, binder))
            .or_else(|| solved_param_ty(db, binder))
            .unwrap_or(Ty::Any),
        // A TYPE value is a value, so it has a type — `Type` (the type of types). A bare type value
        // (a `(typeval …)` node, OR a value the evaluator reduces to a type) types as `Type`; this is
        // what makes a type first-class (it can be passed, returned, checked).
        Resolved::TypeVal(_) => Ty::Type,
        // A lambda's type is its function type `param → … → result` — each parameter's type (its
        // annotation, or `Any` for a bare param) curried onto the body's type. Typing a lambda as its
        // arrow type (rather than the opaque `Any` it used to be) is what lets a HIGHER-ORDER call
        // check the passed function against a `(-> A B)` parameter annotation: unifying the argument's
        // `Fn(A', B')` against the declared `Fn(A, B)` catches a RESULT-type mismatch (`(-> Int Bool)`
        // vs an `Int → Int` lambda) — structurally, so a nested/curried arrow is checked all the way
        // down. A bare-param lambda contributes `Any` in its parameter slot, so it still unifies with
        // any expected parameter type (no over-rejection); only a definite result disagreement faults.
        Resolved::Lambda { params, body } => {
            let result = type_of(db, body);
            params.iter().rev().fold(result, |acc, &p| {
                let pt = type_of(db, crate::eval::param_name_occ(db, p));
                Ty::Fn(Box::new(pt), Box::new(acc))
            })
        }
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

/// Faults in a DEF PARAMETER's annotation `(: name T)` — the signature-side companion of the value
/// annotation checked in `collect_node`. A parameter's TYPE OPERAND `T` must denote a TYPE; a non-type
/// (an unbound name, a value, a malformed type application `(Int64 Int64)`) is REJECTED, not
/// dropped-and-typed-`Any` (the same drop-instead-of-reject gap the value-annotation form had). `param`
/// is a signature parameter occurrence — a bare name (no annotation, no fault) or a `(: name T)` binder.
/// Reported from `compile::collect_faults` (which walks each def's params); a def's body is checked
/// separately, and a bare param never reaches here with a fault.
pub fn param_annotation_faults(db: &mut Db, param: StructId, out: &mut Vec<Reject>) {
    // Only an ANNOTATED param `(: name T)` has a type operand to validate.
    let Some(tail) = db.ast.as_form(param, ":").map(|t| t.to_vec()) else {
        return;
    };
    if tail.len() != 2 {
        return;
    }
    let ty_expr = tail[1];
    // A RUNTIME WIDTH `(: n (UInt m))` with a runtime `m` is its own CDZ0302 (surfaced where the
    // annotation is used in the body); do not also fault it here as "not a type".
    if crate::eval::is_runtime_width_type(db, ty_expr) {
        return;
    }
    // The operand denotes a type → fine. Otherwise reject, exactly as the value-annotation form does:
    // collect the operand's OWN faults (an unbound name → CDZ0101), and if none surfaced (a well-formed
    // non-type: a literal, a compound, `(Int64 Int64)`), add an "expected a type" TypeMismatch (CDZ0203).
    if crate::eval::typeval_of(db, ty_expr).is_none() {
        let before = out.len();
        collect(db, ty_expr, out);
        if out.len() == before {
            trace!(target: "rcdzc::infer", param = param.0, "fault: parameter annotation type is not a type (CDZ0203)");
            out.push(
                Reject::coded(
                    Code::TypeMismatch,
                    "a parameter's annotation requires a type, but found a non-type",
                )
                .at(ty_expr),
            );
        }
    }
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

/// The type of the `k`-th parameter of def `callee`, for constraining an argument passed there from
/// another def's recursive-param solve. Sources, in order: (a) an explicit ANNOTATION on the param; (b)
/// a param whose type is already solved in `db.param_types` (a recursive callee, or one solved earlier);
/// (c) otherwise, collect the callee's OWN body constraints (the same operator/if/self-call collection
/// `solve_recursive_params` runs) over a fresh env and read the k-th param's solved type — this pins a
/// NON-recursive unannotated helper's param (`byte-at`'s `b` ⇒ `Bytes` via `(Bytes.at b i)`) without
/// requiring the helper to have a standalone scheme. Returns `None` (no constraint) if the param stays
/// undetermined or `callee` has no such param. Guarded against re-entry (a cycle) by `db.solving_params`.
fn callee_param_ty(db: &mut Db, callee: usize, k: usize) -> Option<Ty> {
    let params = db.defs[callee].params.clone();
    let p = *params.get(k)?;
    let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
        Some(name_occ) => name_occ,
        None => p,
    };
    if let Some(t) = param_annot_ty(db, binder) {
        return Some(t);
    }
    if let Some(t) = db.param_types.get(&binder) {
        return Some(t.clone());
    }
    if db.solving_params.contains(&callee) {
        return None;
    }
    let body = db.defs[callee].body?;
    db.solving_params.insert(callee);
    let mut fresh = Fresh::new();
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
    let mut binders: Vec<(StructId, Ty)> = Vec::new();
    for pp in &params {
        let b = match db.ast.as_form(*pp, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *pp,
        };
        let var = param_annot_ty(db, b).unwrap_or_else(|| Ty::Var(fresh.var()));
        env.insert(b, var.clone());
        binders.push((b, var));
    }
    let mut subst = Subst::new();
    collect_param_constraints(db, body, &env, callee, &mut subst, &mut fresh);
    db.solving_params.remove(&callee);
    let solved = ground_param(subst.apply(&binders.get(k)?.1));
    match solved {
        Ty::Any | Ty::Var(_) => None,
        other => Some(other),
    }
}

/// Solve the machine type of a LAMBDA parameter `binder` from its uses in the lambda `body` — the
/// lambda analogue of [`solve_recursive_params`], for a lifted closure whose parameter is UNANNOTATED
/// (a bare `(fn (x) …)`). A bare lambda types with a fresh variable at its own occurrence (inference
/// does not thread the use-site arrow back onto it), so `type_of(binder)` is `Any`; but the body's
/// operations DO constrain it — `(* x 2)` pins `x` to an integer. Give the param a fresh variable,
/// walk the body collecting the operand constraints its uses impose (the same `collect_param_constraints`
/// the recursive-def solve uses, with a sentinel def index that no call matches — a lambda body has no
/// self-recursion to a def), then ground the solved variable (a numeric use defaults to Int64, an
/// unconstrained variable stays `Any` → the caller declines rather than invent a width). Returns the
/// grounded type. This is a read-only solve over a fresh `Subst` — it does NOT touch `db.param_types`
/// (a lambda param is not a def param), so it is safe to call at lowering.
pub fn solve_lambda_param_ty(db: &mut Db, binder: StructId, body: StructId) -> Ty {
    let mut fresh = Fresh::new();
    let var = param_annot_ty(db, binder).unwrap_or_else(|| Ty::Var(fresh.var()));
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
    env.insert(binder, var.clone());
    let mut subst = Subst::new();
    // A sentinel def index no `callee == def` comparison matches (a lambda body has no self-recursive
    // call to a `db.defs` entry — its own applications resolve to the param or to real defs).
    collect_param_constraints(db, body, &env, usize::MAX, &mut subst, &mut fresh);
    ground_param(subst.apply(&var))
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
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
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
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
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
                for &arg in args.iter() {
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
                // this def's own parameter variable — the fixpoint.
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
                } else {
                    // A call to ANOTHER def: a parameter passed as the k-th argument is constrained by the
                    // callee's k-th PARAMETER TYPE. Without this, a parameter used ONLY as a call argument
                    // (`(byte-at b i)` — `b` never touched by an operator) stays a free `Var`, grounds to
                    // `Any`, and the recursive-def guard declines a well-typed program. The callee's own
                    // body pins its param type (`byte-at`'s `b` is `Bytes` via `(Bytes.at b i)`), so read
                    // the callee's k-th param type and unify. Only a DETERMINED callee param (not
                    // `Any`/`Var`) constrains — an undetermined one adds nothing, so a genuinely
                    // polymorphic position is never over-constrained.
                    for (i, &arg) in args.iter().enumerate() {
                        let arg_is_param = matches!(
                            resolved_of(db, arg),
                            Resolved::Ref { value } if env.contains_key(&value)
                        ) || matches!(
                            resolved_of(db, arg),
                            Resolved::Param { binder } if env.contains_key(&binder)
                        );
                        if !arg_is_param {
                            continue;
                        }
                        if let Some(pt) = callee_param_ty(db, callee, i)
                            && !matches!(pt, Ty::Any | Ty::Var(_))
                        {
                            let at = arg_ty_in_env(db, arg, env, subst);
                            let _ = crate::unify::unify(subst, &at, &pt);
                        }
                    }
                }
            }
            // Descend into the head (a computed head) and every argument for THEIR own constraints.
            if matches!(resolved_of(db, head), Resolved::Apply { .. }) {
                collect_param_constraints(db, head, env, def, subst, fresh);
            }
            for &arg in args.iter() {
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
        // A boolean connective's operands must be Bool — constrain each (a bare-param operand `(and p …)`
        // pins `p` Bool), then descend.
        Resolved::And { lhs, rhs, .. } => {
            for &op in &[lhs, rhs] {
                let t = arg_ty_in_env(db, op, env, subst);
                let _ = crate::unify::unify(subst, &t, &Ty::Bool);
                collect_param_constraints(db, op, env, def, subst, fresh);
            }
        }
        Resolved::Not { operand } => {
            let t = arg_ty_in_env(db, operand, env, subst);
            let _ = crate::unify::unify(subst, &t, &Ty::Bool);
            collect_param_constraints(db, operand, env, def, subst, fresh);
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
        Resolved::Proj { operand, .. } => {
            collect_param_constraints(db, operand, env, def, subst, fresh)
        }
        Resolved::Record { fields } => {
            for value in fields.values() {
                collect_param_constraints(db, *value, env, def, subst, fresh);
            }
        }
        Resolved::Tuple { elems } => {
            for &e in elems.iter() {
                collect_param_constraints(db, e, env, def, subst, fresh);
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
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
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
/// This computes the scheme for a def
/// whose signature is DETERMINED: an annotated/exported function (its params have definite machine
/// types and the body types reading them — the scheme AGREES with what β-reduction produces at a call,
/// cross-checked in tests), OR an unannotated RECURSIVE def whose parameters the connected solve
/// (`solve_recursive_params`, A2) has pinned. It returns `None` — deferring to β-reduction typing —
/// only when a parameter stays undetermined (`Any`, no use constrained it). This scheme is READ at a
/// runtime call site: `lower` emits a `Core::Call` to a recursive callee whose scheme is `Some`, and
/// `infer` types a recursive call by it.
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
/// The type of a compound-VALUE constructor (`tuple`/`record`/`list` alias) applied to `args` — a
/// compound VARIADIC in the arguments, so it cannot be a fixed `(meta t)` scheme (unlike a sum variant's
/// arrow). A tuple is the product of the arg types; a record maps each `(key value)` pair's key to its
/// value type; a list is `List <elem>` where `<elem>` is the JOIN of the argument types (homogeneous,
/// `Any` for the empty list). Typed the same way the symbol-headed `Resolved::Tuple`/`Record`/`List`
/// forms are, so the name-alias application and the symbol primitive agree.
fn compound_ctor_type(db: &mut Db, prim: crate::resolved::Prim, args: &[StructId]) -> Ty {
    use crate::resolved::Prim;
    match prim {
        Prim::TupleNew => Ty::Tuple(args.iter().map(|&e| type_of(db, e)).collect()),
        Prim::RecordNew => match crate::resolve::read_record_fields(db, args) {
            Ok(fields) => {
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    field_tys.insert(label.clone(), type_of(db, value));
                }
                Ty::Record(std::sync::Arc::new(field_tys))
            }
            // A malformed field list has no well-formed record type — `Any`; the fault is reported by
            // `type_errors`.
            Err(_) => Ty::Any,
        },
        Prim::ListNew => {
            let mut elem_ty = Ty::Any;
            for &e in args {
                elem_ty = elem_ty.join(&type_of(db, e));
            }
            Ty::List(Box::new(elem_ty))
        }
        _ => Ty::Any,
    }
}

fn apply_type(db: &mut Db, head: StructId, args: &[StructId]) -> Ty {
    // CASE-OF-CASE (matches `lower`): a head that reduces to a runtime `if` — `((if c a b) args…)` —
    // types as the `if` of the two branch applications. Each branch's lambda applies (β-reduces) to a
    // concrete result type, so the application's type is that (`Int64`), NOT `Ty::Fn` (the naive type
    // of the `if`, which would then have no machine representation at the boundary). Type each branch
    // applied to the same args and JOIN — an `if`'s two branches must agree, so either branch's type is
    // the result; take the then-branch's (the else must unify with it, checked at the `if`'s own node).
    if let Some((_cond, then_head, else_head)) = crate::eval::reduce_to_if(db, head) {
        let then_ty = apply_type(db, then_head, args);
        if !matches!(then_ty, Ty::Any) {
            return then_ty;
        }
        // The then-branch didn't determine a type (recursive/undetermined); fall back to the else.
        return apply_type(db, else_head, args);
    }
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
    // A compound-VALUE constructor (the `tuple`/`record`/`list` alias) applied — its type is the compound
    // of the argument types, even at ZERO arguments (an empty `(list)` / `(tuple)` is a valid empty
    // compound, NOT the ctor record). This is checked BEFORE the zero-arg identity short-circuit below so
    // `(list)` types as `List Any` rather than as the `list` ctor record's type. (The TupleNew/RecordNew/
    // ListNew arms are the ones that can be nullary; a scheme-typed head falls through.)
    if let Some(
        prim @ (crate::resolved::Prim::TupleNew
        | crate::resolved::Prim::RecordNew
        | crate::resolved::Prim::ListNew),
    ) = crate::eval::meta_apply_of(db, head)
    {
        return compound_ctor_type(db, prim, args);
    }
    // The `map` VALUE-constructor alias applied — `(map (k v) …)` written as a bare NAME head. Its `args`
    // are the ENTRY-PAIR nodes (each a two-element `(key value)` list), NOT curried arguments — so type
    // it as `Map <join keys> <join values>` DIRECTLY (like the `list` alias types `List <join elems>`),
    // never peeling the pairs as a curried application (which would wrongly type `(a 1)` as applying `a`).
    // An empty `(map)` is `Map Any Any`. Homogeneity is `type_errors`' job; this fills the value column.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapNew) {
        let mut key_ty = Ty::Any;
        let mut val_ty = Ty::Any;
        for &entry in args {
            // Read the entry's RAW `(key value)` children — a map entry is structure, not an application
            // (`(a 1)` is the pair a↦1, not `a` applied to `1`), so read the AST list directly.
            if let crate::ast::Struct::List(items) = db.ast.get(entry)
                && items.len() == 2
            {
                let (k, v) = (items[0], items[1]);
                key_ty = key_ty.join(&type_of(db, k));
                val_ty = val_ty.join(&type_of(db, v));
            }
            // A malformed entry (not a 2-element list) — the fault is reported elsewhere; the map's
            // key/value stay whatever the well-formed entries determined.
        }
        return Ty::Map(Box::new(key_ty), Box::new(val_ty));
    }
    // A NULLARY PERFORM `(E.op)` — an effect operation applied to no argument. Its `(meta t)` scheme is
    // `(-> Unit result)` (a `Unit`-domain op whose unit argument is elided in the corpus surface), so the
    // performance's type is `result`, NOT the op record's type (which the zero-arg identity short-circuit
    // below would wrongly return). Checked before that short-circuit, only for an effect operation, so an
    // ordinary nullary def `(g)` is unaffected. (A non-nullary op reaches the scheme-peeling loop below.)
    if crate::eval::effect_op_of(db, head).is_some() {
        let mut fresh = Fresh::new();
        if let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh) {
            let cur = crate::unify::instantiate(&scheme, &mut fresh);
            if let Ty::Fn(param, result) = &cur
                && (args.is_empty() && matches!(**param, Ty::Unit))
            {
                return (**result).clone();
            }
        }
    }
    // A ZERO-ARGUMENT application `(g)` with a non-lambda head is the head value — applying to no
    // arguments is the identity (a nullary def `(def (g) 7)` called). Its type is the head's type.
    // Mirrors the same short-circuit in `lower`, so `(g)` types and lowers as its body value. A
    // RECURSIVE nullary call (`(def (f) (f))`) has no normal form — type it `Any` (the fault is
    // reported by the collection side) rather than recursing into its own body without end.
    if args.is_empty() {
        if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
            && crate::eval::is_recursive(db, body)
        {
            return Ty::Any;
        }
        return type_of(db, head);
    }
    // A NULLARY variant CONSTRUCTOR applied to the unit value — `(None unit)` / `(Nil ())` — constructs
    // the sum (core-semantics.md §Construction MUST Be Via Application). Its ctor `(meta t)` is the bare
    // sum (no arrow — `variant_payload_type` is `None`), so the "peel a curried parameter per arg" loop
    // below would find a non-function and yield `Any`. The application's type IS the sum (the ctor's own
    // `(meta t)`); the unit argument is the payload, not a curried parameter. This is what lets a
    // `(match (None unit) …)` scrutinee type as the sum and route to the sum matcher.
    if crate::eval::variant_disc_of(db, head).is_some()
        && crate::eval::variant_payload_type(db, head).is_none()
    {
        return type_of(db, head);
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No `(meta t)` scheme. A RUNTIME FUNCTION VALUE head whose type is directly a `Ty::Fn` — a
        // closure bound out of a compound by a match binder (`(match t ((T.Mk f) (f 5)))`, where `f`
        // reads the `T.Mk` payload arrow), or a function-typed parameter — has no prelude scheme (a
        // `SumPayload`/`Proj`/`Param` binder is not a prelude entry), but its OWN type carries the arrow.
        // Peel one arrow per argument to get the result: `f : (-> Int64 Int64)` applied to one arg is
        // `Int64`. Without this the application typed `Any`, leaving the enclosing function's return type
        // non-machine ("function return type has no machine representation"). A prelude sum like `Some`
        // resolved WITHOUT this because its ctor scheme threads the payload type; a USER sum's
        // payload-bound closure relies on this arm. (Applied to more args than the arrow takes stops at
        // the non-function tail — a fault reported elsewhere.)
        None => {
            let head_ty = type_of(db, head);
            if matches!(head_ty, Ty::Fn(_, _)) {
                let mut cur = head_ty;
                for _ in args {
                    match cur {
                        Ty::Fn(_, result) => cur = *result,
                        _ => break,
                    }
                }
                return cur;
            }
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
                // FRESHEN the argument's free variables past the head's instantiation counter before
                // unifying: `type_of` types the arg with its OWN private `Fresh` (from 0), so an
                // under-constrained arg (a bare nullary variant `(None) : Option ?0`) shares variable
                // numbers with the head's instantiation (`Some : (-> ?0 (Option ?0))`). Without freshening
                // the two `?0`s alias and `?0 = Option ?0` trips the occurs-check, spuriously rejecting a
                // well-typed `(Some (None))`. Freshening makes them disjoint (`?0 = Option ?1`).
                let at = crate::unify::freshen_free(&type_of(db, arg), &mut fresh);
                // A unify failure here is a real type fault; ignore it for the VALUE (reported by
                // `type_errors`) and continue with the declared result so the shape stays sane.
                let _ = crate::unify::unify(&mut subst, &param, &at);
                cur = *result;
            }
            // Applied to more args than it takes — not a function; the fault is reported elsewhere.
            _ => return Ty::Any,
        }
    }
    // A NULLARY PERFORM: an effect operation declared `(-> Unit T)` performed as `(E.op)` (no argument —
    // the `unit` domain is elided, the corpus surface). After the given args are peeled, if the head is
    // an effect operation and `cur` is still `(-> Unit result)`, the elided unit is the implicit argument
    // — so the performance's type is `result`, not the un-applied arrow. Without this a nullary perform
    // types as its arrow (a `Type`), and using it as a value (`(+ (E.op) 1)`) faults "unify Int64 with
    // Type". (Only for an effect op — an ordinary partial application keeps its arrow type.)
    let applied = subst.apply(&cur);
    if crate::eval::effect_op_of(db, head).is_some()
        && let Ty::Fn(param, result) = &applied
        && matches!(**param, Ty::Unit)
    {
        return subst.apply(result);
    }
    applied
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
                // Freshen the arg's free variables past the scheme's instantiation before unifying (see
                // `apply_type`): an under-constrained arg types with its own from-0 `Fresh` and would
                // otherwise alias this scheme's variables into a spurious occurs-check cycle.
                let at = crate::unify::freshen_free(&type_of(db, arg), &mut fresh);
                let _ = crate::unify::unify(&mut subst, &param, &at);
                cur = *result;
            }
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
}

/// The PAYLOAD type of a variant `variant_head` at the scrutinee's instantiation `scrut_ty` — the type
/// a match-arm payload binder takes. The ctor's scheme is `∀params. payload… → Sum params`;
/// instantiate it, unify its RESULT (`Sum ?params`) against `scrut_ty` (a `Ty::Sum{args}` — the solved
/// scrutinee type) to solve the fresh params from the scrutinee's concrete args, then return the
/// (substituted) FIRST payload. So `(match (s : Option Int64) ((Some x) …))` reads `Some`'s payload as
/// `Int64`, not a free var. For a MONOMORPHIC ctor the scheme has no vars and the result unify is a
/// no-op, so the payload is the declared type directly. `None` if the ctor has no payload (a nullary
/// variant — no binding) or its scheme is not a single-payload arrow.
pub(crate) fn payload_ty_at_instantiation(
    db: &mut Db,
    variant_head: StructId,
    scrut_ty: &Ty,
) -> Option<Ty> {
    let mut fresh = Fresh::new();
    let scheme = crate::eval::scheme_of(db, variant_head, &mut fresh)?;
    let inst = crate::unify::instantiate(&scheme, &mut fresh);
    // The ctor type is a curried arrow `payload… → result`. Peel EVERY arrow: a single-payload variant
    // has one (`p → Sum`); a MULTI-payload variant curries (`p0 → p1 → Sum`) and its payloads are boxed
    // as ONE tuple handle (the same representation `(Cons (tuple p0 p1))` builds), so its bound payload
    // TYPE is the `Ty::Tuple(p0, p1, …)` that the `[Payload, Elem(i)]` access path indexes into.
    let mut payloads = Vec::new();
    let mut cur = inst;
    let result = loop {
        match cur {
            Ty::Fn(p, r) => {
                payloads.push(*p);
                cur = *r;
            }
            other => break other,
        }
    };
    if payloads.is_empty() {
        return None; // nullary (bare sum) — no payload to bind
    }
    // Solve the scheme's params from the scrutinee's concrete instantiation: unify the ctor's RESULT
    // (`Sum ?a`) against the scrutinee type (`Sum Int64`).
    let mut subst = Subst::new();
    let _ = crate::unify::unify(&mut subst, &result, scrut_ty);
    let payload = if payloads.len() == 1 {
        payloads.pop().unwrap()
    } else {
        Ty::Tuple(payloads.into())
    };
    Some(subst.apply(&payload))
}

/// If `expected` is a SUM type with a SINGLE-payload variant whose payload type (at `expected`'s
/// instantiation) agrees with `actual`, the variant's constructor NAME — the "try wrapping the
/// expression in `Some`" suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Derives the name GENERICALLY from the sum's declaration (which variant's payload fits),
/// never a hard-coded `Some`/`Ok` (the no-keys-outside-the-prelude rule). `None` when `expected` is not
/// a sum, or no variant's single payload matches — so a wrap is offered only when it would actually
/// resolve the mismatch. Variants are scanned in declaration order → deterministic.
fn wrap_variant_for(db: &mut Db, expected: &Ty, actual: &Ty) -> Option<String> {
    let Ty::Sum { decl, .. } = expected else {
        return None;
    };
    // Snapshot (name, ctor) pairs first — `payload_ty_at_instantiation` borrows `db` mutably, so we
    // cannot hold a `&TypeDecl` across the calls.
    let variants: Vec<(String, StructId)> = db
        .type_decl_by_occ(*decl)?
        .variants
        .iter()
        .filter_map(|v| v.ctor.map(|c| (v.name.clone(), c)))
        .collect();
    for (name, ctor) in variants {
        // A single-payload variant whose payload agrees with `actual` → wrapping the value in it yields
        // the expected sum. (A multi-payload variant's payload is a tuple, which a bare value never
        // matches, so it is naturally excluded.)
        if let Some(payload) = payload_ty_at_instantiation(db, ctor, expected)
            && payload.agrees_with(actual)
        {
            return Some(name);
        }
    }
    None
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
    db.def_index_by_body(body)
}

/// Check a handler arm's resume VALUE against the operation's declared RESULT type — the resume-value
/// companion of the perform-argument check. The value in `(resume value state)` is returned to the
/// perform site, so it must have the op's result type (`capabilities-and-effects.md` §Performing An
/// Operation Is Typed). A mismatch is CDZ0201. Only checks a TAIL resume in the arm body (the shipping
/// surface); an arm with no resume, or a non-tail resume, is out of scope (E4/E5) and not checked here.
fn check_resume_result_type(db: &mut Db, arm: &crate::resolved::HandleArm, out: &mut Vec<Reject>) {
    // The op's result type: instantiate the op value's `(meta t)` scheme (`(fn () (-> P… Result))`) and
    // peel to the final arrow result. `None` (a malformed op, or no scheme) → skip (its own fault surfaces).
    let mut fresh = Fresh::new();
    let Some(scheme) = crate::eval::scheme_of(db, arm.op, &mut fresh) else {
        return;
    };
    let mut result = crate::unify::instantiate(&scheme, &mut fresh);
    while let Ty::Fn(_, r) = result {
        result = *r;
    }
    // The resume value at the arm body's TAIL. The arm binds its params + state, but the RESUME VALUE's
    // type does not depend on those bindings here — its declared-type check is against a concrete op
    // result type, and a value like `true`/`42`/`(tuple …)` types independently of the (unsubstituted)
    // binders. So read the tail resume off the UN-substituted arm body and type its value.
    let Some(value) = tail_resume_value(db, arm.body) else {
        return; // no tail resume (abortive / non-tail) — out of scope for this check
    };
    let value_ty = type_of(db, value);
    if !value_ty.agrees_with(&result) {
        trace!(target: "rcdzc::infer", value = value.0, "fault: resume value's type does not match the operation's result type (CDZ0201)");
        out.push(
            Reject::coded(
                Code::Malformed,
                format!(
                    "a handler resumes with a value of type {} but the operation's result type is {}",
                    value_ty.render_name(),
                    result.render_name()
                ),
            )
            .at(value),
        );
    }
}

/// The VALUE of a tail `(resume value next-state)` in an arm body `node`, if the body IS such a resume at
/// the tail. `None` if the arm does not tail-resume (abortive, or a non-tail resume). Used by the
/// resume-value/result-type check; reads the ORIGINAL (un-substituted) arm body.
fn tail_resume_value(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Resume { value, .. } => Some(value),
        _ => None,
    }
}

/// Check an application for type faults — the ONE rule's fault side. Instantiate the head's scheme and
/// unify each argument into its curried parameter; a unify failure is the conflicting-use type error.
/// A head with no `(meta t)` scheme (a type constructor, or a not-yet-typed value) is not checked here.
/// Whether `ty` is a DEFINITE non-function type — a ground value type that can never be applied (a
/// scalar Int/Bool/Float/String/Bytes/Unit, or a structural Record/Tuple/Sum/List/Map/Set value). Used
/// to turn "applying a non-function" into a coded reject: only a type KNOWN not to be a function faults,
/// so an UNDETERMINED head (`Ty::Any` — a not-yet-modeled construct or an unresolved variable) is NOT
/// flagged (it falls through to a clean decline, never a spurious reject). `Ty::Fn` is applyable;
/// `Ty::Type` (a type-value) is a constructor-like value handled on its own paths, so it is excluded too.
fn is_definite_non_function(ty: &Ty) -> bool {
    match ty {
        // Applyable or undetermined — not a "definitely can't apply this" case.
        Ty::Fn(_, _) | Ty::Any | Ty::Var(_) | Ty::Type => false,
        // Every other ground/structural value type is a non-function.
        _ => true,
    }
}

fn check_application(db: &mut Db, head: StructId, args: &[StructId], out: &mut Vec<Reject>) {
    // A COMPARISON between a MAP and a RECORD is a type error — they are DISTINCT KINDS (a record's field
    // set is fixed by its form; a map's key set is a runtime collection), so `(= (map …) (record …))` is
    // CDZ0201, NOT the CDZ0203 that a same-kind SHAPE mismatch (two records with different fields) gets
    // (type-system.md §Structural Values Are Comparable Only When Their Shapes Match — the cross-kind
    // case; collections-and-text.md §A Map Associates Keys With Values). The generic scheme-unify below
    // would report `unify::mismatch` = CDZ0203; catch the map/record kind clash FIRST for the right code.
    // (Two maps of different key SETS do NOT reach here as a mismatch — they are the SAME `Map<K,V>` type,
    // so this fires only on a genuine map-vs-record kind clash.)
    if args.len() == 2
        && matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Compare
            )
        )
    {
        let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
        if matches!(
            (&a, &b),
            (Ty::Map(_, _), Ty::Record(_)) | (Ty::Record(_), Ty::Map(_, _))
        ) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: comparing a map to a record — distinct kinds (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                "a map and a record are different types (their comparison is a type error)",
            ));
            return;
        }
    }
    // `Map.insert m k v` — the inserted KEY must agree with the map's key type AND the inserted VALUE with
    // its value type (collections-and-text.md §A Map Associates Keys With Values — keys of ONE type with
    // values of ONE type). A mismatch is CDZ0201 (a map homogeneity violation, exactly as the `(map …)`
    // literal is — coded Malformed), NOT the CDZ0203 the generic scheme-unify below would give. Read the
    // map OPERAND's solved `Ty::Map(k, v)` and compare the arg types via `agrees_with` (the same
    // structural agreement the literal's homogeneity check uses); a still-unsolved map operand
    // (`Ty::Any`/a var) is skipped (its own fault, if any, surfaces elsewhere).
    if args.len() == 3
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapInsert)
        && let Ty::Map(kt, vt) = type_of(db, args[0])
    {
        let key_ty = type_of(db, args[1]);
        let val_ty = type_of(db, args[2]);
        if !kt.agrees_with(&key_ty) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Map.insert key type disagrees with the map's key type (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                "a map associates keys of one type (the inserted key's type differs from the map's)",
            ));
        }
        if !vt.agrees_with(&val_ty) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Map.insert value type disagrees with the map's value type (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                "a map associates values of one type (the inserted value's type differs from the map's)",
            ));
        }
        if !kt.agrees_with(&key_ty) || !vt.agrees_with(&val_ty) {
            // Descend into the operands for their own faults, then stop (do NOT run the generic
            // scheme-unify, which would ALSO report the same mismatch as a CDZ0203 duplicate).
            for &a in args {
                collect(db, a, out);
            }
            return;
        }
    }
    // A LIST constructor (`list` alias) applied — its arguments are its ELEMENTS, and a list is
    // HOMOGENEOUS: every element must share one type (collections-and-text.md §A List Is A Homogeneous
    // Sequence). Unify each element's type against the first; a mismatch (`(list 1 true)`) is CDZ0203. The
    // `list` NAME alias resolves to a `Resolved::Apply` (this path), NOT `Resolved::List` (the symbol
    // form), so this check — not the `Resolved::List` `collect` arm — is what catches a mixed name-alias
    // list. (`ListNew` has no `(meta t)` scheme, so the generic check below would silently pass it.)
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::ListNew)
    ) {
        let mut subst = Subst::new();
        if let Some(&first) = args.first() {
            let first_ty = type_of(db, first);
            for &e in args.iter().skip(1) {
                let et = type_of(db, e);
                if let Err(reject) = crate::unify::unify(&mut subst, &first_ty, &et) {
                    trace!(target: "rcdzc::infer", head = head.0, "fault: list elements differ in type (CDZ0203)");
                    out.push(reject);
                }
            }
        }
        for &e in args {
            collect(db, e, out);
        }
        return;
    }
    // A MAP constructor (`map` alias) applied — its arguments are its ENTRY PAIRS `(key value)`, NOT
    // curried arguments and NOT ordinary sub-expressions (a `(a 1)` entry is the pair a↦1, so it must
    // NOT be checked as "apply `a` to `1`"). Read each entry's RAW `(key value)` children, check key/
    // value homogeneity (all keys one type, all values one type — CDZ0201) + duplicate-const-key, then
    // `collect` faults from each KEY and each VALUE (as values, in scope). The `map` NAME alias resolves
    // to a `Resolved::Apply` (this path), so this — not the `Resolved::Map` `collect` arm — catches the
    // name-alias map's faults; the two share the same rules.
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::MapNew)
    ) {
        // Read the entry pairs' (key, value) child occurrences (a malformed entry is faulted at resolve).
        let entries: Vec<(StructId, StructId)> = args
            .iter()
            .filter_map(|&e| match db.ast.get(e) {
                crate::ast::Struct::List(items) if items.len() == 2 => Some((items[0], items[1])),
                _ => None,
            })
            .collect();
        let mut ksubst = Subst::new();
        let mut vsubst = Subst::new();
        if let Some(&(fk, fv)) = entries.first() {
            let (fkt, fvt) = (type_of(db, fk), type_of(db, fv));
            for &(k, v) in entries.iter().skip(1) {
                let kt = type_of(db, k);
                if crate::unify::unify(&mut ksubst, &fkt, &kt).is_err() {
                    out.push(Reject::coded(
                        Code::Malformed,
                        "a map associates keys of one type (its keys do not share a type)",
                    ));
                }
                let vt = type_of(db, v);
                if crate::unify::unify(&mut vsubst, &fvt, &vt).is_err() {
                    out.push(Reject::coded(
                        Code::Malformed,
                        "a map associates values of one type (its values do not share a type)",
                    ));
                }
            }
        }
        if let Some(reject) = map_duplicate_const_key(db, &entries) {
            out.push(reject);
        }
        for (k, v) in entries {
            collect(db, k, out);
            collect(db, v, out);
        }
        return;
    }
    // A NULLARY variant CONSTRUCTOR applied to the unit value — `(None unit)` / `(Nil ())` — is the
    // canonical construction of a nullary variant (core-semantics.md §Construction MUST Be Via
    // Application). Its ctor `(meta t)` is the bare sum (no arrow — `variant_payload_type` is `None`), so
    // the generic "instantiate the scheme and apply each arg" check below would see a non-function head
    // and wrongly fault "cannot apply a value of type <Sum>". Recognize it here: a variant ctor
    // (`(meta variant)` present) with no payload type, applied to one argument, is a well-formed nullary
    // construction — the argument is the unit payload. (A NON-unit argument is an arity error the arg's
    // own type surfaces; a nullary variant's payload type is unit, checked when it matters at the escape.)
    if crate::eval::variant_disc_of(db, head).is_some()
        && crate::eval::variant_payload_type(db, head).is_none()
    {
        // A nullary variant's argument type IS Unit (core-semantics.md §A Sum Type Constructor Is A
        // Single-Arity Function: "A nullary variant MUST be a constructor whose argument type is Unit").
        // So the ONE argument must be `unit`; a NON-unit payload — `(None 5)`, `(Opt.Nn (tuple 1 2))` —
        // is a malformed construction, NOT a silently-discarded payload. Without this check the argument
        // vanished and the value rendered `(Nn unit)`, fabricating a variant (a soundness hole — an
        // ill-typed program accepted). Reject CDZ0201, the SAME code the corpus assigns a constructor
        // applied to a wrong-type payload (the nullary-Unit and the unary declared-payload cases in
        // 05-compound-types both pin CDZ0201 — a malformed construction, not a plain type mismatch).
        // Check the supplied argument's type against Unit, then descend for the argument's own faults.
        for &arg in args {
            let at = type_of(db, arg);
            if !at.agrees_with(&Ty::Unit) {
                trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, at = %at.render_name(), "fault: nullary variant applied to a non-unit payload (CDZ0201)");
                out.push(Reject::coded(
                    Code::Malformed,
                    format!(
                        "a nullary variant takes the unit value, but a payload of type {} was applied",
                        at.render_name()
                    ),
                ));
            }
            // Descend for the argument's OWN faults (an unbound name in `(None (frob))`).
            collect(db, arg, out);
        }
        return;
    }
    // A LAMBDA head β-reduces. Its faults do NOT surface on their own: the outer `collect` walks the
    // ORIGINAL call (head + argument occurrences), never the reduced body, and β-reduction ERASES the
    // parameter↔argument relationship (the parameter's annotation is dropped when its argument is
    // substituted). So a mistyped argument — `(f 5)` to a `(: x Bool)` parameter, or to a bare
    // parameter the body uses as a Bool/Int — goes unreported and the emitter later produces invalid
    // wasm. Check the call here, at the call site, in two parts:
    if crate::eval::lambda_body(db, head).is_some() {
        // (1) Unify each argument against its PARAMETER's declared type. An annotated parameter
        //     (`(: x Bool)`) has a definite type the argument must agree with; a bare parameter types
        //     `Any` (its body-inferred type isn't a signature here) and unifies with anything, so this
        //     catches the annotated-parameter mismatch (case A) precisely, without over-rejecting.
        if let Some(params) = crate::eval::lambda_params_of(db, head) {
            let mut subst = Subst::new();
            for (&param_occ, &arg) in params.iter().zip(args.iter()) {
                let pt = type_of(db, param_occ);
                let at = type_of(db, arg);
                if let Err(reject) = crate::unify::unify(&mut subst, &pt, &at) {
                    trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "apply: argument conflicts with parameter annotation (type fault)");
                    out.push(reject);
                }
            }
            // OVER-APPLICATION: more arguments than the lambda's arity, and the body's result is not
            // itself a function to absorb them. `((fn (x) (+ x 1)) 5 9)` is a type error, not a decline —
            // the SAME CDZ0201 the corpus assigns an over-applied CONSTRUCTOR ("over-applying a constructor
            // is a type error, not a silent argument drop"). Without this, `apply_lambda` returns
            // Err("applied more arguments…") and the reduced body is never collected, so the extra args
            // silently graded as a to-do. A body whose result IS a function (a curried lambda returning a
            // lambda) legitimately absorbs the extra args — so gate on the reduced result NOT being an
            // arrow (checked via the full-application result type below).
            if args.len() > params.len() {
                // Type the lambda fully applied to its own arity; if the result is not a function, the
                // surplus args over-apply it.
                let applied_ty = apply_type(db, head, &args[..params.len()]);
                if !matches!(applied_ty, Ty::Fn(_, _) | Ty::Any) {
                    trace!(target: "rcdzc::infer", head = head.0, arity = params.len(), args = args.len(), "fault: over-applied lambda (CDZ0203)");
                    // CDZ0203 (`TypeMismatch`) — the SAME code an over-applied CONSTRUCTOR and a
                    // scheme-typed over-application use (applying the fully-consumed value, which is not a
                    // function, to a further argument). Keeps the over-application taxonomy uniform.
                    out.push(Reject::coded(
                        Code::TypeMismatch,
                        format!(
                            "applied {} arguments to a function of arity {} — it is not a function after \
                             its arguments are consumed",
                            args.len(),
                            params.len()
                        ),
                    ));
                }
            }
        }
        // (2) Collect the REDUCED body's own faults. Substituting the concrete arguments turns a
        //     body use of a bare parameter into a use of the actual argument value, so a use at a
        //     conflicting type — `(if 5 1 2)`, `(+ true true)` — now faults where the unreduced
        //     `(if x 1 2)` (x : Any) did not. This is what turns the case-B/C MISCOMPILE (invalid
        //     wasm) into a reported rejection. Runs under the reduction guard; a recursive callee
        //     declines the reduction (no reduced body) and is checked by its own def collection.
        if let Some(mut guard) = db.enter_reduction() {
            let g = guard.db();
            if let Ok(Some(reduced)) = crate::eval::apply_lambda(g, head, args) {
                collect(g, reduced, out);
            }
        } else {
            // The reduction depth limit was hit — the reduced body was NOT collected here, so this
            // application's fault set is INCOMPLETE. Mark the walk limited so it is not memoized (a
            // shallower entry would reduce and collect the body). See `collect_cache`.
            db.collect_limited = true;
        }
        return;
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No scheme: the head is not a function-valued built-in/def. It may still be applyable via one
        // of the paths above (lambda / constructor / compound alias — all handled + returned already),
        // so reaching here means the head is a PLAIN VALUE. If its type is a DEFINITE non-function —
        // applying a scalar `(5 3)`, a Bool `(true 1)`, a Float `(3.5 1)` — that is a MALFORMED
        // application (`core-semantics.md` §Applying A Function Binds Its Parameter To Its Argument: a
        // non-function has no defined result), the CDZ0201 the corpus assigns (09-functions "applying a
        // non-function/boolean/float is a type error"). Reject it here rather than letting lowering
        // decline "value is not applyable" (which grades as a to-do, not the type error it is). Guarded
        // on `is_definite_non_function` so an UNDETERMINED head (`Ty::Any` — a not-yet-modeled construct,
        // an unresolved var) still falls through to a clean decline, never a spurious reject.
        None => {
            // A head with a `(meta apply)` PRIMITIVE is applyable via that primitive even though it has
            // no type SCHEME — the compound-value constructors (`tuple`/`record`/`list` aliases build the
            // compound), a type constructor (`(Int 64)`), etc. Those are NOT "applying a non-function";
            // only a head that is neither scheme-typed NOR a `(meta apply)` primitive AND whose type is a
            // definite non-function is the malformed `(5 3)` / `(true 1)` / `(3.5 1)` case.
            if !args.is_empty() && crate::eval::meta_apply_of(db, head).is_none() {
                let ht = type_of(db, head);
                if is_definite_non_function(&ht) {
                    trace!(target: "rcdzc::infer", head = head.0, ty = %ht.render_name(), "fault: applying a non-function value (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!(
                            "cannot apply a value of type {} — it is not a function",
                            ht.render_name()
                        ),
                    ));
                }
            }
            return;
        }
    };
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // Freshen the arg's free variables past the head's instantiation before unifying — see
                // the same step in `apply_type`. Without it, an under-constrained arg (a bare nullary
                // variant `(None) : Option ?0`) shares variable numbers with the head scheme and the
                // occurs-check spuriously FAULTS a well-typed `(Some (None))` (CDZ0203 "infinite type").
                let at = crate::unify::freshen_free(&type_of(db, arg), &mut fresh);
                if let Err(reject) = crate::unify::unify(&mut subst, &param, &at) {
                    trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "apply: argument conflicts with parameter (type fault)");
                    // A wrong-type payload to a VARIANT CONSTRUCTOR — `(T.Mk "x")` for `(Mk Int64)` — is a
                    // MALFORMED construction (`CDZ0201`), the code the corpus assigns a constructor applied
                    // to a wrong-type/wrong-arity payload (the typed-payload companion of the nullary-Unit
                    // check above). It flows through this SAME instantiated-and-substituted unify as any
                    // application — so a GENERIC/nested construction (`(Ok (Err 9))`, `(Some (None))`) is
                    // NOT over-rejected — only the diagnostic CODE is specialized: a variant-ctor head
                    // reclassifies the unify mismatch from the generic `CDZ0203` to the structural
                    // `CDZ0201`. A non-ctor head (an ordinary function, an operator) keeps `CDZ0203`.
                    let sparam = subst.apply(&param);
                    let sat = subst.apply(&at);
                    if crate::eval::variant_disc_of(db, head).is_some() {
                        out.push(Reject::coded(
                            Code::Malformed,
                            format!(
                                "a variant constructor's payload has declared type {}, but a value of \
                                 type {} was applied",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        ));
                    } else if let (Ty::Sum { decl: da, .. }, Ty::Sum { decl: db_, .. }) =
                        (&sparam, &sat)
                        && da != db_
                        && same_sum_shape(db, *da, *db_)
                    {
                        // Two DISTINCT NOMINAL types (both sums) of the SAME STRUCTURAL SHAPE unified in the
                        // same position — the classic is comparing them, `(= (A.Mk 1) (B.Mk 1))` for
                        // same-shape sums `A`/`B` (the `=` operator is `∀a. a → a → Bool`, so its two
                        // operands unify against one `a` and the nominal difference conflicts here). A
                        // nominal type's identity is its declaration, so this is a comparison ACROSS THE
                        // NOMINAL BOUNDARY — `CDZ0202`, ill-typed, NOT the `false` an untagged structural
                        // comparison would give (`type-system.md` §Nominal Types Are Not Comparable Across
                        // Their Boundary). Two sums of DIFFERENT shape (disjoint variant names — `Option` vs
                        // `Result`) are unrelated types, the plain `CDZ0203` mismatch, so the shape guard
                        // keeps that case on the generic code (the corpus draws exactly this line).
                        out.push(Reject::coded(
                            Code::NominalMismatch,
                            format!(
                                "comparing distinct nominal types {} and {} across the nominal boundary",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        ));
                    } else {
                        out.push(reject);
                    }
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

/// Whether two sum DECLARATIONS have the same STRUCTURAL SHAPE — the same variant NAMES in the same
/// order, each with the same payload arity. Used to tell a NOMINAL-boundary comparison (`CDZ0202`, two
/// same-shape sums `A`/`B` differing only in tag) from an ordinary type mismatch (`CDZ0203`, two sums of
/// DIFFERENT shape — `Option` vs `Result` — which are unrelated types). This is the structural relation
/// nominal identity is an orthogonal tag OVER (`type-system.md` §Nominal Is An Orthogonal Modifier Over
/// Any Structural Type): the tag distinguishes two types that would otherwise be the same shape. Compares
/// the declared variant names + payload counts (the shape the corpus's disjoint-vs-same-shape cases draw
/// the line on); it does not descend payload TYPES (a same-names-different-payload edge is out of scope).
fn same_sum_shape(db: &Db, a: StructId, b: StructId) -> bool {
    let (Some(da), Some(dbecl)) = (db.type_decl_by_occ(a), db.type_decl_by_occ(b)) else {
        return false;
    };
    da.variants.len() == dbecl.variants.len()
        && da
            .variants
            .iter()
            .zip(dbecl.variants.iter())
            .all(|(va, vb)| va.name == vb.name && va.payloads.len() == vb.payloads.len())
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

/// The `record has no field \`key\`` rejection for a member access `member` (`(. operand key)`),
/// enriched with a "did you mean?" suggestion when a field of the record is a near-miss for `key`
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix — the record analogue of
/// the unbound-name suggestion). `operand`'s field names are the candidate set. A close field →
/// message names it AND the reject carries a heuristic ReplaceNode fix on the KEY occurrence (so an
/// editor rewrites exactly the field token, `(. r fild)` → `(. r field)`); no close field → the plain
/// message, no fix.
fn no_field_reject(
    db: &mut Db,
    member: StructId,
    operand: StructId,
    key: &crate::resolved::Symbol,
) -> Reject {
    let fields = crate::eval::record_field_names(db, operand);
    let suggestion = crate::diag::suggest::nearest(&key.name, &fields);
    // The key occurrence is the second child of the `(. operand key)` form — the node the fix rewrites.
    let key_occ = db.ast.as_form(member, ".").and_then(|t| t.get(1).copied());
    match (suggestion, key_occ) {
        (Some(field), Some(occ)) => Reject::coded(
            Code::Malformed,
            format!(
                "record has no field `{}` — did you mean `{field}`?",
                key.name
            ),
        )
        .with_fix(Fix::replace_heuristic(occ, field)),
        _ => Reject::coded(
            Code::Malformed,
            format!("record has no field `{}`", key.name),
        ),
    }
}

/// Whether a key occurrence is a DIRECT LITERAL — a constant written IN the map literal (an int/string/
/// bool/float/unit atom), NOT a name reference. The duplicate-key reject compares only these: a repeated
/// WRITTEN literal (`(map ("a" 1) ("a" 2))`) is the ambiguous duplicate the spec forbids, whereas two
/// distinct NAMES that merely fold to the same value (`(let ((a 5)) (let ((b 5)) (map (a 1) (b 2))))`)
/// are a RUNTIME overwrite (size 1, keys compared by value), NOT a compile-time reject. Reading the key
/// through a name binding would conflate the two, so a name key (`Resolved::Ref`) is NOT a direct literal.
fn is_direct_literal_key(db: &mut Db, key: StructId) -> bool {
    matches!(
        resolved_of(db, key),
        Resolved::Int(_)
            | Resolved::Str(_)
            | Resolved::Bool(_)
            | Resolved::Float(_)
            | Resolved::Unit
    )
}

/// A map literal has a DUPLICATE WRITTEN-LITERAL key → CDZ0201 (the association is ambiguous — which
/// value does the key hold? collections-and-text.md §A Map Associates Keys With Values: each key at most
/// once). Only DIRECT LITERAL keys are checked (see `is_direct_literal_key`): two literal keys that
/// compare structurally equal (`const_compound_eq`) are a duplicate. A NAME key — even two distinct
/// names bound to the same value — is a runtime overwrite (size 1), never a reject. `None` if no duplicate.
fn map_duplicate_const_key(db: &mut Db, entries: &[(StructId, StructId)]) -> Option<Reject> {
    for i in 0..entries.len() {
        if !is_direct_literal_key(db, entries[i].0) {
            continue;
        }
        for j in (i + 1)..entries.len() {
            if is_direct_literal_key(db, entries[j].0)
                && crate::lower::const_compound_eq(db, entries[i].0, entries[j].0) == Some(true)
            {
                return Some(Reject::coded(
                    Code::Malformed,
                    "a map contains each key at most once (a duplicate literal key)",
                ));
            }
        }
    }
    None
}

/// Collect faults at and under `id`, stamping each with its origin node. The recursive `collect_node`
/// pushes this node's own faults and recurses into children (each child stamped by its OWN frame
/// first); afterwards we stamp every fault THIS frame added that is still unanchored with `id` — so a
/// fault ends up anchored to the innermost node whose frame produced it, with no per-`push` threading.
fn collect(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // Recursive-descent DEPTH GUARD (shared with `type_of` and `core_of`). `collect_node` recurses into
    // children, so pathologically deep input would overflow the native stack and ABORT the process.
    // Past the limit, report the resource-limit decline and stop descending — a compiler declines on
    // input it cannot handle, never crashes (`self-hosting-and-bootstrap.md`). See `DESCENT_DEPTH_LIMIT`.
    // Mark the walk LIMITED so this (partial) result is not memoized (see `collect_cache`).
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::infer", node = id.0, "collect depth limit hit → decline (resource limit)");
        db.collect_limited = true;
        out.push(
            Reject::decline(
                "expression nests too deeply to compile (a recursion/resource limit was reached)",
            )
            .at(id),
        );
        return;
    }
    // MEMO: a node's faults are a pure function of its structure + its parts' (memoized) types, so a node
    // collected once is not re-walked. This is what makes a nested call chain's fault walk LINEAR: without
    // it, `check_application` step 2 re-descends each Apply's cached-but-shared reduced body, whose inner
    // call re-descends its own reduced body — an exponential re-walk. Replaying the cached faults (each
    // already carrying its stamped origin) is exact. Only a subtree collected WITHOUT hitting a limit is
    // cached (a limit-clipped partial walk is not the node's true fault set — tracked by `collect_limited`).
    if let Some(cached) = db.collect_cache.get(&id) {
        out.extend(cached.iter().cloned());
        return;
    }
    db.descent_depth += 1;
    let outer_limited = std::mem::replace(&mut db.collect_limited, false);
    let mut sub: Vec<Reject> = Vec::new();
    collect_node(db, id, &mut sub);
    for reject in &mut sub {
        reject.set_origin_if_absent(id);
    }
    // Cache the node's faults iff its walk was complete (no limit tripped inside it). Propagate the
    // limited flag to the enclosing frame (OR it in) so an ancestor is not cached over a clipped child.
    let this_limited = db.collect_limited;
    if !this_limited {
        db.collect_cache.insert(id, sub.clone());
    }
    db.collect_limited = outer_limited || this_limited;
    db.descent_depth -= 1;
    out.extend(sub);
}

fn collect_node(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // A `do` SEQUENCING block resolves to a `Ref` to its LAST form (its value), so the `Resolved::Ref`
    // arm below descends only into that. But the INTERMEDIATE forms are still evaluated (their value
    // discarded), so an ill-typed or provably-trapping intermediate must be caught — descend into EVERY
    // form here (the last one is also covered by the `Ref` descent, harmlessly). Read off the raw AST
    // head since the resolved form has already collapsed to the last form's `Ref`.
    if db.ast.head_name(id) == Some("do")
        && let Some(forms) = db.ast.as_form(id, "do")
    {
        let forms: Vec<StructId> = forms.to_vec();
        for f in forms {
            // A do-local `(def …)` is a DECLARATION, not a value expression — resolving it as one would
            // decline. A VALUE declaration `(def x V)` (or nullary `(def (x) V)`) has its value `V`
            // type-checked eagerly, exactly like a `let` binding's value (a value binding is a fault
            // whether or not the name is used). A FUNCTION declaration `(def (f p…) BODY)` is a lambda:
            // its body is checked on CALL (β-reduced at a reference), like a `let`-bound lambda, so it is
            // not descended into here.
            if db.ast.head_name(f) == Some("def") {
                if let Some(value) = crate::resolve::do_value_def_value(db, f) {
                    collect(db, value, out);
                }
                continue;
            }
            // A do-local `(type …)` / `(effect …)` is a DECLARATION, not a value expression — its sum/
            // effect record is synthesized at load (`db::collect_nested_decls`) and its names resolve
            // through the ordinary decl paths. There is nothing to type-check as a value (resolving the
            // form as one would decline "unbound name `type`"), so skip it, like a `def`.
            if matches!(db.ast.head_name(f), Some("type") | Some("effect")) {
                continue;
            }
            collect(db, f, out);
        }
        return;
    }
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
        // A boolean connective — each operand must be Bool (core-semantics.md §Boolean Connectives
        // Short-Circuit: conjunction/disjunction/negation are OVER boolean values). A non-Bool operand is
        // a type mismatch (CDZ0203), the same class as a non-Bool `if` condition. Then descend for each
        // operand's own faults.
        Resolved::And { lhs, rhs, is_and } => {
            let op = if is_and { "and" } else { "or" };
            for &operand in &[lhs, rhs] {
                let t = type_of(db, operand);
                if !t.agrees_with(&Ty::Bool) {
                    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: connective operand not Bool (CDZ0203)");
                    out.push(Reject::coded(
                        Code::TypeMismatch,
                        format!("`{op}` operand must be Bool, found {}", t.render_name()),
                    ));
                }
                collect(db, operand, out);
            }
        }
        Resolved::Not { operand } => {
            let t = type_of(db, operand);
            if !t.agrees_with(&Ty::Bool) {
                trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: not operand not Bool (CDZ0203)");
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!("`not` operand must be Bool, found {}", t.render_name()),
                ));
            }
            collect(db, operand, out);
        }
        // Member access: the operand must be a record, and it must have the named field. Both faults
        // are CDZ0201 (`core-semantics.md` §Member Access Projects A Record Field — projecting a field
        // of a non-record, or an absent field, has no defined result). A built-in module is CLOSED the
        // same way a user record is: it carries every field it will ever have, and an unrealized field
        // DECLINES when projected rather than being absent (`prelude.rs` — there is no open-module rule).
        Resolved::Member { operand, key } => {
            // Project via the evaluator (reduces refs / a ctor-built module), so a missing field on a
            // built module is caught too. A poison operand reports its OWN fault (via the descent
            // below), so we don't add a redundant "not a record" for it.
            let operand_is_poison = matches!(resolved_of(db, operand), Resolved::Poison(_));
            match crate::eval::member_value(db, operand, &key) {
                crate::eval::Member::Field(_) => {}
                crate::eval::Member::NoField => {
                    trace!(target: "rcdzc::infer", node = id.0, key = %key.name, "fault: record has no such field (CDZ0201)");
                    out.push(no_field_reject(db, id, operand, &key))
                }
                // The operand did not reduce to a compile-time-visible record. Before rejecting, check
                // its TYPE: a RUNTIME record (a call result, an `if` selection) carries a record type,
                // and the access is well-formed iff that type has the field — the field read lowers to
                // an `arr-get` at the field's sorted index (mirrors a tuple projection on a runtime
                // tuple). Only a genuine non-record operand, or a record type lacking the field, faults.
                crate::eval::Member::NotRecord if !operand_is_poison => {
                    match type_of(db, operand) {
                        Ty::Record(fields) if fields.contains_key(&key) => {}
                        Ty::Record(_) => {
                            trace!(target: "rcdzc::infer", node = id.0, key = %key.name, "fault: runtime record has no such field (CDZ0201)");
                            out.push(no_field_reject(db, id, operand, &key))
                        }
                        // An UNCONSTRAINED operand (`Any`) — a bare (unannotated) parameter of a
                        // non-recursive def, whose type is not known until the def inlines at a call
                        // site with a concrete argument. `Any` means "no constraint" (it agrees with
                        // everything), so a field read on it is NOT a fault HERE — the real check runs
                        // on the reduced body at the call site (where the argument's record type flows
                        // in). Rejecting here spuriously fails a well-typed program `(def (get-x r) (. r
                        // x))`, exactly as arithmetic on an `Any` param (`(+ r 1)`) does not fault.
                        Ty::Any => {}
                        other => {
                            trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: member access on a non-record (CDZ0201)");
                            out.push(Reject::coded(
                                Code::Malformed,
                                format!(
                                    "member access requires a record, found {}",
                                    other.render_name()
                                ),
                            ))
                        }
                    }
                }
                crate::eval::Member::NotRecord => {}
            }
            collect(db, operand, out);
        }
        // Tuple projection `(. t N)`: the operand must be a TUPLE, and `N` must be within its static
        // arity. Both faults are CDZ0201, decided at COMPILE TIME — an out-of-arity index is NOT a
        // runtime trap (`type-system.md` §A Tuple Is Split At A Position Into A Prefix And A Suffix;
        // [[an-out-of-arity-tuple-index-traps]]). A poison operand reports its own fault via the descent.
        Resolved::Proj { operand, index } => {
            let operand_is_poison = matches!(resolved_of(db, operand), Resolved::Poison(_));
            // NOT collapsible into a guarded arm (`clippy::collapsible_match`): an IN-RANGE tuple
            // projection must match `Ty::Tuple` and produce NO fault. Guarding the arm with `if index
            // >= len` would let the in-range case fall through to the `_ if !operand_is_poison` arm
            // below, which would spuriously report "requires a tuple, found (Tuple …)".
            #[allow(clippy::collapsible_match)]
            match type_of(db, operand) {
                Ty::Tuple(elems) => {
                    if index >= elems.len() {
                        trace!(target: "rcdzc::infer", node = id.0, index, arity = elems.len(), "fault: tuple index out of arity (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            format!(
                                "tuple index {index} is out of range for a {}-element tuple",
                                elems.len()
                            ),
                        ));
                    }
                }
                // An UNCONSTRAINED operand (`Any`) — a bare (unannotated) parameter of a non-recursive
                // def, whose tuple type is not known until the def inlines at a call site with a
                // concrete argument. `Any` means "no constraint", so a projection on it is NOT a fault
                // HERE — the real check runs on the reduced body at the call site (where the argument's
                // tuple type flows in). Rejecting here spuriously fails a well-typed `(def (fst t) (. t
                // 0))`, exactly as arithmetic on an `Any` param does not fault.
                Ty::Any => {}
                // A non-tuple operand (that is not itself a poison) — projecting a position of a
                // non-tuple has no defined result.
                _ if !operand_is_poison => {
                    trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: tuple projection on a non-tuple (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!(
                            "tuple projection requires a tuple, found {}",
                            type_of(db, operand).render_name()
                        ),
                    ));
                }
                _ => {}
            }
            collect(db, operand, out);
        }
        // A tuple literal: descend into each element for its own faults.
        Resolved::Tuple { elems } => {
            for &e in elems.iter() {
                collect(db, e, out);
            }
        }
        // A list literal is HOMOGENEOUS: every element must share ONE type (collections-and-text.md §A
        // List Is A Homogeneous Sequence). Unify each element's type against the first — a mismatch (a
        // `(list 1 true)`) is CDZ0203, the same class as `if` branches disagreeing. Then descend into
        // each element for its own faults.
        Resolved::List { elems } => {
            let mut subst = Subst::new();
            if let Some(&first) = elems.first() {
                let first_ty = type_of(db, first);
                for &e in elems.iter().skip(1) {
                    let et = type_of(db, e);
                    if let Err(reject) = crate::unify::unify(&mut subst, &first_ty, &et) {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: list elements differ in type (CDZ0203)");
                        out.push(reject);
                    }
                }
            }
            for &e in elems.iter() {
                collect(db, e, out);
            }
        }
        // A map literal is HOMOGENEOUS on BOTH axes: all keys share one type AND all values share one
        // type (collections-and-text.md §A Map Associates Keys With Values — keys of ONE type with values
        // of ONE type). Unify each key against the first key and each value against the first value — a
        // mismatch (`(map (a 1) (b true))` values, or `(map (j 1) (k 2))` with `j`:Int/`k`:Bool keys) is
        // CDZ0201 (a map is ill-typed, not merely a shape mismatch — coded Malformed like the list-of-
        // homogeneous). Independent of the key-is-a-value rule (the keys are ordinary value occurrences).
        // Then descend into each key and value for its own faults. The DUPLICATE-CONSTANT-KEY check is
        // separate (a repeated compile-time-constant key is CDZ0201 — a runtime-computed duplicate is a
        // runtime overwrite, not a reject); it lives in `map_duplicate_const_key` below.
        Resolved::Map { entries } => {
            let mut ksubst = Subst::new();
            let mut vsubst = Subst::new();
            if let Some(&(fk, fv)) = entries.first() {
                let (fkt, fvt) = (type_of(db, fk), type_of(db, fv));
                for &(k, v) in entries.iter().skip(1) {
                    let kt = type_of(db, k);
                    if crate::unify::unify(&mut ksubst, &fkt, &kt).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map keys differ in type (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            "a map associates keys of one type (its keys do not share a type)",
                        ));
                    }
                    let vt = type_of(db, v);
                    if crate::unify::unify(&mut vsubst, &fvt, &vt).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map values differ in type (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            "a map associates values of one type (its values do not share a type)",
                        ));
                    }
                }
            }
            // A repeated COMPILE-TIME-CONSTANT key makes the association ambiguous → CDZ0201.
            if let Some(reject) = map_duplicate_const_key(db, &entries) {
                trace!(target: "rcdzc::infer", node = id.0, "fault: map has a duplicate constant key (CDZ0201)");
                out.push(reject);
            }
            for &(k, v) in entries.iter() {
                collect(db, k, out);
                collect(db, v, out);
            }
        }
        // A `(bin …)` construction: check STATIC well-formedness (CDZ0220 — decidable from the segment
        // list alone), then descend into each segment's value slot for its own faults. Well-formedness:
        // the running bit-cursor from `bits` segments must close to a whole byte before any byte-aligned
        // segment (int/bytes) and at the end (`binary-syntax`: the whole `bin` is byte-aligned); an
        // unsized `(bytes b)` (no dependent size) is only legal as the FINAL segment. A non-const `bits`
        // width already became a CDZ0220 `Poison` at resolve.
        Resolved::Bin { segs } => {
            let mut bit_cursor: u32 = 0; // open bits since the last byte boundary
            for (i, seg) in segs.iter().enumerate() {
                match &seg.kind {
                    crate::resolved::SegKind::Bits { k } => bit_cursor += k,
                    crate::resolved::SegKind::Int { .. } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                "a bin integer segment must start on a byte boundary (preceding bit-fields do not close a byte)",
                            ));
                        }
                    }
                    crate::resolved::SegKind::Bytes { size } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                "a bin bytes segment must start on a byte boundary (preceding bit-fields do not close a byte)",
                            ));
                        }
                        // An UNSIZED bytes segment (splice-all / bind-rest) is legal only as the last
                        // segment — a non-final unsized bytes has no defined boundary.
                        if size.is_none() && i + 1 != segs.len() {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                "a non-final bin bytes segment must have an explicit size (bytes b n)",
                            ));
                        }
                    }
                }
                collect(db, seg.slot, out);
                if let crate::resolved::SegKind::Bytes { size: Some(n) } = &seg.kind {
                    collect(db, *n, out);
                }
            }
            // The whole form must be byte-aligned: any open bits at the end are ill-formed.
            if !bit_cursor.is_multiple_of(8) {
                out.push(Reject::coded(
                    Code::IllFormedBinary,
                    "a bin form's bit-fields must close to a whole number of bytes",
                ));
            }
        }
        // Descend into the new binding/aggregate forms for their own faults.
        Resolved::Let { bindings, body } => {
            for (lhs, value) in bindings {
                // A binding LHS may be an irrefutable PATTERN (`(tuple a b)`), not just a bare name. It is
                // a BINDING POSITION — no alternative arm — so it must be irrefutable, and its shape must
                // agree with the value's type. Validate it against the value's type (a refutable pattern →
                // CDZ0210, a wrong-arity/non-tuple shape → CDZ0201, a non-linear pattern → CDZ0102, a
                // not-yet-supported record/single-variant/list pattern → decline). A bare-name LHS is the
                // trivial irrefutable pattern and validates cheaply. (The binder REFERENCES resolve to a
                // `SumPayload` reading the element out of `value`; this only ensures the binding is
                // well-formed so an ill-formed one faults instead of silently miscompiling.)
                let value_ty = type_of(db, value);
                if let Err(r) = crate::lower::check_binding_pattern(db, lhs, &value_ty) {
                    out.push(r);
                }
                collect(db, value, out);
            }
            collect(db, body, out);
        }
        // A match: every arm body must AGREE in type with the first (like an `if`'s branches), and the
        // scrutinee + bodies are checked for their own faults. (Exhaustiveness — a scalar match with no
        // covering arm — is checked in `lower::lower_match`, where the pattern shapes are classified;
        // the arms-agree check is the type fault here.)
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
            for (_, &value) in fields.iter() {
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
            // A RECORD value-constructor alias applied — `(record (x 1) (y 2))`. Its arguments are
            // `(key value)` FIELD PAIRS, not expressions, so descending into a pair as an expression
            // would resolve the key `x` as an unbound name. Instead validate the field shape (a
            // malformed pair / duplicate field is CDZ0201) and descend into each field's VALUE only —
            // exactly what the symbol-headed `({} …)` form does via `resolve_record`.
            if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordNew)
            ) {
                match crate::resolve::read_record_fields(db, &args) {
                    Ok(fields) => {
                        for (_, &value) in fields.iter() {
                            collect(db, value, out);
                        }
                    }
                    Err(reject) => out.push(reject),
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::MapNew)
            ) {
                // A MAP value-constructor alias applied — `(map (k v) …)`. Its arguments are `(key value)`
                // ENTRY PAIRS, not expressions: descending into a pair as an expression would resolve the
                // key `a` as applied to the value (the `(a 1)`→"cannot apply Int64" fault). `check_application`
                // above already validated homogeneity + descended into each key/value; here we only handle
                // the MALFORMED-entry fault (a non-`(key value)` entry) so it is reported — the well-formed
                // entries' faults are already collected. Do NOT `collect` the raw entry pairs.
                for &entry in args.iter() {
                    if !matches!(db.ast.get(entry), crate::ast::Struct::List(items) if items.len() == 2)
                    {
                        out.push(Reject::coded(
                            Code::Malformed,
                            "a map entry is a (key value) pair",
                        ));
                    }
                }
            } else {
                for &arg in args.iter() {
                    collect(db, arg, out);
                }
            }
        }
        // A type annotation `(: expr T)`: UNIFY the asserted type `T` against `expr`'s type. A failure
        // is the conflicting-use type error (`(: true Int64)` — Bool asserted as Int64 → CDZ0203); a
        // success grounds a deferred width harmlessly. If `T` is not a type this stage reduces, decline
        // the CHECK (no false reject) — the expr still type-checks on its own via the descent below.
        Resolved::Annot { expr, ty_expr } => {
            // A RUNTIME WIDTH is forbidden: `(: 5 (UInt n))` with `n` a runtime value (a parameter, a
            // call result) puts runtime data in a type-determining position, which the type system
            // forbids — an integer type's width MUST be a compile-time natural (`numeric-model.md §An
            // Integer Type Is Indexed By A Compile-Time Width`; `type-system.md §Generics Are
            // Type-Valued Parameters` — a type-valued parameter is resolved at compile time, never from
            // runtime data). This is checked on the ANNOTATION's un-inlined body, so `(def (mk n) (: 5
            // (UInt n)))` rejects (CDZ0302) even though a constant call site `(mk 8)` would fold `n` — a
            // width must be non-dependent regardless of how it happens to be called.
            let runtime_width = crate::eval::is_runtime_width_type(db, ty_expr);
            if runtime_width {
                trace!(target: "rcdzc::infer", node = id.0, "fault: integer width from runtime data (CDZ0302)");
                out.push(Reject::coded(
                    Code::IntOutOfRange,
                    "an integer width must be a compile-time natural, not runtime data",
                ));
            }
            if let Some(annot_ty) = crate::eval::typeval_of(db, ty_expr) {
                // A FLOAT type annotating with a NON-ADMITTED width reduces (via the `Float` constructor)
                // to the sentinel `Ty::Float(Fixed(0))` — a `(: 1.5 (Float 16))` / `(: 1.5 (Float 48))`.
                // A float width outside {32,64} is rejected CDZ0302 (numeric-model.md §A Floating-Point
                // Type Is Indexed By A Compile-Time Width), the float analogue of an out-of-range integer
                // width; caught here at the annotation, before the grounding/unify below (which would
                // otherwise let the sentinel slip through against a deferred literal).
                if let Ty::Float(ft) = &annot_ty
                    && let crate::ty::Width::Fixed(w) = ft.width
                    && !crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&w)
                {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: float width not in the admitted set (CDZ0302)");
                    out.push(Reject::coded(
                        Code::IntOutOfRange,
                        "a floating-point width must be one of the admitted IEEE widths (32 or 64)",
                    ));
                }
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
                        // "try wrapping the expression in `Some`": if the expected sum has a single-payload
                        // variant whose payload IS the value's type, wrapping the value in that ctor makes
                        // it type-check. Derived generically from the sum's declaration.
                        let wrap = wrap_variant_for(db, &annot_ty, &expr_ty);
                        let mut reject = match &wrap {
                            Some(ctor) => Reject::coded(
                                Code::TypeMismatch,
                                format!(
                                    "annotation type {} does not match value type {} — wrap the value in `{ctor}`",
                                    annot_ty.render_name(),
                                    expr_ty.render_name()
                                ),
                            ),
                            None => Reject::coded(
                                Code::TypeMismatch,
                                format!(
                                    "annotation type {} does not match value type {}",
                                    annot_ty.render_name(),
                                    expr_ty.render_name()
                                ),
                            ),
                        };
                        if let Some(ctor) = wrap {
                            reject = reject.with_fix(Fix::wrap_heuristic(
                                expr,
                                format!("({ctor} "),
                                ")",
                                format!("wrap in `({ctor} …)`"),
                            ));
                        }
                        out.push(reject);
                    }
                }
            } else if !runtime_width {
                // The TYPE OPERAND does not denote a type — an unbound name, an integer/compound VALUE,
                // an arbitrary expression, or a non-constructor type applied to arguments (`(Int64 Int64)`).
                // The type position REQUIRES a type, so this is REJECTED, not dropped-and-ignored (the
                // drop-instead-of-reject gap: `typeval_of` → None was treated as "no constraint"). First
                // collect the operand's OWN faults — an UNBOUND NAME surfaces as CDZ0101 there (the same
                // code it gets in value position, `(+ foo 1)`), so a typo in a type is not swallowed. If
                // the operand is well-formed but simply not a type (a literal, a compound, `(Int64 Int64)`),
                // no fault surfaces from that collect, so add an "expected a type" TypeMismatch (CDZ0203).
                // (A RUNTIME WIDTH also makes `typeval_of` return None but is already reported CDZ0302
                // above — excluded here so it is not double-faulted.)
                let before = out.len();
                collect(db, ty_expr, out);
                if out.len() == before {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: annotation type position is not a type (CDZ0203)");
                    out.push(Reject::coded(
                        Code::TypeMismatch,
                        "the type position of an annotation requires a type, but found a non-type",
                    ));
                }
            }
            collect(db, expr, out);
        }
        // EFFECT CONTROL FORMS: descend into every executed sub-expression so a fault inside is caught
        // regardless of whether lowering can yet run the form. A handler's init, each arm's PERFORM
        // (the op projection — a perform-argument type mismatch surfaces here as an ordinary application
        // check via the op value's `(meta t)` arrow) and body, and the handled body all participate in
        // well-formedness. The arm's param/state binders are binder occurrences (not collected as
        // values). This is what makes a wrong-type perform argument a CDZ0203 even while the handler
        // itself declines to run (E1a).
        Resolved::Handle { init, arms, body } => {
            collect(db, init, out);
            for arm in &arms {
                // A HANDLER ARM NAMES AN UNDECLARED OPERATION (CDZ0403). If the arm's op is `(. E k)`
                // where `E` is an effect but `k` is not one of its declared operations, that is a
                // closed-set violation (`capabilities-and-effects.md` §A Handler Arm Names An Operation
                // Its Effect Declares) — CDZ0403, NOT the generic "record has no field" (CDZ0201) the
                // member projection would otherwise emit. When it fires, skip the generic `collect` of the
                // op (which would add the CDZ0201 duplicate).
                if crate::effects::arm_op_names_undeclared_operation(db, arm.op) {
                    out.push(
                        Reject::coded(
                            Code::HandlerUndeclaredOp,
                            "this handler arm names an operation its effect does not declare",
                        )
                        .at(arm.op),
                    );
                } else {
                    collect(db, arm.op, out);
                }
                // RESUME-VALUE / RESULT-TYPE CHECK. The value a handler resumes with — `(resume value
                // state)` — is returned to the perform site, so it MUST have the operation's declared
                // RESULT type (`capabilities-and-effects.md` §Performing An Operation Is Typed And
                // Contributes To The Row — a perform 'yields the operation's declared result type', so an
                // operation is typed exactly as a function application whose body must return the declared
                // type). A mismatch — `(resume true s)` for an `(-> Int64 Int64)` op — is CDZ0201 (the
                // result-type companion of the perform-argument check). Without this the fold silently
                // substitutes the mistyped value as the perform's result (a type-confusion miscompile).
                check_resume_result_type(db, arm, out);
                collect(db, arm.body, out);
            }
            collect(db, body, out);
        }
        Resolved::Resume { value, next_state } => {
            collect(db, value, out);
            collect(db, next_state, out);
        }
        Resolved::Host { body, .. } => {
            // The effect names are label occurrences (an effect name resolves to its record); descend
            // into the delegated body for its faults.
            collect(db, body, out);
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
        // intrinsic value and the atomic leaves have no sub-faults. A `SumPayload` (a variant binder's
        // payload read) has no sub-fault of its own — the scrutinee's faults surface at the match.
        // A bare integer literal whose width DEFAULTED (nothing annotated/inferred it, so it grounds to
        // the default signed `Int64`) but whose VALUE does not fit signed-64 is a MALFORMED literal
        // (CDZ0201) — `9223372036854775808` (Int64.max+1), `0xFFFFFFFFFFFFFFFF` (fits UNSIGNED-64 but a
        // bare literal is signed): a number with no width to blame, not a name and not an out-of-range-
        // for-a-chosen-width (which stays CDZ0302, checked at the annotation in the `Annot` arm). 01-
        // literals "an out-of-range integer literal is a malformed literal, not an unbound name".
        Resolved::Int(v) => {
            // A literal that is the direct operand of an annotation `(: LIT T)` is checked against `T`
            // by the `Annot` arm (CDZ0302 on an over-width annotated literal); its own deferred type
            // here would misfire. So skip an annotated literal — only a literal with NO width in sight
            // (defaulted to `Int64`) is judged malformed here.
            let annotated = db
                .parent_of(id)
                .and_then(|p| db.ast.as_form(p, ":"))
                .and_then(|t| t.first().copied())
                == Some(id);
            if !annotated
                && let Ty::Int(it) = type_of(db, id)
                && !it.width_is_fixed()
                && !v.fits_width(true, crate::ty::DEFAULT_INT_WIDTH)
            {
                trace!(target: "rcdzc::infer", node = id.0, "fault: integer literal exceeds the default Int64 (malformed, CDZ0201)");
                out.push(Reject::coded(
                    Code::Malformed,
                    "integer literal is out of range for Int64",
                ));
            }
        }
        Resolved::Prim(_)
        | Resolved::Ref { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::Param { .. }
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
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
            "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))",
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
            "(module m (def (sum-to n) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))",
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
        let src = "(module m (def (sum-to n) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))";

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
