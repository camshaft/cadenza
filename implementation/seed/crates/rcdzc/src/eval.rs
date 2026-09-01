//! The evaluator — the ONE compile-time reduction of an application through `Meta.apply`.
//!
//! To apply a value to arguments there is a single rule (`prelude-and-resolution.md` §A Form Whose
//! Head Is Not A Grammar Name Is Dispatched By The Kind Of Value Its Head Resolves To): project the
//! head value's `(meta apply)`; if it names something applyable, use it; otherwise the value is "not
//! applyable". The applyable bottom is a native [`Prim`] — an arithmetic operation, or a type
//! constructor. This module owns that reduction so infer and lower share one path.
//!
//! A type constructor applied BUILDS a value by APPENDING arena nodes (`Db::push_*`): `(Int 64)`
//! reduces to a fresh width-64 integer MODULE record — a record whose `(meta t)` is the concrete
//! `Int64` type and whose fields (`max`/`min`/…) are that width's specialization. So `Int64` and
//! `(Int 64)` denote the same reduced module, and projecting a field of it (`Int64.max`) is the
//! ordinary member projection. The built module is a normal arena record, resolved/typed/lowered by
//! the ordinary demand.

use crate::ast::{CompoundCtor, IntValue, Leaf, StructId};
use crate::db::Db;
use crate::fxhash::FxHashMap as HashMap;
use crate::resolve::resolved_of;
use crate::resolved::{Prim, Resolved, Symbol};
use crate::ty::{IntTy, Scheme, Ty, Width};
use crate::unify::Fresh;
use tracing::trace;

/// Read a value's `(meta t)` — its TYPE — as a [`Scheme`], reducing the type-lambda it holds. An
/// operator's `Meta.t` is a compile-time lambda `(fn (a) (-> (Int a) (-> (Int a) (Int a))))`; this
/// binds each lambda parameter to a FRESH variable, reduces the body's type expression symbolically
/// (so `(Int a)` becomes an integer of that fresh WIDTH variable, shared across every `a`), and closes
/// the mentioned variables into the scheme. This is how `infer` reads an operator's type generically
/// as DATA — one rule for every operator, no per-operator Rust. `None` if the value has no `(meta t)`
/// or its type expression is malformed.
pub fn scheme_of(db: &mut Db, id: StructId, fresh: &mut Fresh) -> Option<Scheme> {
    let t = project_meta(db, id, "t")?;
    // MEMOIZE by the `(meta t)` NODE `t` — shared across every application of the same operator (all
    // `+` occurrences project the one prelude `+`'s type-lambda). Reducing that lambda (`type_in_env`)
    // is ~half the cost of typing operator-heavy code, and it is a pure function of `t`'s structure.
    // The cached scheme is built with CANONICAL numbering (a private `Fresh` from 0), independent of
    // the caller's `fresh` counter; every caller `instantiate`s the result, which renames the bound
    // variables to truly-fresh ones — so one shared canonical scheme serves all uses. (Without the
    // cache each of N operator applications re-reduced the same lambda: O(N) redundant work.)
    if let Some(cached) = db.scheme_cache.get(&t) {
        return cached.clone();
    }
    // Compute with a PRIVATE canonical counter (from 0), independent of the caller's `fresh`. The
    // caller's `fresh` is intentionally NOT advanced: every caller `instantiate`s the returned scheme
    // before using it, and `instantiate` renames ALL of the scheme's bound variables to fresh ones
    // from the caller's counter — so the canonical numbers 0..k never survive into a solved type and
    // cannot collide. (This is why one shared canonical scheme is correct for every use site.)
    let mut canon = Fresh::new();
    let scheme = scheme_of_uncached(db, t, &mut canon);
    db.scheme_cache.insert(t, scheme.clone());
    let _ = fresh; // caller's counter untouched — instantiate freshens the canonical vars
    scheme
}

/// Reduce the `(meta t)` node `t` to a [`Scheme`] using the supplied `fresh` for its variable
/// numbering — the uncached core of [`scheme_of`]. Split out so [`scheme_of`] can compute the cached
/// value with a private canonical counter.
fn scheme_of_uncached(db: &mut Db, t: StructId, fresh: &mut Fresh) -> Option<Scheme> {
    // The `(meta t)` value is either a bare type (a scheme with no quantified vars) or a type-lambda
    // whose parameters are the quantified variables.
    match resolved_of(db, t) {
        Resolved::Lambda { params, body } => {
            // Bind each parameter occurrence to a fresh variable; the same parameter used twice in the
            // body shares its variable (so both operands of `+` unify to one width).
            let mut env: HashMap<StructId, TyOrWidth> = HashMap::default();
            let mut ty_vars = Vec::new();
            let mut width_vars = Vec::new();
            let mut sign_vars = Vec::new();
            for &p in params.iter() {
                let n = fresh.var();
                // A parameter's kind (type var vs width var) is decided by HOW it is used; we record a
                // single fresh number and let the use site pick. Track it as both — the body reduction
                // uses it as a width inside `(Int _)` and as a type elsewhere. A PAIRED fresh sign
                // variable makes an integer-typed operand generic over signedness too: `(Int a)` in an
                // operator's type-lambda denotes an integer of width `a` AND this shared sign, so `+`
                // is `∀(width a, sign s_a). (Int^{s_a}_a) → (Int^{s_a}_a) → (Int^{s_a}_a)` — one
                // operator serving signed and unsigned (a UInt64+UInt64 grounds `s_a` unsigned; an
                // Int64/UInt64 mix is a sign conflict, CDZ0301). The sign is shared across the three
                // `(Int a)` occurrences exactly as the width is.
                let s = fresh.var();
                env.insert(p, TyOrWidth { num: n, sign: s });
                ty_vars.push(n);
                width_vars.push(n);
                sign_vars.push(s);
            }
            let ty = type_in_env(db, body, &env)?;
            trace!(target: "rcdzc::eval", node = t.0, scheme = %ty.render_name(&db.name_ctx()), quantified = params.len(), "read (meta t) as a polymorphic scheme");
            // Only the vars actually mentioned matter; keep the lists (unused ones are harmless).
            Some(Scheme {
                ty_vars,
                width_vars,
                sign_vars,
                ty,
            })
        }
        // A non-lambda `(meta t)` is a monomorphic type.
        _ => {
            let scheme = typeval_of(db, t).map(Scheme::mono);
            if let Some(s) = &scheme {
                trace!(target: "rcdzc::eval", node = t.0, ty = %s.ty.render_name(&db.name_ctx()), "read (meta t) as a monomorphic type");
            }
            scheme
        }
    }
}

/// A lambda parameter's fresh variables — used as a type variable OR a width variable depending on the
/// position it appears in (`(Int a)` → width; a bare `a` → type), with a PAIRED sign variable so an
/// integer operand of variable width is generic over signedness too. `num` serves the type/width axis;
/// `sign` is the sign variable used when the parameter is a variable width inside `(Int a)`.
#[derive(Clone, Copy)]
struct TyOrWidth {
    num: u32,
    sign: u32,
}

/// Reduce a type expression to a `Ty` under an environment binding lambda parameters to fresh
/// variables — the symbolic sibling of `typeval_of`. A parameter reference becomes its variable
/// (`Ty::Var`); `(Int a)` becomes an integer whose WIDTH is the parameter's variable; a ground type or
/// a fully-constant constructor reduces as usual.
fn type_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<Ty> {
    match resolved_of(db, id) {
        // A lambda parameter used as a bare type → its type variable.
        Resolved::Param { binder } => env.get(&binder).map(|v| Ty::Var(v.num)),
        // A TUPLE-TYPE LITERAL `(a, s)` that resolves to `Resolved::Tuple` (rather than the `(Tuple …)`
        // application form handled by `Prim::TupleCtor` below) — reduce each element UNDER THE ENV so a
        // type parameter in an element becomes its type variable. Without this arm a NESTED tuple type in
        // a variant payload — e.g. the `(a, s)` inside `(type Iter (Mk s (-> s (Option (a, s)))))` — fell
        // to the concrete `_ => typeval_of` arm, which cannot reduce a param-bearing tuple and returned
        // `None`. That made `scheme_of` for the whole ctor `None`, so `variant_payload_type` read `None`
        // and the constructor was wrongly treated as NULLARY (CDZ0201 "a nullary variant takes the unit
        // value" at every `Mk(x, f)`). The `Prim::TupleCtor` arm handles the `(Tuple …)` spelling; this
        // handles the comma/`Resolved::Tuple` spelling, the same value either way.
        Resolved::Tuple { elems } => {
            let mut tys = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                tys.push(type_in_env(db, e, env)?);
            }
            Some(Ty::Tuple(tys.into()))
        }
        // A bare parameter REFERENCE used as a type (`(-> a (-> a Bool))` — the comparison operand): it
        // resolves to a `Ref` to the parameter occurrence (a body-position name is looked up, not a
        // formal), so follow the ref; if it lands on a parameter in `env`, that is its type variable.
        // (This is the type-position sibling of `width_in_env` following a ref for `(Int a)`.) A ref to
        // anything else reduces concretely below.
        Resolved::Ref { value } if matches!(resolved_of(db, value), Resolved::Param { .. }) => {
            type_in_env(db, value, env)
        }
        // `(Int W)` / `(UInt W)` where W may be a parameter (a width variable) or a constant.
        Resolved::Apply { head, args } => {
            let prim = meta_apply_of(db, head)?;
            match prim {
                Prim::IntCtor | Prim::UIntCtor if args.len() == 1 => {
                    let width = width_in_env(db, args[0], env)?;
                    // When the width is a bound PARAMETER (a width variable), the operand is generic over
                    // signedness too — its sign is that parameter's PAIRED sign variable, so `(Int a)` in
                    // `+`'s type-lambda relates signed and unsigned alike. When the width is CONCRETE
                    // (`(Int 8)`, `(UInt 8)`), the constructor fixes the sign as written — a literal
                    // annotation's type is exactly what it says.
                    let sign = match sign_in_env(db, args[0], env) {
                        Some(s) => crate::ty::Sign::Var(s),
                        None => crate::ty::Sign::Fixed(matches!(prim, Prim::IntCtor)),
                    };
                    Some(Ty::Int(IntTy { sign, width }))
                }
                // `(Float W)` where W may be a parameter (a width variable, in the `+.` operator's
                // type-lambda `(fn (a) … (Float a) …)`) or a constant. Reuses the integer `width_in_env`
                // — a float width IS a `Width`. The float analogue of the `IntCtor` arm.
                Prim::FloatCtor if args.len() == 1 => {
                    let width = width_in_env(db, args[0], env)?;
                    Some(Ty::Float(crate::ty::FloatTy { width }))
                }
                Prim::FnCtor if args.len() == 2 => {
                    let p = type_in_env(db, args[0], env)?;
                    let r = type_in_env(db, args[1], env)?;
                    Some(Ty::Fn(Box::new(p), Box::new(r)))
                }
                // A MULTI-ARGUMENT arrow `(-> A B … R)` (≥3 elements) CURRIES right-associatively into
                // `A -> (B -> (… -> R))` — the standard n-ary function type, matching how a multi-param
                // lambda `(fn (a b …) …)` types and how `apply_type` peels one arrow per argument. Without
                // this a `(-> Int64 Int64 Int64)` param read `None` (only arity 1 + 2 were handled), so a
                // multi-argument closure PARAMETER (e.g. a round-trip consumer `(: g (-> Int64 Int64 Int64))`)
                // solved `Any` and declined "parameter type is ambiguous". Fold from the right: the last
                // element is the result, each earlier element wraps it in one more arrow.
                Prim::FnCtor if args.len() >= 3 => {
                    let (last, rest) = args.split_last()?;
                    let mut acc = type_in_env(db, *last, env)?;
                    for &a in rest.iter().rev() {
                        let p = type_in_env(db, a, env)?;
                        acc = Ty::Fn(Box::new(p), Box::new(acc));
                    }
                    Some(acc)
                }
                // A SINGLE-element arrow `(-> R)` is a NULLARY operation type `Unit -> R` — the unit
                // domain is elided (an effect op `(op get (-> (List Int64)))` performed as `(E.get)`,
                // the same elided-unit convention `apply_type`'s nullary-perform arm honors). Without
                // this a nullary op has no `Fn` scheme, so its result type reads undetermined and the
                // resume-value/result-type check (CDZ0201) can never fire.
                Prim::FnCtor if args.len() == 1 => {
                    let r = type_in_env(db, args[0], env)?;
                    Some(Ty::Fn(Box::new(Ty::Unit), Box::new(r)))
                }
                // `(List a)` inside a type-lambda — the element reduced under the env (so `a` becomes its
                // type variable), then `Ty::List(elem)`. This is what makes `List.len`'s `(meta t)` =
                // `(fn (a) (-> (List a) Int64))` read as the scheme `∀a. (List a) → Int64`.
                Prim::ListCtor if args.len() == 1 => {
                    let elem = type_in_env(db, args[0], env)?;
                    Some(Ty::List(Box::new(elem)))
                }
                // `(Map k v)` inside a type-lambda — key and value reduced under the env (so `k`/`v`
                // become their type variables), then `Ty::Map(key, value)`. This is what makes a `Map`
                // op's `(meta t)` = `(fn (k v) (-> (Map k v) …))` read as the scheme `∀k v. (Map k v) → …`.
                Prim::MapCtor if args.len() == 2 => {
                    let key = type_in_env(db, args[0], env)?;
                    let value = type_in_env(db, args[1], env)?;
                    Some(Ty::Map(Box::new(key), Box::new(value)))
                }
                // `(Set a)` inside a type-lambda — the element reduced under the env, then `Ty::Set(elem)`.
                // Makes a `Set` op's `(meta t)` = `(fn (a) (-> (Set a) …))` read as `∀a. (Set a) → …`.
                Prim::SetCtor if args.len() == 1 => {
                    let elem = type_in_env(db, args[0], env)?;
                    Some(Ty::Set(Box::new(elem)))
                }
                // `(Qty T u)` inside a type-lambda / arrow — the QUANTITY type constructor: inner numeric
                // type `T` reduced under the env, unit `u` a compile-time UNIT (`unit_of`, not a type). This
                // is what lets an effect OP whose result is a quantity — `(op width (-> Unit (Qty Float64
                // (Unit.base #"meter"))))` — read its `(meta t)` = `(fn () (-> Unit (Qty …)))` as a scheme
                // with a determined `(Qty Float64 meter)` RESULT, instead of collapsing the arrow (which made
                // `op_result_type` read the raw op-value record → CDZ0203 "not fully determined"). Mirrors the
                // full evaluator's `QtyCtor` arm (`reduce_ctor`); the scheme path (`type_in_env`) was missing
                // it, so a Qty-result op did not type-check even under an in-program handler. (v-metaprogramming's
                // Quantity `@param` + v-cad/v-notebook — the Quantity-host-op ABI's layer 1.)
                Prim::QtyCtor if args.len() == 2 => {
                    let inner = type_in_env(db, args[0], env)?;
                    let unit = unit_of(db, args[1])?;
                    Some(Ty::Qty {
                        inner: Box::new(inner),
                        unit,
                    })
                }
                // A GENERIC SUM application `(Option a)` inside a type-lambda — each arg reduced under
                // the env (so `a` becomes its type variable), then `Ty::Sum{decl, args}`. This is what
                // makes a generic variant ctor's `(meta t)` = `(fn (a) (-> a (Option a)))` read as the
                // scheme `∀a. a → Option a`. The owning declaration is on the head's `(meta sum-decl)`.
                Prim::SumCtor => {
                    let decl_field = project_meta(db, head, "sum-decl")?;
                    let decl = match resolved_of(db, decl_field) {
                        Resolved::Int(v) => {
                            crate::ast::StructId(v.to_i64().and_then(|n| u32::try_from(n).ok())?)
                        }
                        _ => return None,
                    };
                    let mut arg_tys = Vec::with_capacity(args.len());
                    for &a in args.iter() {
                        arg_tys.push(type_in_env(db, a, env)?);
                    }
                    // Normalize through the shared chokepoint so a generic NEWTYPE's SCHEME result is
                    // `Ty::Nominal { inner }` (its template with these param VARS substituted), not a boxed
                    // `Ty::Sum`. `arg_tys` here are the scheme's fresh param vars (`(Box a)` → `[Var(a)]`),
                    // so the substituted `inner` shares them — when `apply_type` later solves `a := Int64`,
                    // the substitution flows into `inner` too, giving `Ty::Nominal { inner: Int64 }` at the
                    // use site. This is the scheme-position analogue of `reduce_sum_ctor`/`decode_ty`.
                    Some(db.normalize_sum(decl, arg_tys))
                }
                // A `(Tuple T…)` inside a type-lambda — each element reduced UNDER THE ENV, so a type
                // parameter `a` in a tuple element becomes its type variable. This is what lets a generic
                // sum variant carry a TUPLE payload that mentions a param — `(type Box (B (Tuple a Int64))
                // N)`'s `B` ctor `(meta t)` = `(fn (a) (-> (Tuple a Int64) (Box a)))` reads as the scheme
                // `∀a. (Tuple a Int64) → Box a`. The `typeval_of` sibling (`reduce_ctor` TupleCtor) reduces
                // the ground case; this reduces the param-bearing one. Without this arm the reduction fell
                // to `None`, so the ctor arrow was unreadable and the variant looked NULLARY (CDZ0201).
                Prim::TupleCtor => {
                    let mut elems = Vec::with_capacity(args.len());
                    for &a in args.iter() {
                        elems.push(type_in_env(db, a, env)?);
                    }
                    Some(Ty::Tuple(elems.into()))
                }
                // A `(Record (name T)…)` inside a type-lambda — each field's TYPE reduced under the env
                // (the field NAME is a label, not reduced), so a param in a field type becomes its variable.
                // The record companion of the `TupleCtor` arm; mirrors `reduce_ctor`'s `RecordCtor`, which
                // reads each arg as a raw `(name type)` pair and reduces the type. A duplicate field name
                // collapses in the `BTreeMap`, the same fixed-SET rule a record value has.
                Prim::RecordCtor => {
                    let mut fields: std::collections::BTreeMap<Symbol, Ty> =
                        std::collections::BTreeMap::new();
                    for &a in args.iter() {
                        // A field is a `(name type)` pair (the s-expr `(Record (a Int64) …)` spelling) OR a
                        // `(: name type)` annotation triple — the shape the ML record-type surface `{a:
                        // Int64, …}` lowers to. Accept BOTH so a structural record decodes in this scheme
                        // reducer identically to `typeval_of` (the ground twin): without the triple arm the
                        // ML `{…}` field bailed to `None`, so an effect-op with a structural-record RETURN
                        // (`get : -> {a, b}`) had NO `(-> Unit result)` scheme → the nullary-perform site
                        // fell back to the op's meta-record and `St.get()` typed as `(Record (apply …)
                        // (effect-op …) (t …))` instead of `{a, b}`. The `type_in_env` companion of the
                        // `typeval_of` RecordCtor fix.
                        let (name_occ, ty_occ) = match db.ast.get(a) {
                            crate::ast::Struct::List(children) => match children.as_slice() {
                                [n, t] => (*n, *t),
                                [colon, n, t] if db.ast.as_name(*colon) == Some(":") => (*n, *t),
                                _ => return None,
                            },
                            _ => return None,
                        };
                        let name = db.ast.as_name(name_occ)?.to_string();
                        let t = type_in_env(db, ty_occ, env)?;
                        fields.insert(Symbol::plain(name), t);
                    }
                    Some(Ty::Record(std::rc::Rc::new(fields)))
                }
                _ => None,
            }
        }
        // A ref / ground record / built typeval with no free parameters — reduce concretely.
        _ => typeval_of(db, id),
    }
}

/// Reduce a width expression under the environment: a parameter → its width variable, a constant → a
/// fixed width.
fn width_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<Width> {
    match resolved_of(db, id) {
        Resolved::Param { binder } => env.get(&binder).map(|v| Width::Var(v.num)),
        Resolved::Int(v) => v
            .to_i64()
            .and_then(|n| u32::try_from(n).ok())
            .map(Width::Fixed),
        Resolved::Ref { value } => width_in_env(db, value, env),
        _ => None,
    }
}

/// The PAIRED sign variable of a width expression that is a bound parameter, or `None` if the width is
/// concrete (a constant). When a width is a variable, its integer type is generic over signedness too;
/// when concrete, the constructor's own signedness applies. (Follows a `Ref` to reach the parameter,
/// mirroring `width_in_env`.)
fn sign_in_env(db: &mut Db, id: StructId, env: &HashMap<StructId, TyOrWidth>) -> Option<u32> {
    match resolved_of(db, id) {
        Resolved::Param { binder } => env.get(&binder).map(|v| v.sign),
        Resolved::Ref { value } => sign_in_env(db, value, env),
        _ => None,
    }
}

/// β-REDUCE a lambda application: build a copy of `body` in the arena with each of `params` replaced
/// by the corresponding `args`, returning the reduced body's occurrence. This is how applying a
/// function `((fn (x) (+ x 1)) 5)` reduces — substitute the argument for the parameter, then evaluate
/// the substituted body. It APPENDS nodes (like the type-constructor builders); the reduced body is an
/// ordinary subtree resolved/typed/lowered by demand. Substitution is capture-safe here because the
/// arguments are closed (they resolve in the caller's scope, and the copy carries fresh occurrences
/// whose parent is the copied structure — a param inside `args` refers to ITS binder, unaffected).
///
/// `arg_of` maps a parameter occurrence to the argument occurrence substituted for it. A body
/// occurrence that IS a reference to one of these params is replaced by its argument; every other node
/// is structurally copied (its children reduced in turn).
/// PIN (memoize the resolution of) every FREE-VARIABLE name occurrence in `node` — a reference whose
/// binder lies OUTSIDE `lam_body` (the lambda body being β-reduced). A free variable is a CAPTURE of an
/// enclosing `let`/parameter; pinning it (via `resolve_subtree` on the single occurrence) records its
/// resolution against its real binding, so `beta_reduce`'s "captured free variable" arm SHARES the
/// occurrence rather than copying it fresh into the orphan reduced node (where it would re-resolve
/// unbound). A name bound WITHIN `lam_body` (its own param, a body-internal `let`-local) is left
/// unpinned so it copies + re-resolves against the copied scope. Recurses the AST; a nested lambda's
/// captures are relative to the SAME `lam_body` (they bind further out), so the one bound suffices.
///
/// `pub(crate)` so the effects fold reuses the SAME capture-pin (v-effects' mv-class: the multi-shot
/// `cont`-arm k-closure reify `(fn (#kv) C)` β-reduces C more than once, duplicating a detached C whose
/// enclosing-fn param capture would otherwise orphan — pin C's free captures, leave `#kv` unpinned, before
/// the reduce). Single source of truth for capture-pinning across β-reduction and the continuation reify.
pub(crate) fn pin_free_vars(
    db: &mut Db,
    node: StructId,
    lam_body: StructId,
    own_params: &[StructId],
) {
    if db.ast.as_name(node).is_some() {
        // A name occurrence. If it resolves (through refs) to a binder OUTSIDE `lam_body` that is NOT one
        // of the lambda's own params, it is a free capture — pin it. A param formal (substituted by
        // `arg_of`), a body-internal binding (within `lam_body`), or a prelude/global name is left
        // unpinned.
        //
        // A binder that is a MODULE SYNTH RECORD (a nested `(module inner …)` reached by bare name from a
        // sibling def's body — `module_sibling_binds`'s nested-module arm) is a genuine SCOPE capture even
        // though the record is a SYNTHESIZED node (not a `is_user_node`): unlike a prelude global — which
        // re-resolves by name regardless of position, so a fresh copy is fine — a module member is in
        // scope ONLY via the enclosing module's record, which the β-copy's orphan loses. So it MUST be
        // pinned (shared), exactly as a sibling `def`'s lambda binder is. Pin when the binder is a user
        // node OR such a module record.
        //
        // ALSO a SYNTH CAPTURED VALUE binder — the RHS `V` of a `(def x V)` do-local value decl OR a `(x V)`
        // `let` binding, when that `do`/`let` was itself β-COPIED by an ENCLOSING inline (so `V` is a synth
        // node, `is_user_node` false). A do-local/let-local fn `inner` that CAPTURES a runtime-computed
        // sibling `(def m (* n 3))` / `(let ((m (* n 3))) …)` and is then inlined (`(inner n)`) reduces
        // `inner`'s body; `m`'s binder is that binding's RHS. On the ORIGINAL (user) arena the `is_user_node`
        // arm pins it — but when the whole `outer` def is inlined FIRST (a nested/outer application copies the
        // `do`/`let`, making `m`'s RHS synth), the later inline of the copied `inner` sees a SYNTH binder and
        // the `is_user_node` gate skipped it → the copied `m` re-resolved against the orphan and reported a
        // spurious CDZ0101 "unbound name `m`" (`cdz check` CLEAN — gated on `is_user_node` — while `cdz
        // compile` declined: the check≡compile discrepancy). Like a module member, a do-local/let value is in
        // scope ONLY via its lexical position, which the orphan loses, so a synth one MUST pin too. (A
        // CONST-local `(def m 3)` was unaffected — its RHS folds to a constant that re-resolves anywhere; a
        // PARAM capture was unaffected — a `Param` binder is a user sig node. Only a runtime-computed synth
        // value binder slipped through — corpus-bugfix FINDING #19.)
        if let Some(binder) = ref_binder(db, node)
            && (db.is_user_node(binder)
                || db.module_by_synth_record(binder).is_some()
                || is_synth_captured_value_binder(db, binder))
            && !db.is_within(binder, lam_body)
            && !own_params.contains(&binder)
        {
            crate::resolve::resolve_subtree(db, node);
        }
        return;
    }
    if let crate::ast::Struct::List(children) = db.ast.get(node) {
        for c in children.clone() {
            pin_free_vars(db, c, lam_body, own_params);
        }
    }
}

/// Whether `binder` is the VALUE-RHS `V` of a lexical VALUE binding whose binder a β-copy's orphan would
/// lose — either a `(def x V)` do-local value declaration (`do_def_binds` → `Ref { value: V }`) or a
/// `(x V)` `let` binding pair (`last_binder_named` → `Ref { value: V }`). Used by `pin_free_vars` to pin a
/// captured such binder that is a SYNTH node (a copied `do`/`let`, `is_user_node` false) — the
/// `is_user_node` arm already covers the user-arena case; this is the copied-arena twin (corpus-bugfix
/// FINDING #19). A do-local / let-local value is in scope ONLY via its lexical position, which the orphan
/// loses, so a captured one must be pinned + shared like a module member. Recognized by the binder being
/// the SECOND element of its parent form: a bare-name `(def x V)` def, or a `(name V)` two-element binding
/// pair whose grandparent is a `let`. (A `def (f p…) BODY` function resolves to a `Lambda`, not a value
/// `Ref`, so it never reaches here; only a bare-name value binding does.)
fn is_synth_captured_value_binder(db: &Db, binder: StructId) -> bool {
    let Some(parent) = db.parent_of(binder) else {
        return false;
    };
    // `(def x V)` — a do-local VALUE def: sig is a bare name, `binder` is the value (second tail element).
    if let Some(tail) = db.ast.as_form(parent, "def") {
        return tail.len() == 2
            && tail.get(1) == Some(&binder)
            && tail
                .first()
                .is_some_and(|&sig| db.ast.as_name(sig).is_some());
    }
    // `(name V)` — a `let` binding pair: `binder` is the value (second of a two-element pair), the pair's
    // first child is a bare name, and the pair's parent is a `let`'s bindings-LIST (its grandparent is a
    // `let` form whose FIRST tail element is that list — distinct from the `let` body).
    if let crate::ast::Struct::List(kv) = db.ast.get(parent)
        && kv.len() == 2
        && kv[1] == binder
        && db.ast.as_name(kv[0]).is_some()
        && let Some(bindings_list) = db.parent_of(parent)
        && let Some(let_form) = db.parent_of(bindings_list)
        && db
            .ast
            .as_form(let_form, "let")
            .is_some_and(|t| t.first() == Some(&bindings_list))
    {
        return true;
    }
    false
}

/// The binder occurrence a name reference resolves to (following `Ref`s / a `Param`, or a `Lambda`'s
/// body), or `None` if it is not a resolvable reference to a binding (a prelude/global name, a literal, a
/// form). Used by `pin_free_vars` to tell a captured free variable (binding OUTSIDE the lambda) from a
/// body-local one via `is_within(binder, lam_body)`.
///
/// A `Lambda` resolution has no single binder occurrence — its representative is the lambda's BODY, which
/// gives exactly the right `is_within` discrimination: a do-local / module-SIBLING function def (its body
/// is a sibling, OUTSIDE the enclosing lambda's body) is a capture and MUST be pinned — otherwise the
/// β-copy re-parents the reference into an orphan and re-resolving it against the copy's scope finds
/// nothing (a spurious CDZ0101, the do-local/module `(def (f x) (+ (dbl x) 1))` bug). A body-INTERNAL
/// nested def (its body lies WITHIN the lambda body) is not a capture — it re-resolves against the copy.
/// A top-level def also resolves to a `Lambda`; pinning its reference is harmless (it re-resolves to the
/// same def by name regardless of position), exactly as pinning a top-level VALUE def's `Ref` already is.
fn ref_binder(db: &mut Db, id: StructId) -> Option<StructId> {
    match resolved_of(db, id) {
        Resolved::Param { binder } => Some(binder),
        Resolved::Ref { value } => Some(value),
        Resolved::Lambda { body, .. } => Some(body),
        // A MATCH-ARM / tuple-pattern binder resolves to a `SumPayload` reading the enclosing arm's
        // SCRUTINEE (Case 5/6 in `resolve`). Its scope is where the scrutinee is bound; use the scrutinee
        // as the representative for the `is_within` capture test. When an inner lambda in the arm CAPTURES
        // such a binder (`(match (A m) ((A m) ((fn (x) (+ x m)) 3)))`) the scrutinee is bound OUTSIDE the
        // lambda body, so `pin_free_vars` pins the occurrence — and `beta_reduce`'s SumPayload capture-share
        // exception then SHARES it (preserving its `SumPayload` resolution) rather than copying it fresh
        // into the reduced orphan, where it re-resolved unbound (a spurious CDZ0101). Without this arm the
        // binder was invisible to `pin_free_vars`, so the capture was dropped — the bug this fixes. (A
        // binder whose scrutinee is WITHIN the lambda body is body-local, not a capture, and stays unpinned
        // as before.)
        Resolved::SumPayload { scrutinee, .. } => Some(scrutinee),
        _ => None,
    }
}

/// Structurally COPY `body` (fresh occurrences that re-resolve names against the copied scope), with NO
/// substitution — a β-reduction against an empty argument map. Used by recursive-generic monomorphization
/// (`crate::lower::type_specialize`) to clone a generic def's body into a type-specialized copy: a body
/// reference to a param re-resolves to the copy's re-annotated binder, and a self-call re-resolves by name
/// to the original def (re-entering specialization at lower). `own_params` are the def's parameter name
/// occurrences — a body reference to one must copy + re-resolve against the specialized copy's new sig, so
/// it is NOT pinned; every OTHER free reference (a self-call / sibling / do-local function that resolves to
/// a `Lambda` OUTSIDE `body`) IS pinned first, so the copy SHARES the pinned occurrence instead of
/// re-resolving it in the orphan copy. Without the pin a DO-LOCAL generic's self-call — which resolves by
/// LEXICAL do-scope, not the global name index — became unbound in the re-parented copy (a spurious
/// CDZ0101); a top-level/module callee re-resolved by name regardless, so it only bit the do-local path.
/// Mirrors `apply_lambda`'s `pin_free_vars` before `beta_reduce`.
pub fn copy_structural_pub(
    db: &mut Db,
    body: StructId,
    own_params: &[StructId],
    arg_of: &HashMap<StructId, StructId>,
) -> StructId {
    pin_free_vars(db, body, body, own_params);
    // Additionally PIN every SELF-CALL occurrence — a name resolving to the def whose body IS `body`
    // (`is_within(body, body)` is true, so `pin_free_vars` treats it as body-internal and skips it). A
    // self-call must be SHARED so the copy resolves it to the ORIGINAL def and re-enters `type_specialize`
    // (→ memo) at lower. A TOP-LEVEL/module self-call would re-resolve by the global name index anyway, but
    // a DO-LOCAL one resolves by LEXICAL do-scope, which the orphan copy escapes → unbound. Sharing the
    // occurrence closes both uniformly.
    pin_self_calls(db, body, body);
    // `arg_of` SUBSTITUTES a param's references with a replacement node — used to bind a TYPE-VALUED
    // parameter to its CONCRETE type expression at monomorphization (`(Lst t)` → `(Lst Int64)`), so the
    // erased type param needs no binder in the specialized copy (empty for the ordinary value-param case).
    // Record the COPY ROOT so the capture-share exception copies (not shares) a pinned pattern binder
    // whose scrutinee lies WITHIN this body (see `Db::reduction_root`).
    let saved = db.reduction_root.replace(body);
    let reduced = beta_reduce(db, body, arg_of);
    db.reduction_root = saved;
    reduced
}

/// Pin (memoize the resolution of) every SELF-CALL name occurrence in `node` — a reference resolving to a
/// `Lambda` whose body IS `self_body` (the def being copied). `pin_free_vars` excludes it (its representative
/// binder equals `self_body`, so `is_within` reads body-internal); this shares it so the copy's self-call
/// keeps resolving to the original def rather than re-resolving in the orphan copy. See `copy_structural_pub`.
fn pin_self_calls(db: &mut Db, node: StructId, self_body: StructId) {
    if db.ast.as_name(node).is_some() {
        if matches!(resolved_of(db, node), Resolved::Lambda { body, .. } if body == self_body) {
            crate::resolve::resolve_subtree(db, node);
        }
        return;
    }
    if let crate::ast::Struct::List(children) = db.ast.get(node) {
        for c in children.clone() {
            pin_self_calls(db, c, self_body);
        }
    }
}

// Applying a function evaluates its body with each parameter bound to its argument — realized here by
// β-reduction: `arg_of` maps each parameter occurrence to its argument, and the reduced body sees those
// bindings (a free variable of the lambda is pinned to its captured binder, so the body observes the
// captured environment extended with the parameter→argument binding).
//= spec/capabilities/core-semantics.md#applying-a-function-binds-its-parameter-to-its-argument
//# Applying a function to its argument MUST evaluate the function body in an environment that extends the function's captured environment with its parameter bound to the argument.
pub fn beta_reduce(db: &mut Db, body: StructId, arg_of: &HashMap<StructId, StructId>) -> StructId {
    // A name occurrence in a BINDER POSITION (the name slot of a `let` binding pair, or a match-arm
    // pattern binder) NAMES a binding — it is not a reference to be substituted. Its `resolved_of`
    // walks the scope and, when it shadows a parameter of the same name, reaches that param's binder
    // (which IS in `arg_of`), so the substitution branches below would replace the binding NAME with
    // the argument — turning `(let ((x true)) x)` under param `x` into `(let ((99 true)) x)`, whose
    // body `x` then finds no `x` binding and reports a spurious unbound name (a differently-typed
    // shadow additionally MISCOMPILED to an invalid component). A binder is copied structurally like
    // any other node, never substituted. (Only VALUE references — a bare use of the name — are.)
    // A binder-position name (a `let`/match/param binder, or a RECORD FIELD KEY), or a MEMBER-KEY name
    // (the `key` of a `(. operand key)` projection), NAMES a label — it is not a value reference, so it
    // is copied structurally, never substituted. Both a field key and a projection key select a field by
    // its LABEL; substituting the argument for such a name (`(. (record (x 5)) x)` under a param `x`)
    // would corrupt the label into a non-name and reject a valid program (CDZ0201).
    if is_binder_occurrence(db, body) || is_member_key_occurrence(db, body) {
        // A PINNED BARE binder-occurrence — a nested closure's own bare param that a BROAD `resolve_subtree`
        // pinned (e.g. the effects discharge pinning a whole continuation containing `(fn (x) …)`) — must be
        // SHARED, not freshened. Its body-REFERENCES are pinned too (same `resolve_subtree`), so they take
        // the capture-share path below and stay pointing at THIS occurrence; freshening the head alone would
        // fragment the head from its body-refs — `apply_lambda` then keys the substitution on the fresh head
        // while the body still resolves to the shared original, so `count_refs_to == 0`, the argument is
        // never spliced, and the parameter reaches emit as a slot-less `Core::Param` ("parameter reference
        // has no local slot"). Sharing keeps the head and its body-refs on the SAME occurrence — exactly how
        // an ANNOTATED `(: x T)` binder already behaves: its inner name is NOT a binder-occurrence, so it and
        // the body-refs both take the capture-share path and stay consistent (which is why the annotated twin
        // folds where the bare one declines). Scoped to PINNED binders only: an ordinary partial-application
        // / monomorphization copy does not pin a lambda's own param head (`pin_free_vars` excludes own
        // params), so that path still freshens as before — this changes ONLY the broad-pin anomaly.
        if is_binder_occurrence(db, body)
            && db.ast.as_name(body).is_some()
            && db.resolved_subtrees.contains(&body)
        {
            return body;
        }
        let leaf = match db.ast.get(body) {
            crate::ast::Struct::Atom(lid) => db.ast.leaf(*lid).clone(),
            // A non-atom binder (an annotated `(: name T)` binder) copies structurally below.
            _ => return copy_structural(db, body, arg_of),
        };
        return db.push_atom(leaf);
    }
    // A `do` SEQUENCE `(do stmt… last)` resolves to `Ref{last}` (`resolve_do` collapses it — every
    // intermediate is a value expression whose value is discarded). But β-reduction must PRESERVE the do's
    // STRUCTURE: an intermediate may be an EFFECTFUL perform whose value is discarded yet whose EFFECT must
    // run (`(do (Db.put …) v)` — a helper's state-advancing statement before its return). Following the
    // `Ref{last}` collapse via the substitution branch below would substitute the LAST form and DROP the
    // intermediates — silently discarding the `Db.put` when the helper inlines (the helper-call out-state
    // miscompile: the later `Db.get` then misses the write). Copy the do structurally (each item β-reduced,
    // so its param refs still substitute) so the intermediates survive: a PURE intermediate is harmlessly
    // re-collapsed by `resolve_do` at type/lower time, and an effectful one is threaded by the handler
    // fold's `do` arm. This makes the substituting path CONSISTENT with `copy_pure` (empty-subst β-reduce),
    // which already preserves a do structurally — the collapse only ever fired when a substitution chased
    // through the `Ref{last}` to a substituted param, an asymmetry that dropped effects on exactly the
    // inline path. (Checked BEFORE the `Ref`-substitution branch, which is what performed the collapse.)
    if db.ast.as_form(body, "do").is_some() {
        return copy_structural(db, body, arg_of);
    }
    // A reference to a substituted parameter → its argument. A parameter reference resolves to
    // `Ref { value: <param binder occ> }`; if that binder is one we're substituting, use the arg. The
    // ref is followed TRANSITIVELY: a MATCH-ARM BINDER `k` resolves (Case 5) to the scrutinee
    // occurrence, which itself resolves to the param binder — so `k` reaches the substituted argument
    // through the chain. (A chain that does not end at a substituted binder is not substituted.)
    if let Resolved::Ref { value } = resolved_of(db, body) {
        let mut target = value;
        loop {
            if let Some(&arg) = arg_of.get(&target) {
                return arg;
            }
            match resolved_of(db, target) {
                Resolved::Ref { value: next } => target = next,
                _ => break,
            }
        }
    }
    // The parameter OCCURRENCE itself (in the binder position, not a ref) is not substituted — only
    // references are; but a bare param used as a value resolves to `Param{binder}` — substitute it.
    if let Resolved::Param { binder } = resolved_of(db, body)
        && let Some(&arg) = arg_of.get(&binder)
    {
        return arg;
    }
    // A CAPTURED FREE VARIABLE — a name occurrence already RESOLVE-PINNED (`resolve_subtree` fixed its
    // meaning). This is a spliced captured ARGUMENT that survived into a residual (partially-applied)
    // lambda: `((sub n) 3)` curries to `(fn (b) (- n b))`, where `n` is the caller's pinned occurrence;
    // the residual's later full application β-reduces `(- n b)` and reaches `n` here. `n`'s binding is
    // OUTSIDE this body (the caller's param/`let`), so it must be PRESERVED — SHARE the pinned occurrence
    // (keeping its memoized resolution) rather than make a fresh copy that would re-resolve against the
    // residual's scope, where the captured name is unbound (the CDZ0101 the partial-application copy hit).
    // A body-internal `let`-local is NEVER pinned (it resolves lazily, after the copy), so it still falls
    // to `copy_structural` below and correctly re-resolves against the copied scope.
    //
    // EXCEPTION — a MATCH-ARM (or tuple-pattern) BINDER whose SCRUTINEE IS BEING SUBSTITUTED: such a
    // binder resolves to a `SumPayload` READING THE SCRUTINEE (Case 5/6 in `resolve`). If `resolve_subtree`
    // pinned it (it pins the whole lambda body before an application) AND the scrutinee it reads is a
    // parameter THIS reduction is substituting, sharing the binder as-is keeps a `SumPayload` pointing at
    // the ORIGINAL scrutinee node — which β-reduction is replacing in this same copy (a lambda whose param
    // IS the match scrutinee, applied through a nested inline). The shared binder would then read a
    // now-orphaned scrutinee (lowered standalone as a slot-less `Core::Param` — the "no local slot"
    // decline). So such a binder must COPY and re-resolve against the copied match's substituted scrutinee.
    // NARROW to exactly that case: only when the `SumPayload`'s scrutinee resolves to a param in `arg_of`.
    // A binder over an EXTERNAL scrutinee (a genuine capture — an enclosing match's binder captured by an
    // inner lambda) is NOT being substituted, so it keeps sharing (copying it would break its resolution
    // and, on a deep nested-call chain, re-resolve exponentially — the stack-overflow regression).
    if db.ast.as_name(body).is_some() && db.resolved_subtrees.contains(&body) {
        if let Resolved::SumPayload { scrutinee, .. } = resolved_of(db, body)
            && (scrutinee_mentions_substituted(db, scrutinee, arg_of)
                || db
                    .reduction_root
                    .is_some_and(|root| db.is_within(scrutinee, root)))
        {
            // Fall through to `copy_structural` — re-resolve against the copied scrutinee. Three cases need
            // this: (a) the scrutinee IS a substituted param (a lambda whose param is the match scrutinee,
            // applied through a nested inline); (a') the scrutinee is a COMPOUND expression that MENTIONS a
            // substituted param — `(match (Bytes.slice b 1 2) ((Some w) (resume w t)) …)` in an effect
            // handler arm, where the op-param `b` is being substituted by the pure-copied op arg but the
            // scrutinee `(Bytes.slice b 1 2)` is an Apply, not a bare `Param`, so `scrutinee_is_substituted`
            // alone (top-value only) misses it (nv1e/nvC: a resume value bound by a match over a computation
            // on the op-param); (b) the scrutinee is an EXPRESSION WITHIN the body being copied — `(match
            // (f c) ((Ok ct) …))` in a monomorphized def, where the scrutinee `(f c)` is itself copied (a
            // fresh occurrence re-resolving its own `c` against the specialized sig). In all three, SHARING
            // the pinned binder would leave its `SumPayload` reading the ORIGINAL, now-orphaned scrutinee —
            // whose param references (`c`/`b`) have no slot in the copied function, the "parameter reference
            // has no local slot" backend decline. A scrutinee OUTSIDE the copy root that mentions no
            // substituted param is a genuine capture (an enclosing match's binder read by an inner lambda)
            // and keeps sharing (copying it would break its resolution and, on a deep call chain, re-resolve
            // exponentially).
        } else {
            return body;
        }
    }
    // Otherwise structurally copy. A CONSTANT atom (int/bool/float/string leaf) is self-contained — it
    // resolves to its own value regardless of scope, so SHARE it (cheap, no re-resolution needed). A
    // NAME atom, though, resolves by a SCOPE WALK: if it names a binding INSIDE this body (a `let`-local
    // like `x`), sharing the original node would keep its stale resolution to the ORIGINAL (pre-copy)
    // binding — but the copy re-parents everything, so a `let`-local's copied init differs from the
    // original. So COPY a name atom (a fresh occurrence of the same name leaf); `push_list` re-parents
    // it under the copied enclosing form, and its `resolved_of` re-runs against the COPIED scope,
    // binding it to the copied binding (`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the body's `x` must
    // resolve to the COPY's substituted init `(+ arg 1)`, not the original `(+ n 1)`). A param reference
    // never reaches here (the substitution branches above return the arg first); a prelude/free name
    // re-resolves to the same global, harmlessly.
    copy_structural(db, body, arg_of)
}

/// Whether the match scrutinee at `scrutinee` reads a PARAMETER that THIS β-reduction is substituting —
/// i.e. its (transitive `Ref`-chased) binder is a key in `arg_of`. When true, a pattern binder reading
/// this scrutinee must be COPIED (so it re-resolves against the substituted scrutinee in the copy) rather
/// than shared as a capture; when false (the scrutinee is external — a genuine capture), the binder keeps
/// sharing. This is the exact test that distinguishes "the scrutinee is being replaced here" from "the
/// scrutinee is an outer value the closure captured" (see the capture-share exception in `beta_reduce`).
fn scrutinee_is_substituted(
    db: &mut Db,
    scrutinee: StructId,
    arg_of: &HashMap<StructId, StructId>,
) -> bool {
    // The scrutinee is a value reference — a bare param (`Param { binder }`) or a `Ref` chain ending at
    // one. Chase it the same way `beta_reduce`'s substitution branches do; if any link is a substituted
    // key, the scrutinee is being replaced.
    if let Resolved::Param { binder } = resolved_of(db, scrutinee) {
        return arg_of.contains_key(&binder);
    }
    let mut target = scrutinee;
    loop {
        if arg_of.contains_key(&target) {
            return true;
        }
        match resolved_of(db, target) {
            Resolved::Ref { value } => target = value,
            Resolved::Param { binder } => return arg_of.contains_key(&binder),
            _ => return false,
        }
    }
}

/// Whether the scrutinee subtree at `scrutinee` MENTIONS a parameter THIS β-reduction is substituting —
/// anywhere within it, not just as its top value (which is [`scrutinee_is_substituted`]'s job). A COMPOUND
/// scrutinee like `(Bytes.slice b 1 2)` is an application, not a bare `Param`/`Ref`, so
/// `scrutinee_is_substituted` returns false; yet it READS the substituted op-param `b`, so a pattern binder
/// whose `SumPayload` points at it must still be COPIED (to re-resolve against the substituted-scrutinee
/// copy) rather than shared (which would leave the binder reading the original scrutinee's raw `b`, whose
/// substituted occurrence has no slot in the folded body — "parameter reference has no local slot"). Walk
/// the subtree, chasing each `Ref`/`Param` to its binder, and return true if any resolves into `arg_of`.
fn scrutinee_mentions_substituted(
    db: &mut Db,
    scrutinee: StructId,
    arg_of: &HashMap<StructId, StructId>,
) -> bool {
    if scrutinee_is_substituted(db, scrutinee, arg_of) {
        return true;
    }
    if let crate::ast::Struct::List(children) = db.ast.get(scrutinee).clone() {
        return children
            .iter()
            .any(|&c| scrutinee_mentions_substituted(db, c, arg_of));
    }
    false
}

/// Structurally copy `body` (no substitution at THIS node — the caller decided it is not a
/// substituted reference). A CONSTANT atom (int/bool/float/string leaf) is self-contained — it
/// resolves to its own value regardless of scope, so SHARE it (cheap, no re-resolution needed). A NAME
/// atom resolves by a SCOPE WALK: if it names a binding INSIDE this body (a `let`-local like `x`),
/// sharing the original node would keep its stale resolution to the ORIGINAL (pre-copy) binding — but
/// the copy re-parents everything, so a `let`-local's copied init differs from the original. So COPY a
/// name atom (a fresh occurrence of the same name leaf); `push_list` re-parents it under the copied
/// enclosing form, and its `resolved_of` re-runs against the COPIED scope, binding it to the copied
/// binding (`(def (g n) (let ((x (+ n 1))) (+ x x)))` — the body's `x` must resolve to the COPY's
/// substituted init `(+ arg 1)`, not the original `(+ n 1)`). A prelude/free name re-resolves to the
/// same global, harmlessly.
fn copy_structural(db: &mut Db, body: StructId, arg_of: &HashMap<StructId, StructId>) -> StructId {
    match db.ast.get(body).clone() {
        crate::ast::Struct::Atom(lid) => match db.ast.leaf(lid).clone() {
            crate::ast::Leaf::Name(_) => {
                let leaf = db.ast.leaf(lid).clone();
                let copy = db.push_atom(leaf);
                // Record the SOURCE occurrence this copy came from so a diagnostic anchored at the copy
                // (a name that re-resolves UNBOUND in the spliced body — the whole-program CDZ0101 that
                // otherwise loses its location) can be relocated to the real source reference via
                // `source_of_synth`. `body` may itself be a prior copy; the chain resolves to the original
                // user node. Purely a location aid — never changes whether a fault is reported.
                db.synth_name_origin.insert(copy, body);
                copy
            }
            _ => body, // a constant leaf — self-contained, share it.
        },
        crate::ast::Struct::List(children) => {
            let reduced: Vec<StructId> = children
                .iter()
                .map(|&c| beta_reduce(db, c, arg_of))
                .collect();
            db.push_list(reduced)
        }
    }
}

/// Whether `id` is a name occurrence in a BINDER POSITION — the name slot of a `let` binding pair, a
/// match-arm pattern binder, or a `fn`/`def` PARAMETER (bare or the name of a `(: name T)` binder). Such
/// an occurrence NAMES a binding; it is not a value reference, so β-reduction must copy it structurally
/// rather than substitute an argument for it (else a binder that shadows a same-named parameter would be
/// replaced by the argument, destroying the binding). Detected structurally by the parent chain, so no
/// name-specific knowledge is needed:
///
///  - `id` is the FIRST child of a `(name init)` pair whose grandparent is a `let`'s bindings-list.
///  - `id` is the FIRST child (the pattern) of a `(pattern body)` arm whose grandparent is a `match`.
///  - `id` is a `fn`/`def` PARAMETER — a bare name directly in the param list, or the name slot of a
///    `(: name T)` param binder. (Without this, β-COPYING a lambda whose param is annotated `(: p T)`
///    treated the param name as a value reference and copied it detached from its `(: …)` annotation, so
///    the copy's param lost its declared type — an eta-expanded runtime-currying residual, whose awaited
///    param IS annotated `(: $eta (typeval T))`, then declined "parameter type has no machine rep".)
fn is_binder_occurrence(db: &Db, id: StructId) -> bool {
    let Some(pair) = db.parent_of(id) else {
        return false;
    };
    // A `fn`/`def` PARAM whose parent is the param LIST directly (a bare-name param): the param list is a
    // `fn`'s first tail element, or a `def`'s signature list `(name p…)` (params after the def name).
    if is_param_list(db, pair, id) {
        return true;
    }
    // A do-local (or top-level) VALUE `def` NAME — `(def NAME rhs)`, where `id` is the bare-name atom at
    // the def's FIRST tail position. It NAMES a binding (scoped to the following forms of its enclosing
    // `do`), so like a `let`/match binder it is copied STRUCTURALLY, never substituted. Without this,
    // inlining a lambda whose body contains a do-def shadowing a same-named PARAM
    // (`(def (f (: v Int64)) (do (def v (* v 2)) v))`) β-substitutes the argument for the do-def's name
    // slot — the COPIED do-block then no longer DECLARES `v` (`do_local_binds` finds no declaring form),
    // so the trailing reference reports a spurious CDZ0101 unbound (the def-over-param shadow bug; a
    // do-def shadowing a param unbinds the name for all later refs). A `let`-over-param and def-over-def
    // shadow already work; this makes def-over-param consistent. (The fn-def NAME in a signature list
    // `(NAME p…)` is a List head, not this atom slot, and its params are covered by `is_param_list`.)
    if let Some(def_tail) = db.ast.as_form(pair, "def")
        && def_tail.first() == Some(&id)
        && matches!(db.ast.get(id), crate::ast::Struct::Atom(_))
    {
        return true;
    }
    // `id` must be the pattern/name slot — the pair's/arm's/binder's first child. EXCEPTION: a
    // canonical record field is the `(= key value)` ascription triple (Phase B), where the KEY is the
    // SECOND child (index 1), not the first. Such a key is a LABEL under a record form and must be
    // β-immune exactly like the legacy `(key value)` pair's first child — without this a
    // `(def (f (: x Int64)) (record (= x 5)))` substitutes the arg for the key `x`, corrupting
    // `(record (= x 5))` into `(record (= 7 5))` → CDZ0201. Handle it directly, before the first-child gate.
    // The ascription is recognized in EITHER FieldPair spelling: the native `FieldPair` leaf head (M2b, what
    // records now carry after resolution) via `field_pair_parts`, and the transitional name-head `(= …)` via
    // `field_pair`. Restricted to a RECORD grandparent (a MAP entry's key is a VALUE expression that may
    // legitimately reference a param, so it stays substitutable). Missing the native leaf arm reds the §9
    // flagship reducer suite: a record field key colliding with a param (`{ id = id }`) β-substitutes the arg
    // for the key → CDZ0201.
    let crate::ast::Struct::List(pair_children) = db.ast.get(pair) else {
        return false;
    };
    if let Some((key, _)) = db
        .ast
        .field_pair_parts(pair)
        .or_else(|| db.ast.field_pair(pair))
        && key == id
        && let Some(grandparent) = db.parent_of(pair)
        && db
            .ast
            .compound_form_of(grandparent, CompoundCtor::Record)
            .is_some()
    {
        return true;
    }
    if pair_children.first() != Some(&id) {
        return false;
    }
    let Some(grandparent) = db.parent_of(pair) else {
        return false;
    };
    // A `let` binding pair: the grandparent is the bindings-LIST of a `let` — its own parent is a `let`
    // form whose FIRST tail element is that list (the bindings position, distinct from the body).
    if let Some(great) = db.parent_of(grandparent)
        && let Some(let_tail) = db.ast.as_form(great, "let")
        && let_tail.first() == Some(&grandparent)
    {
        return true;
    }
    // A `match` arm: the grandparent is the `match` form and `pair` is one of its ARMS (a tail element,
    // i.e. not the scrutinee at tail position 0).
    if let Some(tail) = db.ast.as_form(grandparent, "match")
        && tail.iter().skip(1).any(|&arm| arm == pair)
    {
        return true;
    }
    // An ANNOTATED `fn`/`def` param `(: name T)`: `id` is the name (first child), `pair` is the `(: …)`
    // binder, and the binder sits in a param list. (The bare-name case is caught above; this is the
    // annotated companion — the name inside `(: name T)` is still a binder, not a reference.)
    if db.ast.as_form(pair, ":").is_some() && is_param_list(db, grandparent, pair) {
        return true;
    }
    // A RECORD FIELD KEY — `id` is the KEY (first child) of a `(key value)` field pair whose grandparent
    // is a record FORM. The key is a LABEL (a field name, `read_key`), NOT a value reference, so it must
    // be immune to argument substitution: without this, calling `(def (f (: x Int64)) (record (x 5)))`
    // β-substitutes the argument for the key `x`, corrupting `(record (x 5))` into `(record (7 5))`,
    // which `read_key` then rejects (CDZ0201 "record field key must be a name"). The field VALUE (the
    // pair's SECOND child) is NOT a binder — it is an ordinary expression that legitimately references
    // the parameter (`(record (y x))`), so only the FIRST child qualifies (checked above: `id` is the
    // pair's first child). The grandparent is a record form in EITHER spelling: the `record` NAME alias
    // `(record …)` or the unshadowable STRING primitive `("record" …)`. (A field pair has exactly two
    // children; a `(meta name)` key is already immune as it is not a bare param name.)
    if pair_children.len() == 2
        && db
            .ast
            .compound_form_of(grandparent, CompoundCtor::Record)
            .is_some()
    {
        return true;
    }
    false
}

/// Whether `id` is a NAME occurrence in a MEMBER-KEY position — the field/member name of a `(. operand
/// key)` projection, where `key` is `id`. Such a name is a LABEL selecting a field (`resolve_member`
/// reads it via `as_name`, like `read_key` for a record field), NOT a value reference, so β-reduction
/// must not substitute an argument for it. Without this, calling `(def (f (: x Int64)) (. r x))`
/// β-substitutes the argument for the projection key `x`, turning `(. r x)` into `(. r 7)` — a member
/// access by a non-name key, which resolves to a malformed projection (CDZ0201 / a positional index on
/// a non-tuple). Mirrors the record-field-key immunity in [`is_binder_occurrence`]; the projection
/// OPERAND (tail position 0) is an ordinary expression and stays substitutable — only the KEY (position
/// 1) is a label. A `(meta name)` key is already immune (not a bare param name).
fn is_member_key_occurrence(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // The parent must be a `(. operand key)` form and `id` its KEY (the second tail element).
    match db.ast.as_form(parent, ".") {
        Some(tail) => tail.len() == 2 && tail[1] == id,
        None => false,
    }
}

/// Whether `list` is a `fn`/`def` PARAMETER LIST that contains `child` as one of its parameters. A `fn`'s
/// param list is its FIRST tail element `(fn (p…) body)`; a `def`'s is its signature `(def (name p…)
/// body)` where the params follow the def name (so `child` is any element except the name at position 0).
fn is_param_list(db: &Db, list: StructId, child: StructId) -> bool {
    let Some(form) = db.parent_of(list) else {
        return false;
    };
    if let Some(tail) = db.ast.as_form(form, "fn")
        && tail.first() == Some(&list)
    {
        // A `fn` param list — `child` is one of its params (any position).
        if let crate::ast::Struct::List(ps) = db.ast.get(list) {
            return ps.contains(&child);
        }
    }
    if let Some(tail) = db.ast.as_form(form, "def")
        && tail.first() == Some(&list)
    {
        // A `def` signature `(name p…)` — `child` is a param iff it is present and NOT the name (pos 0).
        if let crate::ast::Struct::List(sig) = db.ast.get(list) {
            return sig.first() != Some(&child) && sig.contains(&child);
        }
    }
    false
}

/// Whether the argument subtree at `node` reaches a PERFORM — an effect operation (a discharged handler
/// op, an outer handler's op, or a HOST-delegated op), or a bare `resume` — following callee bodies once
/// each (a `visited` set terminates recursion; bounded depth as a native-stack backstop). The
/// evaluate-ONCE trigger's "does this argument carry an observable effect that must not be duplicated by a
/// multi-use param substitution" test. CTX-FREE (an `effect_op_of` head is a perform regardless of which
/// handler owns it), so it lives here in eval rather than needing a `HandlerCtx` — the sibling
/// `effects::arg_reaches_any_perform` gates the FOLD's arm-side dup; this gates the GENERAL β-reduce inline
/// (a pure multi-use helper whose param is bound to an effectful arg, which the fold never sees because the
/// helper does not itself perform).
///
/// A RECURSIVE callee is followed via the `visited` set (walked once), NOT blanket-reported as performing:
/// an over-report is NOT harmless here. It let-wraps a PURE arg instead of substituting it, and in a
/// multi-level inline the leftover param-ref residuals get pinned as false captures and emitted with no
/// local slot ("parameter reference has no local slot" — the cross-module HM-unify miscompile a recursive
/// but effect-free resolver like `apply-sign-go` tripped). Only a genuinely-effectful callee (an
/// `effect_op_of`/`resume` reached on the one walk) or a past-cap depth returns `true`.
fn arg_reaches_perform_evalonce(db: &mut Db, node: StructId) -> bool {
    // `visited` records callee BODIES already entered on the current walk, so a RECURSIVE callee is
    // followed EXACTLY ONCE (its body walked; a self-call re-entry is skipped by the visited check)
    // rather than blanket over-reporting `true`. This is the fix for the "pure recursive helper wrongly
    // flagged as performing" miscompile: a fuel-recursed but effect-FREE helper like `apply-sign-go`
    // (chain-following resolution, no perform anywhere) reaches NO perform, so its multi-use caller param
    // must be SUBSTITUTED, not left-out + let-wrapped. The old `is_recursive → true` over-report let-wrapped
    // a pure arg, leaving the param's refs as residuals that a NESTED inline then pinned as false captures
    // and shared slot-less → "parameter reference has no local slot" at emit (cross-module HM-unify miscompile,
    // v-wasm-opt/v-compiler-ml 2026-08-08). A genuinely-effectful recursive callee still returns `true` (its
    // body's `effect_op_of`/`resume` is seen on the one walk). Depth-capped independently as a native-stack
    // backstop; the visited set is what makes recursion terminate rather than the cap.
    fn walk(
        db: &mut Db,
        node: StructId,
        depth: u32,
        visited: &mut crate::fxhash::FxHashSet<StructId>,
    ) -> bool {
        if depth > 64 {
            return true; // too deep — assume it may perform (safe over-report)
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            if effect_op_of(db, head).is_some() {
                return true;
            }
            // Follow the callee body ONCE (recursive or not). A non-function head (a constructor, a record
            // field-pair label) hides no perform in the head — only its args, covered by the descent below.
            if let Some(callee) = lambda_body(db, head).or_else(|| lambda_body_of_nullary(db, head))
                && visited.insert(callee)
                && walk(db, callee, depth + 1, visited)
            {
                return true;
            }
            return args.iter().any(|&a| walk(db, a, depth + 1, visited));
        }
        if matches!(resolved_of(db, node), Resolved::Resume { .. }) {
            return true;
        }
        match db.ast.get(node).clone() {
            crate::ast::Struct::List(children) => {
                children.iter().any(|&c| walk(db, c, depth + 1, visited))
            }
            crate::ast::Struct::Atom(_) => false,
        }
    }
    let mut visited = crate::fxhash::FxHashSet::default();
    walk(db, node, 0, &mut visited)
}

/// How many times a body reference resolves to the parameter NAME occurrence `binder` — the way
/// `beta_reduce` substitutes (a `Param { binder }`, or a `Ref` whose chain reaches `binder`). Mirrors
/// `effects::count_param_refs` on the eval side; used by the evaluate-once trigger to tell a param used
/// MULTIPLY (whose effectful arg would be duplicated by by-name substitution) from a single-use one (whose
/// arg substitutes exactly once — no duplication, no let-wrap needed).
fn count_refs_to(db: &mut Db, node: StructId, binder: StructId) -> u32 {
    let here = match resolved_of(db, node) {
        Resolved::Param { binder: b } => u32::from(b == binder),
        Resolved::Ref { value } => {
            let mut target = value;
            let mut hit = false;
            loop {
                if target == binder {
                    hit = true;
                    break;
                }
                match resolved_of(db, target) {
                    Resolved::Ref { value: next } => target = next,
                    _ => break,
                }
            }
            u32::from(hit)
        }
        _ => 0,
    };
    let below = match db.ast.get(node).clone() {
        crate::ast::Struct::List(children) => {
            children.iter().map(|&c| count_refs_to(db, c, binder)).sum()
        }
        crate::ast::Struct::Atom(_) => 0,
    };
    here + below
}

/// If `head` reduces to a lambda, apply it to `args` by β-reduction, returning the reduced body's
/// occurrence. Follows a `Ref` to the lambda (so a `let`-bound function applies too). Parameters match
/// positionally: exact arity β-reduces the whole body; MORE args than params reduces then applies the
/// remainder to the result (curried); FEWER args is a partial application, not yet supported (`Err`).
/// A non-lambda head is `Ok(None)` — the caller tries the `(meta apply)` primitive path instead.
pub fn apply_lambda(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
) -> Result<Option<StructId>, String> {
    // MEMOIZE the reduction by `(reduced-lambda-BODY, args)`. β-reduction is a pure function of the
    // lambda body and the argument occurrences (fixed node identities), so a given (body, args) reduces
    // to the same term every time. Without this, a nested call chain `(f (f (f … 0)))` is EXPONENTIAL:
    // each reduction builds a FRESH body copy (new `StructId`s no downstream memo can hit), and both
    // `infer` and `lower` reduce every call and recurse into the reduced term whose nested calls re-reduce.
    //
    // KEY ON THE RESOLVED BODY, not `head` (seq-203, the handler-fn-per-closure-arg 2^N): `beta_reduce`
    // COPIES structurally, so a DOUBLING fan-out `(f_i b) = (+ (f_{i-1} b) (f_{i-1} b))` produces two
    // sibling `(f_{i-1} b)` calls with DISTINCT head-reference `StructId`s — a `head`-keyed cache misses
    // on the copy and re-reduces each, so the fan-out re-reduces 2^N times (compile-hang; a nullary or
    // pure callee escapes via `core_of`/const-fold value-memo, a closure-arg callee does not). `lambda_of`
    // resolves both copies to the SAME shared lambda body, so keying on that body dedups the fan-out (the
    // 2nd copy hits → reuses the reduced `StructId` → the per-node `core_of`/`type_of` memos then collapse
    // the whole tree). SOUND: a lambda body `StructId` encodes its scope — a capturing lambda copied into
    // a different scope is a DISTINCT body node, so keying on the body never shares across distinct
    // captures; a non-lambda head has no body and falls back to `head` (its `Ok(None)` is head-stable).
    // (`Ok(None)`/`Err(msg)` are deterministic per body too, so they cache as well.)
    let cache_head = match lambda_of(db, head) {
        Some((_, body)) => body,
        None => head,
    };
    let key = (cache_head, args.to_vec());
    if let Some(hit) = db.reduce_cache.get(&key) {
        return match hit {
            Some(reduced) => Ok(Some(*reduced)),
            None => Ok(None),
        };
    }
    let result = apply_lambda_uncached(db, head, args, false);
    // Cache only a DEFINITE reduction outcome (a reduced node, or a non-lambda `None`). An `Err`
    // (recursive / partial application) is left uncached — it is cheap to re-derive (a memoized
    // `is_recursive` read / an arity compare) and never triggers the expensive body copy, so caching it
    // buys nothing and keeps the cache values `Copy`-cheap `Option<StructId>`.
    if let Ok(outcome) = &result {
        db.reduce_cache.insert(key, *outcome);
    }
    result
}

/// Apply a RECURSIVE lambda ONE level — β-substitute `args` into `head`'s body WITHOUT the recursion
/// decline, so a caller can inspect whether the concrete arguments select a NON-recursive (base) arm.
/// This is NOT a general recursive reduction: it substitutes once and returns the (still possibly
/// self-referential) reduced body. The caller (the deferred-resume fold's recursion-unfold) MUST verify
/// the result contains no residual self-call before accepting it — an unfold that still recurses would
/// otherwise unroll unboundedly. Uncached (the recursion guard normally keys the cache-declining path,
/// and this deliberately bypasses it), so keep callers few and gated. Returns `Ok(None)` for a non-lambda
/// head, `Err` only for a partial application.
pub fn apply_lambda_one_level_recursive(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
) -> Result<Option<StructId>, String> {
    apply_lambda_uncached(db, head, args, true)
}

fn apply_lambda_uncached(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
    allow_recursive: bool,
) -> Result<Option<StructId>, String> {
    let (params, body) = match lambda_of(db, head) {
        Some(lam) => lam,
        None => return Ok(None),
    };
    // VARARGS (`DESIGN-variable-arity-functions.md` §3.1): a rest LAST-parameter `(.. binder)` absorbs
    // the TRAILING arguments as a single LIST value. Gather `args[fixed..]` (fixed = the count of leading
    // fixed params) into a synth `#list(…)` and rewrite the argument list to `[fixed args…, list]` — of
    // length exactly `params.len()` — so the ORDINARY positional zip below binds the rest binder to that
    // list, reusing curry/pin/evalonce unchanged (and zero trailing args → the empty list `#list()`).
    // Below the fixed count it is left alone (the existing partial-application curry keeps the `(.. binder)`
    // param in the residual). Each gathered arg is `resolve_subtree`-pinned to its CALLER scope BEFORE it
    // is reparented into the synth list (`push_compound` reparents; pinning memoizes resolution against the
    // caller position — the SAME discipline the per-arg `resolve_subtree` in the zip loop uses).
    let gathered_args: Option<Vec<StructId>> =
        if params.last().is_some_and(|&p| is_rest_param(db, p)) && args.len() >= params.len() - 1 {
            let fixed = params.len() - 1;
            for &a in &args[fixed..] {
                crate::resolve::resolve_subtree(db, a);
            }
            // Gather into a LIST when the rest binder is annotated `(List T)` (homogeneous), else into a
            // TUPLE (heterogeneous, the unannotated / `Tuple` default — monomorphized per call-site since a
            // fresh tuple of the concrete arg types is built at THIS application). `DESIGN-variable-arity §3`.
            let rest_param = *params.last().expect("rest param present");
            let ctor = if rest_gathers_list(db, rest_param) {
                crate::ast::CompoundCtor::List
            } else {
                crate::ast::CompoundCtor::Tuple
            };
            let gathered_value = db.push_compound(ctor, args[fixed..].to_vec());
            crate::resolve::resolve_subtree(db, gathered_value);
            Some(
                args[..fixed]
                    .iter()
                    .copied()
                    .chain(std::iter::once(gathered_value))
                    .collect(),
            )
        } else {
            None
        };
    let args: &[StructId] = gathered_args.as_deref().unwrap_or(args);
    // A RECURSIVE function is not β-reducible to a normal form at compile time — decline BEFORE
    // inlining (the static check, not the depth backstop, is what stops the exponential node blow-up on
    // a branching recursive body). This is the one place every lambda application funnels through (both
    // `infer` and `lower` reach a lambda head here, and currying recurses through here), so the one
    // check covers them all. EXCEPTION (`allow_recursive`): the deferred-resume fold's recursion-unfold
    // needs ONE substitution level of a recursive callee to peek whether the concrete arg selects a
    // non-recursive base arm — it verifies no residual self-call survives before accepting, so the
    // unbounded-unroll risk this guard prevents does not apply to that one gated caller.
    if !allow_recursive && is_recursive(db, body) {
        trace!(target: "rcdzc::eval", body = body.0, "decline: recursive function (needs runtime specialization)");
        return Err(
            "a recursive function needs runtime specialization, which is not supported".to_string(),
        );
    }
    trace!(target: "rcdzc::eval", body = body.0, params = params.len(), args = args.len(), "β-reduce lambda application");
    //= spec/capabilities/core-semantics.md#functions-are-single-arity
    //# Partial application MUST be natural: applying a curried function to fewer arguments than its full chain returns a closure awaiting the remaining arguments.
    if args.len() < params.len() {
        // PARTIAL APPLICATION — compile-time currying (`core-semantics.md` §Functions Are
        // Single-Arity: "applying a curried function to fewer arguments than its full chain returns a
        // closure awaiting the remaining arguments"). Substitute the provided args for the leading
        // parameters, then wrap the remaining parameters in a fresh residual lambda
        // `(fn (remaining…) reduced-body)`. Applying that residual to the rest of the arguments
        // β-reduces it the same way, so `((add 3) 4)` folds to `(+ 3 4)` → 7 with no residual
        // function surviving to run time. The residual's parameter binders are FRESH copies (same
        // spelling) so the original lambda is untouched (its param occurrences keep their parent under
        // the original `fn`/`def`); a body reference to a remaining parameter re-resolves, by name,
        // against the residual's scope to the residual's binder — exactly how `beta_reduce` re-resolves
        // a `let`-local across the copy.
        let mut arg_of: HashMap<StructId, StructId> = HashMap::default();
        for (p, a) in params.iter().zip(args.iter()) {
            crate::resolve::resolve_subtree(db, *a);
            // Carry the parameter's annotation onto its argument (`(: arg T)`) so a narrow-width fit-check
            // fires on the substituted value — see `substituted_arg`. (The curry/residual path shares the
            // hole: an out-of-range constant to an annotated leading param must reject here too.)
            let sub = substituted_arg(db, *p, *a);
            arg_of.insert(param_name_occ(db, *p), sub);
        }
        // NOTE: the partial path does NOT pin free vars — the REMAINING (un-applied) params are bound by
        // the lambda's `fn` (outside `body`), so a `pin_free_vars` keyed on `body` would misclassify a
        // remaining-param reference as a capture and share it, breaking the residual's re-resolution. A
        // genuine capture in a residual is handled by the existing pin-of-args + `resolved_subtrees` arm.
        let reduced_body = beta_reduce(db, body, &arg_of);
        let remaining: Vec<StructId> = params[args.len()..]
            .iter()
            .map(|&p| copy_param(db, p))
            .collect();
        let params_list = db.push_list(remaining);
        let fn_head = db.push_name("fn");
        let residual = db.push_list(vec![fn_head, params_list, reduced_body]);
        return Ok(Some(residual));
    }
    // EVALUATE-ONCE for an EFFECTFUL multi-use argument (strict-eval, not call-by-name). β-reduction
    // splices the SHARED argument occurrence at EVERY body reference of a parameter (`beta_reduce` returns
    // the same arg node per site). For a PURE arg that is harmless — a value duplicated is the same value.
    // But an arg that PERFORMS (a host op, an outer/discharged effect) spliced at N references RE-PERFORMS N
    // times: `(sum3 (mk (E.get)))` with `mk s = (T s s s)` emits THREE host `get` calls, want ONE. That
    // violates strict by-value binding (core-semantics.md §Applying A Function binds the parameter to a
    // single evaluated value; §283) and the deterministic host-call sequence (capabilities §75). So for a
    // parameter used MULTIPLY whose argument REACHES a perform, LET-BIND the argument ONCE at the call site:
    // substitute a fresh let-local name at each reference and wrap the reduced body in `(let ((#a arg)) …)`,
    // so the effect runs once and every use reads the bound value. A single-use param, or a pure arg (even
    // multi-use), is UNTOUCHED — its substitution is byte-identical to before (no let, the common inline is
    // unchanged). The in-program-HANDLER analogue already evaluates once via the fold's arm-substitution;
    // this closes the HOST/general β-reduce gap the fold never sees (a pure helper whose param carries the
    // effect). NOTE: keyed on THIS application's own params — `mk`'s application funnels through here (its
    // param `s` is used 3×), where the earlier by-the-OUTER-call attempts missed it (`sum3`'s `t` is
    // single-use). A LAMBDA arg is never let-bound (it is not an effect value — it is applied later).
    let mut evalonce_wraps: Vec<(StructId, StructId)> = Vec::new();
    let mut arg_of: HashMap<StructId, StructId> = HashMap::default();
    for (p, a) in params.iter().zip(args.iter()) {
        // PIN each argument's meaning at the CALL SITE before reducing. `beta_reduce` splices the
        // shared argument occurrence into a freshly `push_list`-built copy of the body, and
        // `push_list` re-parents its children — which moves the argument's root out of the caller's
        // scope. Resolving the argument subtree now memoizes every node against its caller-side
        // position, so the later re-parent cannot change how a caller-bound name inside the argument
        // resolves (`(let ((k 10)) (inc k))` — `k` must stay bound to the caller's `let`). The body's
        // OWN names still resolve lazily after the copy, against the copied scope, as intended.
        //
        // A LAMBDA argument is the exception: pinning its WHOLE subtree freezes its OWN params too, so
        // when that arg lambda is later APPLIED inside a lifted body (`(def (mk (: g (-> A B))) (fn (x)
        // (g x)))` given `(fn (y) (+ y 1))` — `g`'s body substitutes the arg lambda, and `((fn y..) x)`
        // must β-reduce), `beta_reduce`'s pinned-→-share arm shares the arg lambda's own-param refs as-is
        // instead of substituting them, leaving `y` a slot-less `Core::Param` ("parameter reference has no
        // local slot"). So pin only the arg lambda's FREE variables (its captures, bound OUTSIDE it),
        // exactly as `pin_free_vars` does for a returned lambda — leaving its own params unpinned to
        // re-resolve/substitute on the later application. (A LET-bound lambda already works because the
        // binding is a distinct value node, not a param-substituted inline AST — this brings the inline
        // form to parity.)
        if let Some((arg_params, arg_body)) = syntactic_lambda(db, *a) {
            let own: Vec<StructId> = arg_params.iter().map(|&p| param_name_occ(db, p)).collect();
            pin_free_vars(db, arg_body, arg_body, &own);
        } else {
            crate::resolve::resolve_subtree(db, *a);
        }
        // A body reference to a parameter binds to the parameter's NAME occurrence (resolve's
        // `binder_in` returns the name, seeing through a `(: name T)` annotated binder). So key the
        // substitution on the name occurrence too, not the raw signature child (which is the `(:…)`
        // node for an annotated param) — else the arg would never match the body's references.
        // The substituted VALUE carries the parameter's annotation (`(: arg T)`) so its type — in
        // particular a narrow integer WIDTH — is checked against the argument, exactly as a direct
        // `(: value T)` is (`substituted_arg`); otherwise an out-of-range constant `(f 200)` to a
        // `(: a Int8)` param was spliced raw and run to a value the declared type cannot hold.
        let sub = substituted_arg(db, *p, *a);
        let name_occ = param_name_occ(db, *p);
        // EVALUATE-ONCE trigger: a param used ≥2× whose (non-lambda) arg reaches a perform. Do NOT
        // substitute the arg (which `beta_reduce` would SHARE at every site → N performs). Instead LEAVE
        // this param out of `arg_of`, so each body reference is `copy_structural`-copied as a FRESH name
        // occurrence of the parameter's own spelling; then wrap the reduced body in `(let ((name arg)) …)`
        // binding that name to the arg ONCE. Each fresh copy re-resolves (lazily, post-copy) against the
        // copied scope to the wrapping `let` — exactly the hand-written workaround `(let ((s (E.get))) (T s
        // s s))`. (A shared single node can't do this: a name atom has ONE parent, so only one site's scope
        // walk would reach the `let`.) `sub` (the arg, carrying the param's `(: arg T)` annotation) becomes
        // the let-init, keeping the width fit-check. A body-internal binding of the same name still shadows
        // correctly (ordinary lexical scope — the inner binder is a binder-occurrence, copied not renamed).
        if syntactic_lambda(db, *a).is_none()
            && count_refs_to(db, body, name_occ) >= 2
            && arg_reaches_perform_evalonce(db, *a)
        {
            let binder = copy_structural(db, name_occ, &HashMap::default());
            evalonce_wraps.push((binder, sub));
        } else {
            arg_of.insert(name_occ, sub);
        }
    }
    // PIN the body's FREE VARIABLES — a reference in the body that binds OUTSIDE the lambda (an enclosing
    // `let`/param the lambda CAPTURES, e.g. `(let ((k 10)) ((fn (x) (+ x k)) 5))` — `k` binds to the
    // enclosing `let`). Without this, `beta_reduce` would COPY the free name fresh (`copy_structural`),
    // and the copy — re-parented into the freshly-built reduced node (an orphan, parent = None) — would
    // re-resolve against a scope where the capture is UNBOUND (a spurious CDZ0101 / a miscompile). Pinning
    // shares the ORIGINAL occurrence (its resolution memoized against the capture's real binding), the
    // same mechanism arg-pinning uses; `beta_reduce`'s `resolved_subtrees` check (the "captured free
    // variable" arm) then preserves it. A body-INTERNAL `let`-local is NOT pinned (its binder is within
    // the lambda), so it still copies + re-resolves against the copied scope. The lambda's OWN params are
    // excluded (they resolve to the `fn`'s param list, outside `body`, but are substituted by `arg_of`
    // here — pinning them would wrongly SHARE a param ref that must be replaced by its argument).
    let own_params: Vec<StructId> = params.iter().map(|&p| param_name_occ(db, p)).collect();
    pin_free_vars(db, body, body, &own_params);
    // Record the COPY ROOT so the capture-share exception copies (not shares) a pinned pattern binder
    // whose scrutinee lies WITHIN this body (see `Db::reduction_root`).
    let saved = db.reduction_root.replace(body);
    let mut reduced = beta_reduce(db, body, &arg_of);
    db.reduction_root = saved;
    // EVALUATE-ONCE: wrap the reduced body in one `(let ((#a arg)) …)` per effectful multi-use param, so
    // the argument's perform runs ONCE (at the let-init) and every use reads the bound local. Innermost =
    // LAST recorded (a later param's arg may reference an earlier binding), matching left-to-right eval
    // order. Each `#a` name is spliced (shared) at every reference in `reduced`; it lowers to a pure
    // local read, so sharing it is sound (unlike the shared HostCall it replaces). The wrap is re-resolved
    // when the reduced body is (a synthesized `let` like the fold's own let-lifts + the `#cv` bindings).
    for (binder, init) in evalonce_wraps.into_iter().rev() {
        let let_head = db.push_name("let");
        let pair = db.push_list(vec![binder, init]);
        let bindings = db.push_list(vec![pair]);
        reduced = db.push_list(vec![let_head, bindings, reduced]);
    }
    // A do-local RECURSIVE function inside `body` was COPIED by the reduction (fresh occurrences); its
    // recursive self-call now resolves to the COPY's lambda, whose body is not yet in `def_by_body`. So a
    // helper `(def (helper x) (do (def (fac n) …) (fac x)))` inlined at its call site would decline "needs
    // runtime specialization". Register the reduced copy's do-local function defs so the copied recursive
    // call lowers to a `Core::Call` (the copy-time counterpart of `modules::register_do_local_callables`).
    // Idempotent + bounded: this application memoizes (`apply_lambda`), and an already-registered body is
    // skipped — so a non-recursive or already-seen copy adds nothing.
    db.register_reduced_callables(reduced);
    let rest = &args[params.len()..];
    if rest.is_empty() {
        Ok(Some(reduced))
    } else {
        match apply_lambda(db, reduced, rest)? {
            Some(r) => Ok(Some(r)),
            // The reduced body is not a further lambda to absorb `rest`. If it is a (possibly partial)
            // CONSTRUCTOR — a helper `(def (mk x) (Pair x))` over-applied `(mk 3 4)` reduces the body to
            // `(Pair 3)`, leaving `[4]` — synthesize the combined application `((Pair 3) 4)` and return
            // it. A sum constructor is single-arity (core-semantics.md §A Sum Type Constructor Is A Single-
            // Arity Function), so this curried spine is the SAME construction as the flat `(Pair 3 4)`;
            // lowering flattens it via `ctor_spine`. Anything else over-applied (a scalar, a non-ctor
            // record) genuinely takes more arguments than it accepts — keep the error.
            None if spine_bottom_is_variant_ctor(db, reduced) => {
                let mut app = vec![reduced];
                app.extend_from_slice(rest);
                let combined = db.push_list(app);
                crate::resolve::resolve_subtree(db, combined);
                Ok(Some(combined))
            }
            None => Err(crate::diag::OVER_APPLICATION_DECLINE.to_string()),
        }
    }
}

/// Whether the value at `id` is a (possibly partially-applied) sum-VARIANT constructor — a bare ctor
/// record `(Pair)`, or an application spine `((Pair 3))` whose ultimate head is one. Used by the over-
/// application path to tell a constructor result (which absorbs more arguments as payloads) from a scalar
/// result (which genuinely rejects them). Follows a `let`/`def` ref and peels `Apply` heads to the bottom.
fn spine_bottom_is_variant_ctor(db: &mut Db, id: StructId) -> bool {
    let node = match resolved_of(db, id) {
        Resolved::Ref { value } => value,
        _ => id,
    };
    match resolved_of(db, node) {
        Resolved::Apply { head, .. } => spine_bottom_is_variant_ctor(db, head),
        _ => variant_disc_of(db, node).is_some(),
    }
}

/// Copy a lambda PARAMETER occurrence into a fresh binder for a residual (partially-applied) lambda —
/// a bare name atom becomes a fresh atom of the same name, an annotated `(: name T)` binder becomes a
/// fresh list of freshly-copied children. Fresh occurrences so the residual's binders are distinct from
/// the original lambda's (a body reference re-resolves by name to the residual's binder across the copy,
/// as in `beta_reduce`); the empty `arg_of` means nothing is substituted, only re-parented.
fn copy_param(db: &mut Db, param: StructId) -> StructId {
    let empty: HashMap<StructId, StructId> = HashMap::default();
    copy_structural(db, param, &empty)
}

/// The NAME occurrence a parameter binds — seeing through a `(: name T)` annotated binder to `name`,
/// or the occurrence itself for a bare parameter. This is the identity a body reference to the
/// parameter resolves to (via resolve's `binder_in`), so β-substitution keys on it. Mirrors resolve's
/// `param_name` on the evaluator side (a param occurrence is either a name atom or a `(: name T)` list).
pub(crate) fn param_name_occ(db: &Db, param: StructId) -> StructId {
    // A REST parameter `(.. binder)` (varargs, `DESIGN-variable-arity-functions.md` §2): peel the `..`
    // marker to its binder (a bare name or a `(: name T)`), then fall through to the name-of-binder logic.
    if let Some(tail) = db.ast.as_form(param, "..")
        && let Some(&inner) = tail.first()
    {
        return param_name_occ(db, inner);
    }
    if db.ast.as_name(param).is_some() {
        return param;
    }
    if let Some(tail) = db.ast.as_form(param, ":")
        && let Some(&name_occ) = tail.first()
    {
        return name_occ;
    }
    param
}

/// Whether a parameter occurrence is a REST parameter `(.. binder)` — the varargs marker
/// (`DESIGN-variable-arity-functions.md` §2). The binder inside is the name (bare or `(: name T)`) the
/// gathered arguments bind to; `param_name_occ` peels the `..` to reach it.
pub(crate) fn is_rest_param(db: &Db, param: StructId) -> bool {
    db.ast.as_form(param, "..").is_some()
}

/// Whether the function `head` resolves to is VARARGS — its LAST parameter is a rest `(.. binder)`
/// (`DESIGN-variable-arity-functions.md` §3.1). Reads the RAW parameter occurrences via [`lambda_of`]
/// (NOT `lambda_params_of`, which peels each to its name so a `(..)` marker would be invisible), so the
/// application arity checks can allow the surplus arguments a rest param absorbs into its list.
pub(crate) fn callee_is_varargs(db: &mut Db, head: StructId) -> bool {
    lambda_of(db, head)
        .and_then(|(ps, _)| ps.last().copied())
        .is_some_and(|p| is_rest_param(db, p))
}

/// Whether a rest parameter `(.. binder)` gathers its trailing arguments into a homogeneous LIST
/// (`(: name (List T))`) rather than the heterogeneous TUPLE default (`DESIGN-variable-arity-functions.md`
/// §3.1/§3.3). A rest binder annotated with a `(List …)` type gathers a `List`; anything else — an
/// UNANNOTATED `(.. name)` or a `(: name Tuple)` / `(: name (Tuple …))` — gathers a `Tuple` (heterogeneous,
/// monomorphized per call-site). Read off the binder's annotation head.
pub(crate) fn rest_gathers_list(db: &Db, rest_param: StructId) -> bool {
    let Some(inner) = db
        .ast
        .as_form(rest_param, "..")
        .and_then(|t| t.first().copied())
    else {
        return false;
    };
    // `(.. (: name T))` — gather a list iff `T`'s head is `List`; a bare `(.. name)` → tuple.
    db.ast
        .as_form(inner, ":")
        .and_then(|t| t.get(1).copied())
        .is_some_and(|ty| db.ast.head_name(ty) == Some("List"))
}

/// The value to β-substitute for a parameter, carrying the parameter's TYPE ANNOTATION so its argument
/// is CHECKED against the declared type — the fix for the "constant argument launders past a narrow
/// parameter width" hole. For a bare parameter this is just `arg`. For an ANNOTATED parameter `(: name
/// T)` it is the argument WRAPPED in that annotation, `(: arg T)`: β-reduction then splices `(: arg T)`
/// wherever the body referenced the parameter, so the SAME annotation fit-check a direct `(: 200 Int8)`
/// gets fires on the substituted argument — an out-of-range constant (`(f 200)` to `(: a Int8)`) is
/// rejected CDZ0302 instead of being spliced raw as `200` and run to an out-of-range value. Without the
/// wrap, `param_name_occ` sees THROUGH the `(: name T)` binder, so the annotation `T` was discarded and
/// the width was never checked. An in-range constant round-trips unchanged (the annotation grounds it);
/// a runtime argument keeps its own (already-checked) type. The annotation type occurrence `T` is shared
/// from the parameter (pinned by the caller's `resolve_subtree` of the signature) — a type expression
/// resolves position-independently (its names are global), so sharing it into the wrap is sound.
/// Whether `arg` is a 1-of-2 PARTIAL binary-operator application `(+ 10)` / `(< 3)` / `(- 5)` — a curried
/// operator (ch01e). Peels a `Ref` (a `let`-bound partial `(let ((add10 (+ 10))) …)`) to reach the
/// application. Arity-1 `Sub` `(- 5)` is a partial subtraction like the others (prefix negation deprecated
/// in favour of `Num.neg`/`T.neg`), so it is INCLUDED, matching `partial_binop_eta`.
fn is_partial_binop_app(db: &mut Db, arg: StructId) -> bool {
    let mut cur = arg;
    while let Resolved::Ref { value } = resolved_of(db, cur) {
        cur = value;
    }
    if let Resolved::Apply { head, args } = resolved_of(db, cur)
        && args.len() == 1
        && let Some(p) = meta_apply_of(db, head)
        && (p.is_arith() || p.is_comparison() || p.is_float_arith())
    {
        return true;
    }
    false
}

fn substituted_arg(db: &mut Db, param: StructId, arg: StructId) -> StructId {
    // Only an annotated `(: name T)` parameter carries a type to check; a bare param substitutes its arg
    // directly. (`param` may itself be the `(: name T)` list — read its type child `T`.)
    let Some(ty_occ) = db
        .ast
        .as_form(param, ":")
        .and_then(|tail| tail.get(1).copied())
    else {
        // A BARE parameter with NO written annotation — but if its type was RECOVERED from the lambda's
        // storage context (a `(fn (n) …)` passed where a `(-> Int8 Int8)` is declared), that narrow width
        // must ALSO check the argument, exactly as a written `(: n Int8)` would. Without this, a const arg
        // into the body's arithmetic folds at the default Int64 and MISSES the narrow overflow (`(+ n 1)`
        // with n=127 → 128 instead of an Int8 overflow-reject). Synthesize the SAME `(: arg (Int/UInt N))`
        // wrap the annotated path builds, from the recovered NARROW integer type — so the fit-check fires
        // on the substituted argument and travels with it through any later β-copy. Only a bare param whose
        // context recovers a concrete NARROW int (a wide Int64/UInt64 needs no width fit-check; a non-int
        // or unrecovered type substitutes the arg raw, unchanged).
        let name_occ = param_name_occ(db, param);
        if let crate::ty::Ty::Int(it) = crate::infer::type_of(db, name_occ) {
            let (signed, width) = (it.ground_signed(), it.ground_width());
            // Only a NARROW width (< 64) needs the fit-check wrap; a full-width int fits its own slot.
            if width < 64 {
                let ctor = db.push_name(if signed { "Int" } else { "UInt" });
                let w = db.push_atom(Leaf::Int {
                    value: IntValue::from_i64(width as i64),
                    radix: crate::ast::Radix::Dec,
                });
                let ty_node = db.push_list(vec![ctor, w]);
                crate::resolve::resolve_subtree(db, ty_node);
                let colon = db.push_name(":");
                return db.push_list(vec![colon, arg, ty_node]);
            }
        }
        return arg;
    };
    // A bare OPERATOR argument to a FUNCTION-typed parameter is substituted RAW, never wrapped. When the
    // parameter type is a function arrow `(-> …)` and the argument is a bare operator (`meta_apply_of` is a
    // prim and it is not itself an application), wrapping it as `(: + (-> …))` would put an annotated
    // operator in HEAD position once β-reduction splices it (`(apply2 + 3 4)` → `((: + …) 3 4)`), where the
    // operation dispatch cannot reach its `(meta apply)` and declines "value is not applyable". Substituting
    // raw keeps `+` a dispatchable head that folds `(+ 3 4)`. The annotation's only job is the narrow-int
    // fit-check, irrelevant to a function-typed parameter. Gated on the parameter type being an ARROW so a
    // value-prim argument to a non-function parameter (e.g. `Map.empty` to a `(Map …)` param) still gets its
    // wrap — its annotation grounds the value's element types and must be kept.
    if db.ast.as_form(ty_occ, "->").is_some()
        && !matches!(resolved_of(db, arg), Resolved::Apply { .. })
        && meta_apply_of(db, arg).is_some()
    {
        return arg;
    }
    // A 1-of-2 PARTIAL binary-operator application `(+ 10)` as the argument — the ch01e curry: it IS a
    // first-class function of the remaining operand. Like the bare-operator case above, substitute it RAW
    // when the parameter is a function arrow: wrapping it `(: (+ 10) (-> …))` puts an annotated partial in
    // HEAD position once β-reduction splices it — `(apply1 (+ 10) 5)` → `((: (+ 10) …) 5)` — where the
    // dispatch cannot see through to fold `((+ 10) 5)` (the un-annotated `((+ 10) 5)` folds fine). Substituting
    // raw keeps the curried spine foldable. Gated on the arrow parameter type (as above), so a partial op to a
    // non-function parameter is unaffected.
    if db.ast.as_form(ty_occ, "->").is_some() && is_partial_binop_app(db, arg) {
        return arg;
    }
    // Pin the annotation type before it is shared into the synthesized `(: arg T)` — its names resolve
    // the same wherever the wrap lands (they are global type names), exactly as a pinned argument does.
    crate::resolve::resolve_subtree(db, ty_occ);
    let colon = db.push_name(":");
    db.push_list(vec![colon, arg, ty_occ])
}

/// The body occurrence of the lambda `head` reduces to, if any — the stable per-function identity the
/// recursion guard keys on (so a recursive call, which re-enters the same body, is detected).
pub fn lambda_body(db: &mut Db, head: StructId) -> Option<StructId> {
    lambda_of(db, head).map(|(_, body)| body)
}

/// The RAW parameter nodes + body of the lambda `id` reduces to (a bare `(fn …)`, a named def-ref, an
/// annotated/`let`-wrapped lambda), or `None` if `id` is not a function value. Public sibling of
/// `lambda_params_of`/`lambda_body` for callers that need BOTH and the RAW param nodes — notably
/// reflection (`Type.of`), which hands them to `infer::solved_lambda_arrow` (that helper maps each raw
/// param to its own name occurrence, so it must NOT be pre-projected as `lambda_params_of` does).
pub fn lambda_params_and_body(
    db: &mut Db,
    id: StructId,
) -> Option<(std::rc::Rc<[StructId]>, StructId)> {
    lambda_of(db, id)
}

/// The def-BODY occurrence a NULLARY-def head refers to, if `head` names one — a nullary def resolves
/// its name to a `Ref` straight at its body (no `Lambda` wrapper). Returns that body so a caller can
/// ask `is_recursive` of it (a nullary self-call `(def (f) (f))` must be detected as recursive and
/// declined, not inlined without end). `None` for a head that is not a nullary-def reference.
pub fn lambda_body_of_nullary(db: &mut Db, head: StructId) -> Option<StructId> {
    match resolved_of(db, head) {
        Resolved::Ref { value } if db.def_index_by_body(value).is_some() => Some(value),
        Resolved::Ref { value } => lambda_body_of_nullary(db, value),
        _ => None,
    }
}

/// The PARAMETER NAME occurrences of the lambda `head` reduces to, in signature order — each the
/// identity a body reference resolves to and the node whose `type_of` is the parameter's declared
/// type (its annotation, or `Any` for a bare param). Used by infer to check each call argument against
/// its parameter's type at the call site, which β-reduction alone erases (the parameter's annotation
/// is dropped when its argument is substituted into the body).
pub fn lambda_params_of(db: &mut Db, head: StructId) -> Option<Vec<StructId>> {
    let (params, _) = lambda_of(db, head)?;
    Some(params.iter().map(|&p| param_name_occ(db, p)).collect())
}

/// Is the function whose body is `body` (transitively) RECURSIVE — does the static call graph contain
/// a cycle back to `body`? A recursive function cannot be β-reduced to a normal form at compile time
/// (it would inline without end, and one that branches — several self-calls per body, like a CBOR tree
/// reader — explodes EXPONENTIALLY in appended nodes long before any depth bound), so the evaluator
/// reads this BEFORE reducing a call and DECLINES a recursive one. The evaluator only monomorphizes by
/// reduction; a recursive call is instead emitted as a runtime `Core::Call` by [`crate::lower`] (when
/// the callee's signature is determined), so this decline is a hand-off to lowering, not an
/// unimplemented feature. This is the SOUND, PURELY STRUCTURAL recursion signal — a static property of
/// the resolved call graph, computed WITHOUT reducing (no `apply_lambda`, no arena growth), unlike the
/// body-on-the-stack set that false-positived on `(f (f v))`. Cached per body occurrence (the fixed
/// def/`let` structure makes the verdict stable), the same memoization `build_cache` uses.
pub fn is_recursive(db: &mut Db, body: StructId) -> bool {
    if let Some(&v) = db.recursive.get(&body) {
        return v;
    }
    // A cycle back to `body`: search the callee edges; `body` is recursive iff some path of ≥1 call
    // returns to it. The search is an ITERATIVE worklist DFS over the call graph (an explicit stack,
    // no native recursion over the potentially-unbounded graph), and it reuses the Db's pooled
    // buffers rather than allocating per call: take them out (freeing `db` for the walk), clear, use,
    // put back. `visited` bounds the walk to each reachable body once (finite — bodies are arena
    // nodes). Only THIS top-level query is cached; the bodies visited along the way answer "does C
    // reach `body`", not "is C recursive", so they are not memoized here.
    let mut visited = std::mem::take(&mut db.rec_visited);
    let mut worklist = std::mem::take(&mut db.rec_worklist);
    visited.clear();
    worklist.clear();

    // Seed the frontier with `body`'s direct callees; then expand each not-yet-seen callee. A frontier
    // entry equal to `body` closes the cycle. (`body` itself is never inserted into `visited`, so a
    // path returning to it is detected on pop.) Each node's edges come from the memoized `callees_of`,
    // so a body reached from many ancestors is walked (and `resolved_of`-cloned) only ONCE overall.
    ensure_callees(db, body);
    worklist.extend_from_slice(&db.callee_edges[&body]);
    let mut result = false;
    while let Some(node) = worklist.pop() {
        if node == body {
            result = true;
            break;
        }
        if visited.insert(node) {
            // Compute-then-read: `ensure_callees` fills the memo (needs `&mut db`), then the cached
            // slice is pushed into the local `worklist` (which was taken out of `db`, so borrowing
            // `db.callee_edges` immutably here does not conflict). No per-visit allocation.
            ensure_callees(db, node);
            worklist.extend_from_slice(&db.callee_edges[&node]);
        }
    }

    // Return the buffers for the next body's walk to reuse.
    db.rec_visited = visited;
    db.rec_worklist = worklist;
    db.recursive.insert(body, result);
    trace!(target: "rcdzc::eval", body = body.0, recursive = result, "recursion analysis");
    result
}

/// ENSURE `node`'s direct callee edges are computed and cached in `db.callee_edges`, MEMOIZED. The
/// edge set is a pure function of the fixed resolved structure, so it is computed once — via
/// [`collect_callees`] — and cached; the recursion DFS then reads the cached slice directly (no clone,
/// no re-walk). Computing the walk re-`resolved_of`s the subtree and clones a `Resolved` per node, so
/// doing it once per body — rather than once per ancestor that reaches the body — is the O(N²)→O(N)
/// churn removal. Returns nothing: the caller reads `db.callee_edges[&node]` after this returns.
fn ensure_callees(db: &mut Db, node: StructId) {
    if db.callee_edges.contains_key(&node) {
        return;
    }
    let mut edges = Vec::new();
    collect_callees(db, node, &mut edges);
    db.callee_edges.insert(node, edges);
}

/// Collect the callee bodies of the function whose body expression is (rooted at) `node` — a LOCAL,
/// NON-REDUCING syntactic walk of one function's body. For each application, the head's statically-known
/// callee body (via [`callee_body`], following refs to a lambda WITHOUT reducing) is one edge; the walk
/// descends into every sub-expression that runs when this body runs. It STOPS at a nested `fn`
/// boundary — a lambda defined inside this body is a SEPARATE call-graph node (its own calls are edges
/// from IT, explored only if IT is called), so descending into it would conflate two nodes. This keeps
/// the graph precise: over-reporting would decline a terminating fold, under-reporting would let a
/// recursive one reach the (slow, exponential) depth backstop.
///
/// This is the UNCACHED walk; callers in the recursion analysis go through [`callees_of`] to memoize
/// per node. (`collect_callees` recurses into sub-expressions of the SAME body, accumulating into
/// `out`; the memo keys only whole body/callee nodes the DFS visits, not every interior node.)
fn collect_callees(db: &mut Db, node: StructId, out: &mut Vec<StructId>) {
    // A nested `(do e0 … en)` — EVERY item runs (in sequence), so a self-call in ANY of them is a real
    // recursion edge. `resolve_do` collapses a `do` to `Ref{last}` (its intermediates are discarded as
    // pure — no effect model at that layer), which would HIDE a self-call in an intermediate from the
    // callee walk (e.g. `(do (E.emit n) (walk (- n 1)))` — the recursive `(walk …)` is the last item, but
    // an intermediate could also self-call). Read the `do` by raw AST and descend every item, so
    // `is_recursive` sees a self-call wherever it sits in the block. (Checked before the `resolved_of`
    // match, which would only see the collapsed `Ref{last}` and miss the intermediates.)
    if let Some(items) = db.ast.as_form(node, "do") {
        for &item in items.to_vec().iter() {
            // A do-local `(type …)` / `(effect …)` is a DECLARATION with no callees — skip it (descending
            // would resolve its `type`/`effect` head as an unbound value). Every other form runs.
            if matches!(db.ast.head_name(item), Some("type") | Some("effect")) {
                continue;
            }
            // A do-local VALUE def `(def x V)` runs its RHS `V` when the block runs — a self-call in `V`
            // (`(def hh (f …))`) is a REAL recursion edge. But the `def` form resolves to a declaration
            // (`Poison`/`Ref`), NOT an expression, so the general descent below would treat it as a leaf and
            // MISS the call in `V` → `is_recursive` returns false for a fn whose only self-call sits in a
            // do-def RHS → the caller β-reduces it as non-recursive, descending REDUCE_DEPTH_LIMIT levels and
            // mis-typing a heap-scalar recursive result as kind `Type` (the bogus CDZ0201 / >100s hang,
            // recursive-BigInt-rebind bug). Descend into the value-RHS explicitly. (A FUNCTION do-def
            // `(def (g p…) body)` is a separate lifted def — `do_value_def_value` returns None for it, so it
            // is not walked here; its own recursion is analyzed at its own body.)
            if let Some(v) = crate::resolve::do_value_def_value(db, item) {
                collect_callees(db, v, out);
                continue;
            }
            collect_callees(db, item, out);
        }
        return;
    }
    match resolved_of(db, node) {
        Resolved::Apply { head, args } => {
            if let Some(cb) = callee_body(db, head) {
                out.push(cb);
            }
            // Descend into the head ONLY if it is itself an application (a computed head like
            // `((g x) y)`); a head that is a plain function reference is captured by `callee_body`
            // above, and descending into the lambda/ref it names would cross into another node.
            if matches!(resolved_of(db, head), Resolved::Apply { .. }) {
                collect_callees(db, head, out);
            }
            for &a in args.iter() {
                collect_callees(db, a, out);
            }
        }
        Resolved::If { cond, then_, else_ } => {
            collect_callees(db, cond, out);
            collect_callees(db, then_, out);
            collect_callees(db, else_, out);
        }
        // A boolean connective's operands can each contain a call (a self-call in a short-circuit operand
        // is a real recursion edge) — descend both. A negation's single operand likewise.
        Resolved::And { lhs, rhs, .. } => {
            collect_callees(db, lhs, out);
            collect_callees(db, rhs, out);
        }
        Resolved::Not { operand } => collect_callees(db, operand, out),
        // `(try e)` — descend into the fallible operand (its callees are the `try`'s callees).
        Resolved::Try { operand } => collect_callees(db, operand, out),
        // A match's scrutinee and every arm BODY run when this body runs — a self-call inside an arm
        // (e.g. `sum-to`'s match base case) is a real edge, so descend into each. (A pattern is not
        // executed code — it is a probe — so it contributes no callee.)
        Resolved::Match { scrutinee, arms } => {
            collect_callees(db, scrutinee, out);
            for (_, body) in arms {
                collect_callees(db, body, out);
            }
        }
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_callees(db, value, out);
            }
            collect_callees(db, body, out);
        }
        Resolved::Record { fields } => {
            for (_, &value) in fields.iter() {
                collect_callees(db, value, out);
            }
        }
        Resolved::Member { operand, .. } => collect_callees(db, operand, out),
        // A tuple's elements and a projection's operand run when this body runs — a call inside them is
        // a real edge, so descend.
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
            for &e in elems.iter() {
                collect_callees(db, e, out);
            }
        }
        // A map literal runs each entry's key AND value when this body runs.
        Resolved::Map { entries } => {
            for &(k, v) in entries.iter() {
                collect_callees(db, k, out);
                collect_callees(db, v, out);
            }
        }
        // A `(bin …)` runs each segment's value slot (and dependent size) when this body runs.
        Resolved::Bin { segs } => {
            for s in segs.iter() {
                collect_callees(db, s.slot, out);
                match &s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(n) } => {
                        collect_callees(db, *n, out)
                    }
                    crate::resolved::SegKind::Utf8 { size } => collect_callees(db, *size, out),
                    _ => {}
                }
            }
        }
        Resolved::Proj { operand, .. } => collect_callees(db, operand, out),
        // An annotation is transparent — a call inside `(: (f x) T)` is a real edge of this body.
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            collect_callees(db, expr, out)
        }
        // A handler's init, each arm body, and the handled body all run when this body runs — a self-call
        // inside any is a real recursion edge, so descend into each. (The arm's op projection and its
        // param/state binders are not executed code — they contribute no callee.)
        Resolved::Handle { init, arms, body } => {
            collect_callees(db, init, out);
            for arm in arms.iter() {
                collect_callees(db, arm.body, out);
            }
            collect_callees(db, body, out);
        }
        // A resumption's value and next-state run when the arm runs — descend both.
        Resolved::Resume { value, next_state } => {
            collect_callees(db, value, out);
            collect_callees(db, next_state, out);
        }
        // A host delegation's body runs when this body runs — descend (the effect names are not code).
        Resolved::Host { body, .. } => collect_callees(db, body, out),
        // A nested `fn` is a separate node — do NOT descend (its calls are its own edges). A bare ref,
        // a leaf, a prim, a param, a type value, a poison have no callees of THIS body.
        // A `SumPayload` reads the scrutinee's payload; the scrutinee's own callees are collected via the
        // enclosing match's scrutinee position, so a payload reference adds no edge of this body.
        Resolved::Lambda { .. }
        | Resolved::Ref { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::RecordField { .. }
        | Resolved::RecordRest { .. }
        | Resolved::SetRest { .. }
        | Resolved::Int(_)
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Rational { .. }
        | Resolved::Unit
        | Resolved::Prim(_)
        | Resolved::Param { .. }
        | Resolved::TypeVal(_)
        | Resolved::Poison(_) => {}
    }
}

/// The statically-known callee body of an application head, if any — following a `Ref` to the lambda it
/// binds (a `let`-bound function, or a top-level def, which resolves straight to a `Lambda`), WITHOUT
/// reducing. `None` for a head whose callee cannot be known without evaluation (a computed head, a
/// primitive, a non-function) — such an edge is left to the depth backstop rather than forced here.
fn callee_body(db: &mut Db, head: StructId) -> Option<StructId> {
    match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => Some(body),
        Resolved::Ref { value } => {
            // A NULLARY def resolves its name straight to its BODY occurrence (a value, not a `Lambda`
            // wrapper — that is what makes a bare `g` denote the body value). So a call `(g)` reaching
            // this `Ref` has its callee body AT `value`: recognize `value` as a def body and return it,
            // rather than recursing INTO it (which would classify the body's own head, missing the
            // self-call edge). Without this a nullary self-recursion `(def (f) (f))` is not detected as
            // recursive, so the call inlines without end. A `Ref` to anything else (a `let`-bound
            // function, a nested ref) still follows through.
            if db.def_index_by_body(value).is_some() {
                Some(value)
            } else {
                callee_body(db, value)
            }
        }
        _ => None,
    }
}

/// If `id` is SYNTACTICALLY a `(fn (params…) body)` form, its param occurrences + body — WITHOUT reducing
/// or resolving (unlike [`lambda_of`], which follows refs and β-reduces). Used by `apply_lambda` to pin a
/// lambda ARGUMENT's free variables only (excluding its own params), so its own-param body references stay
/// unpinned to substitute when the arg lambda is later applied. `None` for a non-`fn` argument.
fn syntactic_lambda(db: &Db, id: StructId) -> Option<(Vec<StructId>, StructId)> {
    let tail = db.ast.as_form(id, "fn")?;
    let (&params_occ, &body) = (tail.first()?, tail.get(1)?);
    let crate::ast::Struct::List(params) = db.ast.get(params_occ) else {
        return None;
    };
    Some((params.clone(), body))
}

/// If the value at `id` reduces to a lambda, its parameters and body. Follows a `Ref` (a `let`-bound
/// function) AND reduces an `Apply` head (a function RETURNED by another application — `(adder 10)`
/// reduces to the inner `(fn (x) …)`, so `((adder 10) 5)` applies it). This is what makes curried and
/// returned functions work: the head of an application is itself evaluated to a lambda first.
fn lambda_of(db: &mut Db, id: StructId) -> Option<(std::rc::Rc<[StructId]>, StructId)> {
    match resolved_of(db, id) {
        Resolved::Lambda { params, body } => Some((params, body)),
        Resolved::Ref { value } => lambda_of(db, value),
        // A type annotation is transparent to the value — `(: (fn …) (-> A B))` is the lambda.
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => lambda_of(db, expr),
        Resolved::Apply { head, args } => {
            // Reduce the application to a value, then see if THAT is a lambda. Each reduction is a
            // β-reduction, so run it UNDER THE DEPTH GUARD: a self-referential nullary call `(def (f) (f))`
            // resolves `f` to a `Ref` at the body `(f)`, so reducing THIS application re-enters `lambda_of`
            // on the same body — an unbounded loop that would overflow the native stack. Past
            // `REDUCE_DEPTH_LIMIT` the guard denies entry and this yields `None`.
            //
            // A CURRIED SPINE `((((f a) b) c) …)` is a chain of nested `Apply`s — its `head` is itself an
            // `Apply`. Reducing it head-first RECURSIVELY (reduce `head` to a lambda via `lambda_of`, then
            // apply `args`) holds one guard ALIVE per level, so an N-deep spine nests N guards and trips
            // `REDUCE_DEPTH_LIMIT=32` at N≈32 — a LEGITIMATELY terminating program (each level consumes one
            // arg) wrongly declined CDZ0999 (breaker + v-compiler-perf, the REDUCE_DEPTH_LIMIT wall). Fix:
            // FLATTEN the spine (a structural walk, no reduction/guard) into the innermost head + its ordered
            // arg-groups, then reduce LEFT-TO-RIGHT in a LOOP where each step's guard DROPS before the next
            // — so the guard depth stays ~1 (sequential β-reductions, not a deep nest), while a genuinely
            // divergent term still trips the bound within a single step. Each intermediate `current` is a
            // reduced value (a concrete lambda / constant), so `apply_lambda(current, group)`'s internal
            // `lambda_of(current)` is O(1), never re-descending a spine.
            let mut groups: Vec<std::rc::Rc<[StructId]>> = vec![args];
            let mut inner = head;
            while let Resolved::Apply { head: h2, args: a2 } = resolved_of(db, inner) {
                groups.push(a2);
                inner = h2;
            }
            groups.reverse(); // innermost-first application order
            let mut current = inner;
            for group in groups {
                let mut guard = db.enter_reduction()?;
                let g = guard.db();
                current = apply_lambda(g, current, &group).ok().flatten()?;
                // `guard` drops here (end of iteration) → `reduce_depth` decrements before the next step,
                // keeping the spine's cumulative guard depth O(1) instead of O(spine length).
            }
            lambda_of(db, current)
        }
        // A lambda produced by a `let` BODY, capturing the let's bindings — `(let ((y 3)) (fn (x) (+ x
        // y)))`. The lambda ESCAPES the let (it is bound elsewhere and applied later), so its captured
        // bindings must be INLINED into the body BEFORE it is returned: β-reduction copies a returned
        // lambda's body and re-parents it at the application site, so a captured name left as a bare
        // reference would re-resolve against the APPLICATION site's scope (a `(let ((y 100)) …)` there
        // would wrongly shadow the captured `y=3`). Substituting the let's bindings closes the lambda
        // over their values first, so capture is by the CREATION scope (`core-semantics.md` §A Function
        // Value Captures The Bindings In Scope Where It Is Created). Runs under the depth guard (a
        // binding init could itself apply a lambda). A binding whose value is a runtime computation
        // still substitutes its (opaque) core; the corpus capture cases are all constant.
        Resolved::Let { bindings, body } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            let mut arg_of: HashMap<StructId, StructId> = HashMap::default();
            for (_name_occ, init) in &bindings {
                // Pin each binding's meaning at its definition site before the body copy re-parents it
                // (the same reason `apply_lambda` resolves each argument subtree first).
                crate::resolve::resolve_subtree(g, *init);
                // A `let`-bound reference resolves to `Ref { value: <init occurrence> }` (the binding's
                // VALUE, per `last_binder_named` returning `kv[1]`), so `beta_reduce`'s transitive-ref
                // walk keys on the INIT occurrence, not the binder name. Map it to itself: the walk
                // returns the init occurrence directly (not a fresh copy), which resolves to the same
                // captured value regardless of the scope the copied body lands in — closing the lambda
                // over the creation-site binding.
                arg_of.insert(*init, *init);
            }
            let reduced = beta_reduce(g, body, &arg_of);
            lambda_of(g, reduced)
        }
        // A lambda stored in a RECORD field and projected — `(. (record (f (fn …))) f)`. Reduce the
        // member access to the field's value, then that value to a lambda.
        Resolved::Member { operand, key } => match member_value(db, operand, &key) {
            Member::Field(v) => lambda_of(db, v),
            _ => None,
        },
        // A lambda stored in a TUPLE element and projected — `(. (tuple (fn …) 9) 0)`. Reduce the tuple
        // to its element occurrences, index the projected one, and reduce THAT to a lambda.
        Resolved::Proj { operand, index } => {
            let elems = reduce_to_tuple_elems(db, operand)?;
            let elem = *elems.get(index)?;
            lambda_of(db, elem)
        }
        // A HANDLE whose BODY is itself a CLOSURE — `((handle E init (arms…) (fn (x) …)) 10)`. The handle's
        // RESULT is that body closure (the arms only discharge performs INSIDE the body; a body that is a
        // bare lambda performs nothing, so `reduce_handle` returns it unchanged). Reduce the body to a
        // lambda so an applied handle-result closure β-reduces exactly as any other returned closure
        // (`lambda_body`/`apply_lambda` then see through the handle). Without this `((handle …) x)` declined
        // "value is not applyable" (6373, the handle-result twin of the recursive-closure-return gap).
        //
        // CRITICAL — check the BODY directly, NOT the `reduce_handle`-folded whole handle: an ESCAPING
        // CAPTURED CONTINUATION `(handle Amb 0 ((flip (u) s (fn (x) (resume x s)))) (+ 100 (Amb.flip)))`
        // ALSO folds (via `reduce_handle`) to a closure — but that closure is a captured continuation
        // escaping an ARM, the design-blocked E5 captured-k that MUST be refused. Its BODY `(+ 100
        // (Amb.flip))` is NOT a lambda, so gating on `lambda_of(body)` accepts only the closure-IS-the-body
        // case and leaves the escaping-arm case on the refuse path (it declines "not applyable" downstream).
        Resolved::Handle { init, arms, body } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            // GATE (discriminator, unchanged): applyable only when the BODY reduces to a closure. An
            // escaping continuation from an arm folds to a closure whose RAW body is not lambda-shaped, so
            // this gate leaves that design-blocked case on the refuse path.
            lambda_of(g, body)?;
            // DISCHARGE the handler over the body BEFORE closing the closure, so a captured perform RESULT
            // closes over its inner-handled VALUE, not the raw perform. `(handle Ctr 50 (arm) (let ((base
            // (Ctr.tick))) (fn (x) (+ x base))))` — the naive `lambda_of(body)` recurses into the `let` arm
            // and closes the closure over `base := (Ctr.tick)` (the RAW perform), so applying it under an
            // OUTER Ctr handler re-performs the tick (→ 8 not 53) / CDZ0401s with no outer handler.
            // `reduce_handle` folds the capture to its value → `(let ((base 50)) (fn (x) (+ x base)))`,
            // reparented under the handle site. RE-RESOLVE it so `base` inside the closure re-binds to the
            // folded `50` occurrence (the synthesized node was built by `push_list`, unresolved), then close
            // the closure via the `Let` arm over that value. If the fold declines, keep the old behavior
            // (fall back to the raw body-lambda) so no previously-working shape regresses.
            match crate::effects::reduce_handle(g, init, &arms, body, false) {
                Some(folded) => {
                    crate::resolve::resolve_subtree(g, folded);
                    lambda_of(g, folded)
                }
                None => lambda_of(g, body),
            }
        }
        _ => None,
    }
}

/// The `(meta apply)` primitive of the head value at `id`, if the head is applyable — project the
/// `apply` field (in the `meta` namespace) and, following a ref, read the `Prim` it holds. `None`
/// means "not applyable" (no `(meta apply)`, or it is not a primitive). This is the one dispatch step
/// every application takes.
pub fn meta_apply_of(db: &mut Db, id: StructId) -> Option<Prim> {
    let field = project_meta(db, id, "apply")?;
    prim_of(db, field)
}

/// The DISCRIMINANT of the sum variant the value at `id` is a constructor for, if it is one — read off
/// the record's `(meta variant)` channel (an integer literal the sum synthesis wrote). `None` for any
/// value that is not a variant constructor (no `(meta variant)`). This is what a variant application
/// reads at lowering to know which variant it builds (the analogue of a conversion reading its target
/// width off the solved type) — the discriminant lives in the metadata, not in the shared `SumNew`
/// prim. It also MARKS a record as a variant value/constructor, distinguishing a nullary variant
/// (`None` — a value) from a genuine type-value record (both carry `(meta t)`; only a variant carries
/// `(meta variant)`).
pub fn variant_disc_of(db: &mut Db, id: StructId) -> Option<u32> {
    let field = project_meta(db, id, "variant")?;
    // Read the discriminant through the BORROW companion `resolved_ref`: `IntValue::to_i64` takes
    // `&self`, so the by-value `resolved_of` clone of the whole `Resolved` (whose `Int` payload is a
    // heap `IntValue` magnitude) per call was pure churn. `variant_disc_of` is a HOT meta reader (~45
    // call sites across lower/infer/compile/eval — every variant construction / disc-switch reads it),
    // so borrow-dispatch is the fix-35/36 pattern (a memoized query returning a compound by value cloned
    // per call, read only for a borrowed field → borrow). Sibling of the `prim_of` borrow.
    match crate::resolve::resolved_ref(db, field) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok()),
        _ => None,
    }
}

/// The effect-operation IDENTITY of the value at `id`, if it is an effect operation — the declaring
/// effect's declaration occurrence and the operation's index, read off the `(meta effect-op)` channel
/// (an `(effect-op <decl> <index>)` node the effect synthesis wrote, `crate::effects`). `None` for any
/// value that is not an effect operation (no `(meta effect-op)`). This is what a PERFORM reads to know
/// WHICH operation is performed — the analogue of `variant_disc_of` for a variant constructor. It also
/// MARKS a value as an operation (distinguishing `E.op` from an ordinary member value).
pub fn effect_op_of(db: &mut Db, id: StructId) -> Option<(crate::ast::StructId, u32)> {
    let field = project_meta(db, id, "effect-op")?;
    // `(effect-op <decl> <index>)` — decl and index are decimal integer literals.
    let tail = db.ast.as_form(field, "effect-op")?;
    let decl_occ = tail.first().copied()?;
    let index_occ = tail.get(1).copied()?;
    // Borrow-dispatch (as in `variant_disc_of`): `to_i64` borrows, so `resolved_ref` avoids cloning the
    // `Int` payload's heap `IntValue` for each of the two literals read here.
    let decl = match crate::resolve::resolved_ref(db, decl_occ) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok())?,
        _ => return None,
    };
    let index = match crate::resolve::resolved_ref(db, index_occ) {
        Resolved::Int(v) => v.to_i64().and_then(|n| u32::try_from(n).ok())?,
        _ => return None,
    };
    Some((crate::ast::StructId(decl), index))
}

/// The PAYLOAD type of the sum variant the value at `id` constructs — for a variant constructor
/// `(. Sum V)` whose `(meta t)` is `(-> payload Sum)`, the arrow's parameter `payload`. Read via the
/// ordinary `(meta t)` scheme, so it works for any single-payload variant. A NULLARY variant's type is
/// the bare sum (no arrow) → `None` (no payload to bind). A MULTI-payload variant's type is a curried
/// `(-> p0 (-> p1 Sum))` → the FIRST parameter here (Stage-5 binds a single scalar payload; a
/// multi-payload destructure is a later increment, so only the one-payload arrow is meaningful).
pub fn variant_payload_type(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    let mut fresh = Fresh::new();
    let scheme = scheme_of(db, id, &mut fresh)?;
    match scheme.ty {
        Ty::Fn(param, _) => Some(*param),
        _ => None,
    }
}

/// The number of PAYLOAD arguments the variant constructor at `id` takes — the length of its curried
/// arrow chain. A nullary variant's scheme is the bare `Sum` (arity 0); a single-payload variant is
/// `(-> p Sum)` (arity 1); a multi-payload variant curries `(-> p0 (-> p1 Sum))` (arity 2), etc. `None`
/// if `id` is not a variant constructor (no scheme, or a non-variant record). This is the arity a
/// CURRIED application spine `((V a) b)` must reach to build the variant — the same count `lower_sum_new`
/// boxes as payloads. (Peels arrows the way `variant_owner_decl` does, counting rather than discarding.)
pub fn variant_payload_arity(db: &mut Db, id: StructId) -> Option<usize> {
    variant_disc_of(db, id)?;
    let mut fresh = Fresh::new();
    let scheme = scheme_of(db, id, &mut fresh)?;
    let mut ty = scheme.ty;
    let mut arity = 0;
    while let Ty::Fn(_, r) = ty {
        arity += 1;
        ty = *r;
    }
    Some(arity)
}

/// The DECLARATION OCCURRENCE of the sum type the variant constructor at `id` belongs to — the identity
/// by which sum types are compared (`ty.rs` §Two sums are the SAME type iff their `decl` and `args`
/// agree). A variant constructor's `(meta t)` scheme is `(-> payload Sum)` for a payload variant or the
/// bare `Sum` for a nullary one; either way the OWNING sum is the constructor's RESULT type, so read the
/// arrow's result (or the bare type) and return its `Ty::Sum { decl }`. `None` if `id` is not a variant
/// constructor or its result is not a sum. This is what a sum-MATCH uses to verify each arm's pattern
/// constructor belongs to the SCRUTINEE's sum (a `Some` pattern over a `T` scrutinee is a type error),
/// the pattern analogue of the `(meta variant)` discriminant [`variant_disc_of`] reads.
pub fn variant_owner_decl(db: &mut Db, id: StructId) -> Option<crate::ast::StructId> {
    let mut fresh = Fresh::new();
    let scheme = scheme_of(db, id, &mut fresh)?;
    // Peel EVERY arrow to the final result: a single-payload variant `(-> payload Sum)` → `Sum`; a
    // MULTI-payload variant curries `(-> p0 (-> p1 Sum))` → `Sum` (peel both); a nullary variant's scheme
    // is the bare `Sum` (no arrow). The owning sum is always the constructor's ultimate RESULT type.
    let mut result = scheme.ty;
    while let Ty::Fn(_, r) = result {
        result = *r;
    }
    match result {
        Ty::Sum { decl, .. } => Some(decl),
        // A NEWTYPE constructor's result is `Ty::Nominal` (the erased single-variant sum), whose `decl`
        // is the same declaration-occurrence identity — so a `(Mk …)` pattern resolves its owning newtype
        // exactly as a boxed-sum variant resolves its sum.
        Ty::Nominal { decl, .. } => Some(decl),
        _ => None,
    }
}

/// Project a meta-channel field named `key` from the record value at `id`, following a `Ref` to the
/// record. Returns the field's value occurrence, or `None` if the value is not a record or has no such
/// field. The one generic projection, restricted to the `meta` namespace.
///
/// Reduces to the record then SCANS its fields comparing `namespace`/`name` by `&str` — it does NOT
/// build a `Symbol` key. This is the SAME lookup a `BTreeMap::get(&Symbol{"meta", key})` performs (a
/// meta record carries a handful of fields, so the linear scan is as cheap as the log-n probe), but
/// allocating a two-`String` `Symbol` on EVERY call was pure churn: `meta_apply_of` runs for every
/// operator application, and every inline re-projects `(meta apply)` — on a deep inline chain that
/// `Symbol` alloc/free was a top allocation source in the profile. Behaviour-identical (same fields,
/// same match); only the transient key object is gone.
// `_depth` is a `#[cfg(test)]`-only scan-depth probe (feeds `PROJECT_META_FIELDS_VISITED` for the
// zero-user-field-scan perf test); folding it into the loop iterator (clippy's `explicit_counter_loop`
// suggestion) would put the counter on the release path too, so keep the manual test-gated counter.
#[allow(clippy::explicit_counter_loop)]
pub fn project_meta(db: &mut Db, id: StructId, key: &str) -> Option<StructId> {
    let rec = reduce_to_record_id(db, id)?;
    // `reduce_to_record_id` has already resolved `rec` (it dispatches on its `resolved_of`), so the
    // resolved column is FILLED — BORROW it rather than `resolved_of(db, rec)` again, which would CLONE
    // the whole `Resolved` (for a `Resolved::Record`, an `Rc<BTreeMap>` refcount bump + drop = two refcount
    // ops per call). `project_meta` is on the hot per-node path (`meta_apply_of`/`variant_disc_of` during
    // `collect`/`type_errors` — ~42% inclusive on a realistic module), so avoiding the per-call clone is a
    // broad win. `Column::get` returns a borrow, so the fields map is read in place.
    if let crate::arena::Slot::Filled(crate::resolved::Resolved::Record { fields }) =
        db.resolved.get(rec)
    {
        // REVERSE scan the `meta`-namespace field block by `&str` — do NOT build a `Symbol` key. A
        // `BTreeMap::get(&Symbol{Some("meta"), key})` allocates TWO `String`s (the namespace + name) on
        // EVERY call, and `project_meta` is on the hot per-node meta-dispatch path (`meta_apply_of`/
        // `variant_disc_of` run for every application/pattern during `collect`/`type_errors`) — that
        // transient key alloc/free was a top allocation source (a realistic module A/B'd ~1.13× faster with
        // it gone; `btree::search` + `malloc`/`free` churn dropped). Field keys sort by `(namespace, name)`
        // with `None` (a USER field) < `Some("meta")`, so the `meta` fields are a CONTIGUOUS block at the
        // TOP of the map — a REVERSE walk reaches them in O(meta-fields), NEVER touching the O(N) user
        // fields (unlike a forward scan, which `203f8588` rightly avoided for a wide RUNTIME record). A meta
        // record carries only a handful of meta fields, so this is O(1)-ish AND zero-alloc. Skip any
        // namespace ABOVE `meta` (a future namespace, sorted later), match within `meta`, and STOP the
        // moment the walk descends below `Some("meta")` (into user/`None` or a smaller namespace) — no meta
        // field can be below that, so we never scan the wide user block. Behaviour-identical to the built-key
        // `get` (same field, same match), only the transient `Symbol` is gone.
        #[cfg(test)]
        let mut _depth: u64 = 0;
        for (k, &v) in fields.iter().rev() {
            #[cfg(test)]
            {
                _depth += 1;
                crate::db::PROJECT_META_FIELDS_VISITED.with(|c| {
                    if _depth > c.get() {
                        c.set(_depth)
                    }
                });
            }
            match k.namespace.as_deref() {
                // A matching `meta` field — done.
                Some("meta") if &*k.name == key => return Some(v),
                // A `meta` field with a different name — keep scanning the meta block.
                Some("meta") => {}
                // Below the `meta` block (a `None` user field or a namespace < "meta") — no meta field
                // remains further down, so stop rather than walk the O(N) user fields.
                ns if ns < Some("meta") => break,
                // A namespace ABOVE "meta" (sorted after it) — not yet at the meta block, keep descending.
                _ => {}
            }
        }
        return None;
    }
    None
}

/// Project a field `key` from the record value at `id`, returning its value occurrence. The generic
/// record projection both member access and the meta channel use. It REDUCES the operand to a record
/// first: following a `Ref`, and reducing a type-constructor application (so `(. (Int 64) max)`
/// projects `max` off the module `(Int 64)` builds). `None` if the operand is not (reducible to) a
/// record, or has no such field.
pub fn project_field(db: &mut Db, id: StructId, key: &Symbol) -> Option<StructId> {
    let rec = reduce_to_record_id(db, id)?;
    match resolved_of(db, rec) {
        Resolved::Record { fields } => fields.get(key).copied(),
        _ => None,
    }
}

/// Reduce the value at `id` to the occurrence of a RECORD, if it reduces to one: following a `Ref`,
/// reducing a type-constructor application (`(Int 64)` → its module), and following a member
/// projection (a nested record). Returns the record's occurrence, or `None` if it does not reduce to a
/// record. This is the single "reduce to a record" step both projection and its checks use.
pub(crate) fn reduce_to_record_id(db: &mut Db, id: StructId) -> Option<StructId> {
    // The two hot cases — a `Record` (return `id`; fields unused) and a `Ref` (recurse on the Copy
    // `value`) — need no owned payload, so dispatch them through `resolved_ref` (a BORROW, no clone).
    // `reduce_to_record_id` is the recursion behind `project_meta` (the hottest reducer) and most steps
    // are exactly these, so the per-step `resolved_of` clone was ~5% of a realistic compile. `Apply`/
    // `Member` cross a `&mut db` boundary (β-reduction / `member_value`), so they fall to the owned path.
    match crate::resolve::resolved_ref(db, id) {
        Resolved::Record { .. } => return Some(id),
        Resolved::Ref { value } => {
            let value = *value;
            // A self-referential value def — `(def (g) g)`, or a mutual `(def (a) b) (def (b) a)` — resolves
            // to a `Ref` chain that cycles (a node's `Ref` returns to itself or a prior node), so following
            // it unguarded LOOPS FOREVER (the fast borrow path added no bound; the pre-existing owned path
            // relied on an upstream reduction guard this short-circuit skips). Route the Ref hop through the
            // STRUCTURAL depth guard: a `Ref` dereference is an O(1) hop (NOT a β-reduction), so it needs
            // cycle TERMINATION (`REDUCE_DEPTH_LIMIT`) but must NOT charge the cumulative `REDUCE_NODE_BUDGET`
            // — else a large-but-terminating multi-module closure build that exhausts the budget starts
            // denying this trivial hop, so `Option.None`'s `Ref{Option}→record` fails and a well-typed
            // program mis-faults "a Option value has no field `None`" at its component build (the compiler-ml
            // self-host emit-db collision + the sread-eval scaling CDZ0201s — same cumulative-budget root).
            // Past `REDUCE_DEPTH_LIMIT` this still yields `None` so a value cycle bottoms out instead of
            // hanging; `collect_faults` reports the cycle as CDZ0201 (`resolve::value_ref_cycle`).
            let mut guard = db.enter_reduction_structural()?;
            return reduce_to_record_id(guard.db(), value);
        }
        _ => {}
    }
    match resolved_of(db, id) {
        // The `Record`/`Ref` cases are handled above via the borrow; a hit here would be harmless.
        Resolved::Record { .. } => Some(id),
        Resolved::Ref { value } => reduce_to_record_id(db, value),
        Resolved::Apply { head, args } => {
            // Reducing the application to a record is a β-reduction — run it UNDER THE DEPTH GUARD. A
            // self-referential nullary call `(def (f) (f))` resolves `f` to a `Ref` at the body `(f)`,
            // and `meta_apply_of(head)` → `project_field` → `reduce_to_record_id` re-enters this same
            // application, an unbounded loop that would overflow the native stack. Past
            // `REDUCE_DEPTH_LIMIT` the guard denies entry and this yields `None` (does not reduce to a
            // record), matching how a too-deep β-reduction declines elsewhere.
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            // A USER-DEF / LAMBDA head β-reduces — a field read of a call-RETURNED record
            // `(. (mk a b) field)` folds through to the field's value, with NO heap build (the record
            // analogue of the tuple case above). A recursive callee declines here and the projection
            // stays a runtime `Core::Proj`/record read.
            if lambda_body(g, head).is_some() {
                return match apply_lambda(g, head, &args) {
                    Ok(Some(reduced)) => reduce_to_record_id(g, reduced),
                    _ => None,
                };
            }
            // A NEWTYPE constructor `(UserId.Mk rec)` is ERASED — the value IS its single payload (no box),
            // so a field read `(. (UserId.Mk rec) x)` sees THROUGH the tag to the payload record. Reduce
            // the sole payload argument. (Only the single-payload case reduces to a record; a multi-payload
            // newtype's payload is a tuple, reached by tuple projection, not member access.) Gate on the
            // CHEAP `variant_disc_of` (a `(meta variant)` field read) FIRST — a non-ctor head (an ordinary
            // recursive call `(f)`) returns `None` here without the deep `scheme_of` reduction
            // `variant_owner_decl` runs, so an unproductive recursion still declines rather than looping.
            if args.len() == 1
                && variant_disc_of(g, head).is_some()
                && let Some(decl) = variant_owner_decl(g, head)
                && g.newtype_inner.contains_key(&decl)
            {
                return reduce_to_record_id(g, args[0]);
            }
            let prim = meta_apply_of(g, head)?;
            if prim.is_arith() {
                return None;
            }
            let built = reduce_ctor(g, prim, id, &args).ok()?;
            reduce_to_record_id(g, built)
        }
        // A member projection whose result is itself a record (a nested record chain).
        Resolved::Member { operand, key } => {
            let value = match member_value(db, operand, &key) {
                Member::Field(v) => v,
                _ => return None,
            };
            reduce_to_record_id(db, value)
        }
        _ => None,
    }
}

/// The outcome of a member projection over a reducible operand: the field's value occurrence, or a
/// precise fault (a non-record operand, or an absent field on a closed record).
pub enum Member {
    /// The field's value occurrence — project succeeded.
    Field(StructId),
    /// The operand is not a record (member access requires one) → CDZ0201.
    NotRecord,
    /// The operand is a record but has no field `key` (a closed record rejects) → CDZ0201.
    NoField,
}

/// Resolve a member access `(. operand key)` to its field's value occurrence, reducing the operand to
/// a record. Shared by `infer` (types the field's value) and `lower` (lowers it) so both agree on the
/// one projection. If the operand reduces to a record, the field is projected (`Field`) or reported
/// absent (`NoField`); otherwise `NotRecord`.
pub fn member_value(db: &mut Db, operand: StructId, key: &Symbol) -> Member {
    match reduce_to_record_id(db, operand) {
        Some(rec) => match resolved_of(db, rec) {
            Resolved::Record { fields } => match fields.get(key) {
                Some(&v) => {
                    trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, value = v.0, "project: field found");
                    Member::Field(v)
                }
                None => {
                    trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, "project: no such field");
                    Member::NoField
                }
            },
            _ => Member::NotRecord,
        },
        None => {
            trace!(target: "rcdzc::eval", operand = operand.0, key = %key.name, "project: operand is not a record");
            Member::NotRecord
        }
    }
}

/// The field NAMES a member-access operand offers — the candidate set a "no field `x`; did you mean
/// `y`?" suggestion draws from (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
/// Fix). Reduces the operand exactly as [`member_value`] does: if it reduces to a compile-time-visible
/// record, its field names; otherwise the operand's record TYPE's field names (a runtime record — a
/// call result / `if` selection). Empty when the operand is not a record at all (a non-record operand
/// is a different fault, "member access requires a record", with no field to suggest).
///
/// META-CHANNEL fields (`(meta t)`/`(meta apply)`/`(meta variant)`/…, keyed in the `"meta"` namespace)
/// are EXCLUDED: they are the compiler's internal type/apply/discriminant channels on a MODULE record
/// (a prelude sum module like `Option` carries a `(meta t)`), never a user-facing member a source
/// program projects by name. Including them polluted the suggestion list — `(Option.Ok 5)` offered
/// "closest matches: `t`, `None`, `Some`", the internal `t` a baseless variant suggestion. Filtering by
/// namespace is the principled cut (a meta field is `Symbol { namespace: Some("meta"), … }`), so no
/// name-string knowledge lives here.
pub fn record_field_names(db: &mut Db, operand: StructId) -> Vec<std::sync::Arc<str>> {
    // Field names come off `Symbol`'s `Arc<str>` key; the sole consumer feeds them to the `AsRef<str>`
    // suggestion helpers (`suggest::nearest`/`did_you_mean`), so carry the `Arc<str>` (a refcount bump)
    // rather than materializing a fresh `String` per field.
    if let Some(rec) = reduce_to_record_id(db, operand)
        && let Resolved::Record { fields } = resolved_of(db, rec)
    {
        return fields
            .keys()
            .filter(|k| k.namespace.as_deref() != Some("meta"))
            .map(|k| k.name.clone())
            .collect();
    }
    match crate::infer::type_of(db, operand) {
        crate::ty::Ty::Record(fields) => fields
            .keys()
            .filter(|k| k.namespace.as_deref() != Some("meta"))
            .map(|k| k.name.clone())
            .collect(),
        _ => Vec::new(),
    }
}

/// The sorted-position index of field `key` in a RUNTIME record operand — one whose value does not
/// reduce to a compile-time-visible record (returned from a call, selected by an `if`) but whose TYPE
/// is a record carrying the field. `None` if the operand is not a record type, or is a record type
/// lacking the field. At run time a record IS a positional heap array in sorted-key order — the
/// `Ty::Record`/`Core::Record` `BTreeMap` order the backend's `arr-set`/`arr-get` share — so a field
/// read on such an operand lowers to a `Core::Proj` at this index, the same `arr-get` a tuple
/// projection uses. The symmetric counterpart of a tuple projection falling through to a runtime read.
pub fn runtime_member_index(db: &mut Db, operand: StructId, key: &Symbol) -> Option<usize> {
    // A NOMINAL newtype over a record IS that record handle at run time (the tag is erased), so a field
    // read on a runtime nominal operand indexes the inner record's sorted position — strip the tag.
    let operand_ty = crate::infer::type_of(db, operand);
    let crate::ty::Ty::Record(fields) = operand_ty.strip_nominal() else {
        return None;
    };
    // The sorted slot of `key` was a LINEAR `fields.keys().position(…)` scan — O(fields) PER projection, so
    // a wide record projected field-by-field was O(N²). The field order is a pure function of the record
    // type, and every projection of the same record shares its type `Rc`, so build the whole
    // `name → sorted-slot` map ONCE per record type (keyed by the `Rc`'s address) and read it O(1). The
    // `guard` `Rc` pins the allocation so its address can't be reused while cached (ABA safety).
    let ptr = std::rc::Rc::as_ptr(fields) as usize;
    let entry = db.record_field_index.entry(ptr).or_insert_with(|| {
        #[cfg(test)]
        crate::db::RECORD_FIELD_INDEX_KEYS_SCANNED.with(|c| c.set(c.get() + fields.len() as u64));
        let index = fields
            .keys()
            .enumerate()
            .map(|(i, k)| (k.clone(), i))
            .collect();
        crate::db::RecordFieldIndex {
            guard: std::rc::Rc::clone(fields),
            index,
        }
    });
    entry.index.get(key).copied()
}

/// Reduce the value at `id` to the element occurrences of a COMPILE-TIME-VISIBLE tuple, if it reduces
/// to one: a `(tuple …)` literal directly, or following a `Ref` to one. Returns the ordered element
/// occurrences. `None` if the operand is not (statically reducible to) a tuple — a runtime tuple (a
/// function parameter, a `Core::Let`-kept binding) is opaque here, so its projection stays a runtime
/// `Core::Proj` rather than folding. This is the tuple analogue of `reduce_to_record_id`, and it is what
/// makes a projection of a visible tuple fold (no heap) while a projection of a runtime tuple does not.
///
/// A `Ref` to a binding that was KEPT as a runtime `Core::Let` is NOT followed (the value lives in a
/// slot at run time, not visibly here) — so a multi-use `let`-bound tuple projects at run time. A
/// single-use / propagated binding's ref IS followed (it inlines to the tuple literal, which folds).
pub fn reduce_to_tuple_elems(db: &mut Db, id: StructId) -> Option<std::rc::Rc<[StructId]>> {
    match resolved_of(db, id) {
        Resolved::Tuple { elems } => {
            // A CONSTRUCTION-SPREAD tuple `#tuple(a (.. t) b)` — its LOGICAL elements are `t`'s elements
            // FLATTENED in (DESIGN §6), so a static projection `(. u k)` indexes the flattened sequence, not
            // the raw `(.. )`-bearing child list. Inline each spread operand's own (constant-reducible)
            // elements; if a spread operand is RUNTIME (`reduce_to_tuple_elems` → None), the whole tuple is
            // not compile-time-visible, so decline the fold (`?`) and the projection lowers to a runtime
            // `Core::Proj` over the flattened `Core::Tuple` `lower_tuple_spread` builds.
            if elems.iter().any(|&e| db.ast.spread_operand(e).is_some()) {
                let mut out: Vec<StructId> = Vec::with_capacity(elems.len());
                for &e in elems.iter() {
                    if let Some(op) = db.ast.spread_operand(e) {
                        let inner = reduce_to_tuple_elems(db, op)?;
                        out.extend(inner.iter().copied());
                    } else {
                        out.push(e);
                    }
                }
                Some(out.into())
            } else {
                Some(elems)
            }
        }
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                // A kept multi-use runtime binding — opaque; its projection is a runtime read.
                None
            } else {
                reduce_to_tuple_elems(db, value)
            }
        }
        // A tuple that is itself an element projected from another compile-time-visible tuple.
        Resolved::Proj { operand, index } => {
            let elems = reduce_to_tuple_elems(db, operand)?;
            let elem = *elems.get(index)?;
            reduce_to_tuple_elems(db, elem)
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            reduce_to_tuple_elems(db, expr)
        }
        // A `(tuple a b)` written with the shadowable alias name — reduce the application to the
        // symbol-headed primitive (`reduce_ctor` pins the arg scopes, then builds `((,) a b)`) and read
        // its elements, so a constant tuple built via the alias folds exactly as one written `((,) a b)`
        // does. Mirrors `reduce_to_record_id`'s `Apply` arm; run under the depth guard.
        Resolved::Apply { head, args } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            // A USER-DEF / LAMBDA head β-reduces — inlining the (pure) call — so a projection of a
            // call-RETURNED tuple `(. (mk a b) i)` folds straight through to the element `a`/`b`, with
            // NO heap build. This mirrors the lambda branch `core_of`'s `Apply` arm takes when lowering
            // the whole call, so the keep-decision (a projection-only compound folds through) and the
            // fold agree: without this the call reduces to a `Core::Tuple` at lowering but the fold
            // could not see through it, so each projection re-lowered the body and REBUILT the tuple
            // (one `arr-alloc` per projection). A RECURSIVE callee declines here (`apply_lambda`'s
            // `is_recursive` guard) and the projection stays a runtime `Core::Proj` (the heap read).
            if lambda_body(g, head).is_some() {
                return match apply_lambda(g, head, &args) {
                    Ok(Some(reduced)) => reduce_to_tuple_elems(g, reduced),
                    _ => None,
                };
            }
            let prim = meta_apply_of(g, head)?;
            if prim.is_arith() {
                return None;
            }
            let built = reduce_ctor(g, prim, id, &args).ok()?;
            reduce_to_tuple_elems(g, built)
        }
        _ => None,
    }
}

/// The condition occurrence and the two branches' element occurrences of an `if` whose branches both
/// reduce to same-arity visible tuples — the result of [`reduce_to_if_of_tuples`].
type IfOfTuples = (StructId, std::rc::Rc<[StructId]>, std::rc::Rc<[StructId]>);

/// If `id` reduces (through refs/annotations) to an `(if cond then else)`, return its three child
/// occurrences `(cond, then_, else_)`. This is the substrate for pushing a CONSUMER (a projection, a
/// member read) INTO an `if`'s branches: `(consume (if c T E))` → `(if c (consume T) (consume E))`,
/// reusing the EXISTING branch occurrences (NO ast synthesis, NO re-resolution — each keeps the scope
/// it was resolved in). A KEPT multi-use binding is OPAQUE (its value lives in a slot at run time, read
/// back — folding through it would duplicate the computation), so a `Ref` to one stops the reduction;
/// a single-use / propagated `Ref` and an `Annot` are followed. Mirrors `reduce_to_tuple_elems`'s
/// ref/annot/kept handling.
/// If `id` reduces to a runtime `match`, return its AST occurrence — the `(match scrutinee (pat body)…)`
/// list — so the caller can push a surrounding operation into each arm body (the match analogue of
/// [`reduce_to_if`]'s case-of-case). Returns the MATCH FORM's occurrence (not the destructured parts),
/// because a rewrite must rebuild the arms with their PATTERN nodes intact (a pattern's binders are in
/// scope for its rewritten body). Follows a `Ref`/annotation/β-reducible-call to the match, exactly as
/// `reduce_to_if` does, so `((classify c) 5)` — a call returning a match of closures — is reached too.
/// `None` for a kept (multi-use) binding, or anything that is not a match.
pub fn reduce_to_match(db: &mut Db, id: StructId) -> Option<StructId> {
    match resolved_of(db, id) {
        Resolved::Match { .. } => Some(id),
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                None
            } else {
                reduce_to_match(db, value)
            }
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => reduce_to_match(db, expr),
        Resolved::Apply { head, args } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            let reduced = apply_lambda(g, head, &args).ok().flatten()?;
            reduce_to_match(g, reduced)
        }
        _ => None,
    }
}

/// Like [`reduce_to_match`] but STOPS at an application — it follows only `Ref`/`Annot` to a
/// SYNTACTICALLY-PRESENT `(match …)` form, never β-reducing a `Resolved::Apply` callee. Used by the
/// case-of-match FUSION (`fuse_match_into_match`), whose win case is always a match written directly in the
/// scrutinee position: β-reducing a CALL scrutinee (`(step s)`) would disturb a const-closure /
/// specialization-sensitive callee in a still-generic driver body (a spurious CDZ0201). A `kept` (multi-use)
/// binding stops here too, as in `reduce_to_match`.
pub fn reduce_to_match_direct(db: &mut Db, id: StructId) -> Option<StructId> {
    match resolved_of(db, id) {
        Resolved::Match { .. } => Some(id),
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                None
            } else {
                reduce_to_match_direct(db, value)
            }
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            reduce_to_match_direct(db, expr)
        }
        _ => None,
    }
}

/// Peel `id` to a VISIBLE variant constructor — a `(Ctor payload…)` application OR a BARE nullary
/// constructor — returning `(disc, head, args)`. A payload-carrying ctor is a `Resolved::Apply` whose head
/// names a variant (`variant_disc_of` is `Some`); a NULLARY ctor written without an explicit `()` (e.g. a
/// bare `PQ.PQNil`) resolves to a `Resolved::Member` that itself carries the `(meta variant)` disc and
/// takes NO payload, so its `args` is empty. (The `(PQNil ())` applied spelling instead resolves to
/// `Apply{args:[unit]}` — a single unit arg — and is handled by the `Apply` arm; both spellings of a
/// nullary ctor thus reduce, one with an empty arg list, one with a unit arg an `_` binder ignores.)
/// Follows `Ref` (stopping at a kept multi-use binding) and `Annot`, exactly as `reduce_to_match`/
/// `reduce_to_if` do. Does NOT β-reduce a call head — the caller (`fold_ctor_match`) wants a scrutinee
/// that is ALREADY a visible ctor (v-effects β-reduces its stored-thunk compound into the scrutinee before
/// calling), so a call scrutinee stays a runtime match. `None` if not a visible ctor.
fn reduce_to_visible_ctor(db: &mut Db, id: StructId) -> Option<(u32, StructId, Vec<StructId>)> {
    match resolved_of(db, id) {
        Resolved::Apply { head, args } => {
            let disc = variant_disc_of(db, head)?;
            Some((disc, head, args.to_vec()))
        }
        // A BARE nullary variant constructor — `(match PQNil …)`, no explicit `()`. It resolves to a
        // `Member` (not an `Apply`) that carries the variant disc directly and has no payload, so `args` is
        // empty. This is the shape v-effects' recursive-callee-base-arm unfold produces after substituting
        // the base ctor into `q` (`(match PQNil ((PQNil _) …) …)`); without this arm it declined.
        Resolved::Member { .. } => {
            let disc = variant_disc_of(db, id)?;
            Some((disc, id, Vec::new()))
        }
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                None
            } else {
                reduce_to_visible_ctor(db, value)
            }
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            reduce_to_visible_ctor(db, expr)
        }
        _ => None,
    }
}

/// CASE-OF-KNOWN-CTOR fold at eval/classification time — the resolve-time analogue of the lowering's
/// direct-scrutinee `SumNew` collapse (`lower_match_sum` / the `Resolved::SumPayload` fold). Given a
/// `(match S ((Ctor v) body) …)` whose scrutinee `S` peels to a VISIBLE ctor `(Ctor payload)`, select the
/// arm whose constructor matches `S`'s discriminant and return that arm's BODY with the arm's payload
/// binder substituted by the payload — so `(match (Box.Box th) ((Box.Box t) (t unit)))` folds to `(th
/// unit)`. `None` when it is not a match over a visible ctor, or the matching arm is not a plain
/// single-binder `(Ctor v)` (a guarded / literal / nested / wildcard / multi-payload pattern), or no arm
/// matches — the caller leaves those a runtime match.
///
/// Exposed for v-effects' DES-inc-4 fold-classifier (a stored resume-thunk in a compound, match-extracted
/// before apply): its arm-body reduction pass calls this between two β-reduces so the thunk surfaces for
/// apply. The substitution reuses `beta_reduce` keyed on the SCRUTINEE occurrence → the visible ctor: the
/// arm's binder `v` resolves to `SumPayload{scrutinee: S, [Payload]}` (NOT `Ref{v}`), and `beta_reduce`'s
/// SumPayload capture-share exception (`scrutinee_is_substituted`) re-resolves it against the substituted
/// scrutinee, where the `SumPayload` fold projects the `[Payload]` step to the ctor's payload. SLICE 1:
/// single-payload single-binder `(Ctor v)` only; a multi-payload / nested pattern declines to `None` (a
/// later increment). No Core clone — a resolve-time β-reduction on the AST, pre-lower.
pub fn fold_ctor_match(db: &mut Db, node: StructId) -> Option<StructId> {
    let Resolved::Match { scrutinee, arms } = resolved_of(db, node) else {
        return None;
    };
    // The scrutinee must be a VISIBLE ctor — a payload-carrying `(Ctor payload)` OR a bare nullary ctor
    // (empty `args`) — else leave it a runtime match.
    let (disc, _head, args) = reduce_to_visible_ctor(db, scrutinee)?;
    // This fold handles a NULLARY ctor (`args.is_empty()`, slice 0) or a SINGLE-payload ctor
    // (`args.len() == 1`, slices 1/2 — the one payload may itself be a TUPLE destructured by a tuple
    // sub-pattern). A genuine MULTI-arg application declines to `None` (left a runtime match).
    if args.len() > 1 {
        return None;
    }
    // Find the arm whose pattern matches the scrutinee's discriminant and is one this fold handles.
    for (pat, body) in &arms {
        // A guarded arm (`(guard <pat> <cond>)`) is refutable at runtime — its selection depends on the
        // GUARD, which this static fold cannot decide. Skipping it and folding to a LATER arm is only sound
        // if the guarded arm CANNOT match this scrutinee (a different discriminant). But if the guarded arm's
        // inner pattern has the SAME discriminant as the scrutinee, it MIGHT admit at runtime — folding past
        // it to a later same-ctor sibling (`(guard (Wrap v) g) … (Wrap _v) …)` over `(Wrap 5)`) DISCARDS the
        // guard's admit path, so both the guard-hit and the fallback collapse to the sibling body (breaker
        // FINDING #15: a ctor-pattern guard with a same-ctor non-guarded sibling ran the fallback 0 for BOTH
        // multi-dispatch dispatches, where the first should admit → 50). DECLINE the whole fold (leave a
        // runtime match, which evaluates the guard correctly) when a skipped guarded arm's discriminant
        // matches the scrutinee. A guarded arm of a DIFFERENT ctor (or an unresolvable pattern) cannot match,
        // so it is safe to skip and keep scanning (a wildcard-fallback shape has no same-disc guarded arm to
        // block on — the user workaround breaker noted).
        if let Some(inner_pat) = db
            .ast
            .as_form(*pat, "guard")
            .and_then(|g| g.first().copied())
        {
            // The guarded arm's inner pattern head, if it's a `(ctor sub)` shape.
            let guard_ctor_head = match db.ast.get(inner_pat) {
                crate::ast::Struct::List(gkids) if gkids.len() == 2 => Some(gkids[0]),
                _ => None,
            };
            if let Some(head) = guard_ctor_head
                && variant_disc_of(db, head) == Some(disc)
            {
                return None; // an undecided same-ctor guard shadows any later arm — not statically foldable
            }
            continue;
        }
        // The pattern must be a 2-element list `(ctor_head sub_pattern)`: the head names the variant, the
        // second child is the payload sub-pattern (or, for a nullary ctor, a `_` wildcard binding nothing).
        let crate::ast::Struct::List(kids) = db.ast.get(*pat) else {
            continue;
        };
        if kids.len() != 2 {
            continue;
        }
        let (pat_head, binder) = (kids[0], kids[1]);
        // The pattern's ctor must match the scrutinee's discriminant.
        if variant_disc_of(db, pat_head) != Some(disc) {
            continue;
        }
        // SLICE 0: a NULLARY ctor — the scrutinee has no payload, so the arm's `_`/binder binds nothing and
        // no `SumPayload` over this scrutinee appears in the body. Return a structural COPY of the body (no
        // substitution): passing `scrutinee` as the never-matched payload placeholder leaves every node
        // intact while still rebuilding it hygienically (a copied name re-resolves in the enclosing scope),
        // exactly as slices 1/2 copy. This is the base-arm fold v-effects' recursive-insert unfold needs.
        if args.is_empty() {
            let folded = rewrite_sum_payload(db, *body, scrutinee, scrutinee, None);
            return Some(folded);
        }
        let payload = args[0];
        // SLICE 1: a bare-name binder `(Ctor v)` — the WHOLE payload substitutes. The binder's USES
        // resolve to `SumPayload{scrutinee: S, steps:[Payload]}` (NOT `Ref{binder}`), so `beta_reduce`'s
        // binder-keyed substitution can't catch them and a plain copy re-resolves `v` UNBOUND (the copy
        // leaves the match-arm context). Instead a TARGETED rewrite: replace each subtree node whose
        // `resolved_of` is that `SumPayload` with the payload — the resolve-time analogue of the lowering
        // `Resolved::SumPayload` fold over a visible ctor.
        if db.ast.as_name(binder).is_some() {
            let folded = rewrite_sum_payload(db, *body, scrutinee, payload, None);
            return Some(folded);
        }
        // SLICE 2: a TUPLE sub-pattern `(Ctor (tuple v0 v1 …))` destructuring a tuple payload — each
        // binder `vi` resolves to `SumPayload{scrutinee: S, steps:[Payload, Elem(i)]}`. Reduce the payload
        // to its compile-time-visible tuple elements and substitute element `i` for the `[Payload,
        // Elem(i)]` step. Only fold when the payload IS such a visible tuple (`reduce_to_tuple_elems`) —
        // else leave it a runtime match (a payload behind an opaque runtime binding declines to `None`).
        // This is v-DES's real pqueue shape `(PQCons (tuple wake kb rest))`; once the tuple binders
        // substitute, the caller's loop folds an inner single-payload `(KBox k)` match via slice 1.
        if db
            .ast
            .compound_form_of(binder, CompoundCtor::Tuple)
            .is_some()
        {
            let elems = reduce_to_tuple_elems(db, payload)?;
            let folded = rewrite_sum_payload(db, *body, scrutinee, payload, Some(&elems));
            return Some(folded);
        }
        // A same-disc arm whose payload sub-pattern is a LITERAL (`(Wrap 0)`) — REFUTABLE and undecidable by
        // this static fold (the payload may or may not equal the literal at runtime, under multi-dispatch the
        // scrutinee varies per dispatch). Skipping it to fold to a LATER same-disc arm DISCARDS its selection
        // path: breaker FINDING #15 wider face — `(match c ((Wrap 0) 100) ((Wrap v) v))` over two dispatches
        // lost the LITERAL arm (dispatch-2's `(Wrap 0)` took the general arm's 0, not 100). DECLINE the whole
        // fold (runtime match, which tests the literal correctly). Same root as the guarded same-ctor case
        // above. NARROW to a literal sub-pattern (a non-name atom): a NESTED-ctor / record sub-pattern is left
        // to `continue` as before — the caller's loop folds those via a subsequent slice (the DES pqueue
        // `(PQCons (tuple …))` inner-ctor unfold relies on the scan continuing), and over-declining them would
        // regress that path. A literal is the confirmed shadowing face.
        if matches!(db.ast.get(binder), crate::ast::Struct::Atom(_))
            && db.ast.as_name(binder).is_none()
        {
            return None;
        }
        // Any other sub-pattern (nested variant, record) — not a slice this fold serves; keep scanning.
        continue;
    }
    None
}

/// Recursively COPY `body`, replacing every node whose `resolved_of` is a `SumPayload` reading THIS
/// match's scrutinee — the payload-binder substitution `fold_ctor_match` performs after selecting a
/// matching arm. A non-SumPayload node is rebuilt structurally (a list rebuilds its rewritten children; an
/// atom is shared for a constant / copied for a name so a copied scope still resolves).
///
/// - SLICE 1 (`payload_elems == None`): a single-payload binder `(Ctor v)` reads `[Payload]` → substitute
///   the whole `payload` occurrence.
/// - SLICE 2 (`payload_elems == Some(elems)`): a tuple-destructure `(Ctor (tuple v0 v1 …))` — binder `vi`
///   reads `[Payload, Elem(i)]` → substitute `elems[i]` (the payload tuple's element occurrence). The bare
///   `[Payload]` shape does not occur in a tuple-destructure arm's body (no binder reads the whole tuple),
///   but is handled for safety by substituting the whole `payload`.
///
/// A deeper/other path (a nested `(Ctor (Ctor v))`, an out-of-range `Elem`) is left intact — the caller
/// only folds the two slices above, so no such deeper `SumPayload` over `S` appears in a matched body.
fn rewrite_sum_payload(
    db: &mut Db,
    body: StructId,
    scrutinee: StructId,
    payload: StructId,
    payload_elems: Option<&[StructId]>,
) -> StructId {
    use crate::core::PathStep;
    // A node that RESOLVES to a payload binder of THIS match — a `SumPayload` reading `scrutinee`. Project
    // its `steps` into the visible payload: `[Payload]` → the whole payload; `[Payload, Elem(i)]` → the
    // payload tuple's element `i` (slice 2). Anything else is left as-is (rebuilt structurally below).
    if let Resolved::SumPayload {
        scrutinee: s,
        steps,
        ..
    } = resolved_of(db, body)
        && s == scrutinee
    {
        match (steps.len(), steps.first(), steps.get(1)) {
            // `[Payload]` — the whole payload (slice 1, and a defensive whole-tuple read in slice 2).
            (1, Some(PathStep::Payload), _) => return payload,
            // `[Payload, Elem(i)]` — element `i` of the payload tuple (slice 2). Only when we were handed
            // the tuple's elements and `i` is in range; else fall through to a structural rebuild (leaving
            // the SumPayload as a runtime read rather than mis-projecting).
            (2, Some(PathStep::Payload), Some(&PathStep::Elem(i))) => {
                if let Some(elems) = payload_elems
                    && let Some(&elem) = elems.get(i)
                {
                    return elem;
                }
            }
            _ => {}
        }
    }
    match db.ast.get(body).clone() {
        // A CONSTANT leaf is self-contained (shares); a NAME atom that is NOT the substituted binder is
        // copied fresh so a copied enclosing scope re-resolves it correctly (matching `copy_structural`).
        crate::ast::Struct::Atom(lid) => match db.ast.leaf(lid).clone() {
            crate::ast::Leaf::Name(_) => {
                let leaf = db.ast.leaf(lid).clone();
                let copy = db.push_atom(leaf);
                // Same source-provenance record as `copy_structural` (see there): a name copied by the
                // sum-payload rewrite keeps a back-reference to its source occurrence so an unbound
                // re-resolution in the copy relocates to the author's node rather than losing its span.
                db.synth_name_origin.insert(copy, body);
                copy
            }
            _ => body,
        },
        crate::ast::Struct::List(children) => {
            let rewritten: Vec<StructId> = children
                .iter()
                .map(|&c| rewrite_sum_payload(db, c, scrutinee, payload, payload_elems))
                .collect();
            db.push_list(rewritten)
        }
    }
}

pub fn reduce_to_if(db: &mut Db, id: StructId) -> Option<(StructId, StructId, StructId)> {
    match resolved_of(db, id) {
        Resolved::If { cond, then_, else_ } => Some((cond, then_, else_)),
        Resolved::Ref { value } => {
            if db.kept_bindings.contains(&value) {
                None
            } else {
                reduce_to_if(db, value)
            }
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => reduce_to_if(db, expr),
        // An application whose (non-recursive) callee returns an `if` — `(choose b)` where `choose`'s
        // body is `(if b (fn …) (fn …))`. β-reduce the call to its `if` result, then reduce THAT. This
        // is what lets a runtime-branch-selected function `((choose b) 5)` reach the case-of-case
        // rewrite (the head `(choose b)` reduces to the `if`). Runs under the depth guard so a recursive
        // callee (which can't reduce to a normal form) yields `None` rather than diverging.
        Resolved::Apply { head, args } => {
            let mut guard = db.enter_reduction()?;
            let g = guard.db();
            let reduced = apply_lambda(g, head, &args).ok().flatten()?;
            reduce_to_if(g, reduced)
        }
        _ => None,
    }
}

/// If `id` reduces to an `(if cond T E)` whose BOTH branches reduce to compile-time-visible tuples of
/// the SAME arity, return `(cond, te, ee)` — the condition occurrence and the two branches' element
/// occurrences. This lets a projection push INTO the branches: `(. (if c T E) i)` → `(if c T[i] E[i])`,
/// reusing the EXISTING element occurrences as the new `if`'s branches (NO ast synthesis, NO
/// re-resolution). The un-projected sibling elements drop exactly as they do for a plain tuple
/// projection (`reduce_to_tuple_elems` — a trapping sibling that folds away is never built, so never
/// evaluated), and `cond` is evaluated in both forms, so the fold is value- and trap-preserving. `None`
/// when the operand is not such an `if` (via [`reduce_to_if`], which stops at a kept multi-use binding),
/// or the branch tuples disagree in arity (then the projection stays a runtime `Core::Proj`).
pub fn reduce_to_if_of_tuples(db: &mut Db, id: StructId) -> Option<IfOfTuples> {
    let (cond, then_, else_) = reduce_to_if(db, id)?;
    let te = reduce_to_tuple_elems(db, then_)?;
    let ee = reduce_to_tuple_elems(db, else_)?;
    (te.len() == ee.len()).then_some((cond, te, ee))
}

/// The primitive the value at `id` denotes, following a `Ref` — `+` resolves to a `Ref` to its
/// `(intrinsic +)` node, which resolves to `Prim::Add`. `None` if not a primitive.
pub fn prim_of(db: &mut Db, id: StructId) -> Option<Prim> {
    // Dispatch through the BORROW companion `resolved_ref`, not `resolved_of`: this reads only the
    // `Prim` tag (a `Copy` enum) or the `Ref`'s `value` occurrence (a `Copy` `StructId`) — neither needs
    // the cloned payload, so the by-value `resolved_of` clone of the whole `Resolved` per call was pure
    // churn. `prim_of` is a hot per-node meta reader (called across lower/infer/eval — a realistic 3000-
    // def module showed ~7.8% inclusive, ~2.5% in `Resolved::clone`), so borrow-dispatch is the fix-35/36
    // pattern (a memoized query returning a compound by value cloned per call for a Copy field → borrow).
    match crate::resolve::resolved_ref(db, id) {
        Resolved::Prim(p) => Some(*p),
        Resolved::Ref { value } => {
            let value = *value;
            prim_of(db, value)
        }
        _ => None,
    }
}

/// Reduce a TYPE-CONSTRUCTOR application: apply the constructor `prim` to `args`, returning the
/// occurrence its result reduces to (a built module record for `Int`/`UInt`; a `(typeval …)` for
/// `->`), or `Err` with a decline/reject message. Arithmetic prims are NOT handled here — they fold in
/// `lower`; this is the type-level reduction the evaluator performs.
///
/// `Int`/`UInt` require a single constant WIDTH argument and build the integer module for it. `->`
/// requires two type arguments and builds a function type-value. A wrong arity or a non-constant width
/// is a reject.
///
/// `origin` is the occurrence being reduced (the `(tuple …)`/`(record …)` application node) — a
/// STABLE, UNIQUE per-construction-site identity `push_list` assigns freshly. The value-constructor
/// build-once cache keys on it (see the `TupleNew`/`RecordNew` arm); the type-constructor arms key on
/// their argument VALUES and ignore it.
pub fn reduce_ctor(
    db: &mut Db,
    prim: Prim,
    origin: StructId,
    args: &[StructId],
) -> Result<StructId, String> {
    match prim {
        Prim::IntCtor | Prim::UIntCtor => {
            let signed = matches!(prim, Prim::IntCtor);
            if args.len() != 1 {
                return Err(format!(
                    "{} takes one width argument",
                    if signed { "Int" } else { "UInt" }
                ));
            }
            // A width the compiler cannot resolve to a compile-time natural, OR a resolved one OUTSIDE the
            // admitted range `1..=64`, reduces to the invalid sentinel width 0, so the built module's type
            // is `Ty::Int(Fixed(0))` — which `int_bounds`/`fits_width` reject as CDZ0302 exactly as an
            // explicit `(UInt 0)` is (the bad width is rejected AT THE ANNOTATION, not silently dropped to
            // the default Int64, and not carried to a decline at emit). This covers a MALFORMED concrete
            // width (negative/over-u32/non-integer), a NON-CONSTANT one (a RUNTIME parameter, an unbound
            // name, a non-constant computation — an integer type MUST be indexed by a compile-time width,
            // numeric-model.md §An Integer Type Is Indexed By A Compile-Time Width), AND an OVER-CEILING
            // one (`(UInt 65)`/`(UInt 128)` — a fixed-size integer wider than 64 bits is reserved to the
            // opt-in big-integer layer, not the width-indexed constructor, options/numeric-model/ #Widths
            // above 64 are reserved). This is the constructor's admitted-RANGE gate, the integer analogue
            // of the `FloatCtor` arm's admitted-SET gate (`{32,64}`). (A width VARIABLE `(Int a)` inside an
            // operator's `(meta t)` scheme never reaches here — it is read symbolically by `width_in_env`
            // as `Width::Var`, not by `read_width`.)
            let width = match read_width(db, args[0]) {
                WidthRead::Fixed(w) if (1..=64).contains(&w) => w,
                _ => 0,
            };
            // Build once per (ctor, width): the first demand appends the module, every later demand —
            // repeated on this occurrence, or another `(Int 64)` elsewhere — returns the same node, so
            // the reduction is idempotent and the arena holds one module per width.
            let key = crate::db::BuildKey {
                prim,
                args: vec![width as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                trace!(target: "rcdzc::eval", signed, width, node = cached.0, "ctor (Int/UInt): build-cache hit");
                return Ok(cached);
            }
            let built = build_int_module(db, signed, width);
            db.cache_build(key, built);
            trace!(target: "rcdzc::eval", signed, width, node = built.0, "ctor (Int/UInt): built integer module");
            Ok(built)
        }
        Prim::FloatCtor => {
            if args.len() != 1 {
                return Err("Float takes one width argument".to_string());
            }
            // A width the compiler cannot resolve to a compile-time natural, OR one outside the admitted
            // set {32,64}, reduces to the sentinel width 0 — so the built module's type is
            // `Ty::Float(Fixed(0))`, which `build_float_module` (and the annotation check) reject as
            // CDZ0302, exactly as an out-of-admitted-set integer width is (numeric-model.md §A
            // Floating-Point Type Is Indexed By A Compile-Time Width). This is where the admitted-SET
            // membership is enforced (unlike integers' 1..=64 range) — the constructor is the one gate.
            let width = match read_width(db, args[0]) {
                WidthRead::Fixed(w) if crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&w) => w,
                _ => 0,
            };
            let key = crate::db::BuildKey {
                prim,
                args: vec![width as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                trace!(target: "rcdzc::eval", width, node = cached.0, "ctor (Float): build-cache hit");
                return Ok(cached);
            }
            let built = build_float_module(db, width);
            db.cache_build(key, built);
            trace!(target: "rcdzc::eval", width, node = built.0, "ctor (Float): built float module");
            Ok(built)
        }
        Prim::FnCtor => {
            // A SINGLE-element arrow `(-> R)` is a nullary type `Unit -> R` — the elided-unit convention
            // (a nullary effect op `(op get (-> R))`); a two-element `(-> P R)` is the ordinary `P -> R`; a
            // MULTI-argument arrow `(-> A B … R)` (≥3) CURRIES right-associatively into `A -> (B -> (… -> R))`
            // — the standard n-ary function type. Without the curry a `(-> Int64 Int64 Int64)` annotation
            // (e.g. a round-trip consumer's `(: g (-> Int64 Int64 Int64))`) errored "-> takes one or two type
            // arguments" and the param solved `Any`, declining "parameter type is ambiguous".
            let fn_ty = match args.len() {
                0 => return Err("-> takes at least one type argument".to_string()),
                1 => {
                    let r = typeval_of(db, args[0])
                        .ok_or_else(|| "-> result is not a type".to_string())?;
                    crate::ty::Ty::Fn(Box::new(crate::ty::Ty::Unit), Box::new(r))
                }
                _ => {
                    // Fold from the right: the last element is the result, each earlier element wraps it in
                    // one more arrow. `(-> A B R)` → `A -> (B -> R)`.
                    let (last, rest) = args.split_last().expect("len >= 2");
                    let mut acc = typeval_of(db, *last)
                        .ok_or_else(|| "-> result is not a type".to_string())?;
                    for &a in rest.iter().rev() {
                        let p = typeval_of(db, a)
                            .ok_or_else(|| "-> parameter is not a type".to_string())?;
                        acc = crate::ty::Ty::Fn(Box::new(p), Box::new(acc));
                    }
                    acc
                }
            };
            trace!(target: "rcdzc::eval", ty = %fn_ty.render_name(&db.name_ctx()), "ctor (->): built function type-value");
            Ok(encode_typeval(db, &fn_ty))
        }
        // `Qty` — the quantity-TYPE constructor: `(Qty T u)` builds `Ty::Qty { inner: T, unit: u }`, the
        // type an annotation `(: e (Qty T u))` checks against. The first arg is the inner numeric type
        // (`typeval_of`); the second is a compile-time UNIT (`unit_of`). A malformed inner/unit rejects.
        Prim::QtyCtor => {
            if args.len() != 2 {
                return Err("Qty takes an inner type and a unit".to_string());
            }
            let inner =
                typeval_of(db, args[0]).ok_or_else(|| "Qty inner is not a type".to_string())?;
            let unit = unit_of(db, args[1]).ok_or_else(|| "Qty unit is not a unit".to_string())?;
            let qty = crate::ty::Ty::Qty {
                inner: Box::new(inner),
                unit,
            };
            trace!(target: "rcdzc::eval", ty = %qty.render_name(&db.name_ctx()), "ctor (Qty): built quantity type-value");
            Ok(encode_typeval(db, &qty))
        }
        // `Type.of e` — COMPILE-TIME TYPE REFLECTION. Reduce to the type-VALUE of `e`'s inferred type: run
        // `type_of` on the argument (the value→`Ty` half) and `encode_typeval` the result (the `Ty`→value
        // half). So `(: x (Type.of y))` reduces `(Type.of y)` to `y`'s type-value, which the annotation
        // then checks `x` against. The eval→infer call is the same established direction the operator/
        // width arms already use; `type_of` is memoized + depth-guarded. A value that types as `Any`
        // (unresolved) reflects as `Any`, faulted where it is used, not here.
        Prim::TypeOf => {
            if args.len() != 1 {
                return Err("Type.of takes one value argument".to_string());
            }
            // Reflect via `infer::reflected_ty`, NOT the bottom-up `type_of`. The `type_of` Lambda arm
            // types each parameter from its NAME occurrence alone, so an UNANNOTATED param is `Any` —
            // `(fn (x) (+ x 1))` reflects `(-> Any Int64)` even though the body pins `x` to `Int64`. Two
            // such functions with genuinely different domains (`(fn (x) (+ x 1))` vs `(fn (b) (if b 0 1))`)
            // then reflect the SAME `(-> Any Int64)` and `Type.eq` returns a wrong `true` — a reflection-
            // soundness miscompile (and the dual: an unannotated vs an equal ANNOTATED domain reflected
            // `false`). `reflected_ty` grounds each unannotated fn param from the body (via
            // `solved_lambda_arrow`, the same solve `lower_lambda_value` runs) — for a bare function AND
            // for one stored inside a compound (`(tuple f 0)`, `(list f)`, a record/map field), recursing
            // the compound element types. A genuinely-unconstrained param stays `Any` (a polymorphic
            // `(fn (x) x)`), the honest domain. A value with no fn to ground reflects exactly as `type_of`
            // types it. `type_of` itself is left untouched — layout/emit consume it and erase `Any` param
            // holes rather than comparing them, so only reflection needs the grounded arrow.
            let ty = crate::infer::reflected_ty(db, args[0]);
            trace!(target: "rcdzc::eval", ty = %ty.render_name(&db.name_ctx()), "ctor (Type.of): reflected value type");
            Ok(encode_typeval(db, &ty))
        }
        // `Tuple` — VARIADIC over its element TYPES: reduce each arg to a `Ty`, build `Ty::Tuple`. Its
        // arity + element types ARE the type, so `(: e (Tuple T…))` checks the value's shape against it
        // (a wrong arity/element is CDZ0203 at the annotation, via `agrees_with`).
        Prim::TupleCtor => {
            let mut elems = Vec::with_capacity(args.len());
            for (i, &a) in args.iter().enumerate() {
                let t =
                    typeval_of(db, a).ok_or_else(|| format!("Tuple element {i} is not a type"))?;
                elems.push(t);
            }
            let tup = crate::ty::Ty::Tuple(elems.into());
            trace!(target: "rcdzc::eval", ty = %tup.render_name(&db.name_ctx()), "ctor (Tuple): built tuple type-value");
            Ok(encode_typeval(db, &tup))
        }
        // `List` — ONE element type: `(List Int64)` builds `Ty::List(Int64)`, the type an annotation
        // `(: e (List T))` checks against.
        Prim::ListCtor => {
            if args.len() != 1 {
                return Err("List takes one element-type argument".to_string());
            }
            let elem =
                typeval_of(db, args[0]).ok_or_else(|| "List element is not a type".to_string())?;
            let list_ty = crate::ty::Ty::List(Box::new(elem));
            trace!(target: "rcdzc::eval", ty = %list_ty.render_name(&db.name_ctx()), "ctor (List): built list type-value");
            Ok(encode_typeval(db, &list_ty))
        }
        // `Map` — a KEY type and a VALUE type: `(Map Int64 Int64)` builds `Ty::Map(Int64, Int64)`, the
        // type an annotation `(: m (Map K V))` checks against. Two arguments, unlike `List`'s one.
        Prim::MapCtor => {
            if args.len() != 2 {
                return Err("Map takes a key type and a value type".to_string());
            }
            let key = typeval_of(db, args[0]).ok_or_else(|| "Map key is not a type".to_string())?;
            let value =
                typeval_of(db, args[1]).ok_or_else(|| "Map value is not a type".to_string())?;
            let map_ty = crate::ty::Ty::Map(Box::new(key), Box::new(value));
            trace!(target: "rcdzc::eval", ty = %map_ty.render_name(&db.name_ctx()), "ctor (Map): built map type-value");
            Ok(encode_typeval(db, &map_ty))
        }
        // `Set` — ONE element type: `(Set Int64)` builds `Ty::Set(Int64)`, the type an annotation
        // `(: s (Set T))` checks against. One argument, like `List`.
        Prim::SetCtor => {
            if args.len() != 1 {
                return Err("Set takes one element-type argument".to_string());
            }
            let elem =
                typeval_of(db, args[0]).ok_or_else(|| "Set element is not a type".to_string())?;
            let set_ty = crate::ty::Ty::Set(Box::new(elem));
            trace!(target: "rcdzc::eval", ty = %set_ty.render_name(&db.name_ctx()), "ctor (Set): built set type-value");
            Ok(encode_typeval(db, &set_ty))
        }
        // `Record` — VARIADIC over `(name type)` field pairs: each arg is a raw `(name T)` list; read the
        // field name (first child) and reduce the type (second child) to a `Ty`. The field-name SET +
        // per-field types ARE the record type, so `(: e (Record (a T)…))` checks the value's field set and
        // field types against it (a wrong field type is CDZ0203 at the annotation, via `unify`). A
        // duplicate field name collapses in the `BTreeMap` — the same fixed-SET rule a record VALUE has.
        Prim::RecordCtor => {
            let mut fields: std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty> =
                std::collections::BTreeMap::new();
            for (i, &a) in args.iter().enumerate() {
                // The record-TYPE field is EITHER the canonical `(: name T)` ascription (the shared
                // binder node — DESIGN-record-type-syntax Phase A) OR the legacy `(name T)` head-app
                // pair. Read both to `(name, type)`; accepting the ascription here is the additive half
                // of RT3, needed BEFORE the encoder flips to emit ascription. Strictly widening — an
                // ascription in this position previously failed the `len == 2` pair check and errored,
                // so no currently-accepted input changes. The pair arm is pruned once head-app is
                // extinct (OQ-C).
                let (name_occ, ty_occ) = if let Some(asc) = db.ast.as_form(a, ":") {
                    if asc.len() != 2 {
                        return Err(format!("Record field {i} ascription is not (: name type)"));
                    }
                    (asc[0], asc[1])
                } else {
                    match db.ast.get(a) {
                        crate::ast::Struct::List(children) if children.len() == 2 => {
                            (children[0], children[1])
                        }
                        _ => return Err(format!("Record field {i} is not a (name type) pair")),
                    }
                };
                let name = db
                    .ast
                    .as_name(name_occ)
                    .ok_or_else(|| format!("Record field {i} has no name"))?
                    .to_string();
                let t = typeval_of(db, ty_occ)
                    .ok_or_else(|| format!("Record field `{name}` is not a type"))?;
                fields.insert(crate::resolved::Symbol::plain(name), t);
            }
            let rec = crate::ty::Ty::Record(std::rc::Rc::new(fields));
            trace!(target: "rcdzc::eval", ty = %rec.render_name(&db.name_ctx()), "ctor (Record): built record type-value");
            Ok(encode_typeval(db, &rec))
        }
        // The compound-VALUE constructors reached via the shadowable `tuple`/`record` alias names.
        // Reduce the application to the NATIVE ctor-LEAF primitive form — `(tuple a b)` → `(#tuple a b)`,
        // `(record (x 1) …)` → `(#record (x 1) …)` — so it resolves to `Resolved::Tuple`/
        // `Resolved::Record` and every downstream machinery (the compile-time-visible fold in
        // `reduce_to_tuple_elems`/`reduce_to_record_id`, member projection, the host-escape walk,
        // `type_of`, `lower`) treats it IDENTICALLY to a value written with the native ctor-leaf primitive
        // (the ctor-leaf head is unshadowable, recognized by kind via `compound_ctor_leaf`; the M3
        // reader-flip removed the legacy string head).
        //
        // WARNING: `push_list` RE-PARENTS the arg subtrees under the new head, which would orphan a field/
        // element value like the runtime param `a` in `(record (x a) …)` from its lexical scope (a
        // spurious unbound name — the re-parenting hazard `apply_lambda` documents). So PIN each arg's
        // resolution first (`resolve_subtree` memoizes every node against its CURRENT scope), exactly as
        // β-reduction pins a call argument before splicing it; the later re-parent then cannot change
        // how any node inside an arg resolves.
        Prim::TupleNew | Prim::RecordNew | Prim::ListNew | Prim::SetNew => {
            // Build once per construction site. The built node is `(head-str arg…)` over the EXACT SAME
            // arg occurrences, so it is a pure function of the SITE being reduced — the same source
            // application demanded repeatedly (a `let`-bound tuple projected field-by-field: `(+ (. t 0)
            // (+ (. t 1) …))` re-reduces `t` at EVERY projection) must return the ONE built node, not
            // rebuild the whole N-element tuple each time. Without this the fold was O(N²): N projections
            // × an O(N) `push_list` rebuild (the shared-tuple projection chain).
            //
            // The key is O(1): the prim plus `origin` — the StructId of the application being reduced,
            // which `push_list` assigns FRESH per occurrence. NOT the argument ids: β-reduction SHARES a
            // constant-atom occurrence (`copy_structural` returns the original id for a literal), so in a
            // NESTED `(tuple n (tuple n n))` the outer and inner tuples share the same first-arg `n`
            // occurrence — keying on an argument would collide the inner build onto the outer and recurse
            // without end (a stack overflow). Keying on all N arg ids instead would re-introduce the
            // O(N²) as an O(N) key HASH per projection. `origin` is unique and O(1); it mirrors the
            // `Int`/`UInt` build-once cache (which keys on the width value, likewise unique per module).
            let key = crate::db::BuildKey {
                prim,
                args: vec![origin.0 as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                trace!(target: "rcdzc::eval", node = cached.0, "ctor (tuple/record value): build-cache hit");
                return Ok(cached);
            }
            for &a in args {
                crate::resolve::resolve_subtree(db, a);
            }
            let head = db.push_atom(Leaf::Ctor(match prim {
                Prim::TupleNew => CompoundCtor::Tuple,
                Prim::ListNew => CompoundCtor::List,
                Prim::SetNew => CompoundCtor::Set,
                _ => CompoundCtor::Record,
            }));
            let mut children = vec![head];
            children.extend_from_slice(args);
            let built = db.push_list(children);
            db.cache_build(key, built);
            Ok(built)
        }
        // `map` NAME alias — rewrite `(map (k v) …)` to the symbol-headed `("map" (k v) …)`, resolved by
        // `resolve_map`. The args are the ENTRY-PAIR nodes (each a `(key value)` list); pin each entry's
        // resolution first (the same re-parent hazard the tuple/record arm guards — a runtime key/value
        // like `(map (a 1))` with a bound `a` must keep its lexical scope through the re-parent). Mirrors
        // the `TupleNew`/`RecordNew`/`ListNew` arm, build-once cached on the site.
        Prim::MapNew => {
            let key = crate::db::BuildKey {
                prim,
                args: vec![origin.0 as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                return Ok(cached);
            }
            for &entry in args {
                crate::resolve::resolve_subtree(db, entry);
            }
            let head = db.push_atom(Leaf::Ctor(CompoundCtor::Map));
            let mut children = vec![head];
            children.extend_from_slice(args);
            let built = db.push_list(children);
            db.cache_build(key, built);
            Ok(built)
        }
        // `Map.swap`/`Map.take` — the value-yielding forms — reduce to the tuple `(tuple (Map.lookup m k)
        // (Map.insert m k v))` (swap) / `(tuple (Map.lookup m k) (Map.remove m k))` (take): the prior/
        // removed value optional PAIRED with the new map. Synthesized as AST so the ordinary pipeline
        // lowers it AND `reduce_to_tuple_elems` reduces it the same way — so `(. (Map.swap …) 0)` folds to
        // just the lookup (the corpus projects element 0), dropping the unused new map with no heap build.
        // `args` are `[m, k]` (take) or `[m, k, v]` (swap). The operand occurrences are REUSED (kept in
        // their lexical scope, as leaves of the new applications); `resolve_subtree` pins each. Build-once
        // cached on the site (the `origin`), like `MapNew`.
        Prim::MapSwap | Prim::MapTake => {
            let key = crate::db::BuildKey {
                prim,
                args: vec![origin.0 as i64],
            };
            if let Some(cached) = db.cached_build(&key) {
                return Ok(cached);
            }
            if args.len() < 2 {
                return Err(
                    "Map.swap/Map.take takes a map, a key (and a value for swap)".to_string(),
                );
            }
            let (map, k) = (args[0], args[1]);
            // `(. Map lookup)` applied to `(m k)` — the prior/removed value optional.
            let lookup_app = {
                let dot = db.push_name(".");
                let map_mod = db.push_name("Map");
                let lookup = db.push_name("lookup");
                let member = db.push_list(vec![dot, map_mod, lookup]);
                db.push_list(vec![member, map, k])
            };
            // The new-map half: `(Map.insert m k v)` for swap, `(Map.remove m k)` for take.
            let update_app = {
                let dot = db.push_name(".");
                let map_mod = db.push_name("Map");
                if prim == Prim::MapSwap {
                    let insert = db.push_name("insert");
                    let member = db.push_list(vec![dot, map_mod, insert]);
                    db.push_list(vec![member, map, k, args[2]])
                } else {
                    let remove = db.push_name("remove");
                    let member = db.push_list(vec![dot, map_mod, remove]);
                    db.push_list(vec![member, map, k])
                }
            };
            crate::resolve::resolve_subtree(db, lookup_app);
            crate::resolve::resolve_subtree(db, update_app);
            let head = db.push_atom(Leaf::Ctor(CompoundCtor::Tuple));
            let built = db.push_list(vec![head, lookup_app, update_app]);
            db.cache_build(key, built);
            Ok(built)
        }
        _ => Err(NOT_A_CTOR_PRIM.to_string()),
    }
}

/// The internal sentinel `reduce_ctor` returns for a prim it does not build — a non-constructor
/// OPERATION prim (`list-at`, `map-insert`, …) reaches it ONLY via `lower`'s constructor catch-all when
/// its full-arity arm did not match, i.e. the operation was applied to the WRONG number of arguments.
/// `lower` recognizes this exact string and rewrites it into an honest wrong-arity/partial-application
/// decline (naming the operation), so the internal wording never surfaces to a user (the "decline
/// HONESTLY, not with an internal message" discipline — `self-hosting-and-bootstrap.md`).
pub const NOT_A_CTOR_PRIM: &str = "not a type constructor";

/// Reduce a GENERIC SUM type-constructor application `(Option Int64)` to its `Ty::Sum { decl, name,
/// args }` DIRECTLY. `head` is the applied sum record (`Option`), carrying the owning declaration on
/// its `(meta sum-decl)` channel; `args` are the type arguments, each reduced to a `Ty`. `None` if the
/// head has no `(meta sum-decl)` (not a generic sum), an arg is not a type, or the declaration is
/// unknown. (The analogue of `reduce_ctor` for `TupleCtor`, but keyed on the head's metadata rather
/// than the prim alone — one prim serves every generic sum.)
///
/// COST: Returns the `Ty` value, NOT an encoded `(typeval …)` arena node: its one caller (`typeval_of`'s
/// `SumCtor` arm) wants the `Ty` and previously got it by `encode_typeval`-ing this result then
/// `typeval_of`-DECODING it right back — a full encode→decode arena round-trip PER LEVEL, so a
/// deeply-nested `(Option (Option … Int64))` annotation was O(N²) in arena node churn (measured: 400
/// deep = 179ms, 800 = 697ms, pure quadratic). Handing back the `Ty` skips both halves.
pub fn reduce_sum_ctor(db: &mut Db, head: StructId, args: &[StructId]) -> Option<crate::ty::Ty> {
    let decl_field = project_meta(db, head, "sum-decl")?;
    let decl = match resolved_of(db, decl_field) {
        Resolved::Int(v) => crate::ast::StructId(v.to_i64().and_then(|n| u32::try_from(n).ok())?),
        _ => return None,
    };
    let mut arg_tys = Vec::with_capacity(args.len());
    for &a in args {
        arg_tys.push(typeval_of(db, a)?);
    }
    // Normalize through the SHARED chokepoint: an erasable GENERIC newtype (`(type Box (Mk a))`) at this
    // instantiation becomes `Ty::Nominal { inner }` (its template with `arg_tys` substituted), NOT a boxed
    // `Ty::Sum` — the same decision `decode_ty` makes for the wire form. Without this the generic ctor's
    // result typed as `Ty::Sum`, so the erased value's binder read the wrong (boxed) representation.
    let sum = db.normalize_sum(decl, arg_tys);
    trace!(target: "rcdzc::eval", ty = %sum.render_name(&db.name_ctx()), "ctor (Sum): built generic sum type-value");
    Some(sum)
}

/// The type-VALUE the node at `id` reduces to, if any — a ground type (`Bool`/`Unit` via `(meta t)`),
/// a `(typeval …)` built node, or a type-constructor application reduced. This is how a type
/// expression yields a `Ty`. Returns `None` if the node is not a type.
/// Whether `id` reduces to the TYPE-REFLECTION MODULE (`Type`) — the prelude record whose `of`/`eq`
/// fields are the `type-of`/`type-eq` operations. Recognized STRUCTURALLY (its `of` field's `(meta
/// apply)` is `Prim::TypeOf`), NOT by the name "Type", so `Type` is privileged by nothing
/// (`prelude-and-resolution.md §Nothing Is Privileged By Name`). Used by `typeval_of` to let `Type`
/// denote the kind-of-types (`Ty::Type`) in a parameter annotation — a type-valued parameter.
/// If `id` (in a type position) resolves to a TYPE-VALUED PARAMETER — a `def`/`fn` parameter whose own
/// annotation is `Type` (so `type_of` its binder is `Ty::Type`) — the binder occurrence; else `None`.
/// Follows a `Ref` chain to the parameter (a bare type-position name resolves to a `Ref` to the param's
/// name occurrence). Used by `typeval_of` to reduce `t` in `(: b t)` / `(: b (Box t))` to a stable type
/// variable. NOT a name key — purely "does this reference resolve to a param typed `Ty::Type`".
fn type_valued_param_binder(db: &mut Db, id: StructId) -> Option<StructId> {
    // Chase a `Ref` chain (a type-position bare name → `Ref` to the param name occurrence) to a `Param`.
    let mut cur = id;
    let binder = loop {
        match resolved_of(db, cur) {
            Resolved::Param { binder } => break binder,
            Resolved::Ref { value } if value != cur => cur = value,
            _ => return None,
        }
    };
    // The parameter is type-valued iff its OWN annotation is `Type` (`Ty::Type`). `param_annot_ty` reads
    // the `(: name T)` annotation via `typeval_of(T)` — which for `Type` yields `Ty::Type` (T1). A bare
    // (unannotated) param, or one annotated with an ordinary type, is NOT a type-valued parameter.
    match crate::infer::type_of(db, binder) {
        crate::ty::Ty::Type => Some(binder),
        _ => None,
    }
}

fn is_type_reflection_module(db: &mut Db, id: StructId) -> bool {
    let of = Symbol::plain("of");
    match member_value(db, id, &of) {
        Member::Field(v) => meta_apply_of(db, v) == Some(Prim::TypeOf),
        _ => false,
    }
}

pub fn typeval_of(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    // READ-THROUGH MEMO (post-load only). `typeval_of` is env-free and deterministic per `StructId` once
    // the load-time newtype precompute+fixpoint has run, so a node's denoted type-VALUE caches. Without it
    // a wide structural-record type ANNOTATION re-reduces from scratch at each of its ~1M referencing uses
    // (resolve→infer→lower), exhausting the per-query reduction budget → spurious CDZ0999 (the DB-records
    // whole-project case). Only `Some` is cached: a `None` may be BUDGET-induced (`enter_reduction`
    // declines near the limit) and `reduce_nodes` resets per query, so a real later demand must be free to
    // recompute it under a fresh budget. Gated on `typeval_memo_live` — off during the pre-normalization
    // precompute (a `Ty::Sum` reduced before `newtype_inner` is complete must not be captured); the
    // per-node entry is invalidated by `resolve::forget_subtree` when a lowering-time re-parent changes the
    // node's resolved form.
    if db.typeval_memo_live
        && let crate::arena::Slot::Filled(t) = db.typeval.get(id)
    {
        return Some(t.clone());
    }
    let result = typeval_of_uncached(db, id);
    if db.typeval_memo_live
        && let Some(t) = &result
    {
        db.typeval.fill(id, t.clone());
    }
    result
}

fn typeval_of_uncached(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    // A TYPE-VALUED PARAMETER used in a type position — `(: t Type)`'s `t`, referenced as a bare type
    // `(: b t)` or as a type-constructor argument `(: b (Box t))`. At the DEFINITION site the parameter's
    // concrete type-value is unknown (it arrives at the call), so `t` reduces to a stable TYPE VARIABLE
    // `Ty::Var(binder)` — the annotation `(Box t)` becomes the generic `(Box ?t)`, which the call site
    // monomorphizes by substituting the passed type-value (the type-valued-parameter vertical, T3). Keyed
    // on the binder's `StructId` so every occurrence of `t` in the signature+body shares ONE variable.
    // Only a param whose OWN type is `Ty::Type` (a type-valued param); an ordinary value param is not a
    // type. Checked FIRST because such a reference resolves to a `Ref`/`Param`, which the arms below would
    // otherwise treat as a non-type (returning `None`).
    if let Some(binder) = type_valued_param_binder(db, id) {
        return Some(crate::ty::Ty::Var(binder.0));
    }
    match resolved_of(db, id) {
        // A `let`-bound (or otherwise referenced) name whose init IS a type expression — `(let ((t Int64))
        // t)`, the body `t` a `Ref` to the binding's init `Int64`. Follow the reference to its value and
        // reduce THAT, so a type-value flows through a binding exactly as a unit does through `unit_of`'s
        // `Ref` arm. Guarded to a Ref whose OWN `(meta t)` projection is absent (a ground-type MODULE
        // record like `Int64` is also `Resolved::Ref`-shaped but reduces via its `(meta t)` below), so the
        // module path is unchanged; only a binding-to-a-type-expression takes this follow-through.
        Resolved::Ref { value }
            if value != id
                && project_meta(db, id, "t").is_none()
                && variant_disc_of(db, id).is_none() =>
        {
            typeval_of(db, value)
        }
        // A ground-type record: read its `(meta t)`, which holds the type-value.
        Resolved::Record { .. } | Resolved::Ref { .. } => {
            // A VARIANT-CONSTRUCTOR record is NOT a type-value even though it carries a `(meta t)` — its
            // `(meta t)` is the constructor's own type (`(-> P Sum)` or, for a nullary variant, the sum
            // `Sum`), and a NULLARY variant used bare (`None`) is a VALUE of the sum, not the sum TYPE.
            // Its `(meta variant)` channel is the mark that distinguishes it: a record carrying it is a
            // sum value/constructor, so it does not reduce to a type here (a bare `None` types as the
            // sum via `type_of`, not as `Ty::Type`; `(: x Option)` still reduces `Option` — the SUM
            // record — because that record has no `(meta variant)`).
            if variant_disc_of(db, id).is_some() {
                return None;
            }
            // Either a `(meta t)` ground-type record, OR an already-built `(typeval …)` record.
            if let Some(t_field) = project_meta(db, id, "t") {
                return typeval_of(db, t_field);
            }
            // The TYPE-REFLECTION MODULE (`Type`) in a type position denotes the KIND OF TYPES,
            // `Ty::Type` — so `(: t Type)` declares a TYPE-VALUED PARAMETER (`type-system.md §Generics Are
            // Type-Valued Parameters`; the type-valued-parameter vertical). Recognized STRUCTURALLY — its
            // `of` field reduces to `Prim::TypeOf` — never by the name "Type" (no key outside the prelude).
            // This revises the prior "`Type` in a bare type position is not a type" stance to give the
            // type-valued-parameter model a spellable annotation.
            if is_type_reflection_module(db, id) {
                return Some(crate::ty::Ty::Type);
            }
            None
        }
        // A native ground type prim (`Bool`/`Unit`) held in a `(meta t)` field.
        Resolved::Prim(p) => p.ground_type(),
        // The prelude UNIT value (the empty product) used in a TYPE position → `Ty::Unit`. The pervasive
        // nullary-variant idiom writes a unit-typed payload as lowercase `(None unit)` / `(Nil unit)` /
        // `(Red unit)` (prelude.rs §"the pervasive nullary-variant idiom"; guide PatternMatching), where the
        // bare `unit` follows a `Ref` to `Resolved::Unit`. `unit` is NOT a type parameter (excluded in
        // `collect_type_params`), so a variant payload `unit` reduces here to the concrete `Ty::Unit` — the
        // type-position lenient-alias for the capital `Unit` (concierge ruling A, 2026-08-03), so
        // `(type (Box a) (Full a) (Nil unit))` has EXACTLY one param `a` (no spurious `unit` param + stray
        // Var → fixes the rust Set/Map non-Ord decline) while the taught `(None unit)` idiom keeps resolving.
        Resolved::Unit => Some(crate::ty::Ty::Unit),
        // A built `(typeval …)` node carries the type directly.
        Resolved::TypeVal(t) => Some(t),
        // A `let` whose BODY is a type expression — `(let ((t Int64)) t)` evaluates to the body's value,
        // a type-value. The bindings are pure (they only introduce names the body's `Ref`s resolve
        // through), so the `let`'s type-value IS its body's. Reduce the body (its `t` reference follows to
        // the binding's init `Int64` via the `Ref` arm above). Lets a bound type flow to a boundary export.
        Resolved::Let { body, .. } => typeval_of(db, body),
        // An ANNOTATED occurrence `(: e T)` whose annotation is the KIND OF TYPES (`T` reduces to
        // `Ty::Type`) — the node denotes a TYPE-VALUE, so follow through to `e` and reduce THAT. This is
        // how a TYPE-VALUED PARAMETER reference reaches `typeval_of` when it sits in a VALUE-argument
        // position (`(Type.eq t Int64)`): unlike a bare type-position use (`(: b (Box t))`, which resolves
        // straight to the `Param` and hits `type_valued_param_binder` at the top), a value-argument
        // occurrence of `t` resolves to `Annot { expr: <t param ref>, ty_expr: Type }`. Without following
        // it, `typeval_of` returned `None` and `Type.eq t Int64` declined "requires two type-values" even
        // though `t` is a compile-time type-value (monomorphization substitutes the concrete type). Only a
        // `Type`-KINDED annotation follows through — an ordinary value annotation `(: 5 Int64)` is NOT a
        // type-value (its `ty_expr` reduces to `Int64`, not `Ty::Type`), so it still returns `None`.
        Resolved::Annot { expr, ty_expr }
            if typeval_of(db, ty_expr) == Some(crate::ty::Ty::Type) =>
        {
            typeval_of(db, expr)
        }
        // A TUPLE-TYPE LITERAL in the COMMA spelling `(A, B)` that resolves to `Resolved::Tuple` (rather
        // than the `(Tuple …)` application the `Prim::TupleCtor` arm below handles) — reduce each element
        // to its type and build `Ty::Tuple`. Without this arm a comma-spelled tuple type nested in a
        // variant payload — `(Int64, Int64 -> Option((Bool, Int64)))` — fell through to `_ => None`, so
        // the ctor's payload type was unreadable and the variant looked NULLARY (CDZ0201 at construction).
        // The ground-case twin of the `type_in_env` `Resolved::Tuple` arm (this is the concrete reducer;
        // that one carries the type-lambda env). An element that is not itself a type reduces to `None`,
        // propagating (a malformed tuple-type is not silently accepted).
        Resolved::Tuple { elems } => {
            let mut tys = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                tys.push(typeval_of(db, e)?);
            }
            Some(crate::ty::Ty::Tuple(tys.into()))
        }
        // A type-constructor application — reduce it, then read the built value's type.
        Resolved::Apply { head, args } => {
            // A GENERIC type whose declared NAME coincides with its variant's name — `(type Box (Box a))`
            // — applied in a TYPE position `(Box Int64)`: the head resolves to the VARIANT constructor
            // (`SumNew`, a `variant_disc_of` node) rather than the type constructor (`SumCtor`), because a
            // same-name occurrence prefers the value binding. In a type position the applied form can only
            // mean the TYPE, so recover the owning declaration from the variant head and build the sum type
            // directly — the same `Ty` `reduce_sum_ctor` produces from a `SumCtor` head. (A bare same-name
            // type `Pitch` in a type position already reduces via the `Resolved::Record`/`Ref` `(meta t)`
            // arm; only the APPLIED generic form reached this variant-head gap.) Guarded to a head that
            // genuinely IS a variant constructor whose owning decl has type PARAMETERS (an applied form of a
            // monomorphic sum's same-name variant would be a wrong-arity value application, not a type).
            if variant_disc_of(db, head).is_some()
                && let Some(decl) = variant_owner_decl(db, head)
                && db
                    .type_decl_by_occ(decl)
                    .is_some_and(|td| !td.params.is_empty())
            {
                let mut arg_tys = Vec::with_capacity(args.len());
                for &a in args.iter() {
                    arg_tys.push(typeval_of(db, a)?);
                }
                return Some(db.normalize_sum(decl, arg_tys));
            }
            let prim = meta_apply_of(db, head)?;
            if prim.is_arith() {
                return None;
            }
            // A GENERIC SUM application `(Option Int64)` — build `Ty::Sum{decl,args}` from the head's
            // `(meta sum-decl)` + the applied type args (keyed on the head's metadata, not the prim).
            // `reduce_sum_ctor` returns the `Ty` directly (no arena encode→decode round-trip — that was
            // the O(N²) on deeply-nested generic annotations).
            if prim == Prim::SumCtor {
                return reduce_sum_ctor(db, head, &args);
            }
            // The COLLECTION type constructors build their `Ty` DIRECTLY from the reduced argument types —
            // no `reduce_ctor`→`encode_typeval` arena round-trip. That round-trip re-SERIALIZES the whole
            // built `Ty` into fresh AST nodes at EVERY nesting level, so a depth-N annotation `(List (List
            // … Int64))` encoded O(N) nodes per level → O(N²) appended nodes (and this reduction is re-run
            // per referencing occurrence, so the whole `check`/resolve was super-linear). Building the `Ty`
            // in place — exactly what `reduce_ctor`'s arms below compute before encoding — and returning it
            // is the same fix `reduce_sum_ctor` already applies for generic sums. Arity/element faults fall
            // through to `reduce_ctor` (which reports them) by returning `None` here.
            match prim {
                Prim::ListCtor if args.len() == 1 => {
                    return Some(crate::ty::Ty::List(Box::new(typeval_of(db, args[0])?)));
                }
                Prim::SetCtor if args.len() == 1 => {
                    return Some(crate::ty::Ty::Set(Box::new(typeval_of(db, args[0])?)));
                }
                Prim::MapCtor if args.len() == 2 => {
                    let key = typeval_of(db, args[0])?;
                    let value = typeval_of(db, args[1])?;
                    return Some(crate::ty::Ty::Map(Box::new(key), Box::new(value)));
                }
                // `Tuple`/`Record` are the remaining twins of the collection fast paths above — WITHOUT a
                // direct build they take the `reduce_ctor`→`encode_typeval` arena round-trip, which
                // re-serializes the whole built `Ty` per nesting level → O(depth²) nodes on a deeply-nested
                // `(Tuple (Tuple …) Int64)` / `(Record (f (Record …)))` annotation (the fix that already
                // covers List/Set/Map). Build the `Ty` in place from the reduced children — exactly what
                // `reduce_ctor`'s `Tuple`/`Record` arms compute before encoding — and return it.
                Prim::TupleCtor => {
                    let mut elems = Vec::with_capacity(args.len());
                    for &a in args.iter() {
                        elems.push(typeval_of(db, a)?);
                    }
                    return Some(crate::ty::Ty::Tuple(elems.into()));
                }
                Prim::RecordCtor => {
                    let mut fields: std::collections::BTreeMap<
                        crate::resolved::Symbol,
                        crate::ty::Ty,
                    > = std::collections::BTreeMap::new();
                    for &a in args.iter() {
                        // A field is a `(name type)` pair (the s-expr spelling `(Record (a Int64) …)`) OR a
                        // `(: name type)` annotation triple — the shape the ML record-type surface `{a:
                        // Int64, …}` lowers to (`Record(a : Int64, …)`, each field a 3-element `(: a Int64)`).
                        // Accept BOTH so a structural record decodes identically in EITHER surface: without
                        // the triple arm the ML `{…}` field bailed to `None`, so a variant payload / effect-op
                        // return typed as a bare `{…}` was lost (variant read NULLARY, perform site read the
                        // op meta-record). A malformed field / non-name / non-type still falls through to
                        // `reduce_ctor` via `None`.
                        let crate::ast::Struct::List(pair) = db.ast.get(a) else {
                            return None;
                        };
                        let (name_occ, ty_occ) = match pair.as_slice() {
                            [n, t] => (*n, *t),
                            // `(: name type)` — the leading `:` annotation head, then name + type.
                            [colon, n, t] if db.ast.as_name(*colon) == Some(":") => (*n, *t),
                            _ => return None,
                        };
                        let name = db.ast.as_name(name_occ)?.to_string();
                        let t = typeval_of(db, ty_occ)?;
                        fields.insert(crate::resolved::Symbol::plain(name), t);
                    }
                    return Some(crate::ty::Ty::Record(std::rc::Rc::new(fields)));
                }
                _ => {}
            }
            let built = reduce_ctor(db, prim, id, &args).ok()?;
            typeval_of(db, built)
        }
        _ => None,
    }
}

/// Reduce an occurrence to the compile-time UNIT it denotes, or `None` if it is not a unit expression.
/// A unit is a value in the free abelian group over base dimensions (`ty::Unit`), BUILT by the unit
/// prims: `Unit.one` (empty map), `(Unit.base #"meter")` (a single base), `(Unit.* a b)` / `(Unit./ a
/// b)` / `(Unit.^ u n)` (the group operations). This is the units analogue of `read_width` — it reads a
/// compile-time value off the argument, since a unit is not an HM-typed value the scheme machinery
/// tracks. Recurses through `Ref` (a binding) and the applied unit prims. The base-dimension NAME comes
/// from a `Leaf::Sym` (resolved to a `Str`, Layer 1). `Unit.one` may appear either bare (a nullary
/// `Resolved::Prim(UnitOne)` via its `(meta apply)`) or applied to no args.
pub fn unit_of(db: &mut Db, id: StructId) -> Option<crate::ty::Unit> {
    use crate::ty::Unit;
    match resolved_of(db, id) {
        Resolved::Ref { value } => unit_of(db, value),
        // `Unit.one` used BARE — the member access `(. Unit one)` resolves to the builder record; its
        // `(meta apply)` is `UnitOne`, so reducing it (no args) is the dimensionless unit.
        Resolved::Record { .. } | Resolved::Member { .. } => match meta_apply_of(db, id) {
            Some(Prim::UnitOne) => Some(Unit::one()),
            _ => None,
        },
        Resolved::Prim(Prim::UnitOne) => Some(Unit::one()),
        // An APPLICATION of a unit builder: dispatch on the head's `(meta apply)` prim.
        Resolved::Apply { head, args } => {
            let prim = meta_apply_of(db, head)?;
            match prim {
                Prim::UnitOne => Some(Unit::one()),
                Prim::UnitBase => {
                    // `(Unit.base #"meter")` — the base-dimension name is the symbol's TEXT, read directly
                    // off the `#"…"` literal (a `SymbolConst`; also accept a plain `Str` for a string-keyed
                    // caller). A non-symbol/string argument is not a well-formed base unit.
                    let name = match resolved_of(db, *args.first()?) {
                        Resolved::SymbolConst(s) | Resolved::Str(s) => s,
                        _ => return None,
                    };
                    Some(Unit::base(name))
                }
                Prim::UnitMul => {
                    let a = unit_of(db, *args.first()?)?;
                    let b = unit_of(db, *args.get(1)?)?;
                    Some(a.mul(&b))
                }
                Prim::UnitDiv => {
                    let a = unit_of(db, *args.first()?)?;
                    let b = unit_of(db, *args.get(1)?)?;
                    Some(a.div(&b))
                }
                Prim::UnitPow => {
                    let a = unit_of(db, *args.first()?)?;
                    // The exponent is a compile-time integer literal (may be negative).
                    let n = match resolved_of(db, *args.get(1)?) {
                        Resolved::Int(v) => v.to_i64()?,
                        _ => return None,
                    };
                    Some(a.pow(n))
                }
                // The ORDINARY arithmetic operators used on UNITS compose them, so unit composition reads
                // as ordinary math on both surfaces (`meter * second`, `meter / second`, `meter ^ 2`) —
                // no backtick-escaped `Unit.*` name. `*`/`/` fire only when BOTH operands reduce to units
                // (a `(* 2 3)` numeric multiply has non-unit operands → falls through to `None` here and
                // is handled as arithmetic elsewhere); `^` (arena `BitXor`) is unit-power when the base is
                // a unit and the exponent a compile-time int. This is the same "dispatch by operand kind,
                // not by name" the quantity operators use.
                Prim::Mul if args.len() == 2 => {
                    let a = unit_of(db, *args.first()?)?;
                    let b = unit_of(db, *args.get(1)?)?;
                    Some(a.mul(&b))
                }
                Prim::Div if args.len() == 2 => {
                    let a = unit_of(db, *args.first()?)?;
                    let b = unit_of(db, *args.get(1)?)?;
                    Some(a.div(&b))
                }
                Prim::BitXor if args.len() == 2 => {
                    let a = unit_of(db, *args.first()?)?;
                    let n = match resolved_of(db, *args.get(1)?) {
                        Resolved::Int(v) => v.to_i64()?,
                        _ => return None,
                    };
                    Some(a.pow(n))
                }
                Prim::UnitPrefix => {
                    // `(Unit.prefix kilo u)` — read the prefix's `(num den)` scale off its `(meta scale)`
                    // channel and apply it to `u`. The first arg is the prefix record, the second the
                    // unit being scaled.
                    let (num, den) = prefix_scale_of(db, *args.first()?)?;
                    let u = unit_of(db, *args.get(1)?)?;
                    u.scaled(num, den)
                }
                Prim::UnitOf => {
                    // `(Unit.of #"foot")` / `(Unit.of #"mbps")` / `(Unit.of #"furlong")` — the family-unit
                    // name is the symbol's text. Consult the built-in family registry for its DIMENSION (a
                    // `(base, exponent)` list — atomic or derived) + scale, compose the dimension via
                    // `base`/`mul`/`pow`, then `scaled(num, den)`. A name NOT in the built-in table may be
                    // a USER `Unit.define` — reduce its base-unit expression + apply the declared scale.
                    // An unknown name fails to reduce (declines).
                    let name = match resolved_of(db, *args.first()?) {
                        Resolved::SymbolConst(s) | Resolved::Str(s) => s,
                        _ => return None,
                    };
                    if let Some((dim_pairs, num, den)) = db.unit_families.get(&name).cloned() {
                        let mut dim = Unit::one();
                        for (base, exp) in &dim_pairs {
                            dim = dim.mul(&Unit::base(base.clone()).pow(*exp));
                        }
                        return dim.scaled(num, den);
                    }
                    // A user-declared family unit: `(Unit.define #"furlong" base num den)` — the unit is
                    // `base` (reduced) scaled by `num/den`. First matching declaration wins (a conflicting
                    // one is CDZ0502, reported by the checker).
                    let def = db
                        .unit_defines
                        .iter()
                        .find(|(n, _, _, _)| *n == name)
                        .map(|(_, base, num, den)| (*base, *num, *den))?;
                    let (base_occ, num, den) = def;
                    let base_unit = unit_of(db, base_occ)?;
                    base_unit.scaled(num, den)
                }
                Prim::UnitDefine => {
                    // `(Unit.define #"furlong" base num den)` as a VALUE reduces to the defined unit:
                    // `base` (reduced) scaled by `num/den`. Its declaration effect (registering the name)
                    // is the load-time scan; here it is just the unit it denotes.
                    let base = unit_of(db, *args.get(1)?)?;
                    let num = match resolved_of(db, *args.get(2)?) {
                        Resolved::Int(v) => v.to_i128()?,
                        _ => return None,
                    };
                    let den = match resolved_of(db, *args.get(3)?) {
                        Resolved::Int(v) => v.to_i128()?,
                        _ => return None,
                    };
                    base.scaled(num, den)
                }
                // `Qty.unit q` — the UNIT of `q`, read off its solved `Ty::Qty`. This is the inverse of
                // `Qty.of`'s unit argument: a quantity's unit, recovered as a first-class unit value so
                // `(Qty.of new (Qty.unit y))` reuses `y`'s unit without re-spelling it. The eval→infer
                // call is the established direction (the operator/width arms already read `type_of`); a
                // non-quantity argument (its type is not `Ty::Qty`) is not a unit expression → declines.
                Prim::QtyUnit => match crate::infer::type_of(db, *args.first()?) {
                    crate::ty::Ty::Qty { unit, .. } => Some(unit),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

/// The VALUE-argument occurrence of a quantity operand — the `v` in `(Qty.of v u)` — for synthesizing a
/// runtime unit conversion over it (the erased magnitude). `None` if `id` is not a `Qty.of` application
/// (a quantity that isn't a literal construction — e.g. a param-bound one — has no value occurrence to
/// reuse; the runtime-conversion caller declines it). The unit-layer analogue of reaching a `Qty.of`'s
/// first argument.
pub fn qty_value_occ(db: &mut Db, id: StructId) -> Option<StructId> {
    match resolved_of(db, id) {
        Resolved::Ref { value } => qty_value_occ(db, value),
        Resolved::Apply { head, args } => {
            if meta_apply_of(db, head) == Some(Prim::QtyOf) && args.len() == 2 {
                Some(args[0])
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The exact scale factor `(num, den)` a PREFIX record carries on its `(meta scale)` channel
/// (`prelude::prefix_record`) — `kilo` → `(1000, 1)`, `milli` → `(1, 1000)`, `mebi` → `(1048576, 1)`.
/// Read the same way `variant_disc_of` reads `(meta variant)`. `None` if `id` is not a prefix record
/// (no `(meta scale)`, or a malformed ratio) — so `(Unit.prefix notaprefix u)` fails to reduce cleanly.
fn prefix_scale_of(db: &mut Db, id: StructId) -> Option<(i128, i128)> {
    let field = project_meta(db, id, "scale")?;
    // `(num den)` — a plain two-element list of integer literals.
    let (n, d) = match db.ast.get(field) {
        crate::ast::Struct::List(items) if items.len() == 2 => (items[0], items[1]),
        _ => return None,
    };
    let num = match resolved_of(db, n) {
        Resolved::Int(v) => v.to_i128()?,
        _ => return None,
    };
    let den = match resolved_of(db, d) {
        Resolved::Int(v) => v.to_i128()?,
        _ => return None,
    };
    Some((num, den))
}

/// How a `(Int W)`/`(UInt W)` width argument reads. A concrete non-negative natural that fits `u32` is
/// `Fixed`. A concrete but NON-NATURAL width — a negative or over-`u32` integer, or a bool/float/type-
/// value in width position — is `Malformed`. A width that is not a concrete value at all — an unbound
/// name, a runtime parameter, or a non-constant computation — is `NotConst`. Both `Malformed` and
/// `NotConst` are INVALID widths: an integer type MUST be indexed by a compile-time natural
/// (numeric-model.md §An Integer Type Is Indexed By A Compile-Time Width), so the caller reduces either
/// to the sentinel width 0 → CDZ0302, rejecting the bad width at the annotation rather than dropping it.
/// (The variants stay distinct so a future caller could message them differently; `reduce_ctor` treats
/// both as reject.)
enum WidthRead {
    Fixed(u32),
    Malformed,
    NotConst,
}

/// Classify the width argument of `(Int W)`/`(UInt W)`. The old reader narrowed with `u32::try_from`
/// and returned `None` for anything that failed — which silently DROPPED a negative/non-natural width
/// (the annotation was ignored and the literal kept its default Int64) instead of rejecting it. This
/// distinguishes a MALFORMED concrete width (reject) from a NON-CONSTANT one (decline), so the caller
/// can reject the former with CDZ0302 exactly as `(UInt 0)` already is.
fn read_width(db: &mut Db, id: StructId) -> WidthRead {
    match resolved_of(db, id) {
        // A concrete integer literal: a non-negative natural that fits `u32` is the width. A negative
        // or over-`u32` literal is a malformed (non-natural) width.
        Resolved::Int(v) => match v.to_i64() {
            Some(n) if (0..=i64::from(u32::MAX)).contains(&n) => WidthRead::Fixed(n as u32),
            _ => WidthRead::Malformed,
        },
        Resolved::Ref { value } => read_width(db, value),
        // A concrete NON-integer value in width position (a Bool, a type-value, a type/module record)
        // is a non-natural width — reject, don't drop.
        Resolved::Bool(_) | Resolved::TypeVal(_) | Resolved::Record { .. } => WidthRead::Malformed,
        // A float LITERAL reaches here as a decline-poison (floats are unsupported), but a concrete
        // float in width position is still a non-natural width → malformed. Anything else (an unbound
        // name, a width variable, a non-constant computation) is not a concrete width → decline.
        _ => match db.ast.get(id) {
            crate::ast::Struct::Atom(lid)
                if matches!(db.ast.leaf(*lid), crate::ast::Leaf::Float(_)) =>
            {
                WidthRead::Malformed
            }
            _ => WidthRead::NotConst,
        },
    }
}

/// Whether the type expression at `id` is a width-indexed numeric type — `(Int W)`/`(UInt W)` or
/// `(Float W)` — whose WIDTH is RUNTIME DATA (a function PARAMETER, or a reference chain reaching one).
/// Such a width puts a runtime value in a type-determining position, which the type system forbids (a
/// width must be a compile-time value, `numeric-model.md §An Integer Type Is Indexed By A Compile-Time
/// Width` / §A Floating-Point Type Is Indexed By A Compile-Time Width). Checked on the UN-INLINED
/// annotation body so `(def (mk n) (: 5 (UInt n)))` rejects even though a constant call site would fold
/// `n` — the width is non-dependent by declaration, not by how it is called. Only a genuine
/// runtime-value width is flagged; a constant width, an unbound name, or a not-yet-built thing is not
/// (those are handled elsewhere — a malformed/out-of-set width by `read_width`, an unbound name by scope).
/// A `(Float n)` runtime width MUST reach here: the admitted-set check (`is_ill_formed_float_width`) reads
/// `read_width`, which yields `NotConst` for a runtime `n` and so does NOT flag it — without this predicate
/// a runtime float width would reduce to the sentinel `Float0` and slip past `cdz check` (PR #425).
pub fn is_runtime_width_type(db: &mut Db, id: StructId) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return false;
    };
    match meta_apply_of(db, head) {
        Some(Prim::IntCtor | Prim::UIntCtor | Prim::FloatCtor) if args.len() == 1 => {
            width_is_runtime(db, args[0])
        }
        _ => false,
    }
}

/// Whether the type expression at `id` is a `(Float W)` (the `FloatCtor` applied to one argument),
/// regardless of what the width is. Lets a runtime-width diagnostic name the FLOAT axis (admitted IEEE
/// widths) rather than the integer one when the offending type is a float. A companion of
/// `is_runtime_width_type`, which admits `FloatCtor` alongside the integer ctors.
pub fn is_float_ctor_type(db: &mut Db, id: StructId) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return false;
    };
    meta_apply_of(db, head) == Some(Prim::FloatCtor) && args.len() == 1
}

/// Whether a width expression resolves to a RUNTIME value — a function parameter (through any `Ref`
/// chain). A constant literal, an unbound name, or anything else is not runtime data.
fn width_is_runtime(db: &mut Db, id: StructId) -> bool {
    match resolved_of(db, id) {
        Resolved::Param { .. } => true,
        Resolved::Ref { value } => width_is_runtime(db, value),
        _ => false,
    }
}

/// If the type expression at `id` is `(Int W)`/`(UInt W)` with a CONCRETE width OUTSIDE the admitted
/// `1..=64` range — an over-ceiling `(UInt 65)`/`(UInt 128)` or a zero `(UInt 0)` — return
/// `(signed, original_width)`. `reduce_ctor` clamps such a width to the sentinel 0, so the built `Ty` is
/// `Int0` and its own name no longer reveals what was written; this reads the ORIGINAL width off the
/// annotation for a diagnostic that names it ("width 65 exceeds the 64-bit ceiling") instead of the
/// misleading clamped "UInt0". `None` for an in-range width, a non-concrete width (an unbound name, a
/// runtime parameter — those are diagnosed by scope / `is_runtime_width_type`), or a non-integer-ctor
/// type. Keyed on the constructor prim + `read_width`, so no width literal is special-cased here.
pub fn out_of_range_int_width(db: &mut Db, id: StructId) -> Option<(bool, u32)> {
    match int_width_fault(db, id) {
        Some(IntWidthFault::OverCeiling { signed, width }) => Some((signed, width)),
        _ => None,
    }
}

/// The ill-formed-width verdict for an integer type `(Int W)`/`(UInt W)` — the TOTAL classification the
/// well-formedness checks need. `OverCeiling` is a concrete natural width outside `1..=64` (`(UInt 65)`,
/// `(UInt 0)`) — it names the written width for the diagnostic. `Malformed` is a NON-NATURAL width: a
/// NEGATIVE literal (`(Int -8)`), or a non-number in width position (a Bool `(Int true)`, a float, a
/// type-value) — there is no width number to name, only the signedness. `read_width` already distinguishes
/// these (`WidthRead::Fixed` vs `WidthRead::Malformed`); the old `out_of_range_int_width` collapsed the
/// Malformed case to `None`, so a negative/non-natural width SLIPPED PAST `cdz check` (silently exit 0)
/// while each backend caught it independently at selection — a check-vs-emit gap. Well-formedness is TOTAL
/// (numeric-model.md §"A bit width outside the range the model admits MUST be rejected at compile time"),
/// so both the value- and parameter-annotation checks route through this so `check` rejects it and BOTH
/// backends inherit it. A NON-CONSTANT / runtime width (`WidthRead::NotConst`) is `None` here — handled by
/// `is_runtime_width_type` / scope. Keyed on the ctor prim + `read_width`; no width literal special-cased.
pub enum IntWidthFault {
    OverCeiling { signed: bool, width: u32 },
    Malformed { signed: bool },
}

pub fn int_width_fault(db: &mut Db, id: StructId) -> Option<IntWidthFault> {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return None;
    };
    let signed = match meta_apply_of(db, head) {
        Some(Prim::IntCtor) => true,
        Some(Prim::UIntCtor) => false,
        _ => return None,
    };
    if args.len() != 1 {
        return None;
    }
    match read_width(db, args[0]) {
        WidthRead::Fixed(w) if !(1..=64).contains(&w) => {
            Some(IntWidthFault::OverCeiling { signed, width: w })
        }
        WidthRead::Malformed => Some(IntWidthFault::Malformed { signed }),
        // An in-range fixed width, or a non-constant (runtime/unbound) width, is not an ill-formed width
        // here (the latter is diagnosed by `is_runtime_width_type` / scope).
        WidthRead::Fixed(_) | WidthRead::NotConst => None,
    }
}

/// If the type expression at `id` is a WIDTH constructor `(Int W)`/`(UInt W)`/`(Float W)` whose single
/// width argument `W` is an UNBOUND NAME (resolves to `Poison(Unbound)`), return that argument's node.
/// This is the width-position analogue of an unbound TYPE name: `(: a (Int hello))` is the newcomer reflex
/// of putting a name where a width literal belongs, but unlike `(List hello)` (a type position, caught by
/// the nested-type-var walk) the WIDTH position is not a type, so it slips past both that walk and
/// `int_width_fault` — `read_width` reads an unbound name as `WidthRead::NotConst` (indistinguishable from
/// a legitimately BOUND width variable like `(Int a)` in a generic), and the fault classifier returns
/// `None`, so the annotation silently reduces to the sentinel `Int0` and `cdz check` exits 0. Here we
/// separate the two: only a `Poison(Unbound)` arg is a fault; a `Resolved::Param`/`Ref` (a bound width
/// var) is a VALID generic width and returns `None`. Keyed on the ctor prim; a compound `(Int 64)` literal
/// width or a valid type never matches. Consumed by the annotation checks to surface a width-specific
/// CDZ0101 instead of nothing.
pub fn unbound_width_arg(db: &mut Db, id: StructId) -> Option<(StructId, &'static str)> {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return None;
    };
    // The ctor-appropriate sized-type EXAMPLE for the diagnostic — a `Float` width names `Float64`, not the
    // misleading `Int64`.
    let example = match meta_apply_of(db, head) {
        Some(Prim::IntCtor) => "Int64",
        Some(Prim::UIntCtor) => "UInt64",
        Some(Prim::FloatCtor) => "Float64",
        _ => return None,
    };
    if args.len() != 1 {
        return None;
    }
    match resolved_of(db, args[0]) {
        Resolved::Poison(reject) if reject.code == Some(crate::diag::Code::Unbound) => {
            Some((args[0], example))
        }
        _ => None,
    }
}

/// If the type expression at `id` is a `(Float W)` with a CONCRETE width OUTSIDE the admitted IEEE set
/// `{32, 64}` — a `(Float 8)` / `(Float 16)` / `(Float 128)`, or a non-natural width `(Float -8)` /
/// `(Float true)` — return `true`. The float analogue of `int_width_fault`, but the admitted set is a
/// MEMBERSHIP (`ADMITTED_FLOAT_WIDTHS`) not a range, and every ill-formed float width shares ONE message
/// ("must be one of the admitted IEEE widths (32 or 64)"), so this is a bool rather than a fault enum. A
/// NON-CONSTANT / runtime width (`WidthRead::NotConst`) is `false` here — `is_runtime_width_type`-style
/// paths and the sentinel-0 reduction already reject a runtime float width at the annotation. Keyed on the
/// `FloatCtor` prim + `read_width`; no width literal special-cased. Used to descend a COMPOUND annotation
/// (`(List (Float 8))`) where the reduced container type looks well-formed and the bad width would
/// otherwise slip past `cdz check` — the check-vs-emit gap the bare `(Float 8)` reject already closes.
pub fn is_ill_formed_float_width(db: &mut Db, id: StructId) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return false;
    };
    if meta_apply_of(db, head) != Some(Prim::FloatCtor) || args.len() != 1 {
        return false;
    }
    match read_width(db, args[0]) {
        WidthRead::Fixed(w) => !crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&w),
        WidthRead::Malformed => true,
        WidthRead::NotConst => false,
    }
}

/// The CONCRETE natural width of an ill-formed `(Float W)` at `id` — `Some(w)` when `W` reduces to a fixed
/// natural OUTSIDE the admitted IEEE set `{32, 64}` (a `(Float 8)` / `(Float 16)` / `(Float 128)`), and
/// `None` for an admitted width, a MALFORMED (negative / non-numeric) width (no width number to name), a
/// non-constant width, or a non-`(Float _)` type. The companion of [`is_ill_formed_float_width`] that also
/// hands back the number, so a diagnostic can choose a repair target (snap to the nearest admitted width)
/// — the float analogue of `IntWidthFault::OverCeiling`'s width. Keyed on the ctor prim + `read_width`.
pub fn out_of_set_float_width(db: &mut Db, id: StructId) -> Option<u32> {
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return None;
    };
    if meta_apply_of(db, head) != Some(Prim::FloatCtor) || args.len() != 1 {
        return None;
    }
    match read_width(db, args[0]) {
        WidthRead::Fixed(w) if !crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&w) => Some(w),
        WidthRead::Fixed(_) | WidthRead::Malformed | WidthRead::NotConst => None,
    }
}

/// Encode a `Ty` as a `(typeval …)` arena node the resolver decodes back to `Resolved::TypeVal`.
/// Compact and total over the type shapes the evaluator builds (integers, bool, unit, function).
pub fn encode_typeval(db: &mut Db, ty: &crate::ty::Ty) -> StructId {
    let head = db.push_name("typeval");
    let payload = encode_ty(db, ty);
    db.push_list(vec![head, payload])
}

/// Encode a `Ty` into its BARE cdzast sub-AST — the structural payload alone (`(-> p r)` / `(List
/// elem)` / `(Record (field T)…)` / scalar), WITHOUT the `(typeval …)` wrapper `encode_typeval` adds.
/// For a consumer that already tags the type by POSITION and wants the clean structural subtree —
/// cadenza-docs I2 attaches `(ty <this>)` to each doc-item (the `(ty …)` head already marks it a type,
/// so the `(typeval …)` value-encoding wrapper would be redundant noise: `(ty (-> …))`, not `(ty
/// (typeval (-> …)))`). Same total encoding + canonical head-names as `encode_typeval`'s payload (they
/// share `encode_ty`), so it round-trips through `resolve::decode_ty` identically. (v-syntax hand-off,
/// cadenza-docs I2 resolved-type projection.)
pub fn encode_ty_payload(db: &mut Db, ty: &crate::ty::Ty) -> StructId {
    encode_ty(db, ty)
}

/// Encode a `Ty` into an arena subtree (the payload of a `(typeval …)` node). The dual `decode_ty`
/// lives in `resolve` where a `(typeval …)` node is read.
fn encode_ty(db: &mut Db, ty: &crate::ty::Ty) -> StructId {
    use crate::ty::Ty;
    match ty {
        Ty::Bool => db.push_name("Bool"),
        Ty::Unit => db.push_name("Unit"),
        Ty::Int(it) => {
            let ctor = db.push_name(if it.ground_signed() { "Int" } else { "UInt" });
            let w = match it.width {
                Width::Fixed(w) => w as i64,
                // A deferred/var width in a built type grounds to the default for encoding.
                _ => crate::ty::DEFAULT_INT_WIDTH as i64,
            };
            let width = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(w),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![ctor, width])
        }
        Ty::Fn(p, r) => {
            let arr = db.push_name("->");
            let pe = encode_ty(db, p);
            let re = encode_ty(db, r);
            db.push_list(vec![arr, pe, re])
        }
        // A tuple: `(Tuple <elem>…)` — the head then each element type encoded in order.
        Ty::Tuple(elems) => {
            let mut items = vec![db.push_name("Tuple")];
            for e in elems.iter() {
                items.push(encode_ty(db, e));
            }
            db.push_list(items)
        }
        // A record type-value: `(Record (: name T)…)` in canonical (sorted) field order — the capitalized
        // `Record` head (the TYPE; the VALUE head is lowercase `record`), each field the canonical
        // `(: name T)` ASCRIPTION node (RT3, DESIGN-record-type-syntax) — the shared binder node, not a
        // bespoke pair. Round-trips with `decode_ty`'s `"Record"` arm (which reads the ascription); the
        // `BTreeMap` iteration IS the canonical order.
        Ty::Record(fields) => {
            let mut items = vec![db.push_name("Record")];
            for (name, t) in fields.iter() {
                let colon = db.push_name(":");
                let fname = db.push_name(&name.name);
                let fty = encode_ty(db, t);
                items.push(db.push_list(vec![colon, fname, fty]));
            }
            db.push_list(items)
        }
        // A sum type-value: `(Sum <name> <decl> arg…)` — the nominal name (for rendering), the
        // declaration occurrence (the identity, an integer literal so it round-trips), and the type ARGS
        // (empty for a monomorphic sum). Round-trips with `resolve::decode_ty`'s `"Sum"` arm. This is the
        // shape `sums::synthesize` also builds for a sum record's `(meta t)`, so the two must agree.
        Ty::Sum { decl, args, .. } => {
            // NAME is recovered from `decl` (no longer carried on the type) so the wire format
            // `(Sum NAME <decl> arg…)` stays byte-identical — `decode_ty` reads+discards it. Look it up
            // (immutable borrow) into an owned string before the `push_name` mutable borrow.
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "<sum>".to_string());
            let head = db.push_name("Sum");
            let nm = db.push_name(&name);
            let d = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(decl.0 as i64),
                radix: crate::ast::Radix::Dec,
            });
            // `(Sum NAME <decl> arg…)` — the type ARGS follow, so a generic instantiation round-trips
            // (a monomorphic sum encodes no args, byte-identical to before).
            let mut items = vec![head, nm, d];
            for a in args.iter() {
                items.push(encode_ty(db, a));
            }
            db.push_list(items)
        }
        // A nominal type-value: `(Nominal <name> <decl> (args…) <inner>)` — the declared name (render),
        // the declaration occurrence (identity, an integer literal), the type ARGS (the instantiation, in
        // an `(args …)` group — empty for a monomorphic/recursive nominal), and the encoded UNDERLYING
        // type (the machine-rep hint). Round-trips with `resolve::decode_ty`'s `"Nominal"` arm. Without
        // this arm the catch-all encoded a `Ty::Nominal` as `Unit`, so a ctor arrow `(-> Int64 UserId)`
        // round-tripped to `(-> Int64 Unit)` — the newtype vanished.
        Ty::Nominal {
            decl, args, inner, ..
        } => {
            // NAME recovered from `decl` so the wire `(Nominal NAME <decl> (args…) <inner>)` is unchanged.
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "<nominal>".to_string());
            let head = db.push_name("Nominal");
            let nm = db.push_name(&name);
            let d = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(decl.0 as i64),
                radix: crate::ast::Radix::Dec,
            });
            let args_head = db.push_name("args");
            let mut args_items = vec![args_head];
            for a in args.iter() {
                args_items.push(encode_ty(db, a));
            }
            let args_node = db.push_list(args_items);
            let i = encode_ty(db, inner);
            db.push_list(vec![head, nm, d, args_node, i])
        }
        // A list type-value: `(List <elem>)` — the head then the element type. Round-trips with
        // `decode_ty`'s `"List"` arm.
        Ty::List(elem) => {
            let head = db.push_name("List");
            let e = encode_ty(db, elem);
            db.push_list(vec![head, e])
        }
        // A map type-value: `(Map <key> <value>)` — the head then the two type arguments (key first).
        // Round-trips with `decode_ty`'s `"Map"` arm.
        Ty::Map(k, v) => {
            let head = db.push_name("Map");
            let ke = encode_ty(db, k);
            let ve = encode_ty(db, v);
            db.push_list(vec![head, ke, ve])
        }
        // A set type-value: `(Set <elem>)` — the head then the element type. Round-trips with `decode_ty`'s
        // `"Set"` arm.
        Ty::Set(elem) => {
            let head = db.push_name("Set");
            let e = encode_ty(db, elem);
            db.push_list(vec![head, e])
        }
        // A bytes type-value: the bare name `Bytes` (a leaf). Round-trips with `decode_ty`'s `"Bytes"`
        // arm. Without this the catch-all below encoded it as `Unit`, so a `(-> … Bytes)` scheme
        // round-tripped to `(-> … Unit)` and `Bytes.of`/`Bytes.len` mis-typed.
        Ty::Bytes => db.push_name("Bytes"),
        // A string type-value: the bare name `String` (a leaf), the exact `Bytes` analogue. Round-trips
        // with `decode_ty`'s `"String"` arm. Without it the catch-all encoded `String` as `Unit`, so a
        // `(-> String Sum)` variant-constructor scheme round-tripped to `(-> Unit Sum)` — a `String`-payload
        // variant `(type Tag (Named String) …)` then unified its `"x"` argument against `Unit` ("cannot
        // unify Unit with String"). The same round-trip hole the `Bytes` arm above fixed for bytes.
        Ty::String => db.push_name("String"),
        // `Char`/`Symbol` — the remaining monomorphic leaves, the exact `Bytes`/`String` analogue.
        // Round-trip with `decode_ty`'s `"Char"`/`"Symbol"` arms. Without these the catch-all below
        // encoded them as `Unit`, so a `Char`/`Symbol` nested in a compound type-value (a `(Tuple Char …)`
        // element, or a variant payload — which boxes as a tuple) collapsed to `Unit` and a `(Ch Char)`
        // variant's `#\a` argument then unified against `Unit` ("Char and Unit must be the same type").
        Ty::Char => db.push_name("Char"),
        Ty::Symbol => db.push_name("Symbol"),
        // The arbitrary-precision integer: the bare name `BigInt` (a monomorphic leaf, the `String`
        // analogue). Round-trips with `decode_ty`'s `"BigInt"` arm; without it the catch-all would
        // encode it as `Unit` (the same round-trip hole the `Bytes`/`String`/`Char`/`Symbol` arms fix).
        Ty::BigInt => db.push_name("BigInt"),
        // The exact rational: the bare name `Rational` (a monomorphic leaf, like `BigInt`). Round-trips
        // with `decode_ty`'s `"Rational"` arm; without it the catch-all would encode it as `Unit`.
        Ty::Rational => db.push_name("Rational"),
        // A float type-value: the `(Float N)` HEAD form (like `(Int N)`), so the WIDTH round-trips
        // FAITHFULLY through `decode_ty`'s `(Float N)` arm — INCLUDING the sentinel width 0 a
        // non-admitted `(Float 16)` reduces to (a bare alias name would lose it: width 0 has no alias,
        // so it would collapse to `Float64` and slip past the admitted-set check). The admitted-set
        // membership is enforced at the annotation (`infer`) + the constructor (`reduce_ctor`), not by
        // dropping the width here.
        Ty::Float(ft) => {
            let ctor = db.push_name("Float");
            let w = match ft.width {
                Width::Fixed(w) => w as i64,
                _ => crate::ty::DEFAULT_FLOAT_WIDTH as i64,
            };
            let width = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(w),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![ctor, width])
        }
        // A quantity type-value: `(Qty <inner-ty> (unit (base <name> <exp>)…))` — the inner numeric type
        // then the unit encoded as a canonical `(unit …)` node listing each base's `(base NAME EXP)` pair
        // in sorted order (the dimensionless unit is the empty `(unit)`). Round-trips with
        // `resolve::decode_ty`'s `"Qty"` arm. WARNING: WITHOUT this arm the catch-all below would encode a
        // quantity as `Unit`, so a `(-> T (Qty T u))` scheme (Qty.of) would round-trip to `(-> T Unit)`
        // and mis-type — the same round-trip hole the `Bytes`/`String` arms fixed for those leaves.
        Ty::Qty { inner, unit } => {
            let head = db.push_name("Qty");
            let inner_node = encode_ty(db, inner);
            let mut unit_items = vec![db.push_name("unit")];
            for (name, exp) in unit.entries() {
                let base = db.push_name("base");
                let nm = db.push_name(name);
                let e = db.push_atom(Leaf::Int {
                    value: IntValue::from_i64(*exp),
                    radix: crate::ast::Radix::Dec,
                });
                unit_items.push(db.push_list(vec![base, nm, e]));
            }
            // Encode the SCALE ratio to the dimension's reference unit — a `(scale NUM DEN)` item — so a
            // NON-reference unit (`kilometer` = 1000/1, `foot` = 381/1250, `KiB` = 1024/1) survives the
            // encode→decode round-trip a TYPE-VALUE takes (e.g. a `(: q (Qty Int64 (Unit.prefix kilo …)))`
            // parameter annotation, whose Qty type is encoded then `decode_ty`'d back). WITHOUT this the
            // decode rebuilt the unit at scale 1/1 (the reference), so a scaled-unit param silently became
            // its reference unit → a cross-scale combine/comparison with it computed on RAW magnitudes with
            // no conversion (`quantity_scales_differ` saw equal scales) — a silent miscompile (`(+ (a: Qty
            // km) (b: Qty m))` with a=2,b=500 gave 502, not 2500). Emitted ONLY for a non-1/1 scale so the
            // common reference-unit form stays byte-identical (decode defaults a missing `(scale …)` to 1/1).
            let (sn, sd) = unit.scale();
            if (sn, sd) != (1, 1) {
                let scale_head = db.push_name("scale");
                let n = db.push_atom(Leaf::Int {
                    value: IntValue::from_i128(sn),
                    radix: crate::ast::Radix::Dec,
                });
                let d = db.push_atom(Leaf::Int {
                    value: IntValue::from_i128(sd),
                    radix: crate::ast::Radix::Dec,
                });
                unit_items.push(db.push_list(vec![scale_head, n, d]));
            }
            let unit_node = db.push_list(unit_items);
            db.push_list(vec![head, inner_node, unit_node])
        }
        // A TYPE VARIABLE — `(Var N)`, the variable's number. It reaches a built type-value when a
        // TYPE-VALUED PARAMETER is used INSIDE a compound type that takes the `reduce_ctor`→`encode_typeval`
        // round-trip: the arrow `(-> t Int64)` builds `Ty::Fn(Var(t), Int64)`, and `FnCtor` has no direct
        // fast path (unlike List/Set/Map/Tuple/Record), so the whole `Ty` is encoded — WITHOUT this arm the
        // catch-all below stubbed `Var` as `Unit`, so `t` collapsed and the def scheme read `(-> (-> Unit
        // Int64) …)` instead of `(-> (-> a Int64) …)`. The domain then defaulted to `Unit` and a real
        // argument (`(fn (n) n) : (-> Int64 Int64)`) failed CDZ0203 "expected (-> Unit Int64)". A bare `t`
        // (`typeval_of` returns `Ty::Var` directly, never encoded) and `(List t)` (a direct fast path) were
        // unaffected — only a param under an ARROW hit this hole. Round-trips with `decode_ty`'s `"Var"` arm.
        Ty::Var(n) => {
            let head = db.push_name("Var");
            let num = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(*n as i64),
                radix: crate::ast::Radix::Dec,
            });
            db.push_list(vec![head, num])
        }
        // The KIND-OF-TYPES `Ty::Type` — a first-class type value (`(: t Type)`'s `Type`). It reaches a
        // built type-value the SAME way `Ty::Var` does: inside a compound that takes the
        // `reduce_ctor`→`encode_typeval` round-trip. `(: g (-> Type Int64))` builds `Ty::Fn(Ty::Type,
        // Int64)`, and `FnCtor` has no direct fast path, so the whole `Ty` is encoded — WITHOUT this arm the
        // catch-all below stubbed `Type` as `Unit`, so the def scheme read `(-> (-> Unit Int64) …)` instead
        // of `(-> (-> Type Int64) …)`, and passing a real `(-> Type Int64)` closure failed CDZ0203
        // "argument is a Type, but a value of type Unit is expected". The exact `Type` twin of the `Var`
        // round-trip hole (same class as Bytes/String/Char/Symbol/Qty/Nominal). A bare `(: t Type)`
        // (`typeval_of` returns `Ty::Type` directly, never encoded) was unaffected — only `Type` under a
        // compound (an arrow domain/result) hit this. Round-trips with `decode_ty`'s `"Type"` arm.
        Ty::Type => db.push_name("Type"),
        // Any shouldn't reach a built type-value in Milestone A; encode Unit as a safe stub.
        _ => db.push_name("Unit"),
    }
}

/// Build the integer MODULE record for `(signed, width)` by appending arena nodes, returning its
/// occurrence. The module is a record whose `(meta t)` holds the concrete integer type-value and whose
/// fields carry the width's specialization: `max`/`min` as the width's bounds. (Operations like `+`
/// join it width-specialized in Milestone B; Milestone A pins the type + bounds so `(. (Int 64) max)`
/// folds through the ordinary projection.)
pub fn build_int_module(db: &mut Db, signed: bool, width: u32) -> StructId {
    let it = IntTy::fixed(signed, width);
    let ty = crate::ty::Ty::Int(it);

    let mut fields: Vec<StructId> = Vec::new();

    // `(meta t)` — the concrete integer type-value.
    let ty_node = encode_typeval(db, &ty);
    fields.push(meta_field(db, "t", ty_node));

    // `max`/`min` — the width's bounds, computed from the WIDTH PARAMETER (not a per-type table), at
    // arbitrary precision so `UInt64.max = 2^64 - 1` (which exceeds `i64`) is exact. Any width the
    // constructor admits (1..=64, including odd widths like `(UInt 7)`/`(UInt 24)`) yields that width's
    // bounds. A width the fold can't represent leaves the bound unrealized (declines when projected).
    match int_bounds(signed, width) {
        Some((max, min)) => {
            fields.push(named_int_field(db, "max", max, signed, width));
            fields.push(named_int_field(db, "min", min, signed, width));
        }
        None => {
            fields.push(unrealized_field(db, "max"));
            fields.push(unrealized_field(db, "min"));
        }
    }

    // `wrap` — the truncating conversion INTO this width, built identically to the prelude's named-width
    // modules (`prelude::wrap_field`) so `(Int N).wrap` and `Int8.wrap` denote the same operation. Its
    // `(meta t)` is `(fn (a) (-> (Int a) TARGET))` (generic source, this width as target); `(meta apply)`
    // the shared `wrap` intrinsic.
    fields.push(wrap_field(db, signed, width));

    // The record head is the NATIVE ctor-LEAF `Leaf::Ctor(Record)` (the ordinary NAME `record` is a
    // shadowable prelude alias). A compiler-synthesized module record builds directly with the ctor-leaf
    // head so it resolves structurally to `Resolved::Record`, independent of any user binding of the name
    // `record` — unshadowable exactly like the legacy string head it replaces (dual-read).
    let head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];
    children.append(&mut fields);
    db.push_list(children)
}

/// Build a fixed-width FLOAT MODULE record for `width` — the constructor-side twin of
/// `prelude::float_module_record` (the two MUST build the same shape so `(Float N)` and the named-width
/// `Float64` are one module). Carries `(meta t)` = the concrete float type-value + the `of-int`
/// operation (`Int64 → (Float width)`, the `float-of-int` intrinsic). A `width` of 0 (the sentinel a
/// NON-admitted `(Float N)` reduces to) yields `Ty::Float(Fixed(0))`, which the annotation check rejects
/// CDZ0302.
pub fn build_float_module(db: &mut Db, width: u32) -> StructId {
    let ty = crate::ty::Ty::Float(crate::ty::FloatTy::fixed(width));
    let ty_node = encode_typeval(db, &ty);
    let of_int = float_of_int_field(db, width);
    let of = float_of_field(db, width);
    let fields = vec![meta_field(db, "t", ty_node), of_int, of];
    let head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];
    for f in fields {
        children.push(f);
    }
    db.push_list(children)
}

/// The module's `of-int` field, appended on `&mut Db` — the constructor-side twin of
/// `prelude::float_of_int_type` + its op record (the two MUST build the same shape so `(Float N).of-int`
/// and `Float64.of-int` are one operation). `(of-int (record ((meta t) (fn () (-> Int64 (Float width))))
/// ((meta apply) (intrinsic float-of-int))))`.
fn float_of_int_field(db: &mut Db, width: u32) -> StructId {
    // `(fn () (-> Int64 (Float width)))` — the zero-param scheme wrapper (so `scheme_of` reads a SCHEME,
    // not a bare type-value).
    let int64 = db.push_name("Int64");
    let target = {
        let ctor = db.push_name("Float");
        let w = db.push_atom(Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![ctor, w])
    };
    let arr = db.push_name("->");
    let body = db.push_list(vec![arr, int64, target]);
    let fn_head = db.push_name("fn");
    let params = db.push_list(vec![]);
    let scheme = db.push_list(vec![fn_head, params, body]);
    let t_field = meta_field(db, "t", scheme);
    let apply = {
        let head = db.push_name("intrinsic");
        let who = db.push_name("float-of-int");
        db.push_list(vec![head, who])
    };
    let apply_field = meta_field(db, "apply", apply);
    let rec_head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
    let rec = db.push_list(vec![rec_head, t_field, apply_field]);
    let k = db.push_name("of-int");
    let eq = db.push_name("="); // seq-276: canonical (= name value) record entry
    db.push_list(vec![eq, k, rec])
}

/// The module's `of` field, appended on `&mut Db` — the constructor-side twin of `prelude::float_of_type`
/// plus its op record (the two MUST build the same shape so `(Float N).of` and `Float64.of` are one op).
/// Builds `(of (record ((meta t) (fn (a) (-> (Float a) (Float width)))) ((meta apply) (intrinsic
/// float-of))))` — GENERIC over the source float width `a`, this module's `width` as the target.
fn float_of_field(db: &mut Db, width: u32) -> StructId {
    let float_a = {
        let ctor = db.push_name("Float");
        let a = db.push_name("a");
        db.push_list(vec![ctor, a])
    };
    let target = {
        let ctor = db.push_name("Float");
        let w = db.push_atom(Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![ctor, w])
    };
    let arr = db.push_name("->");
    let body = db.push_list(vec![arr, float_a, target]);
    let fn_head = db.push_name("fn");
    let a_param = db.push_name("a");
    let params = db.push_list(vec![a_param]);
    let scheme = db.push_list(vec![fn_head, params, body]);
    let t_field = meta_field(db, "t", scheme);
    let apply = {
        let head = db.push_name("intrinsic");
        let who = db.push_name("float-of");
        db.push_list(vec![head, who])
    };
    let apply_field = meta_field(db, "apply", apply);
    let rec_head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
    let rec = db.push_list(vec![rec_head, t_field, apply_field]);
    let k = db.push_name("of");
    let eq = db.push_name("="); // seq-276: canonical (= name value) record entry
    db.push_list(vec![eq, k, rec])
}

/// The module's `wrap` field, appended on `&mut Db` — the constructor-side twin of `prelude::wrap_field`
/// (the two MUST build the same shape so `(Int N).wrap` and the named-width `Int8.wrap` are one
/// operation). `(wrap (record ((meta t) (fn (a) (-> (Int a) TARGET))) ((meta apply) (intrinsic wrap))))`.
fn wrap_field(db: &mut Db, signed: bool, width: u32) -> StructId {
    // `(fn (a) (-> (Int a) TARGET))`.
    let int_a = {
        let int = db.push_name("Int");
        let a = db.push_name("a");
        db.push_list(vec![int, a])
    };
    let target = {
        let ctor = db.push_name(if signed { "Int" } else { "UInt" });
        let w = db.push_atom(Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: crate::ast::Radix::Dec,
        });
        db.push_list(vec![ctor, w])
    };
    let arr = db.push_name("->");
    let body = db.push_list(vec![arr, int_a, target]);
    let fn_head = db.push_name("fn");
    let a_param = db.push_name("a");
    let params = db.push_list(vec![a_param]);
    let lambda = db.push_list(vec![fn_head, params, body]);
    // `("record" ((meta t) lambda) ((meta apply) (intrinsic wrap)))` — the record primitive is the
    // string head `"record"`.
    let rec_head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(db, "t", lambda);
    let prim = intrinsic_node(db, "wrap");
    let apply_field = meta_field(db, "apply", prim);
    let record = db.push_list(vec![rec_head, t_field, apply_field]);
    let k = db.push_name("wrap");
    let eq = db.push_name("="); // seq-276: canonical (= name value) record entry
    db.push_list(vec![eq, k, record])
}

/// The (max, min) bounds of a `(signed, width)` integer as arbitrary-precision [`IntValue`]s, computed
/// FROM THE WIDTH — a signed N-bit is `-(2^(N-1)) ..= 2^(N-1)-1`, an unsigned N-bit is `0 ..= 2^N-1`.
/// `None` only for a width the fold does not model (0, or > 64). Odd widths (`7`, `24`, `48`) are
/// ordinary — the bounds are a function of `width`, so nothing is hard-coded per named type. Shared
/// with the prelude's named-width modules so a named width and `(Int N)` share one bounds computation.
pub fn int_bounds(signed: bool, width: u32) -> Option<(IntValue, IntValue)> {
    if width == 0 || width > 64 {
        return None;
    }
    if signed {
        // max = 2^(width-1) - 1, min = -(2^(width-1)).
        let half = 1u128 << (width - 1);
        Some((IntValue::from_u128(half - 1), IntValue::from_neg_u128(half)))
    } else {
        // max = 2^width - 1, min = 0. (width==64 → 2^64-1, exact via u128.)
        let max = (1u128 << width) - 1;
        Some((IntValue::from_u128(max), IntValue::zero()))
    }
}

/// A `(= (meta KEY) VALUE)` meta field (appended). Mirrors the prelude's `meta_field` but on `&mut Db`.
/// seq-276: emitted in the canonical FieldPair `(= key value)` form (not a bare `((meta k) v)` pair), so
/// the compiler's own synthesized module/sum/float records satisfy the value-entry reader's require-`=`
/// rule — `read_record_fields` reads it via `field_pair` into the identical fields map, so every consumer
/// (`project_field`/`member_value`, which read that map) is unaffected.
fn meta_field(db: &mut Db, key: &str, value: StructId) -> StructId {
    let eq = db.push_name("=");
    let meta_head = db.push_name("meta");
    let key_name = db.push_name(key);
    let meta_key = db.push_list(vec![meta_head, key_name]);
    db.push_list(vec![eq, meta_key, value])
}

/// An `(intrinsic NAME)` node (appended) — the arena form a native primitive value takes, mirroring
/// `prelude::intrinsic_node` on `&mut Db` (so a constructor-built module's `wrap` field carries the
/// same intrinsic the prelude's named-width modules do).
fn intrinsic_node(db: &mut Db, name: &str) -> StructId {
    let head = db.push_name("intrinsic");
    let who = db.push_name(name);
    db.push_list(vec![head, who])
}

/// A `(name (: INT (Int/UInt WIDTH)))` record field — an arbitrary-precision integer constant ANNOTATED
/// with the module's own width, so projecting the field (`UInt8.max`) yields a value that types at that
/// width (not the signed-64 a bare literal would default to). The annotation is the ordinary `(: e T)`
/// node; the literal grounds to `(signed, width)` through it. This is what carries `UInt8.max`'s type as
/// `UInt8` and `UInt64.max`'s value (2^64-1, > i64) at its true width.
fn named_int_field(db: &mut Db, name: &str, value: IntValue, signed: bool, width: u32) -> StructId {
    let k = db.push_name(name);
    let lit = db.push_atom(Leaf::Int {
        value,
        radix: crate::ast::Radix::Dec,
    });
    // The type expression `(Int width)` / `(UInt width)`.
    let ctor = db.push_name(if signed { "Int" } else { "UInt" });
    let w = db.push_atom(Leaf::Int {
        value: IntValue::from_i64(width as i64),
        radix: crate::ast::Radix::Dec,
    });
    let ty_expr = db.push_list(vec![ctor, w]);
    // `(: lit ty_expr)`.
    let colon = db.push_name(":");
    let annot = db.push_list(vec![colon, lit, ty_expr]);
    let eq = db.push_name("="); // seq-276: canonical (= name value) record entry
    db.push_list(vec![eq, k, annot])
}

/// A `(name (unrealized name))` field — the field exists but declines when projected.
fn unrealized_field(db: &mut Db, name: &str) -> StructId {
    let k = db.push_name(name);
    let head = db.push_name("unrealized");
    let who = db.push_name(name);
    let v = db.push_list(vec![head, who]);
    let eq = db.push_name("="); // seq-276: canonical (= name value) record entry
    db.push_list(vec![eq, k, v])
}
