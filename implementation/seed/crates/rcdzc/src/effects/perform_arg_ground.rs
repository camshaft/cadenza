//! Perform-ARGUMENT type GROUNDING — `ground_perform_arg_ty` plus its under-determined-leaf commit helpers.
//! Grounds an under-constrained perform / host-op argument against the operation's DECLARED parameter type,
//! so the guest flatten emits the WIT-authoritative width / shape instead of a CDZ0910 stack imbalance.
//! THREE axes of under-determination: a free-var payload (a bare `(None)` : `(Option _)` — bound by `unify`),
//! an `Any`-element empty compound (a bare `(list)` : `(List Any)` — committed to the declared element, since
//! `Any` is not a `Var` and `unify` leaves it untouched; SHAPE 104), and a DEFERRED-width int literal
//! (committed to the declared FIXED width; SHAPE 103/104 width). Extracted from the effects module to keep
//! each file under the source-size lint.

use crate::ast::StructId;
use crate::db::Db;
use crate::resolve::resolved_of;
use crate::resolved::Resolved;

/// Ground a PERFORM ARGUMENT's under-constrained type against the operation's DECLARED parameter type
/// (`capabilities-and-effects.md` §Performing An Operation Is Typed — "performing an operation MUST check
/// its arguments against the operation's declared parameter types"). A bare `(None)` / `#list()` perform
/// arg infers `Option(_)` / `List(_)` — an ungrounded payload/element var — because `type_of(perform)`
/// unifies each arg into the op parameter only in its OWN LOCAL subst (to derive the perform's RESULT type,
/// `infer/construct.rs`), and never writes that grounding back to the arg node; so the arg node MEMOIZES the
/// unground `(Option _)`. At the host boundary the option/list arm guards on `abi_val_type(payload).is_some()`,
/// which FAILS for the unground var: the host-import functype builder drops the param (0 core slots) while the
/// arg marshal still pushes the folded value (1 slot) → "values remaining on stack" (CDZ0910), and the op
/// silently does not host-delegate. This hook — the perform-arg twin of [`ground_seed_if_handle_init`] — grounds
/// the arg's type against the op's declared param at memoization time, so every later read (incl. the emit) sees
/// the concrete type. GATED to a CONCRETE declared param (no free var): a generic op param leaves the arg
/// unchanged (no-op) and avoids cross-`Fresh` var aliasing, so it pins exactly a genuinely-declared width/
/// payload/element type. Fixes the whole under-constrained-perform-arg class (SHAPE 103 const-None option arg,
/// SHAPE 104 bare empty-list arg). Diagnosed by v-wit-boundary + v-core-opt; general form of
/// [`crate::infer::ground_handler_state_ty`].
pub fn ground_perform_arg_ty(db: &mut Db, id: StructId, t: crate::ty::Ty) -> crate::ty::Ty {
    // THREE grounding axes, all keyed off the op's DECLARED parameter type at this arg position:
    //   • a FREE-VAR shape — a bare `(None)` → `(Option _)` (SHAPE 103): the `unify` below binds the payload
    //     var to the declared one.
    //   • an `Any` shape — a bare EMPTY compound, e.g. `(list)` → `(List Any)` (SHAPE 104): the empty compound
    //     has no element value to ground FROM, and `Any` is NOT a `Var`, so `unify`/`subst.apply` leave it
    //     untouched — `commit_underdetermined_to_declared` adopts the concrete declared element for it below.
    //   • a DEFERRED-WIDTH int — a bare literal `3` (defaults to `Int64`), or a compound whose element is one
    //     (`#tuple(3 7)` → `(Tuple Int64 Int64)`). A deferred int has NO free var (its width DEFAULTS to
    //     Int64), so the free-var gate alone SKIPPED it — yet a HOST-op param is WIT-authoritative (`s32`), so
    //     the guest value's default `Int64` diverged from the component width and the guest flatten CDZ0910'd
    //     (the bare-tuple / SHAPE-103-104 width sibling; v-wit-boundary #10014/#10016 hold the clean decline).
    //     `unify` does NOT fix this: `unify_width`'s `Width::Deferred` arm is a NO-OP (a deferred width "agrees
    //     with any width" and STAYS deferred — the deferred-literal polymorphism the rest of inference relies
    //     on), so we COMMIT the deferred int widths to the declared FIXED widths EXPLICITLY below — a targeted
    //     narrowing at this perform-arg site (the analogue of an annotation `(: arg <op-param-ty>)`), WITHOUT
    //     changing global unify semantics. Sound: an out-of-range literal still faults CDZ0302 (the perform-arg
    //     range-check `width_fault_against_ty` at application.rs descends compounds), so narrowing the width
    //     cannot silently truncate an over-range value.
    if !crate::infer::ty_has_free_var(db, &t) && !ty_has_deferred_int(&t) && !ty_has_any(&t) {
        return t;
    }
    let Some(parent) = db.parent_of(id) else {
        return t;
    };
    // `parent` must be a perform `(E.op arg…)` — an Apply whose head is an effect operation — and `id` one of
    // its ARGUMENTS (not the head). `resolved_of` returns an owned `Resolved` (the `db` borrow ends here), so
    // the `scheme_of` mutation below is free of a borrow conflict.
    let (head, pos) = match resolved_of(db, parent) {
        Resolved::Apply { head, args } if crate::eval::effect_op_of(db, head).is_some() => {
            match args.iter().position(|&a| a == id) {
                Some(p) => (head, p),
                None => return t,
            }
        }
        _ => return t,
    };
    // Peel the op's declared `(meta t)` scheme to the parameter type at argument position `pos`.
    let mut fresh = crate::unify::Fresh::new();
    let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh) else {
        return t;
    };
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut param_ty = None;
    for i in 0..=pos {
        match cur {
            crate::ty::Ty::Fn(p, r) => {
                if i == pos {
                    param_ty = Some(*p);
                    break;
                }
                cur = *r;
            }
            _ => break,
        }
    }
    let Some(param_ty) = param_ty else {
        return t;
    };
    // Only ground against a CONCRETE declared param (no free var): a generic param is a no-op and could
    // otherwise alias the arg's own `Fresh` vars. This pins exactly the declared payload/element/width.
    if crate::infer::ty_has_free_var(db, &param_ty) {
        return t;
    }
    // FREE-VAR axis: bind an `(Option _)` / `(List _)` payload/element var to the declared one.
    let mut subst = crate::unify::Subst::new();
    let ncx = db.name_ctx();
    let _ = crate::unify::unify(&mut subst, &t, &param_ty, &ncx);
    let grounded = subst.apply(&t);
    // ANY + DEFERRED-WIDTH axes: commit each `Any` value leaf (an empty-compound element — a bare `(list)` :
    // `(List Any)`, which the free-var axis above CANNOT ground because `Any` is not a `Var` and `unify`
    // leaves it untouched — SHAPE 104) and each deferred int width/sign (left deferred by `unify_width`'s
    // no-op `Deferred` arm) to the declared param's concrete sub-type / FIXED width.
    commit_underdetermined_to_declared(&grounded, &param_ty)
}

/// Whether `t` carries an integer with a DEFERRED width or sign (a bare literal that has not been narrowed),
/// descending compounds. A deferred int is NOT a free var (its width DEFAULTS to `Int64`), so
/// [`ground_perform_arg_ty`] needs this SEPARATE gate to notice a `#tuple(3 7)`-shaped perform arg whose
/// element widths must still be committed to a WIT-authoritative declared param.
fn ty_has_deferred_int(t: &crate::ty::Ty) -> bool {
    use crate::ty::{Sign, Ty, Width};
    match t {
        Ty::Int(it) => matches!(it.width, Width::Deferred) || matches!(it.sign, Sign::Deferred),
        Ty::Tuple(elems) => elems.iter().any(ty_has_deferred_int),
        Ty::List(e) | Ty::Set(e) => ty_has_deferred_int(e),
        Ty::Map(k, v) => ty_has_deferred_int(k) || ty_has_deferred_int(v),
        Ty::Record(fields) => fields.values().any(ty_has_deferred_int),
        Ty::Sum { args, .. } => args.iter().any(ty_has_deferred_int),
        Ty::Qty { inner, .. } => ty_has_deferred_int(inner),
        _ => false,
    }
}

/// Whether `t` carries an `Any` leaf, descending compounds. An `Any` element arises for an EMPTY compound
/// (a bare `(list)` infers `(List Any)`, an empty map/set likewise) — an unconstrained element with no value
/// to ground FROM. `Any` is NOT a `Var`, so [`crate::infer::ty_has_free_var`] misses it and the FREE-VAR
/// axis's `unify`+`subst.apply` leaves it untouched; [`ground_perform_arg_ty`] needs this SEPARATE gate so it
/// still commits the `Any` to the WIT-authoritative declared element (SHAPE 104). Nominal is NOT descended
/// (its `inner` is a machine-rep template — matches `commit_underdetermined_to_declared`'s carve-out).
fn ty_has_any(t: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match t {
        Ty::Any => true,
        Ty::Tuple(elems) => elems.iter().any(ty_has_any),
        Ty::List(e) | Ty::Set(e) => ty_has_any(e),
        Ty::Map(k, v) => ty_has_any(k) || ty_has_any(v),
        Ty::Record(fields) => fields.values().any(ty_has_any),
        Ty::Sum { args, .. } => args.iter().any(ty_has_any),
        Ty::Qty { inner, .. } => ty_has_any(inner),
        _ => false,
    }
}

/// Commit each UNDER-DETERMINED leaf in `value` to the corresponding more-defined leaf in `declared`,
/// walking both types in PARALLEL and descending matching compound shapes. Two axes:
///   • an `Any` value leaf (the empty-compound element — a bare `(list)` infers `(List Any)`, no element
///     value to ground FROM) adopts the whole declared sub-type — SHAPE 104. (A `Var` payload is already
///     grounded by the caller's `unify`+`subst.apply` FREE-VAR axis before this walk, so it needs no arm
///     here; `Any` is NOT a `Var` and `unify` leaves it untouched, hence this explicit commit.)
///   • a DEFERRED int width/sign adopts the declared FIXED width/sign — the perform-arg width grounding
///     (SHAPE 103/104 width), the analogue of an annotation.
/// An already-fixed width, a non-`Any`/non-deferred leaf, a non-fixed declared leaf, or a shape mismatch is
/// left as-is (returns the `value` sub-type unchanged) — so it never FORCES a genuine fixed-width mismatch
/// (that is rejected earlier at `unify_width`, CDZ0301). Nominal is intentionally NOT descended (its `inner`
/// is a derived machine-rep template that must not be rebuilt here) — a nominal-wrapped under-determined arg
/// stays ungrounded (never a miscompile: worst case an invalid-wasm reject).
fn commit_underdetermined_to_declared(
    value: &crate::ty::Ty,
    declared: &crate::ty::Ty,
) -> crate::ty::Ty {
    use crate::ty::{IntTy, Sign, Ty, Width};
    match (value, declared) {
        // `Any` VALUE leaf adopts the concrete declared sub-type (the empty-compound element — SHAPE 104).
        // A concrete value against an `Any` declared leaf keeps the value (falls to the `_` arm below), so a
        // genuinely `Any`-typed op param never corrupts a determined arg.
        (Ty::Any, d) => d.clone(),
        (Ty::Int(vit), Ty::Int(dit)) => {
            let width = match (vit.width, dit.width) {
                (Width::Deferred, Width::Fixed(w)) => Width::Fixed(w),
                _ => vit.width,
            };
            let sign = match (vit.sign, dit.sign) {
                (Sign::Deferred, Sign::Fixed(s)) => Sign::Fixed(s),
                _ => vit.sign,
            };
            Ty::Int(IntTy { sign, width })
        }
        (Ty::Tuple(ve), Ty::Tuple(de)) if ve.len() == de.len() => Ty::Tuple(
            ve.iter()
                .zip(de.iter())
                .map(|(v, d)| commit_underdetermined_to_declared(v, d))
                .collect(),
        ),
        (Ty::List(v), Ty::List(d)) => Ty::List(Box::new(commit_underdetermined_to_declared(v, d))),
        (Ty::Set(v), Ty::Set(d)) => Ty::Set(Box::new(commit_underdetermined_to_declared(v, d))),
        (Ty::Map(vk, vv), Ty::Map(dk, dv)) => Ty::Map(
            Box::new(commit_underdetermined_to_declared(vk, dk)),
            Box::new(commit_underdetermined_to_declared(vv, dv)),
        ),
        (Ty::Record(vf), Ty::Record(df)) => {
            let committed: std::collections::BTreeMap<crate::resolved::Symbol, Ty> = vf
                .iter()
                .map(|(name, vt)| {
                    let ct = match df.get(name) {
                        Some(dt) => commit_underdetermined_to_declared(vt, dt),
                        None => vt.clone(),
                    };
                    (name.clone(), ct)
                })
                .collect();
            Ty::Record(std::rc::Rc::new(committed))
        }
        (Ty::Sum { decl, args: va }, Ty::Sum { args: da, .. }) if va.len() == da.len() => Ty::Sum {
            decl: *decl,
            args: va
                .iter()
                .zip(da.iter())
                .map(|(v, d)| commit_underdetermined_to_declared(v, d))
                .collect(),
        },
        (Ty::Qty { inner: vi, unit }, Ty::Qty { inner: di, .. }) => Ty::Qty {
            inner: Box::new(commit_underdetermined_to_declared(vi, di)),
            unit: unit.clone(),
        },
        _ => value.clone(),
    }
}
