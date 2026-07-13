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
        // A symbol literal (`#"metre"`) is the monomorphic `Ty::Symbol` (DISTINCT from `Ty::String`).
        Resolved::SymbolConst(_) => Ty::Symbol,
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
            // A NOMINAL newtype over a record is erased at run time to that record, so a field read sees
            // through the tag (`strip_nominal`) to the inner record's field type — `(. u x)` on `u :
            // UserId` (a `(type UserId (Mk (Record (x Int64) …)))`) types as the inner `x`'s type.
            _ => match type_of(db, operand).strip_nominal() {
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
                        // Over a NOMINAL NEWTYPE the `Payload` step UNWRAPS the tag to its underlying
                        // type (a runtime no-op — the value is unchanged). `(Mk n)` over a `Ty::Nominal {
                        // inner: Int64 }` binds `n : Int64`.
                        if let Ty::Nominal { inner, .. } = &cur {
                            *inner.clone()
                        } else {
                            match payload_ty_at_instantiation(db, head, &cur) {
                                Some(t) => t,
                                None => return Ty::Any,
                            }
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
        // `nan` — the canonical NaN Float VALUE (a bare prim naming a value). Types as `Ty::Float` (a
        // bare `nan` grounds to Float64), so `(= nan nan)` type-checks like any float equality.
        Resolved::Prim(crate::resolved::Prim::FloatNan) => Ty::float(),
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

/// The CDZ0302 fault if the value at `value` is an integer LITERAL that does not fit the NARROW integer
/// type the type-expression `ty_expr` denotes, else `None`. The literal analogue of "Annotations
/// Constrain" (numeric-model.md §A Bare Integer Literal Is Grounded By Its Annotation, Subject To A Range
/// Check): a bare literal has no intrinsic width, so an annotation FIXES its type subject only to a range
/// check — a literal outside the width is REJECTED, never truncated. Shared by the value annotation
/// `(: value T)` and the annotated LET BINDER `((: name T) value)` so both range-check identically. Only
/// a literal + a fixed-width integer type can fault here; a non-literal value's agreement is a separate
/// unify (CDZ0203), and a deferred/Var width imposes no bound.
fn literal_width_fault(db: &mut Db, value: StructId, ty_expr: StructId) -> Option<Reject> {
    let annot_ty = crate::eval::typeval_of(db, ty_expr)?;
    let Ty::Int(it) = &annot_ty else { return None };
    let crate::ty::Width::Fixed(w) = it.width else {
        return None;
    };
    // The bound value's CONSTANT integer value, if it has one: a bare literal `200`, OR a value that
    // FOLDS to a constant (`(+ 100 100)` → 200) — the same computed constants the value annotation
    // `(: (+ 100 100) Int8)` range-checks. `core_of` performs the fold; a runtime value (a param, a call
    // result) does not fold to `ConstInt` and imposes no compile-time bound (it is checked by its own
    // type / traps at run time).
    let v = match resolved_of(db, value) {
        Resolved::Int(v) => v,
        _ => match crate::lower::core_of(db, value) {
            crate::core::Core::ConstInt(v) => v,
            _ => return None,
        },
    };
    if !v.fits_width(it.ground_signed(), w) {
        return Some(int_out_of_range_reject(
            &annot_ty,
            it.ground_signed(),
            w,
            &v,
            ty_expr,
        ));
    }
    None
}

/// The CDZ0302 reject for an integer literal `v` that overflows the `(signed, width)` type `annot_ty`,
/// carrying — when possible — a WIDEN fix: replace the annotation `ty_expr` with the SMALLEST aliased
/// width ({8,16,32,64}) of the same signedness the literal DOES fit (`(: 999 Int8)` → `Int16`), the
/// rustc-style "value doesn't fit; use a wider type" repair (`spec/capabilities/diagnostics.md` §A
/// Diagnostic Carries A Route To A Fix). Only a WIDER width is offered (the search starts above `w`) and
/// only when one fits — a value beyond `Int64`/`UInt64`, or a NEGATIVE into any UNSIGNED width (which no
/// widening rescues), gets the bare reject: switching signedness is a larger intent guess the compiler
/// must not make. Replacing the whole `ty_expr` rewrites either spelling — a bare `Int8` or a `(Int 8)`
/// compound — to the bare `Int16`. Heuristic: widening clears the range fault, but whether the author
/// meant a wider type (vs. a different literal) is theirs to confirm. Shared by both CDZ0302 literal-range
/// sites (the value annotation `(: v T)` and the let-binder/param `((: name T) v)`), so both carry the fix.
fn int_out_of_range_reject(
    annot_ty: &Ty,
    signed: bool,
    w: u32,
    v: &crate::ast::IntValue,
    ty_expr: StructId,
) -> Reject {
    let reject = Reject::coded(
        Code::IntOutOfRange,
        int_out_of_range_message(annot_ty, signed, w),
    );
    match crate::ty::ALIASED_INT_WIDTHS
        .iter()
        .copied()
        .filter(|&aw| aw > w)
        .find(|&aw| v.fits_width(signed, aw))
    {
        Some(fit) => {
            let stem = if signed { "Int" } else { "UInt" };
            reject.with_fix(Fix::replace_heuristic(ty_expr, format!("{stem}{fit}")))
        }
        None => reject,
    }
}

/// The CDZ0302 message for an integer literal that overflows the annotated type: names the type and,
/// when the width is a well-formed one whose range renders exactly, appends `(the valid range is
/// min..=max)`. A malformed width (no exact range) falls back to the type-name-only message.
fn int_out_of_range_message(annot_ty: &Ty, signed: bool, w: u32) -> String {
    match int_width_range(signed, w) {
        Some(range) => format!(
            "integer literal does not fit the annotated type {} (the valid range is {range})",
            annot_ty.render_name(),
        ),
        None => format!(
            "integer literal does not fit the annotated type {}",
            annot_ty.render_name()
        ),
    }
}

/// The inclusive value range a `(signed, width)` integer type holds, rendered `min..=max` (rustc's
/// "the range is `-128..=127`" phrasing) — a signed N-bit holds `-(2^(N-1)) ..= 2^(N-1) - 1`, an
/// unsigned N-bit `0 ..= 2^N - 1`. Names the concrete bounds a CDZ0302 out-of-range literal missed, so
/// the message says WHICH range rather than only the type name. Returns `None` for a width the `i128`/
/// `u128` arithmetic can't hold exactly (`w == 0`, or `> 127` signed / `> 128` unsigned — only a
/// MALFORMED width, since a well-formed integer type is `1..=64`); the caller then omits the range
/// clause rather than the helper panicking on a shift overflow.
fn int_width_range(signed: bool, w: u32) -> Option<String> {
    if w == 0 {
        return None;
    }
    if signed {
        if w > 127 {
            return None;
        }
        let max = (1i128 << (w - 1)) - 1;
        let min = -(1i128 << (w - 1));
        Some(format!("{min}..={max}"))
    } else {
        if w > 128 {
            return None;
        }
        // `1u128 << 128` overflows; `w == 128` max is `u128::MAX` directly.
        let max = if w == 128 {
            u128::MAX
        } else {
            (1u128 << w) - 1
        };
        Some(format!("0..={max}"))
    }
}

/// The GERUND naming the additive/relational operation a CDZ0501 dimensional fault arose in — "adding",
/// "subtracting", "comparing" — so the message reads as an action ("adding quantities of incompatible
/// dimension"). `prim` is the operator's [`crate::resolved::Prim`] (the `is_additive` set: `+`/`-` plus
/// the comparisons); anything else (or an unrecognized head) falls back to the neutral "combining".
fn additive_op_gerund(prim: Option<crate::resolved::Prim>) -> &'static str {
    use crate::resolved::Prim;
    match prim {
        Some(Prim::Add) => "adding",
        Some(Prim::Sub) => "subtracting",
        Some(Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq | Prim::Compare) => "comparing",
        _ => "combining",
    }
}

/// The NOUN for the same operation — "addition", "subtraction", "comparison" — used in the "… requires
/// equal dimensions" clause of a CDZ0501 message. Falls back to "this operation".
fn additive_op_noun(prim: Option<crate::resolved::Prim>) -> &'static str {
    use crate::resolved::Prim;
    match prim {
        Some(Prim::Add) => "addition",
        Some(Prim::Sub) => "subtraction",
        Some(Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq | Prim::Compare) => "comparison",
        _ => "this operation",
    }
}

/// `""` for exactly one, `"s"` otherwise — the plural suffix for a count in a diagnostic.
fn plural_s(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// "N open bit(s)" — the count of bit-field bits accumulated since the last byte boundary, for a
/// CDZ0220 byte-alignment message.
fn open_bits_phrase(bits: u32) -> String {
    let open = bits % 8;
    format!("{open} open bit{}", plural_s(open))
}

/// The actionable "add N more bit(s) to reach K byte(s)" hint for a CDZ0220 byte-alignment fault — how
/// far the running bit-cursor is from the next byte boundary, and the byte count once closed. `bits` is
/// the total accumulated bit-field width; only called when it is NOT a whole number of bytes. When there
/// is a lower byte boundary above zero (`bits >= 8`), the alternative "drop M bits" is offered too; a
/// sub-byte total (`bits < 8`) omits it (dropping would reach zero bytes — not useful advice).
fn bits_to_byte_boundary_hint(bits: u32) -> String {
    let pad = (8 - (bits % 8)) % 8;
    let bytes = bits.div_ceil(8);
    let over = bits % 8;
    let add = format!(
        "add {pad} more bit{} to reach {bytes} byte{}",
        plural_s(pad),
        plural_s(bytes)
    );
    if bits >= 8 {
        format!(
            "{add} (or drop {over} bit{} to the previous boundary)",
            plural_s(over)
        )
    } else {
        add
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
            // Each pattern constrains the scrutinee's type. A LITERAL pattern pins it to the literal's
            // type (`(match n (0 …) (_ …))` → `n : Int64`). When the scrutinee is DIRECTLY A PARAMETER
            // being solved, a STRUCTURAL pattern pins its SHAPE: `(match xs ((Cons (tuple h t)) …))` means
            // `xs` is that sum, so a recursive tree-walker's parameter (`xs : Code`, `node : Core`) is
            // inferred rather than left a free var (which would ground to `Any` and decline). Restricted
            // to a scrutinee that IS a parameter — a scrutinee that is a CALL RESULT (`(List.at xs i)`)
            // carries its own instantiation that this shape-unify would corrupt, so it is left to the
            // ordinary application constraints.
            let st = arg_ty_in_env(db, scrutinee, env, subst);
            let scrut_is_param = matches!(
                resolved_of(db, scrutinee),
                Resolved::Ref { value } if env.contains_key(&value)
            ) || matches!(
                resolved_of(db, scrutinee),
                Resolved::Param { binder } if env.contains_key(&binder)
            );
            for (pat, _) in &arms {
                if let Some(pt) = literal_pattern_ty(db, *pat) {
                    let _ = crate::unify::unify(subst, &st, &pt);
                } else if scrut_is_param && let Some(pt) = pattern_implied_ty(db, *pat, fresh) {
                    let _ = crate::unify::unify(subst, &st, &pt);
                }
            }
            // A parameter RETURNED DIRECTLY by one arm is constrained by the OTHER arms' result type (the
            // arm bodies agree in type). `(match xs (Nil ys) ((Cons …) (Cons …)))` → the `Nil` arm returns
            // the parameter `ys` and the `Cons` arm returns a `Code`, so `ys : Code` — a pass-through /
            // accumulator parameter otherwise only echoed out is inferred rather than left a free var.
            // NARROW on purpose: only an arm body that IS a bare parameter reference is constrained, and
            // only against a SIBLING arm's DETERMINED (non-var, non-`Any`) result. Unifying arbitrary arm
            // types against each other over-constrains (a sibling's own unsolved var spuriously conflicts);
            // this pins exactly the echoed-parameter case without touching well-typed programs.
            let arm_param = |db: &mut Db, body: StructId| -> Option<StructId> {
                match resolved_of(db, body) {
                    Resolved::Ref { value } if env.contains_key(&value) => Some(value),
                    Resolved::Param { binder } if env.contains_key(&binder) => Some(binder),
                    _ => None,
                }
            };
            let arm_list: Vec<StructId> = arms.iter().map(|(_, b)| *b).collect();
            for &body in &arm_list {
                let Some(pbinder) = arm_param(db, body) else {
                    continue;
                };
                let Some(pvar) = env.get(&pbinder).cloned() else {
                    continue;
                };
                for &other in &arm_list {
                    if other == body || arm_param(db, other).is_some() {
                        continue; // self, or another echoed param — no determined type to borrow
                    }
                    let ot = arg_ty_in_env(db, other, env, subst);
                    let applied = subst.apply(&ot);
                    if !matches!(applied, Ty::Any | Ty::Var(_)) {
                        let _ = crate::unify::unify(subst, &pvar, &applied);
                    }
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

/// The type a STRUCTURAL match pattern requires of the value it matches — the pattern's implied
/// scrutinee type, with a fresh variable for every binder/wildcard sub-position. Used by
/// `collect_param_constraints` (only when the scrutinee IS a parameter) to thread a pattern's SHAPE onto
/// that parameter, so a recursive tree-walker's parameter type is inferred rather than left a free var.
/// Returns `None` for a pattern whose shape does not further constrain the scrutinee (a bare binder or a
/// wildcard — matches anything — or a non-constructor head).
///
/// A `(tuple p0 … pn)` implies `Ty::Tuple([implied(p0) …])` (a var where a sub-position is a bare
/// binder). A variant pattern `(V p)` / bare `V` implies the variant's owning SUM at the instantiation
/// its payload sub-pattern implies — instantiate `V`'s ctor scheme `payload… → Sum`, unify the payload
/// against the inner pattern's implied type, and return the partly-solved `Sum` (a nullary variant is the
/// bare `Sum`). A literal / bare-name pattern implies `None`.
///
/// The ctor scheme is instantiated with the CALLER's `fresh` so its variables never collide with the
/// solver's own variables (a collision would bind the wrong variable in the shared `subst`).
fn pattern_implied_ty(db: &mut Db, pat: StructId, fresh: &mut Fresh) -> Option<Ty> {
    if let Some(elems) = db
        .ast
        .as_form(pat, "tuple")
        .or_else(|| db.ast.as_ctor_form(pat, "tuple"))
        .map(<[StructId]>::to_vec)
    {
        let tys: Vec<Ty> = elems
            .iter()
            .map(|&e| pattern_implied_ty(db, e, fresh).unwrap_or_else(|| Ty::Var(fresh.var())))
            .collect();
        return Some(Ty::Tuple(tys.into()));
    }
    if let Some(g) = db.ast.as_form(pat, "guard").map(<[StructId]>::to_vec)
        && g.len() == 2
    {
        return pattern_implied_ty(db, g[0], fresh);
    }
    let (head, args): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => match children.first().copied() {
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, Vec::new()),
            Some(first) => (first, children[1..].to_vec()),
            None => return None,
        },
        crate::ast::Struct::Atom(_) => (pat, Vec::new()),
    };
    let scheme = crate::eval::scheme_of(db, head, fresh)?;
    let inst = crate::unify::instantiate(&scheme, fresh);
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
    // Only a SUM result is a variant constructor; anything else (an ordinary function) is not a pattern.
    if !matches!(result, Ty::Sum { .. }) {
        return None;
    }
    let mut subst = Subst::new();
    if !payloads.is_empty()
        && let Some(&arg) = args.first()
        && let Some(implied) = pattern_implied_ty(db, arg, fresh)
    {
        let payload_ty = if payloads.len() == 1 {
            payloads[0].clone()
        } else {
            Ty::Tuple(payloads.clone().into())
        };
        let _ = crate::unify::unify(&mut subst, &payload_ty, &implied);
    }
    Some(subst.apply(&result))
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
    // `Qty.of x u` — attach a compile-time unit to a numeric value. Its result type is `(Qty T u)` where
    // `T` is the VALUE argument's type and `u` is the VALUE of the second argument (a compile-time unit,
    // read by `eval::unit_of` — NOT an HM-unified variable, exactly as `Prim::Wrap` reads its target
    // width off the solved type rather than the scheme). Checked before the scheme-peeling loop because
    // the unit is not expressible as a static `(meta t)` arrow. If the unit does not reduce (a malformed
    // unit expression), fall through to the generic path (which yields `Any`, faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf) && args.len() == 2
    {
        let inner = type_of(db, args[0]);
        if let Some(unit) = crate::eval::unit_of(db, args[1]) {
            return Ty::Qty {
                inner: Box::new(inner),
                unit,
            };
        }
    }
    // `Qty.value q` — recover the underlying numeric value, DISCARDING the unit. Its result is the
    // quantity's INNER type; a non-quantity argument yields `Any` (faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyValue)
        && args.len() == 1
    {
        return match type_of(db, args[0]) {
            Ty::Qty { inner, .. } => *inner,
            _ => Ty::Any,
        };
    }
    // `Unit.in target q` — explicit conversion. The result is a quantity at the TARGET unit (read from
    // arg0 by `unit_of`), carrying q's inner numeric type: `(Unit.in metre (Qty.of 3.0 km))` :
    // `(Qty Float64 metre)`. A dimensional mismatch (target dimension ≠ q's) is reported by
    // `check_application` (CDZ0501); here we fill the value-column type. If the target unit doesn't
    // reduce, or q isn't a quantity, fall through (→ Any, faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
        && args.len() == 2
        && let (Some(target), Ty::Qty { inner, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
    {
        return Ty::Qty {
            inner,
            unit: target,
        };
    }
    // A binary OPERATOR applied to QUANTITIES — the dimensional result type (units-of-measure.md §How
    // Arithmetic Composes Dimensions). Only engages when an operand is a `Ty::Qty` (two bare numbers take
    // the ordinary scheme path). `+`/`-` keep the shared unit (result `(Qty T u)`); `*`/`/` COMPOSE units
    // by the group product/quotient (result `(Qty T (u_a·u_b))` / `(Qty T (u_a/u_b))` — a `Unit.one`
    // result renders dimensionless); comparisons yield `Bool`. The unit COMPOSITION is why this is not an
    // HM scheme — a unit multiplies, it does not unify. A dimensional/​numeric FAULT is reported by
    // `check_application`; here we only fill the value-column TYPE. (When the units disagree for `+`/`-`,
    // the fault fires there and this type is unused; we still return the lhs quantity so the shape stays
    // sane.)
    if args.len() == 2
        && let Some(prim) = crate::eval::meta_apply_of(db, head)
    {
        {
            let a = type_of(db, args[0]);
            let b = type_of(db, args[1]);
            let a_qty = matches!(a, Ty::Qty { .. });
            let b_qty = matches!(b, Ty::Qty { .. });
            if a_qty || b_qty {
                match prim {
                    crate::resolved::Prim::Add | crate::resolved::Prim::Sub => {
                        // The result unit: when both operands are at the SAME unit (scale), keep it (the
                        // common Layer-1 case — no conversion); when they share a dimension but DIFFER in
                        // scale (`metre` + `kilometre`), the result is the dimension's REFERENCE unit (the
                        // deterministic common unit each operand converts to — units-of-measure.md
                        // §Combining Units Of One Dimension Is Well-Formed). The inner numeric type is the
                        // lhs quantity's (both share it; the fault check enforces agreement).
                        if let (
                            Ty::Qty {
                                inner: ia,
                                unit: ua,
                            },
                            Ty::Qty { unit: ub, .. },
                        ) = (&a, &b)
                        {
                            let unit = if ua == ub {
                                ua.clone()
                            } else {
                                ua.at_reference()
                            };
                            return Ty::Qty {
                                inner: Box::new((**ia).clone()),
                                unit,
                            };
                        }
                        if let Ty::Qty { inner, unit } = &a {
                            return Ty::Qty {
                                inner: Box::new((**inner).clone()),
                                unit: unit.clone(),
                            };
                        }
                        return b;
                    }
                    crate::resolved::Prim::Mul | crate::resolved::Prim::Div => {
                        // Compose the units. A bare-number operand contributes `Unit.one` (scaling keeps
                        // the other's dimension — `(* (Qty 2 metre) 3)` stays metre).
                        let ua = match &a {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let ub = match &b {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let unit = if matches!(prim, crate::resolved::Prim::Mul) {
                            ua.mul(&ub)
                        } else {
                            ua.div(&ub)
                        };
                        // The inner numeric type is a quantity operand's inner (both share it).
                        let inner = match (&a, &b) {
                            (Ty::Qty { inner, .. }, _) | (_, Ty::Qty { inner, .. }) => {
                                (**inner).clone()
                            }
                            _ => Ty::Any,
                        };
                        return Ty::Qty {
                            inner: Box::new(inner),
                            unit,
                        };
                    }
                    crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq => return Ty::Bool,
                    _ => {}
                }
            }
        }
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

/// The integer TEXT a FLOAT LITERAL denotes when it is integer-valued AND fits the `expected` integer
/// type — the replacement for the mirror of the `of-int` coercion: an `Int`-expected position given a
/// float literal (`(+ 2 2.0)`). A reader-normalized `Decimal` is integer-valued exactly when its base-10
/// `exponent` is ≥ 0 (its significand carries no trailing zeros — `2.0` → `{[2], exp:0}`, `300.0` →
/// `{[3], exp:2}`, but `2.5` → `{[25], exp:-1}`). We reconstruct the integer as `significand · 10^exp`,
/// then REJECT it unless the value fits `expected`'s `(signed, width)` — so the suggested `2.0` → `2`
/// type-checks in ONE shot (the D7/D9 one-shot rule: never suggest a spelling that just cascades to the
/// next mismatch, here a CDZ0302 out-of-range). No prelude float→int conversion exists, so a NON-integer
/// float (`2.5`) or an out-of-range one gets `None` — an honest "no clean fix" rather than a wrong one
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Returns the minus-signed
/// decimal text (`-0.0` → `0`, since the integer zero has no sign).
fn integer_text_of_float_literal(
    d: &crate::ast::Decimal,
    expected: crate::ty::IntTy,
) -> Option<String> {
    // Only a non-negative exponent is a whole number (a normalized `Decimal` has no trailing-zero
    // significand, so a fractional value keeps a negative exponent).
    if d.exponent < 0 {
        return None;
    }
    // Base-256 significand → base-10 digit string (Horner, little-endian digits printed reversed), the
    // same conversion `Decimal::to_f64_bits` performs. An empty significand is zero.
    let mut digits: Vec<u8> = vec![0];
    for &byte in &d.significand {
        let mut carry = byte as u32;
        for dig in digits.iter_mut() {
            let v = (*dig as u32) * 256 + carry;
            *dig = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let mut s: String = digits.iter().rev().map(|d| (b'0' + d) as char).collect();
    let trimmed = s.trim_start_matches('0');
    s = if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    };
    // Multiply by 10^exponent = append that many zeros (unless the value is zero).
    if s != "0" {
        for _ in 0..d.exponent {
            s.push('0');
        }
    }
    // Build the value to range-check it against the expected width. Parse the decimal digit string into a
    // big-endian magnitude (Horner: acc = acc*10 + digit), then apply the sign — the inverse of the digit
    // build above, reused so the width check sees the exact integer.
    let mut mag: Vec<u8> = Vec::new(); // little-endian base-256 during build
    for ch in s.bytes() {
        let mut carry = (ch - b'0') as u32;
        for byte in mag.iter_mut() {
            let v = (*byte as u32) * 10 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            mag.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    while mag.last() == Some(&0) {
        mag.pop();
    }
    mag.reverse();
    let negative = d.negative && !mag.is_empty(); // zero is never negative
    let value = crate::ast::IntValue {
        negative,
        magnitude: mag,
    };
    if !value.fits_width(expected.ground_signed(), expected.ground_width()) {
        return None;
    }
    Some(if negative { format!("-{s}") } else { s })
}

/// If `expected` is a SUM type with a SINGLE-payload variant whose payload type (at `expected`'s
/// instantiation) agrees with `actual`, the variant's constructor NAME — the "try wrapping the
/// expression in `Some`" suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Derives the name GENERICALLY from the sum's declaration (which variant's payload fits),
/// never a hard-coded `Some`/`Ok` (the no-keys-outside-the-prelude rule). `None` when `expected` is not
/// a sum, or no variant's single payload matches — so a wrap is offered only when it would actually
/// resolve the mismatch. Variants are scanned in declaration order → deterministic.
fn wrap_variant_for(db: &mut Db, expected: &Ty, actual: &Ty) -> Option<String> {
    // Both a boxed `Ty::Sum` and an erased single-variant `Ty::Nominal` newtype offer the wrap: `(: n
    // Box)` for `(type Box (Wrap Int64))` suggests `(Wrap n)` exactly as `(: n Option)` suggests
    // `(Some n)` — the ctor is derived from the declaration either way.
    let decl = nominal_or_sum_decl(expected)?;
    // Snapshot (name, ctor) pairs first — `payload_ty_at_instantiation` borrows `db` mutably, so we
    // cannot hold a `&TypeDecl` across the calls.
    let variants: Vec<(String, StructId)> = db
        .type_decl_by_occ(decl)?
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

/// The UNDERLYING structural type of the nominal NEWTYPE declared at `decl`, or `None` if `decl` is not
/// an erasable newtype (so it stays an ordinary boxed `Ty::Sum`). A newtype is a SINGLE-variant sum
/// whose runtime box is erased — the realization of `type-system.md §Nominal Is An Orthogonal Modifier`
/// (the tag "adds nothing to the value's runtime representation", §156). The returned type is the
/// nominal's `inner` (a caller wraps it as `Ty::Nominal { decl, name, inner }`); the underlying shape:
///  - 0 payloads (`(type Marker (The))`) → `Ty::Unit`,
///  - 1 payload  (`(type UserId (Mk Int64))`) → that payload's type,
///  - n payloads (`(type Point (Mk Int64 Int64))`) → `Ty::Tuple([payload tys…])` — the same shape a
///    multi-payload variant already boxes, so the erased value IS that tuple handle.
///
/// A declaration is an erasable newtype iff ALL of:
///  1. it has EXACTLY ONE variant (a multi-variant sum needs the discriminant → stays boxed),
///  2. it is MONOMORPHIC (no type parameters — a generic single-variant sum's erasure is a follow-up;
///     until then it stays boxed, which is still correct), AND
///  3. no payload type mentions the declaration's OWN name (directly or nested). A recursive
///     single-variant sum (`(type Stream (More (Tuple Int64 Stream)))`) is NOT erased: its self-reference
///     `decode`s to a BOXED `Ty::Sum`, so erasing the outer while the inner stays boxed would be an
///     inconsistent representation for the same type. It stays a boxed `Ty::Sum` (decline-to-erase). AND
///  4. the underlying type is SUM-FREE (no `Ty::Sum`/`Ty::Nominal` anywhere inside). A payload that is
///     itself a sum/nominal (`(type W (Mk (Option Int64)))`, or a newtype wrapping another newtype) has
///     a HANDLE runtime representation, and erasing it would conflict with a wrapped newtype's own
///     erasure — so it stays boxed. This covers every idiomatic newtype (over an int, float, string,
///     bytes, list, or a tuple of those); nested-sum erasure is a possible follow-up.
///
/// **Why this decodes payloads DIRECTLY (not via `scheme_of`).** It is precomputed once at load into
/// `Db::newtype_inner`, which `decode_ty` then consults to normalize a `Sum` into a `Nominal`. Using the
/// CACHED `scheme_of` here would cache the ctor's result type as `Sum` (the pre-normalization value),
/// and that stale scheme would then make `(Mk 42)` type as `Sum` even after `decode_ty` starts returning
/// `Nominal` — a representation split. Decoding the payload occurrences directly (via `typeval_of`,
/// uncached for types) avoids seeding any scheme with a soon-to-be-stale result, and the sum-free
/// restriction makes the result independent of which other decls are already cached.
pub(crate) fn newtype_underlying(db: &mut Db, decl: StructId) -> Option<Ty> {
    let (payload_count, payloads, params) = {
        let td = db.type_decl_by_occ(decl)?;
        // (1) exactly one variant. GENERICS are now IN scope — a generic newtype's template carries its
        // params as `Ty::Var(i)` (positional), substituted per-instantiation at `decode_ty`.
        if td.variants.len() != 1 {
            return None;
        }
        let v = &td.variants[0];
        (v.payloads.len(), v.payloads.clone(), td.params.clone())
    };
    // A NULLARY single variant is a nominal Unit (`(type Marker (The))`).
    if payload_count == 0 {
        return Some(Ty::Unit);
    }
    // Decode each payload TYPE occurrence directly to a template `Ty` (no `scheme_of` — see the doc
    // comment), mapping a declaration PARAM name to `Ty::Var(its index)`. A single payload's type IS the
    // underlying template; multiple payloads box as one tuple (the shape a multi-payload variant already
    // builds), so the underlying type is their `Ty::Tuple`.
    let mut tys = Vec::with_capacity(payload_count);
    for p in payloads {
        tys.push(decode_payload_template(db, p, &params)?);
    }
    let inner = if tys.len() == 1 {
        tys.pop().unwrap()
    } else {
        Ty::Tuple(tys.into())
    };
    // (3) + (4): a self-reference decodes to a `Ty::Sum` naming this decl, and any other sum/nominal
    // payload is also a `Ty::Sum` here (nothing is normalized yet during the load-time precompute) — the
    // single sum-free check covers BOTH the recursion guard and the nested-sum restriction. A param
    // `Ty::Var` is sum-free, so a generic `(Box a)` PASSES (its inner is erasable); a `(Box (Option a))`
    // correctly STAYS BOXED (its `Option` payload decodes to a `Ty::Sum`).
    if ty_contains_sum(&inner) {
        return None;
    }
    Some(inner)
}

/// Decode a newtype payload TYPE occurrence to a template `Ty`, mapping a declaration PARAMETER name
/// (`params[i]`) to `Ty::Var(i)` — the positional slot `decode_ty` later substitutes the instantiation's
/// arg into. A NON-param type occurrence (a concrete `Int64`, `(List Int64)`, a self-reference `Box`,
/// another sum `(Option …)`) decodes via the ordinary `typeval_of` (a self-reference / sum yields a
/// `Ty::Sum`, which the sum-free guard then rejects). Descends the structural type forms so a param
/// NESTED in the payload — `(List a)`, `(Tuple a Int64)` — becomes a `Var` at its position while the rest
/// decodes concretely. This is the load-time, `scheme_of`-free dual of the substitution `decode_ty` does.
fn decode_payload_template(db: &mut Db, occ: StructId, params: &[String]) -> Option<Ty> {
    // A bare name that IS a param → its positional `Ty::Var`.
    if let Some(name) = db.ast.as_name(occ)
        && let Some(i) = params.iter().position(|p| p == name)
    {
        return Some(Ty::Var(i as u32));
    }
    // A compound type form whose ARGUMENTS may mention params — descend the shapes that carry element
    // types, mapping each child through this template decoder. A head we don't special-case (a concrete
    // scalar, a sum/self-ref) falls through to `typeval_of` below.
    if let crate::ast::Struct::List(children) = db.ast.get(occ) {
        let children = children.clone();
        match children.first().and_then(|&h| db.ast.as_name(h)) {
            Some("Tuple") => {
                let mut elems = Vec::with_capacity(children.len() - 1);
                for &c in &children[1..] {
                    elems.push(decode_payload_template(db, c, params)?);
                }
                return Some(Ty::Tuple(elems.into()));
            }
            Some("List") if children.len() == 2 => {
                return Some(Ty::List(Box::new(decode_payload_template(
                    db,
                    children[1],
                    params,
                )?)));
            }
            _ => {}
        }
    }
    // A concrete type occurrence with no free param — decode normally. (A self-reference or another sum
    // yields a `Ty::Sum` here, which `newtype_underlying`'s sum-free guard rejects — staying boxed.)
    crate::eval::typeval_of(db, occ)
}

/// Whether `ty` contains a `Ty::Sum` or `Ty::Nominal` anywhere (the newtype-erasure sum-free guard) — a
/// type whose runtime representation involves a nominal handle, which must not be erased into a newtype's
/// underlying value. Descends the structural compounds (tuple/record/list/fn); the scalar leaves are free.
fn ty_contains_sum(ty: &Ty) -> bool {
    match ty {
        Ty::Sum { .. } | Ty::Nominal { .. } => true,
        Ty::Tuple(elems) => elems.iter().any(ty_contains_sum),
        Ty::Record(fields) => fields.values().any(ty_contains_sum),
        Ty::List(elem) => ty_contains_sum(elem),
        Ty::Map(k, v) => ty_contains_sum(k) || ty_contains_sum(v),
        Ty::Set(elem) => ty_contains_sum(elem),
        Ty::Qty { inner, .. } => ty_contains_sum(inner),
        Ty::Fn(p, r) => ty_contains_sum(p) || ty_contains_sum(r),
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::Bytes
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::Float(_)
        | Ty::Type
        | Ty::Var(_)
        | Ty::Any => false,
    }
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

/// The CDZ0405 non-exhaustive-HANDLER rejection, enriched with an "add the missing arm" fix (the effect
/// analogue of `non_exhaustive_sum_reject`'s missing-match-arm fix — `spec/capabilities/diagnostics.md`
/// §A Diagnostic Carries A Route To A Fix, realizing `capabilities-and-effects.md` §A Handler Discharges
/// Its Effect's "SHOULD identify the omitted operations"). Names the omitted operations and appends a
/// TEMPLATE arm per omission to the handler's arms LIST — each `(op (_p0 …) s (resume unit s))` in the
/// canonical bare-op surface, with the op's arm arity of `_`-prefixed parameter binders (so it does not
/// itself warn unused), the state binder `s`, and a stateless `(resume unit s)` body the author fills
/// in. Heuristic: the arms are shaped right but their bodies are the author's to write. `handle_id` is
/// the `(handle seed (arms…) body)` form (internal shape after desugar); the fix anchors on its arms
/// LIST (child index 2), whose source span the in-place desugar preserved. Falls back to the plain
/// reject (no fix) if that arms node is absent.
fn non_exhaustive_handler_reject(
    db: &Db,
    handle_id: StructId,
    missing: &[crate::effects::MissingOp],
) -> Reject {
    // One template arm per missing op: `(op (_p0 …) s (resume unit s))`. A nullary op → empty params;
    // an N-ary op → N `_`-prefixed binders (so an unfilled placeholder does not itself warn unused). The
    // state binder is `s` and the body a `(resume unit s)` placeholder the author fills in.
    let arms: Vec<String> = missing
        .iter()
        .map(|m| {
            let binders: Vec<String> = (0..m.arity).map(|i| format!("_p{i}")).collect();
            format!("({} ({}) s (resume unit s))", m.name, binders.join(" "))
        })
        .collect();
    // The message NAMES the omitted operations AND spells the arm(s) to add inline — the guidance is
    // legible even without applying the structural fix (rustc "patterns `X` and `Y` not covered" style).
    let names: Vec<String> = missing.iter().map(|m| format!("`{}`", m.name)).collect();
    let message = format!(
        "this handler does not discharge every operation its effect declares: operation{} {} not \
         handled — a handle must discharge its effect's whole operation set; add {}",
        if missing.len() == 1 { "" } else { "s" },
        join_and_names(&names),
        arms.join(" "),
    );
    // The arms LIST is the handle form's 3rd child (internal shape `[handle, seed, arms, body]`). The
    // in-place desugar preserved its source span, so a structural `InsertArms` fix can splice the arms in
    // (unlike the fresh synthesized arms-list a rewrite-returning desugar would leave span-less).
    let arms_node = match db.ast.get(handle_id) {
        crate::ast::Struct::List(items) if items.len() == 4 => Some(items[2]),
        _ => None,
    };
    match arms_node {
        Some(arms_list) => Reject::coded(Code::HandlerNotExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(arms_list, arms)),
        None => Reject::coded(Code::HandlerNotExhaustive, message),
    }
}

/// Join names as `a`, `a and b`, or `a, b, and c` — the English list the "not handled" message reads
/// naturally with (matching the sibling `join_and` in `lower.rs`).
fn join_and_names(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
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

/// Whether `ty` is a DEFINITE non-sum type a variant pattern could never match — a scalar (`Int`, `Bool`,
/// `Float`, `Unit`), a `String`/`Bytes`, a `List`/`Map`/`Set`, a tuple, or a record. Excludes `Ty::Sum`
/// and `Ty::Nominal` (a variant pattern's legitimate targets) AND the UNDETERMINED types `Any`/`Var`/
/// `Type`/`Fn` — an unsolved scrutinee must still DECLINE, never be rejected here (a not-yet-inferred
/// parameter grounds to `Any`; rejecting it would fault a program a later solve types fine). So a variant
/// pattern over such a scrutinee is a genuine confusion, whereas over an undetermined one it is unknown.
fn definite_non_sum_scalar(ty: &Ty) -> bool {
    !matches!(
        ty,
        Ty::Sum { .. } | Ty::Nominal { .. } | Ty::Any | Ty::Var(_) | Ty::Type | Ty::Fn(_, _)
    )
}

/// Whether the match PATTERN at `pat` is headed by a VARIANT CONSTRUCTOR — `(C.Red)`, `(Some x)`, a bare
/// nullary variant name `None`, or such a pattern under a `(guard <pat> <cond>)` wrapper. Peels the guard,
/// then reads the pattern's constructor head the way the binding-pattern classifier does (a bare atom /
/// `(. Sum V)` member used whole, or a `(head arg…)` application's head) and asks `variant_owner_decl`
/// whether it names a sum's variant. `false` for a literal / bare binder / wildcard / tuple pattern — none
/// of which is variant-specific, so none conflicts with a scalar scrutinee.
fn pattern_is_variant_ctor(db: &mut Db, pat: StructId) -> bool {
    // Peel a `(guard <inner-pat> <cond>)` wrapper — the variant-ness is the inner pattern's.
    let inner = match db.ast.as_form(pat, "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pat,
    };
    let head = match db.ast.get(inner) {
        crate::ast::Struct::Atom(_) => inner,
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor is the pattern itself.
            Some(first) if db.ast.as_name(first) == Some(".") => inner,
            Some(first) => first,
            None => return false,
        },
    };
    crate::eval::variant_owner_decl(db, head).is_some()
}

/// Whether the lambda `head` REFERENCES its parameter whose name occurrence is `param_occ` anywhere in
/// its body — i.e. the parameter is USED, so its argument appears (substituted) in the reduced body and
/// need not be re-descended for faults. A body reference to a parameter resolves (via resolve's
/// `binder_in`) to a `Resolved::Ref` whose transitive target is the parameter's binder, or to a
/// `Resolved::Param { binder }`; the walk is a purely structural scan of the body's raw children (no
/// reduction, no lowering), bounded by the body size. A `false` result means the parameter is DEAD (the
/// body ignores it), so its argument's own faults are not otherwise collected and it must be descended.
fn param_is_referenced(db: &mut Db, head: StructId, param_occ: StructId) -> bool {
    let Some(body) = crate::eval::lambda_body(db, head) else {
        return false;
    };
    references_binder(db, body, param_occ)
}

/// Structural scan: does the resolved subtree at `node` contain a reference that resolves to the binder
/// `target` (a parameter's name occurrence)? Follows a `Resolved::Ref`/`Param` to its binder identity;
/// recurses the raw AST children otherwise. Bounded by the subtree size (no reduction).
fn references_binder(db: &mut Db, node: StructId, target: StructId) -> bool {
    match resolved_of(db, node) {
        Resolved::Param { binder } => binder == target,
        Resolved::Ref { mut value } => {
            // Follow the ref chain to a binder; a parameter reference's chain ends at the param binder.
            loop {
                if value == target {
                    return true;
                }
                match resolved_of(db, value) {
                    Resolved::Ref { value: next } => value = next,
                    Resolved::Param { binder } => return binder == target,
                    _ => break,
                }
            }
            false
        }
        _ => {
            // Not a direct reference — recurse the raw children (a name in binder position, a literal, an
            // operator head all simply do not match). This visits every value position of the body.
            if let crate::ast::Struct::List(children) = db.ast.get(node) {
                let children = children.clone();
                return children.iter().any(|&c| references_binder(db, c, target));
            }
            false
        }
    }
}

/// The diagnostic code for a LIST HOMOGENEITY violation between two element types that do not unify —
/// the same taxonomy line the numeric operators and `if`-branches draw (05-compound-types §A List Is An
/// Ordered Homogeneous Sequence). A HOMOGENEITY conflict is CDZ0201 (a MALFORMED list) in exactly two
/// shapes: two DISTINCT NUMERIC types (`Int64` vs `Float64` — the no-silent-promotion rule), and two
/// SAME-KIND compounds of DIFFERENT SHAPE (records of different field sets, or tuples of different arity
/// — the field set / arity IS the type, so they are genuinely different types the list cannot hold
/// together, which the equality path likewise rejects as incompatible shapes). Every OTHER disagreement
/// (a cross-KIND scalar clash `Int64` vs `Bool`, or a scalar-vs-compound) keeps the generic structural
/// mismatch CDZ0203 the unify produced. Mirrors the `if`-branch / connective taxonomy.
fn list_homogeneity_code(a: &Ty, b: &Ty) -> Code {
    let both_numeric =
        matches!(a, Ty::Int(_) | Ty::Float(_)) && matches!(b, Ty::Int(_) | Ty::Float(_));
    let both_records = matches!(a, Ty::Record(_)) && matches!(b, Ty::Record(_));
    let both_tuples = matches!(a, Ty::Tuple(_)) && matches!(b, Ty::Tuple(_));
    if both_numeric || both_records || both_tuples {
        Code::Malformed
    } else {
        Code::TypeMismatch
    }
}

/// Check every `(Unit.define #"name" base num den)` declaration for a CONFLICT — a name bound to two
/// different conversions (`units-of-measure.md` §A Named Unit's Conversion Is Unique) — and push CDZ0502
/// for each. A name conflicts when its reduced unit (`base` scaled by `num/den`) differs from the
/// BUILT-IN family unit of that name, or from an EARLIER `Unit.define` of it; an agreeing redeclaration
/// is admissible (a `Unit` compares by dimension AND scale, so "agrees" is exact). Runs once over
/// `db.unit_defines` (these are top-level declarations, not def bodies, so `type_errors` never reaches
/// them). `pub(crate)` — called from `compile::collect_faults`.
pub(crate) fn check_unit_defines(db: &mut Db, out: &mut Vec<Reject>) {
    // The declaration rows, in source order (name, base occurrence, scale).
    let defines: Vec<(String, StructId, i128, i128)> = db.unit_defines.clone();
    // The reduced unit for each name seen so far (built-ins seeded lazily on first mention).
    let mut seen: std::collections::HashMap<String, crate::ty::Unit> =
        std::collections::HashMap::new();
    for (name, base_occ, num, den) in &defines {
        // Reduce this declaration to the unit it denotes.
        let Some(base) = crate::eval::unit_of(db, *base_occ) else {
            continue; // a malformed base — its fault surfaces via the ordinary descent
        };
        let Some(this_unit) = base.scaled(*num, *den) else {
            continue;
        };
        // Compare against the BUILT-IN family unit of this name (reduce its registry entry once).
        if let Some((dim_pairs, bnum, bden)) = db.unit_families.get(name).cloned() {
            let mut u = crate::ty::Unit::one();
            for (b, e) in &dim_pairs {
                u = u.mul(&crate::ty::Unit::base(b.clone()).pow(*e));
            }
            if let Some(builtin) = u.scaled(bnum, bden)
                && builtin != this_unit
            {
                out.push(Reject::coded(
                    Code::UnitConflict,
                    format!("unit `{name}` is already a built-in unit with a different conversion"),
                ));
                continue;
            }
        }
        // Compare against an EARLIER declaration of the same name.
        match seen.get(name) {
            Some(prior) if *prior != this_unit => {
                out.push(Reject::coded(
                    Code::UnitConflict,
                    format!("unit `{name}` is declared twice with different conversions"),
                ));
            }
            _ => {
                seen.insert(name.clone(), this_unit);
            }
        }
    }
}

fn check_application(db: &mut Db, head: StructId, args: &[StructId], out: &mut Vec<Reject>) {
    // `Unit.in target q` — the TARGET unit must share q's DIMENSION (you can convert metres to
    // kilometres, not metres to seconds). A cross-dimension conversion is CDZ0501 (units-of-measure.md
    // §A Dimensional Mismatch Is An Error). Read the target unit + q's unit; descend into q for its own
    // faults, then return (skip the generic scheme-unify — `Unit.in` has no HM scheme).
    if args.len() == 2
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
    {
        if let (Some(target), Ty::Qty { unit: qu, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
            && !target.same_dimension(&qu)
        {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Unit.in target dimension differs from the quantity's (CDZ0501)");
            out.push(Reject::coded(
                Code::DimensionMismatch,
                "converting a quantity to a unit of a different dimension",
            ));
        }
        collect(db, args[1], out);
        return;
    }
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
        // Comparing a SYMBOL to the plain STRING it wraps is a comparison ACROSS THE NOMINAL BOUNDARY —
        // CDZ0202 (17-symbols "a string compared to a symbol is a type error"). A Symbol is a nominal over
        // String; a nominal value never silently compares equal to the untagged shape it was declared
        // distinct from (`type-system.md` §Nominal Types Are Not Comparable Across Their Boundary), the
        // same rule the nominal-record-vs-plain-record case pins. Reported HERE for the right code (the
        // generic scheme-unify below would give the plain CDZ0203); fires on either operand order.
        if matches!((&a, &b), (Ty::Symbol, Ty::String) | (Ty::String, Ty::Symbol)) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: comparing a Symbol to a String across the nominal boundary (CDZ0202)");
            out.push(Reject::coded(
                Code::NominalMismatch,
                "a Symbol and a String are not comparable across the nominal boundary",
            ));
            return;
        }
    }
    // A built-in arithmetic/comparison/equality operator with a TEXT operand (String/Bytes) against a
    // SCALAR operand (a number, a Bool, a Char) is a CROSS-KIND clash — a malformed operation, CDZ0201,
    // NOT the CDZ0203 a same-kind SHAPE mismatch (two tuples of different arity, Int-vs-Bool) gets. This
    // mirrors the map-vs-record kind clash above: `(+ 1 "two")`, `(< 1 "x")`, `(> "x" 1)` compare a
    // heap-allocated text value against an unboxed scalar — there is no shared order or arithmetic across
    // that kind boundary (type-system.md §Structural Values Are Comparable Only When Their Shapes Match —
    // the cross-kind case; 07-type-system.sexp pins these at CDZ0201, the equality-companion code, while
    // Int-vs-Bool `(< 1 true)` — two scalars — stays the generic CDZ0203). The generic scheme-unify below
    // would report `unify::mismatch` = CDZ0203; catch the text/scalar clash FIRST for the right code. Only
    // fires when EXACTLY one side is text and the other is a definite scalar — String-vs-String (a valid
    // comparison), text-vs-compound, and two scalars all fall through to the generic path unchanged.
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
                    | crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            )
        )
    {
        let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
        let is_text = |t: &Ty| matches!(t, Ty::String | Ty::Bytes);
        let is_scalar = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char);
        if (is_text(&a) && is_scalar(&b)) || (is_scalar(&a) && is_text(&b)) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: text operand against a scalar operand — distinct kinds (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                format!(
                    "a {} and a {} are different types (this operation is not defined across that kind boundary)",
                    a.render_name(),
                    b.render_name()
                ),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
    }
    // DIMENSIONAL check on a binary operator applied to QUANTITIES (units-of-measure.md §A Dimensional
    // Mismatch Is An Error). Fires BEFORE the generic scheme-unify (whose `∀a. (Int a) → …` scheme has no
    // notion of a unit) and `return`s to avoid a duplicate report:
    //  - `+`/`-`/comparison/`=`: the two operands' DIMENSIONS must be EQUAL (Layer 1 = same unit exactly;
    //    conversion between different units of one dimension is Layer 2). A length + a time, or a quantity
    //    + a bare number (no implicit dimensionless coercion), is CDZ0501. The inner numeric types must
    //    ALSO agree (the numeric core is unchanged under a unit — an Int64 metre + a Float64 metre is the
    //    numeric mismatch CDZ0301, reported here so it is not masked).
    //  - `*`/`/`: ALWAYS well-formed on quantities — the dimensions COMPOSE (they are not required equal),
    //    so no dimensional fault; the result unit is computed in `apply_type`. (A quantity × a bare number
    //    is Layer 2 scaling; in Layer 1 a mixed quantity/non-quantity `*`/`/` is left to the generic path.)
    if args.len() == 2 {
        let prim = crate::eval::meta_apply_of(db, head);
        let is_additive = matches!(
            prim,
            Some(
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Compare
            )
        );
        let is_multiplicative = matches!(
            prim,
            Some(crate::resolved::Prim::Mul | crate::resolved::Prim::Div)
        );
        let a0 = type_of(db, args[0]);
        let b0 = type_of(db, args[1]);
        let any_qty = matches!(a0, Ty::Qty { .. }) || matches!(b0, Ty::Qty { .. });
        // `*`/`/` on a quantity is ALWAYS well-formed dimensionally (the dimensions compose, not required
        // equal), so it must NOT reach the generic scheme-unify below (which would unify a `Ty::Qty`
        // against `(Int a)` → a spurious CDZ0203). Skip the fault check, descend for operand faults, and
        // return — the result unit is computed in `apply_type`.
        if is_multiplicative && any_qty {
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        if is_additive {
            let a = a0;
            let b = b0;
            // Only engage when at least one operand is a quantity — two bare numbers take the ordinary
            // numeric path (CDZ0301 etc.), unchanged.
            if matches!(a, Ty::Qty { .. }) || matches!(b, Ty::Qty { .. }) {
                match (&a, &b) {
                    (
                        Ty::Qty {
                            inner: ia,
                            unit: ua,
                        },
                        Ty::Qty {
                            inner: ib,
                            unit: ub,
                        },
                    ) => {
                        // COMPATIBILITY is DIMENSIONAL, not by-unit: two units of one dimension at
                        // DIFFERENT scales (`metre` + `kilometre`) are well-formed and auto-convert; only
                        // a DIMENSION mismatch (`metre` + `second`) is CDZ0501 (units-of-measure.md §A
                        // Dimensional Mismatch Is An Error / §Combining Units Of One Dimension Is
                        // Well-Formed). So gate on `same_dimension` (the exponent map), NOT `==` (which
                        // also compares scale — that distinction is TYPE identity, checked at annotation).
                        if !ua.same_dimension(ub) {
                            trace!(target: "rcdzc::infer", head = head.0, "fault: combining quantities of incompatible dimension (CDZ0501)");
                            out.push(Reject::coded(
                                Code::DimensionMismatch,
                                format!(
                                    "{} quantities of incompatible dimension: {} and {} — {} requires \
                                     equal dimensions (units are never silently converted across \
                                     dimensions)",
                                    additive_op_gerund(prim),
                                    ua.render_human(),
                                    ub.render_human(),
                                    additive_op_noun(prim),
                                ),
                            ));
                        } else {
                            // Same dimension — the INNER numeric types must still agree (no promotion
                            // under a unit): unify them and report a numeric mismatch as CDZ0301.
                            let mut subst = Subst::new();
                            if let Err(reject) = crate::unify::unify(&mut subst, ia, ib) {
                                out.push(reject);
                            }
                        }
                        for &arg in args {
                            collect(db, arg, out);
                        }
                        return;
                    }
                    // A quantity combined additively with a NON-quantity (a bare number) — no implicit
                    // dimensionless coercion (the numeric core's no-silent-promotion discipline applied to
                    // dimensions). CDZ0501.
                    _ => {
                        trace!(target: "rcdzc::infer", head = head.0, "fault: combining a quantity with a non-quantity (CDZ0501)");
                        out.push(Reject::coded(
                            Code::DimensionMismatch,
                            format!(
                                "{} a quantity and a plain number: {} and {} — a quantity has a \
                                 dimension a bare number lacks, and there is no implicit \
                                 dimensionless coercion",
                                additive_op_gerund(prim),
                                a.render_name(),
                                b.render_name(),
                            ),
                        ));
                        for &arg in args {
                            collect(db, arg, out);
                        }
                        return;
                    }
                }
            }
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
    // `Set.of list` — the set is HOMOGENEOUS: its elements (the list's) must share one type
    // (collections-and-text.md §A Set Is A Collection Of Unique Elements — elements of ONE type). A
    // mismatch is CDZ0201 (a SET homogeneity violation — the corpus codes it like the map homogeneity
    // cases), NOT the CDZ0203 the list-element unify would give on its own. So check the list argument's
    // element types HERE and, on a mismatch, report CDZ0201 + descend into the elements, stopping before
    // the generic path lets the inner list emit CDZ0203. (A homogeneous list flows through unchanged.)
    if args.len() == 1 && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::SetOf)
    {
        // Read the list argument's element occurrences — the `(list …)` string-head form, the `list`
        // name-alias application, or a `Resolved::List`.
        let list = args[0];
        let elems: Vec<StructId> = match resolved_of(db, list) {
            Resolved::List { elems } => elems.to_vec(),
            Resolved::Apply { head: lh, args: la }
                if crate::eval::meta_apply_of(db, lh) == Some(crate::resolved::Prim::ListNew) =>
            {
                la.to_vec()
            }
            _ => Vec::new(), // not a visible list literal — the generic path handles it
        };
        let mut subst = Subst::new();
        let mut mixed = false;
        if let Some(&first) = elems.first() {
            let first_ty = type_of(db, first);
            for &e in elems.iter().skip(1) {
                let et = type_of(db, e);
                if crate::unify::unify(&mut subst, &first_ty, &et).is_err() {
                    mixed = true;
                }
            }
        }
        if mixed {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Set.of elements do not share one type (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                "a set contains elements of one type (its elements do not share a type)",
            ));
            for &e in &elems {
                collect(db, e, out);
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
                if crate::unify::unify(&mut subst, &first_ty, &et).is_err() {
                    // The unify reports the generic CDZ0203. But list homogeneity draws the SAME taxonomy
                    // line the numeric operators and `if`-branches do (05-compound-types): a HOMOGENEITY
                    // violation between two DISTINCT NUMERIC types (`(list 1 2.5)` — Int64 vs Float64, the
                    // no-silent-promotion rule) or two SAME-KIND-DIFFERENT-SHAPE compounds (`(list (record
                    // (a 1)) (record (b 2)))` — records of different field sets; `(list (tuple 1 2) (tuple 1
                    // 2 3))` — tuples of different arity, where the field set / arity IS the type) is a
                    // MALFORMED list (CDZ0201), not the generic structural mismatch (CDZ0203) a cross-KIND
                    // scalar clash (`(list 1 true)` — Int64 vs Bool) is. Reclassify exactly those two shapes;
                    // every other element disagreement keeps the unify's CDZ0203.
                    let code = list_homogeneity_code(&first_ty, &et);
                    trace!(target: "rcdzc::infer", head = head.0, ?code, "fault: list elements differ in type");
                    out.push(Reject::coded(
                        code,
                        format!(
                            "list elements must share one type: {} and {}",
                            first_ty.render_name(),
                            et.render_name()
                        ),
                    ));
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
        let mut reduced_ok = false;
        if let Some(mut guard) = db.enter_reduction() {
            let g = guard.db();
            if let Ok(Some(reduced)) = crate::eval::apply_lambda(g, head, args) {
                collect(g, reduced, out);
                reduced_ok = true;
            }
        } else {
            // The reduction depth limit was hit — the reduced body was NOT collected here, so this
            // application's fault set is INCOMPLETE. Mark the walk limited so it is not memoized (a
            // shallower entry would reduce and collect the body). See `collect_cache`.
            db.collect_limited = true;
        }
        // (3) Descend the ARGUMENTS for their OWN faults (an unbound name / malformed application in an
        //     argument, whether or not the body uses it). When the reduction SUCCEEDED (`reduced_ok`), a
        //     USED argument's faults are already in the reduced body (step 2), so descend only the DEAD
        //     arguments — the ones the body does not reference, hence absent from the reduced body. This
        //     is what keeps a deep call chain `(f (f … (f 0)))` LINEAR: `f` uses its parameter, so no
        //     argument is dead and no argument is re-descended (the O(N³) came from re-walking every
        //     argument here AND in the reduced body, compounded per level). When the reduction did NOT
        //     produce a body (a recursive callee, the depth limit), NOTHING covered the arguments, so
        //     descend them ALL — matching the pre-change behavior for that case.
        let params = crate::eval::lambda_params_of(db, head).unwrap_or_default();
        for (i, &arg) in args.iter().enumerate() {
            let covered = reduced_ok
                && params
                    .get(i)
                    .is_some_and(|&p| param_is_referenced(db, head, p));
            if !covered {
                collect(db, arg, out);
            }
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
                            "{} {} — it is not a function",
                            crate::diag::NOT_A_FUNCTION_PREFIX,
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
                    } else if let (Some(da), Some(db_)) =
                        (nominal_or_sum_decl(&sparam), nominal_or_sum_decl(&sat))
                        && da != db_
                        && same_sum_shape(db, da, db_)
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
                    } else if let (Ty::Float(_), Ty::Int(actual_int)) = (&sparam, &sat) {
                        // An INTEGER operand where a FLOAT is expected (`(+. x 2.0)` with `x : Int64`) —
                        // the numeric model has NO silent promotion, so this is CDZ0301. But it has a
                        // mechanical repair: the corpus-blessed `(<FloatType>.of-int …)` conversion
                        // (`06-numeric-model.sexp` — `(+. (Float64.of-int 1) 2.0)`). The float type's own
                        // rendered name gives the module (`Float64`/`Float32`), so the ctor is derived, not
                        // hard-coded (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
                        // Fix). The reject KEEPS its unify-produced message + code; the fix rides alongside.
                        //
                        // ⚠ `of-int : Int64 → Float` — it takes EXACTLY `Int64`. So a bare
                        // `(Float64.of-int x)` is correct ONLY when the operand is already `Int64`; for a
                        // NARROWER int (`x : Int32`) it must first widen `(Int64.of x)`, giving the nested
                        // `(Float64.of-int (Int64.of x))`. Emit the operand-width-aware form so the
                        // suggested fix actually type-checks in one shot (verified by test), not a fix that
                        // just cascades to the next mismatch.
                        let float_name = sparam.render_name();
                        let (prefix, suffix, verb) = if actual_int.ground_width() == 64
                            && actual_int.ground_signed()
                        {
                            (
                                format!("({float_name}.of-int "),
                                ")".to_string(),
                                format!(
                                    "convert the integer to {float_name} with `{float_name}.of-int`"
                                ),
                            )
                        } else {
                            // Non-Int64 operand: widen to Int64 first, then to the float.
                            (
                                format!("({float_name}.of-int (Int64.of "),
                                "))".to_string(),
                                format!(
                                    "convert to {float_name} with `{float_name}.of-int (Int64.of …)`"
                                ),
                            )
                        };
                        out.push(reject.with_fix(Fix::wrap_heuristic(arg, prefix, suffix, verb)));
                    } else if let (Ty::Int(expected), Ty::Int(_)) = (&sparam, &sat)
                        && crate::ty::ALIASED_INT_WIDTHS.contains(&expected.ground_width())
                    {
                        // Two INTEGER types of different width/sign (`(+ a b)` with `a:Int32`, `b:Int64`)
                        // — CDZ0301, no silent promotion. The repair is the corpus-blessed CHECKED
                        // conversion `(<TargetInt>.of operand)` (`06-numeric-model.sexp` — `(+ 2
                        // (Int64.of 1))`), converting the mis-typed operand to the EXPECTED type. The
                        // target's name is the expected int type's `render_name()` (`Int64`/`UInt8`/…),
                        // derived not hard-coded. GATED to an ALIASED width ({8,16,32,64}): only those are
                        // BOUND names — a non-aliased `Int48` renders to a name nothing binds, so
                        // suggesting `(Int48.of …)` would itself be unbound (worse than no fix), so a
                        // non-aliased target gets the plain reject. Heuristic: `.of` is CHECKED (traps out
                        // of range), and which operand to convert is a guess (convert THIS operand to the
                        // other's type).
                        let int_name = sparam.render_name();
                        out.push(reject.with_fix(Fix::wrap_heuristic(
                            arg,
                            format!("({int_name}.of "),
                            ")",
                            format!("convert to {int_name} with `{int_name}.of` (checked)"),
                        )));
                    } else if let (Ty::Int(expected), Ty::Float(_)) = (&sparam, &sat)
                        && let crate::ast::Struct::Atom(lid) = db.ast.get(arg)
                        && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
                        && let Some(int_text) = integer_text_of_float_literal(&dec, *expected)
                    {
                        // The MIRROR of the `of-int` coercion above: an INTEGER is expected but a FLOAT
                        // LITERAL is supplied (`(+ 2 2.0)`) — CDZ0301, no silent promotion. Unlike the
                        // int→float direction there is NO prelude float→int conversion op to wrap with, so
                        // a fix is possible ONLY for an integer-VALUED literal (`2.0`, `300.0`), whose
                        // clean repair is to drop the fractional form: REPLACE `2.0` with `2`. A
                        // non-integer literal (`2.5`) or an out-of-range one yields no `int_text` and falls
                        // through to the plain reject — honest, since truncating/rounding is a semantic
                        // choice the compiler must not make for the author (`spec/capabilities/
                        // diagnostics.md` §A Diagnostic Carries A Route To A Fix). The replacement is
                        // integer-valued and range-checked against the expected width, so it type-checks in
                        // ONE shot. HEURISTIC, not verified — like the sibling `of-int` wrap: the edit
                        // resolves the mismatch, but WHICH way the author meant to resolve it is a guess
                        // (drop `.0` to keep integers, vs. make the whole expression float `(+. 2.0 2.0)`).
                        // `--verify-fixes` upgrades it to verified on the recompile, so an agent still gets
                        // the machine-applicable signal without the compiler claiming intent it can't know.
                        out.push(reject.with_fix(Fix::replace_heuristic(arg, int_text)));
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
                    format!(
                        "{} {}",
                        crate::diag::NOT_A_FUNCTION_PREFIX,
                        other.render_name()
                    ),
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
/// The declaration occurrence of a NOMINAL type — a boxed `Ty::Sum` OR an erased `Ty::Nominal` newtype —
/// so the nominal-boundary comparison (`CDZ0202`) fires for BOTH: a single-variant sum that erased to a
/// `Ty::Nominal` (`(type A (Mk Int64))`) has the same "distinct declarations of the same shape are not
/// comparable" property its boxed multi-variant sibling does. `None` for a non-nominal type.
fn nominal_or_sum_decl(ty: &Ty) -> Option<StructId> {
    match ty {
        Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => Some(*decl),
        _ => None,
    }
}

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
            // A do-local `(type …)` / `(effect …)` / `(module …)` is a DECLARATION, not a value
            // expression — its record is synthesized at load (`db::collect_nested_decls` / the module
            // scan) and its names resolve through the ordinary decl paths. There is nothing to type-check
            // as a value (resolving the form as one would decline, e.g. "`module` is not an expression
            // here"), so skip it, like a `def`. (A module member's own body IS checked on demand through
            // the member access that reaches it, exactly like a def called through its name.)
            if matches!(
                db.ast.head_name(f),
                Some("type") | Some("effect") | Some("module")
            ) {
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
                // Two DISTINCT NUMERIC types (an `Int64` branch and a `Float64` branch) are the no-silent-
                // promotion rule (numeric-model.md #Numeric Types Do Not Silently Promote) — a MALFORMED
                // program (CDZ0201), NOT the structural type mismatch (CDZ0203) a cross-KIND disagreement
                // (Int vs Bool, a compound vs a scalar, two tuples of different arity) is. So an `if` whose
                // branches are both numeric but of different numeric type is CDZ0201; every other branch
                // disagreement stays CDZ0203. (02-binding "a conditional with integer and floating-point
                // branches is a type error" wants CDZ0201; the tuple-arity/kind cases want CDZ0203.)
                let both_numeric = matches!(then_ty, Ty::Int(_) | Ty::Float(_))
                    && matches!(else_ty, Ty::Int(_) | Ty::Float(_));
                let code = if both_numeric {
                    Code::Malformed
                } else {
                    Code::TypeMismatch
                };
                trace!(target: "rcdzc::infer", node = id.0, then_ty = %then_ty.render_name(), else_ty = %else_ty.render_name(), ?code, "fault: if branches differ");
                out.push(Reject::coded(
                    code,
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
        // Short-Circuit: each operand is type-checked as a Bool whether or not it is evaluated). A non-Bool
        // operand is a MALFORMED program (CDZ0201) — the operand simply is not the required type, like a
        // binary operator's operand, NOT the structural-shape mismatch (CDZ0203) a cross-kind disagreement
        // is (02-binding "a boolean connective with a non-boolean operand is a type error" wants CDZ0201).
        // Then descend for each operand's own faults.
        Resolved::And { lhs, rhs, is_and } => {
            let op = if is_and { "and" } else { "or" };
            for &operand in &[lhs, rhs] {
                let t = type_of(db, operand);
                if !t.agrees_with(&Ty::Bool) {
                    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: connective operand not Bool (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!("`{op}` operand must be Bool, found {}", t.render_name()),
                    ));
                }
                collect(db, operand, out);
            }
        }
        Resolved::Not { operand } => {
            let t = type_of(db, operand);
            if !t.agrees_with(&Ty::Bool) {
                trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: not operand not Bool (CDZ0201)");
                out.push(Reject::coded(
                    Code::Malformed,
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
                    // A NOMINAL newtype over a record is erased to that record at run time, so member
                    // access sees through the tag (`strip_nominal`): the access is well-formed iff the
                    // inner record type has the field. A nominal over a NON-record stays a non-record
                    // fault (the `other` arm).
                    match type_of(db, operand).strip_nominal() {
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
                                format!(
                                    "a bin integer segment must start on a byte boundary, but {} of \
                                     bit-fields precede it — {}",
                                    open_bits_phrase(bit_cursor),
                                    bits_to_byte_boundary_hint(bit_cursor),
                                ),
                            ));
                        }
                    }
                    crate::resolved::SegKind::Bytes { size } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                format!(
                                    "a bin bytes segment must start on a byte boundary, but {} of \
                                     bit-fields precede it — {}",
                                    open_bits_phrase(bit_cursor),
                                    bits_to_byte_boundary_hint(bit_cursor),
                                ),
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
                    format!(
                        "a bin form's bit-fields must close to a whole number of bytes, but they total \
                         {bit_cursor} bit{} — {}",
                        plural_s(bit_cursor),
                        bits_to_byte_boundary_hint(bit_cursor),
                    ),
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
                // An ANNOTATED binder `((: name T) value)` constrains the bound value's TYPE — and, when
                // `T` is a NARROW integer width and the value is an integer LITERAL, RANGE-CHECKS it,
                // exactly as a value annotation `(: value T)` does (CDZ0302 for a literal outside the
                // width). `check_binding_pattern` above only checks type AGREEMENT (`(: a Bool) 5` →
                // CDZ0203), and a deferred literal agrees with any int width, so an out-of-range narrow
                // literal (`(: a Int8) 200`) slipped through and ran to a value the type cannot hold. The
                // binder annotation must apply its width fit-check to the bound value, the let-binder
                // analogue of the parameter-substitution fit-check.
                if let Some(ann) = db.ast.as_form(lhs, ":")
                    && ann.len() == 2
                    && let Some(reject) = literal_width_fault(db, value, ann[1])
                {
                    out.push(reject);
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
            // EXHAUSTIVENESS is well-formedness — a non-exhaustive match is a compile-time error
            // (`core-semantics.md` §Matching Is Exhaustive Or Rejected) whether or not the enclosing def
            // is reached, exactly as an unbound name in an uncalled sibling is. The emit path lowers only
            // reached (and, for the standalone walk, nullary-exported) bodies, so `check` missed a
            // non-exhaustive match on a function PARAMETER; surface it here, where `collect_node` visits
            // EVERY match in every body. `match_nonexhaustive_fault` returns only the CDZ0210 (carrying
            // its "add the missing arm" fix), never a not-yet-lowerable decline — so this adds the
            // actionable fix to `check`/`--json`/`fix` without raising false alarms.
            if let Some(r) = crate::lower::match_nonexhaustive_fault(db, id) {
                out.push(r);
            }
            // SCRUTINEE / PATTERN TYPE COMPATIBILITY: a VARIANT-constructor pattern (`(C.Red)`, `(Some x)`)
            // over a scrutinee whose type is a DEFINITE NON-SUM — a scalar `(match 5 ((C.Red) …))`, a Bool,
            // a Float, a String — is a type confusion: the scrutinee has no variants to dispatch on. Reject
            // CDZ0203 (the same code `(: 5 Bool)` gets) rather than letting `lower_match` DECLINE it ("a
            // match pattern that is not a scalar literal or `_`"), which grades a genuine type error as a
            // to-do. Guarded on a DEFINITE non-sum scrutinee (`definite_non_sum_scalar`) so an undetermined
            // scrutinee (`Any` / an unsolved var — a not-yet-inferred param) still declines cleanly, never a
            // spurious reject; a mismatched-SUM pattern (a foreign variant over a different sum) is caught by
            // `pattern_constraints` in lowering with its own message, so this only adds the SCALAR case.
            let st = type_of(db, scrutinee);
            if definite_non_sum_scalar(&st) {
                for (pat, _) in &arms {
                    if pattern_is_variant_ctor(db, *pat) {
                        out.push(Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "a variant pattern cannot match a scrutinee of type {} — it is not a sum",
                                st.render_name()
                            ),
                        ));
                        break;
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
            } else if crate::eval::lambda_body(db, head).is_some() {
                // A LAMBDA head — `check_application` already collected the REDUCED body (step 2), which
                // contains each argument substituted at its use sites, so the used arguments' faults are
                // ALREADY gathered. Re-descending the raw arguments here would DOUBLE the walk of every
                // argument subtree — and on a deep call chain `(f (f … (f 0)))` (where each argument is
                // itself the next call) that doubling, compounded per level, is what made the fault walk
                // O(N³) (each of the N `collect(arg)` frames restarts a full O(N) descent, whose own
                // `type_of` adds another factor). Skip it. A DEAD argument (one the body never uses, so it
                // is NOT in the reduced body) still needs checking — `check_application` descends those
                // (only the un-substituted args) so their faults are not lost.
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::SetOf)
            ) {
                // `Set.of list` — `check_application` above already checked the list's element HOMOGENEITY
                // as a SET fault (CDZ0201) and, on a mismatch, descended into the elements + returned. So
                // descending into the list ARG here would RE-derive the list's own CDZ0203 (a redundant,
                // wrong-coded duplicate of the fault already reported). Instead descend into each ELEMENT
                // directly (a nested per-element fault still surfaces), never the list node as a whole.
                let list = args[0];
                let elems: Vec<StructId> = match resolved_of(db, list) {
                    Resolved::List { elems } => elems.to_vec(),
                    Resolved::Apply { head: lh, args: la }
                        if crate::eval::meta_apply_of(db, lh)
                            == Some(crate::resolved::Prim::ListNew) =>
                    {
                        la.to_vec()
                    }
                    // Not a visible list literal (a runtime list operand) — collect it normally.
                    _ => {
                        collect(db, list, out);
                        Vec::new()
                    }
                };
                for &e in &elems {
                    collect(db, e, out);
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
                        out.push(int_out_of_range_reject(
                            &annot_ty,
                            it.ground_signed(),
                            w,
                            &v,
                            ty_expr,
                        ));
                    }
                } else if let (
                    Ty::Qty {
                        inner: ai,
                        unit: au,
                    },
                    Ty::Qty {
                        inner: ei,
                        unit: eu,
                    },
                ) = (&annot_ty, &type_of(db, expr))
                    && au != eu
                {
                    // A DIMENSIONAL annotation conflict — the value derives one dimension but the
                    // annotation asserts another (`(: (* (Qty 2 metre) (Qty 3 metre)) (Qty Float64
                    // metre))` — the product is metre², annotated metre). This is the dimensional
                    // specialization of the annotation conflict: CDZ0501, not the CDZ0203 the generic
                    // unify below would give (units-of-measure.md §A Dimensional Mismatch Is An Error;
                    // the erasure fence's dimensional case). The inner types are irrelevant when the
                    // units already disagree — the dimension is the conflict.
                    let _ = (ai, ei);
                    trace!(target: "rcdzc::infer", node = id.0, "fault: annotation at a dimension the expression does not derive (CDZ0501)");
                    out.push(Reject::coded(
                        Code::DimensionMismatch,
                        format!(
                            "this expression has dimension {} but is annotated {} — the annotation \
                             must match the dimension the expression derives",
                            eu.render_human(),
                            au.render_human(),
                        ),
                    ));
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
                    // Name the nearest DECLARED operation of the effect + carry a replace fix on the
                    // mistyped op key (the effect-op analogue of the absent-field "did you mean?").
                    match crate::effects::nearest_declared_op(db, arm.op) {
                        Some((key_occ, candidate)) => out.push(
                            Reject::coded(
                                Code::HandlerUndeclaredOp,
                                format!(
                                    "this handler arm names an operation its effect does not declare \
                                     — did you mean `{candidate}`?"
                                ),
                            )
                            .at(arm.op)
                            .with_fix(Fix::replace_heuristic(key_occ, candidate)),
                        ),
                        None => out.push(
                            Reject::coded(
                                Code::HandlerUndeclaredOp,
                                "this handler arm names an operation its effect does not declare",
                            )
                            .at(arm.op),
                        ),
                    }
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
            // A HANDLER MUST DISCHARGE ITS EFFECT'S WHOLE OPERATION SET (CDZ0405,
            // `capabilities-and-effects.md` §A Handler Discharges Its Effect). A `handle E` names one
            // effect whose operations are a closed set, so — like a match over a sum — it must bind EVERY
            // operation; a handler missing one leaves that operation of the effect it claims to discharge
            // without a home. Only checked when NO arm named an undeclared operation (a CDZ0403 arm makes
            // the discharged effect ambiguous, so its own fault is the one to report). Names the omitted
            // operations AND carries an "add the missing arm" fix — a template arm per omission appended
            // to the arms LIST (the sibling of `non_exhaustive_sum_reject`'s missing-match-arm fix).
            let has_undeclared = arms
                .iter()
                .any(|arm| crate::effects::arm_op_names_undeclared_operation(db, arm.op));
            if !has_undeclared {
                let missing = crate::effects::handler_missing_operations(db, &arms);
                if !missing.is_empty() {
                    out.push(non_exhaustive_handler_reject(db, id, &missing));
                }
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
        | Resolved::SymbolConst(_)
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
