//! `lower::call_lower` — call/application lowering: the runtime function-value application spine
//! (`runtime_fn_spine`/`apply_spine`), eta-expansion of partial ctor/closure/operator applications,
//! runtime lambda lifting + capture collection (`lower_lambda_value`/`collect_captures`), the
//! inline-cost + emit-once heuristics, monomorphizing call specialization (`emit_call_or_specialize`/
//! `type_specialize`/`callee_def_index`), and the binding-use analysis that drives let-keeping
//! (`collect_binding_uses`/`lambda_is_capturing`/`should_keep_binding`). Split out of `lower.rs`;
//! behaviour-preserving. Items keep their visibility (`pub(crate) use call_lower::*` in `lower`
//! re-exports each at its own vis); private items are `pub(super)` and reach the tree via `use super::*`.

use super::*;

/// Peel a CURRIED APPLICATION SPINE `(((f a) b) c)` into its ultimate RUNTIME FUNCTION-VALUE head and
/// the full left-to-right argument list `[a, b, c]`. The curried surface `((g n) 1)` is the SAME
/// full-arity application as `(g n 1)` — each nested `Apply` head contributes its own arguments, and the
/// bottom head is the one runtime fn value they all apply to. Returns `Some((f, args))` iff that bottom
/// head is a runtime function value (a fn-typed param / heap-held closure — `head_is_runtime_fn_value`)
/// of arrow type; `None` if the head is anything else (a lambda that β-reduces, a constructor, a prim, a
/// def), so those keep their own lowering paths. The accumulated args feed ONE `call_indirect`; if their
/// count doesn't match a lifted lambda's arity (a genuine partial or over-application) it declines at
/// select rather than fabricating an intermediate closure.
pub(crate) fn runtime_fn_spine(db: &mut Db, id: StructId) -> Option<(StructId, Vec<StructId>)> {
    match resolved_of(db, id) {
        Resolved::Apply { head, args } => {
            // A nested application head — recurse and PREPEND the deeper spine's args (they bind to the
            // function's leading parameters), then this level's args.
            if let Some((fn_head, mut spine_args)) = runtime_fn_spine(db, head) {
                spine_args.extend_from_slice(&args);
                return Some((fn_head, spine_args));
            }
            // Bottom of the spine: the head is a direct runtime fn value applied to this level's args.
            if head_is_runtime_fn_value(db, head)
                && matches!(crate::infer::type_of(db, head), crate::ty::Ty::Fn(_, _))
            {
                return Some((head, args.to_vec()));
            }
            None
        }
        _ => None,
    }
}

/// Peel a nested-`Apply` SPINE to its ULTIMATE (bottom) head and the full left-to-right argument list,
/// REGARDLESS of what the bottom head is — a builtin operation, a constructor, a lambda, a def, anything.
/// `((f a) b)` → `(f, [a, b])`; a non-`Apply` `id` → `(id, [])`. Unlike `runtime_fn_spine`/`ctor_spine`
/// (which return `None` unless the bottom head is a runtime-fn-value / constructor), this ALWAYS returns
/// the bottom head, so a caller can inspect it (its `meta_apply_of`/`scheme_of`) uniformly across the flat
/// `(f a b)` and nested-parens `((f a) b)` surfaces — the same application, so treated identically. Peels
/// through `Ref`/`Annot` wrappers on the head (via `peel_ref_annot`) so a `let`-bound partial is flattened
/// too. Used by the builtin-partial-application check so a nested spine reaches the same gate + arity test
/// the flat form does (the flat form's head IS the bottom head; the nested form's immediate head is an
/// inner `Apply` whose `meta_apply_of` is `None`, which would otherwise skip the check).
pub(crate) fn apply_spine(db: &mut Db, id: StructId) -> (StructId, Vec<StructId>) {
    // FUEL bounds the peel, exactly as `ctor_spine` does: a RECURSIVE nullary def `(def (f) (f))` has its
    // head-ref point back to the SAME apply, so an unfueled peel-and-recurse cycles forever (a stack
    // overflow / SIGABRT). A real application spine is a handful deep; 64 is far above any genuine spine
    // and stops the pathological cycle by returning the current node as the bottom head (a false-negative
    // for the partial-builtin check on a >64-deep spine — never a real program, and never a false reject).
    apply_spine_fueled(db, id, 64)
}

pub(super) fn apply_spine_fueled(
    db: &mut Db,
    id: StructId,
    fuel: u32,
) -> (StructId, Vec<StructId>) {
    if fuel == 0 {
        return (id, Vec::new());
    }
    match resolved_of(db, id) {
        Resolved::Apply { head, args } => {
            let peeled = peel_ref_annot(db, head);
            let (bottom, mut spine_args) = apply_spine_fueled(db, peeled, fuel - 1);
            spine_args.extend_from_slice(&args);
            (bottom, spine_args)
        }
        _ => (id, Vec::new()),
    }
}

/// Peel a CURRIED CONSTRUCTOR SPINE `((V a) b)` into the variant-constructor head `V` and the full
/// left-to-right payload list `[a, b]`. A sum constructor is a single-arity function (core-semantics.md
/// §A Sum Type Constructor Is A Single-Arity Function; §Functions Are Single-Arity: `(f a b)` desugars to
/// `((f a) b)`), so a multi-payload variant written curried — `((Pair 3) 4)` — is the SAME construction
/// as the flat `(Pair 3 4)`: each nested `Apply` head contributes its own arguments and the bottom head
/// is the one variant constructor they all apply to. Returns `Some((ctor, args))` iff the bottom head is
/// a variant constructor (`variant_disc_of` is `Some`) reached through at least one nested `Apply` (a
/// FLAT `(Pair 3 4)` has a non-`Apply` head and takes the ordinary `lower_sum_new` path directly — this
/// is only for the nested-parens surface). `None` for any other head, so lambdas/prims/defs keep their
/// paths. The caller checks the gathered count against the variant's payload arity before building.
pub(crate) fn ctor_spine(db: &mut Db, id: StructId) -> Option<(StructId, Vec<StructId>)> {
    // FUEL bounds the peel. The Ref-follow + lambda-reduction below can chain (a partial ctor bound to a
    // ref, itself the reduction of a helper); more importantly a RECURSIVE nullary def `(def (f) (f))`
    // has its head-ref point back to the SAME `(f)` apply, so an unfueled follow-and-recurse cycles
    // forever (a stack overflow). A real constructor spine is bounded by the variant's payload arity (a
    // handful); 64 is far above any genuine spine and stops the pathological cycle with a clean `None`.
    ctor_spine_fueled(db, id, 64)
}

/// Follow a chain of `Resolved::Ref` (a `let`/`def` binding) and `Resolved::Annot` (`(: expr T)`, a value
/// annotation transparent to the value's identity) to the underlying value node — so a constructor reached
/// through a binding or an annotation wrapper (both introduced by `apply_lambda` when it inlines a HOF that
/// took a constructor as a first-class arg) is found. Bounded by a small fuel so a self-referential ref
/// cannot cycle. Stops at the first node that is neither a `Ref` nor an `Annot`.
pub(crate) fn peel_ref_annot(db: &mut Db, id: StructId) -> StructId {
    let mut cur = id;
    for _ in 0..64 {
        match resolved_of(db, cur) {
            Resolved::Ref { value } => cur = value,
            Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => cur = expr,
            // A tuple PROJECTION `(. c i)` whose operand `c` reduces to a compile-time-visible tuple
            // follows to that element's occurrence — so a partial ctor stashed in a COMPOUND LITERAL and
            // then projected + applied (`((. (tuple (T.Mk 10) 0) 0) 5)`, or the `let`-bound
            // `(let ((p (tuple (T.Mk 10) 0))) ((. p 0) 5))`) reaches the constructor spine and COMPLETES to
            // a flat `(T.Mk 10 5)` construction — the compound analogue of the `let`-ref follow, so a
            // partial ctor in a static tuple flattens like the inline `((T.Mk 10) 5)`. Without this the
            // projected partial ctor was lowered as a MALFORMED `SumNew` (too-few payloads) and the
            // application `call_indirect`'d it as a closure → INVALID WASM. Only a STATICALLY-VISIBLE
            // operand tuple folds here; a runtime tuple (a param / kept binding) has no visible element,
            // so `reduce_to_tuple_elems` returns `None` and the projection stays a runtime `Core::Proj`
            // (the genuine runtime-closure case, handled elsewhere / declines).
            Resolved::Proj { operand, index } => {
                match crate::eval::reduce_to_tuple_elems(db, operand) {
                    Some(elems) if index < elems.len() => cur = elems[index],
                    _ => break,
                }
            }
            // A record field access `(. r f)` whose operand `r` is a compile-time-visible record follows to
            // the field's value occurrence — the record-field analogue of the tuple-projection follow above,
            // so a partial ctor stashed in a record field (`(record (f (T.Mk 7)))` then `((. r f) 3)`) also
            // reaches the ctor spine and completes. `member_value` returns the field value occurrence for a
            // visible record (a runtime record has none → `None` → the access stays a runtime read).
            Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key)
            {
                crate::eval::Member::Field(field) => cur = field,
                _ => break,
            },
            _ => break,
        }
    }
    cur
}

/// ETA-EXPAND a bare payload constructor `ctor_head` (a `(. Sum V)` record whose scheme is `(-> p0 … Sum)`)
/// into a genuine runtime CLOSURE — `(fn (__eta0 … __eta{n-1}) (ctor_head __eta0 … __eta{n-1}))` lifted to a
/// funcref-table slot via `lower_lambda_value`. This is needed when the constructor VALUE cannot be inlined
/// away — it is stored in a heap data structure (a tuple/list/record) and applied later — so, unlike the
/// `ctor_spine` inline-fold path, a real first-class function value must exist (the same lift a `(fn …)` or a
/// named function value gets). A lambda / named fn stored in a tuple already lifts; a bare constructor did
/// not, declining "a runtime closure application has no matching function type".
///
/// The constructor's arrow is FULLY KNOWN from its scheme (`scheme_of`), so the synthesized lambda needs no
/// inference context: each `__eta{k}` param's type is SEEDED into `db.param_types` from the k-th arrow
/// parameter, and the body `(ctor __eta…)` lowers to `SumNew` typed as the sum. `None` if the scheme is
/// unavailable or a payload/result type has no machine representation (declines cleanly, as before).
pub(crate) fn eta_ctor_closure(db: &mut Db, ctor_head: StructId) -> Option<Core> {
    // The ctor's curried scheme `(-> p0 (-> p1 … Sum))` — peel each parameter type + the final sum.
    let mut fresh = crate::unify::Fresh::new();
    let scheme = crate::eval::scheme_of(db, ctor_head, &mut fresh)?;
    let inst = crate::unify::instantiate(&scheme, &mut fresh);
    let mut payload_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = inst;
    while let crate::ty::Ty::Fn(p, r) = cur {
        payload_tys.push(*p);
        cur = *r;
    }
    if payload_tys.is_empty() {
        return None; // a nullary variant is a value, not a function — no closure needed.
    }
    // Synthesize `(fn (__eta0 …) (ctor_head __eta0 …))`. A distinct binder occurrence per param, and a
    // distinct reference occurrence per body use (both `push_name` the same spelling; scope resolution ties
    // the body ref to the param binder). The `__` prefix cannot collide with a source name.
    let n = payload_tys.len();
    let param_occs: Vec<StructId> = (0..n).map(|k| db.push_name(&format!("__eta{k}"))).collect();
    let mut body_children = Vec::with_capacity(1 + n);
    body_children.push(ctor_head);
    for k in 0..n {
        body_children.push(db.push_name(&format!("__eta{k}")));
    }
    let body = db.push_list(body_children);
    let params_list = db.push_list(param_occs.clone());
    let fn_head = db.push_name("fn");
    let lambda = db.push_list(vec![fn_head, params_list, body]);
    crate::resolve::resolve_subtree(db, lambda);
    // SEED each synthesized param's machine type from the ctor scheme, so `lower_lambda_value`'s
    // `type_of(param)` (which reads `db.param_types`) and the body's `SumNew` typing resolve without any
    // inference context on the synthesized nodes. `param_name_occ` maps the binder to its name occurrence
    // (the key `type_of`/`db.param_types` use).
    let (params, ty_body_ok) = match resolved_of(db, lambda) {
        Resolved::Lambda { params, body } => (params, body),
        _ => return None, // synthesis did not classify as a lambda — bail (declines below)
    };
    let _ = ty_body_ok;
    for (k, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        db.param_types
            .entry(occ)
            .or_insert_with(|| payload_tys[k].clone());
    }
    // Lower the synthesized lambda as a runtime closure value.
    match resolved_of(db, lambda) {
        Resolved::Lambda { params, body } => Some(lower_lambda_value(db, lambda, &params, body)),
        _ => None,
    }
}

/// A PARTIALLY-applied constructor `(T.Mk 10)` as a runtime value: synthesize the equivalent
/// explicit lambda over the REMAINING payloads, splicing the already-supplied args into the body so they
/// are CAPTURED by the closure — `(fn (__eta0 …__eta{r-1}) (ctor <supplied…> __eta0 …))`. This is exactly
/// the shape a source `(fn (y) (T.Mk 10 y))` takes, which lowers+runs correctly today. The supplied args
/// become free variables in the lambda body → the lambda-lift captures them into the closure env, and the
/// lifted func reads them back — no bare-scalar-in-a-handle-slot (the miscompile). `supplied` are the args
/// already applied (`< arity`); the lambda binds the `arity - supplied.len()` remaining payloads.
pub(super) fn partial_ctor_eta_closure(
    db: &mut Db,
    ctor_head: StructId,
    supplied: &[StructId],
) -> Option<Core> {
    let mut fresh = crate::unify::Fresh::new();
    let scheme = crate::eval::scheme_of(db, ctor_head, &mut fresh)?;
    let inst = crate::unify::instantiate(&scheme, &mut fresh);
    let mut payload_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = inst;
    while let crate::ty::Ty::Fn(p, r) = cur {
        payload_tys.push(*p);
        cur = *r;
    }
    let m = supplied.len();
    if m == 0 || m >= payload_tys.len() {
        return None; // not a genuine partial (bare ctor → eta_ctor_closure; full/over → the build path)
    }
    let r = payload_tys.len() - m; // remaining params to eta-abstract
    // Body: `(ctor supplied[0] … supplied[m-1] __eta0 … __eta{r-1})`. The supplied occurrences are the
    // caller's own arg nodes (already lowered/typed in this scope), spliced verbatim so they capture.
    let remaining_occs: Vec<StructId> =
        (0..r).map(|k| db.push_name(&format!("__eta{k}"))).collect();
    let mut body_children = Vec::with_capacity(1 + m + r);
    body_children.push(ctor_head);
    for &a in supplied {
        body_children.push(a);
    }
    for k in 0..r {
        body_children.push(db.push_name(&format!("__eta{k}")));
    }
    let body = db.push_list(body_children);
    let params_list = db.push_list(remaining_occs.clone());
    let fn_head = db.push_name("fn");
    let lambda = db.push_list(vec![fn_head, params_list, body]);
    crate::resolve::resolve_subtree(db, lambda);
    let params = match resolved_of(db, lambda) {
        Resolved::Lambda { params, .. } => params,
        _ => return None,
    };
    // Seed each remaining param's machine type from the ctor scheme (the payloads AFTER the supplied ones).
    for (k, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        db.param_types
            .entry(occ)
            .or_insert_with(|| payload_tys[m + k].clone());
    }
    match resolved_of(db, lambda) {
        Resolved::Lambda { params, body } => Some(lower_lambda_value(db, lambda, &params, body)),
        _ => None,
    }
}

/// A PARTIAL application of a runtime CLOSURE `(g 3)` (fewer args than its curried arity) as a runtime
/// value: synthesize the equivalent explicit lambda over the REMAINING params, splicing the already-
/// supplied args into the body so they (and the closure `g` itself) are CAPTURED — `(fn (__eta0 …
/// __eta{r-1}) (g <supplied…> __eta0 …))`. This is exactly the shape a source `(fn (y) (g 3 y))` takes,
/// which lowers+runs correctly today: the synthesized body is a FULL application of `g` (→ `Core::CallClosure`),
/// and the lambda-lift captures `g` + the supplied args into the residual closure's env (no bare-scalar-in-a-
/// handle-slot — the miscompile the old under-arity `CallClosure` produced). The runtime-closure twin of
/// [`partial_ctor_eta_closure`]; `supplied` are the args already applied (`< arity`), the lambda binds the
/// `arity - supplied.len()` remaining params, whose types come from the closure's curried arrow type.
pub(super) fn partial_closure_eta_closure(
    db: &mut Db,
    fn_head: StructId,
    supplied: &[StructId],
) -> Option<Core> {
    // The closure's curried arrow `(-> p0 (-> p1 … result))` — peel each param type. (The head's type is
    // grounded by `check`; a non-arrow head has no params and is not a partial, handled by the caller.)
    let mut param_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = crate::infer::type_of(db, fn_head);
    while let crate::ty::Ty::Fn(p, r) = cur {
        param_tys.push(*p);
        cur = *r;
    }
    let m = supplied.len();
    if m == 0 || m >= param_tys.len() {
        return None; // not a genuine partial (0 args / full / over-application handled elsewhere)
    }
    // SCALAR-CAPTURE ONLY (for now): the residual lambda CAPTURES each supplied arg + the head closure. A
    // scalar capture is copied by value (no refcount), so it round-trips cleanly; but a COMPOUND (heap-value)
    // capture spliced from the caller's own occurrence is double-owned (the caller created it AND the residual
    // closure captures it) and is NOT dup'd/dropped at the capture site → a LEAK (`live-objects` mismatch),
    // the capture-escape hazard v-effects flagged (its #5007 collect_captured_escape_dup_sites machinery is the
    // real fix). Until that dup-site wiring is reused here, DECLINE a partial whose supplied args (or the head
    // itself, a closure handle) include a heap value — a scalar-only partial (the corpus `(f 3)` case) folds
    // soundly, a compound-capturing one keeps declining (reject-don't-miscompile, as before). fn_head is a
    // closure HANDLE (heap) but is captured by the same lambda-lift every source closure uses (proven), so it
    // is not the leak source — only the spliced compound ARG occurrences are; gate on those.
    if supplied.iter().any(|&a| !is_scalar(db, a)) {
        trace!(target: "rcdzc::lower", head = fn_head.0, "partial-closure eta: a supplied arg is a COMPOUND capture (leak hazard) → decline pending capture-escape dup wiring");
        return None;
    }
    let r = param_tys.len() - m; // remaining params to eta-abstract
    // Body: `(fn_head supplied[0] … supplied[m-1] __eta0 … __eta{r-1})`. The `fn_head` + supplied occurrences
    // are the caller's own nodes (already lowered/typed in this scope), spliced verbatim so they capture.
    let remaining_occs: Vec<StructId> =
        (0..r).map(|k| db.push_name(&format!("__eta{k}"))).collect();
    let mut body_children = Vec::with_capacity(1 + m + r);
    body_children.push(fn_head);
    for &a in supplied {
        body_children.push(a);
    }
    for k in 0..r {
        body_children.push(db.push_name(&format!("__eta{k}")));
    }
    let body = db.push_list(body_children);
    let params_list = db.push_list(remaining_occs.clone());
    let fn_kw = db.push_name("fn");
    let lambda = db.push_list(vec![fn_kw, params_list, body]);
    crate::resolve::resolve_subtree(db, lambda);
    let params = match resolved_of(db, lambda) {
        Resolved::Lambda { params, .. } => params,
        _ => return None,
    };
    // Seed each remaining param's machine type from the closure's arrow (the params AFTER the supplied ones).
    for (k, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        db.param_types
            .entry(occ)
            .or_insert_with(|| param_tys[m + k].clone());
    }
    match resolved_of(db, lambda) {
        Resolved::Lambda { params, body } => Some(lower_lambda_value(db, lambda, &params, body)),
        _ => None,
    }
}

pub(super) fn ctor_spine_fueled(
    db: &mut Db,
    id: StructId,
    fuel: u32,
) -> Option<(StructId, Vec<StructId>)> {
    if fuel == 0 {
        return None;
    }
    // Follow a `let`/`def` REF to the value it binds, so a partial constructor stashed in a binding —
    // `(let ((g (Pair 3))) (g 4))`, where the head `g` refs the half-applied `(Pair 3)` — flattens the
    // same as the inline `((Pair 3) 4)`. Without this, the ref head is neither an `Apply` (so no deeper
    // spine) nor a constructor record (so no bottom), and the partial ctor reaches "not applyable". A ref
    // that cycles back to `id` (a recursive nullary self-call) is caught by the fuel bound above.
    // Also PEEL a value ANNOTATION `(: expr T)`: when a HOF `(def (app (: g (-> Int64 W)) x) (g x))` is
    // inlined with `g ↦ W.Mk`, `apply_lambda` wraps the substituted arg in the param's annotation, so the
    // body becomes `((: W.Mk (-> Int64 W)) x)` — the head is an `Annot` hiding the constructor. A value
    // annotation is transparent to WHAT the value is, so peel it to reach the ctor spine underneath.
    let node = peel_ref_annot(db, id);
    let Resolved::Apply { head, args } = resolved_of(db, node) else {
        return None;
    };
    // A nested application head — recurse to the bottom constructor and PREPEND the deeper spine's args
    // (they bind to the constructor's leading payloads), then this level's args.
    if let Some((ctor, mut spine_args)) = ctor_spine_fueled(db, head, fuel - 1) {
        spine_args.extend_from_slice(&args);
        return Some((ctor, spine_args));
    }
    // Bottom of the spine: the head is a variant constructor applied to this level's args. Reached either
    // through the recursive arm above (the OUTER `Apply`'s head is the inner `Apply`) or directly when a
    // followed ref lands on a half-applied constructor `(Pair 3)`. A genuine FLAT construction never lands
    // here — its own head is the bare `(. Sum V)` record, so `ctor_spine` on it returns `None` at the
    // `let…else` (a record is not an `Apply`) and the flat `Some(Prim::SumNew)` path builds it. PEEL a
    // ref/annotation on the head too, so a `g ↦ W.Mk` (bare ctor) or `(: W.Mk T)` (annotated bare ctor)
    // head reaches the constructor and builds from this level's args — the inlined-HOF shape above.
    let ctor_head = peel_ref_annot(db, head);
    if crate::eval::variant_disc_of(db, ctor_head).is_some() {
        return Some((ctor_head, args.to_vec()));
    }
    // The head is a LAMBDA (a def / `fn`) applied to this level's args, and that application REDUCES to a
    // constructor — `((mk1 3) 4)` where the head `(mk1 3)` applies the helper `(def (mk1 x) (Pair x))` to
    // `3`, reducing to `(Pair 3)`. β-reduce `head` over `args` under the depth guard, then peel the
    // reduced spine (the reduced `(Pair 3)` yields the bottom ctor + `[3]`); the caller's outer level
    // appends its own `[4]`. `args` are CONSUMED by the reduction, so they are NOT re-appended here. Only
    // attempted when the head is a lambda (`lambda_body` is `Some`) — a runtime-closure/prim head keeps its
    // path. `apply_lambda` DECLINES a recursive callee (returns `Err`), and the `enter_reduction` guard
    // plus the fuel bound stop a deep/cyclic chain from inlining without end.
    if crate::eval::lambda_body(db, head).is_some()
        && let Some(mut guard) = db.enter_reduction()
    {
        let g = guard.db();
        if let Ok(Some(reduced)) = crate::eval::apply_lambda(g, head, &args)
            && reduced != head
        {
            return ctor_spine_fueled(g, reduced, fuel - 1);
        }
    }
    None
}

/// Lower a `(fn (param…) body)` that survives as a RUNTIME value — LAMBDA-LIFT it to a standalone
/// function and produce a `Core::Closure` naming its funcref-table slot + capture set. Single-parameter
/// only (the curried surface reduces multi-param application via partial application upstream). The
/// lambda's FREE VARIABLES are captured BY VALUE into the closure cell; each capturing reference in the
/// body is recorded (`db.captured_ref`) so lowering that reference in the lifted body reads the env cell
/// (`Core::Captured`) rather than following through to the (out-of-scope) binding. The param + result
/// machine types come from the lambda's body (`type_of`).
/// Whether lambda occurrence `id` is lexically NESTED inside another `(fn …)` form — i.e. it is a
/// closure RETURNED (or otherwise built) by an enclosing lambda, the curried closure-returning-closure
/// shape. Walks enclosing forms outward via `parent_of`, testing each for a `fn` head. Used to keep a
/// `Unit`-parameter lambda on that (pre-existing-broken) curried boxed-closure path DECLINING rather than
/// compiling into an "indirect call type mismatch" trap. A stop at the enclosing DEF/module is implicit:
/// a def body is not a `fn` form, so the walk simply finds no enclosing lambda for a top-level closure.
pub(super) fn lambda_is_nested_in_lambda(db: &Db, id: StructId) -> bool {
    let mut cur = db.parent_of(id);
    while let Some(p) = cur {
        if db.ast.as_form(p, "fn").is_some() {
            return true;
        }
        cur = db.parent_of(p);
    }
    false
}

pub(super) fn lower_lambda_value(
    db: &mut Db,
    id: StructId,
    params: &[StructId],
    body: StructId,
) -> Core {
    // At least one parameter (a nullary lambda value has no use here). A multi-parameter lambda IS
    // supported — it lifts to an `(env, p1, …, pn) -> result` function and is applied at FULL arity via
    // one `call_indirect` (see the `Core::CallClosure` lowering). A PARTIAL application of a runtime
    // multi-param closure (runtime currying) still declines at the application site, not here.
    if params.is_empty() {
        return Core::Poison(Reject::decline(
            crate::diag::NULLARY_LAMBDA_NO_CLOSURE_DECLINE,
        ));
    }
    // FLATTEN a directly-nested curried lambda `(fn (n) (fn (m) …))` into ONE multi-param lift
    // `(fn (n m) …)`. A curried arrow type `(-> A (-> B R))` has exactly ONE machine representation
    // across a program, and the SUGAR form `(fn (a b) …)` — and every CALLER that spine-flattens a curried
    // application `((f x) y)` into a single `Core::CallClosure { args:[x,y] }` (see the runtime-closure
    // spine in `core_of`) — assumes the FLAT `(env, a, b) -> r` lift. Left un-flattened, a nested-unary
    // value lifts CHAINED instead (an outer `(env, a) -> i32` returning a closure handle, plus a separate
    // inner `(env, b) -> r`); a spine-flattened call then references a `(env, a, b) -> r` functype the
    // chained value does NOT implement, so the module validates structurally but TRAPS 'indirect call type
    // mismatch' at run time. Merging the param lists (and using the innermost non-lambda body) makes the
    // nested-unary value lift to the SAME single function the sugar form does, so both representations
    // coincide. This is sound for the only supported application shape — a FULL-arity call: a genuine
    // PARTIAL application `(f x)` (stopping short of the flattened arity) still declines at select, exactly
    // as it did for the chained representation. Only DIRECT nesting is merged (the body IS a lambda, no
    // intermediate work between the arrows); an outer lambda that computes before returning the inner
    // (`(fn (n) (let … (fn (m) …)))`) is left as-is.
    let mut params: Vec<StructId> = params.to_vec();
    let mut body = body;
    while let Resolved::Lambda {
        params: inner_params,
        body: inner_body,
    } = resolved_of(db, body)
    {
        params.extend_from_slice(&inner_params);
        body = inner_body;
    }
    let params: &[StructId] = &params;
    let param_occs: Vec<StructId> = params
        .iter()
        .map(|&p| crate::eval::param_name_occ(db, p))
        .collect();
    // Collect the ORDERED, DISTINCT capture set — the enclosing-binding occurrences the body references
    // (other than ANY of its own params / top-level defs / the prelude), first-reference order. Each
    // capturing REFERENCE occurrence is recorded → its capture index, so the lifted body reads it from
    // the env.
    let mut captures: Vec<StructId> = Vec::new();
    let mut capture_refs: Vec<(StructId, usize)> = Vec::new();
    let ok = collect_captures(db, body, &param_occs, id, &mut captures, &mut capture_refs);
    if !ok {
        return Core::Poison(Reject::unsupported(
            "a closure captures a value with no runtime representation",
        ));
    }
    // The lambda's EXPECTED arrow from its CONTEXT (a variant-payload position `(T.Susp (fn …))`, a
    // built-in `Some`/`Ok` payload, or an annotation) — the "thread the use-site arrow back" the bottom-up
    // `type_of` omits. `None` when the context declares no arrow (a HOF call site the call's own unify
    // covers, or a genuinely unconstrained position). Its `Ty::Fn(P0, P1, … R)` gives per-parameter
    // expected types and the result type to fall back on when body-solving leaves them `Any`.
    let expected = crate::infer::expected_arrow_for_lambda(db, id);
    // Peel the expected arrow into a per-parameter type list + the final result type: an N-param lambda's
    // expected arrow is curried `P0 → P1 → … → R`. `expected_param_tys[i]` is param `i`'s expected type;
    // `expected_ret` is `R` after peeling all N params.
    let (expected_param_tys, expected_ret) = {
        let mut ptys: Vec<crate::ty::Ty> = Vec::new();
        let mut cur = expected.clone();
        for _ in 0..param_occs.len() {
            match cur {
                Some(crate::ty::Ty::Fn(p, r)) => {
                    ptys.push(*p);
                    cur = Some(*r);
                }
                _ => {
                    cur = None;
                    break;
                }
            }
        }
        (ptys, cur)
    };
    // Each parameter's solved machine type. A bare `(fn (x) …)` types `Any` at its own occurrence
    // (inference does not thread the use-site arrow back), so SOLVE it from the body's uses
    // (`solve_lambda_param_ty`, the lambda analogue of the recursive-def A2 solve), then fall back to the
    // EXPECTED arrow's parameter type (its storage context). A param neither the body nor the context
    // constrains to a machine type declines below (no invented width).
    let mut param_tys: Vec<(StructId, crate::ty::Ty)> = Vec::new();
    for (i, &p) in param_occs.iter().enumerate() {
        let mut pt = match crate::infer::type_of(db, p) {
            crate::ty::Ty::Any => crate::infer::solve_lambda_param_ty(db, p, body),
            t => t,
        };
        if matches!(pt, crate::ty::Ty::Any)
            && let Some(ep) = expected_param_tys.get(i)
            && !matches!(ep, crate::ty::Ty::Any)
        {
            pt = ep.clone();
        }
        // A `Unit` parameter is representable at the boxed-closure boundary — it occupies NO wasm slot
        // (elided from the functype's params by `select_function_of`, the read analogue of a Unit result's
        // zero-result functype and a Unit argument pushing nothing), so a `(-> Unit T)` closure (the
        // canonical lazy THUNK `Susp(Unit -> …)`) lifts fine. Only a param that is neither machine-repr NOR
        // Unit (an unrepresentable compound closure param) declines.
        //
        // EXCEPTION — a Unit param declines when the lambda participates in a CURRIED CLOSURE-RETURNING-
        // CLOSURE chain: either its own RESULT is a function (`(-> Unit (-> A B))`, case B), or it is a
        // lambda lexically NESTED inside another lambda (a returned inner closure, `(-> A (-> Unit B))`,
        // case C). That boxed curried path is PRE-EXISTING BROKEN independent of Unit (the all-`Int64`
        // `(-> A (-> A B))` boxed form ALSO traps "indirect call type mismatch" on trunk); the Unit-arg
        // elision would merely let a Unit program compile far enough to hit that same trap. Declining keeps
        // the outcome SAFE (a refusal, not a miscompile) for those shapes, while the value-result thunk
        // `(-> Unit T)` — the shape the lazy-`Iter` proposal needs — compiles. (Fixing the underlying
        // curried boxed-closure trap is a separate, larger increment; filed to the PM.)
        if matches!(pt, crate::ty::Ty::Unit)
            && (matches!(crate::infer::type_of(db, body), crate::ty::Ty::Fn(..))
                || lambda_is_nested_in_lambda(db, id))
        {
            return Core::Poison(Reject::decline(crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE));
        }
        if !matches!(pt, crate::ty::Ty::Unit)
            && crate::backend::wasm::lir::valtype_of(&pt).is_none()
        {
            return Core::Poison(Reject::decline(crate::diag::CLOSURE_PARAM_NO_REPR_DECLINE));
        }
        // RECORD the solved param type so the LIFTED BODY's own `type_of(p)` reads it — otherwise the
        // body computes `p`'s type bottom-up as `Any` (it has no annotation and no def entry), and a use
        // like `(C.A p)` that boxes `p` into a sum/tuple payload declines "element of type Any". `type_of`
        // reads a `Param` from `db.param_types`, so seeding it here threads the closure's solved parameter
        // type into the body. Insert only a DETERMINED type, and only if absent, so a genuinely-annotated
        // or already-solved param is never overwritten.
        if !matches!(pt, crate::ty::Ty::Any) {
            db.param_types.entry(p).or_insert_with(|| pt.clone());
        }
        param_tys.push((p, pt));
    }
    // The RESULT type is the body's; if that is `Any` (a body that returns e.g. a sum whose payload
    // depends on an as-yet-unpinned param), fall back to the expected arrow's result type.
    let ret_ty = match crate::infer::type_of(db, body) {
        crate::ty::Ty::Any => expected_ret.clone().unwrap_or(crate::ty::Ty::Any),
        t => t,
    };
    // A `Unit` result is REPRESENTABLE as a ZERO-RESULT functype (the serializer emits a Unit-returning
    // lifted lambda as `0x60 <params> <>`, and `closure_type_index` matches it as a zero-result shape), so
    // it must NOT decline here — mirror the `Unit`-param exception above (a Unit occupies no wasm slot). A
    // result that is NEITHER machine-repr NOR Unit is genuinely unrepresentable and still declines.
    if !matches!(ret_ty, crate::ty::Ty::Unit)
        && crate::backend::wasm::lir::valtype_of(&ret_ty).is_none()
    {
        return Core::Poison(Reject::decline(crate::diag::CLOSURE_RESULT_NO_REPR_DECLINE));
    }
    // Every captured value must have a machine representation too (it is boxed into the cell).
    for &cap in &captures {
        if crate::backend::wasm::lir::valtype_of(&crate::infer::type_of(db, cap)).is_none() {
            return Core::Poison(Reject::decline(
                crate::diag::CLOSURE_CAPTURE_NO_REPR_DECLINE,
            ));
        }
    }
    // Record each capturing reference → its capture index + type, so `core_of` on that reference (when
    // the LIFTED body is lowered) produces a `Core::Captured` reading the env cell. Keyed by the
    // reference OCCURRENCE (unique per use), so it never collides with an ordinary ref elsewhere.
    for (ref_occ, index) in capture_refs {
        let ty = crate::infer::type_of(db, captures[index]);
        db.captured_ref.insert(ref_occ, (index, ty));
    }
    // Register the lift (dedup by body occurrence); its position in `db.lifted` is its table slot.
    let code = db.lift_lambda(crate::lower::LiftedLambda {
        body,
        params: param_tys,
        ret_ty,
        captures: captures.clone(),
    });
    trace!(target: "rcdzc::lower", node = id.0, body = body.0, code, n_params = params.len(), n_captures = captures.len(), "lift lambda → Core::Closure");
    Core::Closure {
        code,
        captures: captures.into(),
    }
}

/// Collect the lambda body's FREE-VARIABLE capture set into `captures` (ordered, distinct, first-use
/// order) and record each capturing REFERENCE occurrence → its capture index into `capture_refs`.
/// Returns `false` if a capture cannot be represented (a decline). A reference is a capture iff it
/// resolves to a USER-program binding (a `let` init / an enclosing parameter) lexically OUTSIDE the
/// lambda `lam_id`; a reference to the lambda's own `param`, a top-level def, or a prelude name is NOT a
/// capture. The captured VALUE identity is the binding occurrence the reference resolves to (so two
/// references to the same free variable share ONE capture slot).
pub(super) fn collect_captures(
    db: &mut Db,
    node: StructId,
    params: &[StructId],
    lam_id: StructId,
    captures: &mut Vec<StructId>,
    capture_refs: &mut Vec<(StructId, usize)>,
) -> bool {
    match resolved_of(db, node) {
        // A bare parameter USE: `Resolved::Param { binder }`. One of the lambda's OWN params → not a
        // capture; an enclosing param → a capture keyed by that binder.
        Resolved::Param { binder } => {
            if params.contains(&binder) {
                return true;
            }
            // DEGENERATE SELF-CAPTURE — the reference occurrence IS its own binder (`binder == node`).
            // A legitimate capture reference and its binder are DISTINCT occurrences (the binder sits in a
            // param list / `let`; the use sits in the body). They coincide only as a copy artifact: when a
            // lambda ARGUMENT is `resolve_subtree`-pinned at a call site and that lambda is later itself
            // copied (its enclosing def inlined), a pinned OWN-param body reference is shared into the copy
            // while the param LIST is copied fresh — leaving a body occurrence that resolves to the
            // ORIGINAL param binder, now orphaned (no slot at the build site). Emitting it produced an
            // invalid module (a bare env-read / `local.get` with no backing local). A sound α-renaming fix
            // to the copy machinery is a separate, larger change; until then DECLINE rather than miscompile
            // (reject-don't-miscompile). This does NOT hit a genuine capture (distinct binder/use nodes).
            if binder == node {
                return false;
            }
            record_capture(binder, node, captures, capture_refs);
            return true;
        }
        Resolved::Ref { value } => {
            // A top-level def is global — never captured.
            if db.def_index_by_body(value).is_some() {
                return true;
            }
            // The ref's target may be a PARAMETER — including a SYNTHESIZED one, produced when a recursive
            // callee is specialized (its fn-typed argument `g` threads through as a fresh param binder).
            // Such a target is NOT a user node, so the `is_user_node` bailout below would wrongly treat it
            // as a global and skip the capture — leaving the reference to lower as a bare `Core::Param`
            // with no slot in the lifted body (an invalid module). Classify by the RESOLVED target: a
            // param that is not one of the lambda's OWN params is an enclosing binding to capture, keyed by
            // that param binder (so two references to the same threaded `g` share one capture slot).
            if let Resolved::Param { binder } = resolved_of(db, value) {
                if params.contains(&binder) {
                    return true; // the lambda's own parameter, reached through a ref — not a capture.
                }
                record_capture(binder, node, captures, capture_refs);
                return true;
            }
            // A non-user SYNTHESIZED ref — usually a prelude name or a reduced constant (global, not
            // captured), BUT a capture-once fold produces a chain of synthesized alias bindings
            // (`#a → #seed → n`): the hoisted `#a` capture binding aliases the handle `#seed` binding,
            // which aliases the enclosing param `n`. The single-level `resolved_of(value)` Param check
            // above sees only `Ref{#seed}` (not a Param) and would fall through to skip here, leaking the
            // enclosing param as a slot-less `Core::Param` in the lifted closure. So CHASE the alias
            // chain to its terminal first: a chain that bottoms out at an enclosing PARAM IS a capture,
            // keyed by the ORIGINAL body occurrence `node` (the chain is pure resolution aliases, so the
            // terminal param's value equals this reference's value). A chain that bottoms out anywhere
            // else (a prelude name / a reduced constant / a self-cycle) is genuinely global — not captured.
            if !db.is_user_node(value) {
                let mut cursor = value;
                for _ in 0..64 {
                    match resolved_of(db, cursor) {
                        Resolved::Param { binder } => {
                            if !params.contains(&binder) {
                                record_capture(binder, node, captures, capture_refs);
                            }
                            return true;
                        }
                        Resolved::Ref { value: next } if next != cursor => cursor = next,
                        _ => return true,
                    }
                }
                return true;
            }
            if !db.is_within(value, lam_id) {
                // A USER binding outside the lambda — a capture. Its identity is `value` (the binding
                // occurrence); this reference occurrence (`node`) reads it from the env.
                record_capture(value, node, captures, capture_refs);
                return true;
            }
            // Within the lambda (its params, a nested let) — recurse through the target for a nested
            // capture (e.g. a `let`-local whose init references a free variable).
            return collect_captures(db, value, params, lam_id, captures, capture_refs);
        }
        _ => {}
    }
    // A NESTED LAMBDA `(fn (inner-params) inner-body)` in the body — its OWN params are bound WITHIN it,
    // so they are neither captures of the outer lambda nor self-captures; only its FREE variables (which,
    // if bound outside the OUTER lambda, are the outer's captures too) matter. Descend into the inner
    // BODY with the inner params ADDED to the excluded set, and SKIP the inner param list (whose binder
    // occurrences would otherwise trip the `binder == node` self-capture guard, spuriously declining any
    // lifted lambda containing a nested lambda). This keeps a nested applied lambda `((fn (y) …) x)`
    // analyzable so it β-reduces at lowering rather than declining here.
    if let Some(tail) = db.ast.as_form(node, "fn")
        && let (Some(&inner_params_occ), Some(&inner_body)) = (tail.first(), tail.get(1))
        && let crate::ast::Struct::List(inner_params) = db.ast.get(inner_params_occ)
    {
        let inner_param_occs: Vec<StructId> = inner_params
            .clone()
            .iter()
            .map(|&p| crate::eval::param_name_occ(db, p))
            .collect();
        let mut combined = params.to_vec();
        combined.extend_from_slice(&inner_param_occs);
        return collect_captures(db, inner_body, &combined, lam_id, captures, capture_refs);
    }
    // Descend into the AST children (a form's operands, an if's branches, a `let`'s bindings).
    match db.ast.get(node) {
        crate::ast::Struct::List(children) => {
            let children: Vec<StructId> = children.clone();
            children
                .iter()
                .all(|&c| collect_captures(db, c, params, lam_id, captures, capture_refs))
        }
        crate::ast::Struct::Atom(_) => true,
    }
}

/// Record a captured binding: assign `binder` a capture slot (first-use order, deduped) and map the
/// capturing reference occurrence `ref_occ` to that slot.
pub(super) fn record_capture(
    binder: StructId,
    ref_occ: StructId,
    captures: &mut Vec<StructId>,
    capture_refs: &mut Vec<(StructId, usize)>,
) {
    let index = captures
        .iter()
        .position(|&b| b == binder)
        .unwrap_or_else(|| {
            captures.push(binder);
            captures.len() - 1
        });
    capture_refs.push((ref_occ, index));
}

/// A lambda application whose β-reduction DECLINED with `msg`: emit a runtime `Core::Call` if it
/// declined because the callee is a RECURSIVE top-level def with a DETERMINED signature; otherwise
/// propagate the decline. This is the ONE place a recursive call becomes a real wasm call instead of an
/// unbounded inline. A non-recursive decline (a partial application, a bad head) is NOT a call — its
/// message is passed through unchanged.
pub(super) fn lower_recursive_call_or_decline(
    db: &mut Db,
    head: StructId,
    args: &[StructId],
    msg: String,
) -> Core {
    // A REDUCTION-BUDGET decline (a non-normalizing / explosively-growing term — a self-applying lambda
    // whose reduction the total-work budget stopped) is a resource-limit rejection, the SAME "declined at
    // a bound, not crashed" class as the unproductive-recursion CDZ0999. Code it so, so it is a diagnosed
    // reject rather than a bare uncoded decline (the compiler stops and reports, never hangs).
    if msg.contains("reduction budget") {
        return Core::Poison(Reject::coded(Code::RecursionBound, msg));
    }
    // Only a RECURSION decline becomes a call; every other decline (partial application, over-arity)
    // propagates as-is. The recursion decline is the one `apply_lambda` raises via `is_recursive`.
    let is_recursion_decline = msg.contains("recursive function needs runtime specialization");
    if !is_recursion_decline {
        return Core::Poison(Reject::decline(msg));
    }
    // Resolve the head to the top-level def it names. Only a NAMED top-level def can be emitted as a
    // standalone wasm function (its index is stable in the layout); a computed/anonymous recursive head
    // has no such identity, so it still declines.
    let callee = match callee_def_index(db, head) {
        Some(d) => d,
        // A computed/anonymous recursive head has no stable top-level identity to emit a call to, so it
        // declines — the same "needs runtime specialization" reason as the catalogued family. Tag it with
        // the stable id (seq-286 coded-tracking; surfaced as a reachable codeless decline by v-cdz-smith's
        // reachability sweep). `declined` preserves the message, so the `is_recursion_decline` .contains
        // gate above still matches.
        None => {
            return Core::Poison(Reject::declined(
                crate::diag::DeclineId::RecursiveFunctionRuntimeSpecialization,
                msg,
            ));
        }
    };
    emit_call_or_specialize(db, head, callee, args)
}

/// The body-size floor (in bounded AST nodes) at/above which the cost heuristic PREFERS emitting a def
/// once and calling it over inlining it at every site. Below it a def is small enough that inlining is
/// never a net loss (the call frame would cost as much as the body). DELIBERATELY CONSERVATIVE: the
/// operator kept always-inline as the observable default during the compiler-port, so the heuristic only
/// changes codegen for genuinely LARGE, multiply-called, all-runtime-arg helpers — the case where
/// duplication is unambiguously wasteful. The precise value awaits real code-size data from the
/// self-hosted compiler (Addendum 4: "tune it THEN"); this floor is high enough that it is inert on
/// ordinary small helpers and only trips on the clear wins.
pub(super) const INLINE_COST_THRESHOLD_DEFAULT: u32 = 40;

/// The minimum number of call sites at/above which the cost heuristic prefers emit-once. A def called
/// ONCE gains nothing from a shared function (one inline = one copy either way, and inlining folds
/// better), so the heuristic only fires at ≥2 sites where the duplication it avoids is real.
pub(super) const INLINE_MIN_CALLERS_DEFAULT: usize = 2;

/// The effective body-size floor for emit-once. Normally `INLINE_COST_THRESHOLD_DEFAULT` (40); the
/// `CDZ_INLINE_COST_THRESHOLD` env override exists ONLY for the emit-once set-widening CO-MEASURE slice —
/// lowering it flips MORE large-ish helpers to emit-once (fewer duplicated inline copies → smaller emit,
/// the operator's emit-fn-elimination goal), so v-compiler-perf / v-wasm-opt can sweep the
/// size/node-collapse curve across candidate values in one session (a process per value) instead of a
/// land+measure round-trip per guess. UNSET → the default → byte-identical codegen; slice-3 commits the
/// empirically-chosen floor as the new default and this knob is retired. Read once and cached (env is
/// read-only config, fixed for the process), so it never touches the memoized hot path.
pub(super) fn inline_cost_threshold() -> u32 {
    static CACHE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    // FAIL-FAST on a PRESENT-but-invalid value (unset → default is fine): a typo'd
    // `CDZ_INLINE_COST_THRESHOLD=4O` must NOT silently measure the default while the sweeper believes it
    // swept 40 — that yields misleading sweep data (the whole point of this knob is a trustworthy sweep).
    // ONLY `NotPresent` (genuinely unset) takes the default → byte-identical codegen; a present-but-
    // unparseable value panics, and `NotUnicode` (present but not valid UTF-8) is ALSO present-but-invalid
    // → panic, not the default (a set-but-garbage var must never masquerade as unset).
    *CACHE.get_or_init(|| match std::env::var("CDZ_INLINE_COST_THRESHOLD") {
        Ok(v) => v.parse().expect("CDZ_INLINE_COST_THRESHOLD must be a u32"),
        Err(std::env::VarError::NotPresent) => INLINE_COST_THRESHOLD_DEFAULT,
        Err(e) => panic!("CDZ_INLINE_COST_THRESHOLD is set but invalid: {e}"),
    })
}

/// The effective call-site floor for emit-once — see [`inline_cost_threshold`]. `CDZ_INLINE_MIN_CALLERS`
/// overrides it for the same set-widening co-measure sweep; UNSET → 2 → byte-identical.
pub(super) fn inline_min_callers() -> usize {
    static CACHE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    // FAIL-FAST on a PRESENT-but-invalid value (only a genuinely-UNSET var → default), the sibling of
    // `inline_cost_threshold`'s guard: a typo'd `CDZ_INLINE_MIN_CALLERS` must not silently sweep the default,
    // and a present-but-non-UTF-8 (`NotUnicode`) value is present-but-invalid → panic, not the default.
    *CACHE.get_or_init(|| match std::env::var("CDZ_INLINE_MIN_CALLERS") {
        Ok(v) => v.parse().expect("CDZ_INLINE_MIN_CALLERS must be a usize"),
        Err(std::env::VarError::NotPresent) => INLINE_MIN_CALLERS_DEFAULT,
        Err(e) => panic!("CDZ_INLINE_MIN_CALLERS is set but invalid: {e}"),
    })
}

/// The COST-HEURISTIC decision (Addendum 4): should this non-recursive call to named top-level def
/// `callee` be EMITTED ONCE and called, rather than β-reduced (inlined) at this site? The unannotated
/// default is otherwise always-inline; this returns `true` only for the clear duplication win AND only
/// when emit-once is provably SOUND — never overriding the mandatory-inline invariant.
///
/// SOUNDNESS (the mandatory-inline invariant, Addendum 4): inlining is REQUIRED whenever a call's result
/// is demanded at COMPILE TIME (a `const` arg, a type-valued position, a generic instantiation, or a
/// constant fold). This gate never touches those cases:
/// - a GENERIC callee (`scheme.ty_vars` non-empty) or one with a `const` PARAM is excluded — those are
///   specializations `emit_call_or_specialize` already owns, not ordinary runtime calls;
/// - and it fires only when SOME argument CAPTURES A RUNTIME BINDING (`arg_captures_runtime_binding`).
///   That single condition is the proof: a call with a runtime-dependent argument can NOT be reduced to a
///   compile-time value (the same reason `type_specialize` rejects such an arg for a `const` param), so no
///   `const`/type/generic/fold position could ever demand this call's result — emitting it as a runtime
///   `Core::Call` strands nothing. (A fully-closed call, by contrast, MIGHT be const-demanded, so it is
///   left to inline/fold.)
///
/// COST (the actual tradeoff): the callee body is at/above the effective cost threshold (`inline_cost_threshold`,
/// default 40) nodes AND it is called at ≥ the effective min-callers floor (`inline_min_callers`, default 2)
/// sites — large and duplicated. `@inline-never`/`@inline-always` are handled by
/// the caller BEFORE this (they are explicit overrides), so this only governs the UNANNOTATED default.
pub(super) fn should_emit_once_by_cost(db: &mut Db, callee: usize, args: &[StructId]) -> bool {
    // PER-CALLEE eligibility (body-size + monomorphic + non-const + fan-out) is MEMOIZED in `db.emit_shared`
    // (keyed by callee), computed once by `emit_once_callee_eligible` — this used to be recomputed on the hot
    // lower path at EVERY call site (a bounded_node_count walk + scheme resolve + call-site-count each time).
    if !emit_once_callee_eligible(db, callee) {
        return false;
    }
    // PER-ARGS SOUNDNESS GATE (NOT memoizable per-callee — depends on the actual args): at least one argument
    // must capture a runtime binding, so the result can never be compile-time-demanded (see the doc-comment
    // invariant). A fully-closed call is left to inline/fold.
    args.iter().any(|&a| arg_captures_runtime_binding(db, a))
}

/// The PER-CALLEE half of `should_emit_once_by_cost`, MEMOIZED in `db.emit_shared` (the operator-directed
/// emit-once column, keyed by callee def index): `true` iff the callee is a large-body, monomorphic, non-`const`,
/// high-fan-out def. Independent of the call site's args, so it is computed ONCE per callee and cached (the hot
/// lower path then does a map lookup + the per-args soundness gate, instead of re-walking the body / re-resolving
/// the scheme / re-counting call-sites at every site). Byte-identical to the pre-memo per-callee gates.
pub(super) fn emit_once_callee_eligible(db: &mut Db, callee: usize) -> bool {
    if let Some(&v) = db.emit_shared.get(&callee) {
        return v;
    }
    // Only memoize a DEFINITE eligibility. `_uncached` returns `None` when the callee's scheme is not yet
    // determined (`def_scheme` returns a TRANSIENT uncached `None` mid-scheme-solve, deliberately, to avoid
    // poisoning later queries — infer.rs `solving_params`/`solving_schemes`). Caching the transient `false`
    // would PERMANENTLY disable emit-once for a callee that was merely queried mid-solve, so once its scheme
    // determines emit-once would never fire (a behavior change vs the pre-memo per-call recompute; Copilot
    // PR#959 id 3693671598). Treat "not yet determined" as ineligible FOR THIS QUERY (return false) but do
    // NOT cache it — the next query recomputes once the scheme is ready.
    match emit_once_callee_eligible_uncached(db, callee) {
        Some(eligible) => {
            db.emit_shared.insert(callee, eligible);
            eligible
        }
        None => false, // scheme not yet determined — ineligible now, but NOT memoized
    }
}

/// Returns `Some(eligible)` for a DEFINITE per-callee decision (safe to memoize), or `None` when the
/// callee's scheme is not yet determined (`def_scheme == None` mid-solve — a transient the caller must NOT
/// cache; see [`emit_once_callee_eligible`]).
pub(super) fn emit_once_callee_eligible_uncached(db: &mut Db, callee: usize) -> Option<bool> {
    // CHEAPEST GATE FIRST: a SMALL body is the common case and always inlines — bail before any resolve/scheme
    // work. `bounded_node_count` saturates at the threshold, so this is O(threshold), not O(body).
    let Some(body) = db.defs[callee].body else {
        return Some(false);
    };
    let cost_threshold = inline_cost_threshold();
    if bounded_node_count(db, body, cost_threshold) < cost_threshold {
        return Some(false);
    }
    // A monomorphic, non-`const` scheme only — a generic / `const`-param callee is a specialization the shared
    // path already handles (and MUST inline/specialize, not be diverted by cost). A `None` here is TRANSIENT
    // (scheme mid-solve, uncached by `def_scheme`) → signal "not yet determined" so the caller does not cache.
    let Some(scheme) = crate::infer::def_scheme(db, callee) else {
        return None; // undetermined signature — recompute later, do NOT memoize
    };
    if !scheme.ty_vars.is_empty() {
        return Some(false);
    }
    let callee_params = db.defs[callee].params.clone();
    let has_const_param = callee_params.iter().any(|&p| {
        db.const_params
            .contains(&crate::eval::param_name_occ(db, p))
    });
    if has_const_param {
        return Some(false);
    }
    // COST GATE: called at ≥ `inline_min_callers()` sites (the whole-program call-site index, built once +
    // cached) — the effective floor is `INLINE_MIN_CALLERS_DEFAULT` (2), env-overridable via the
    // `inline_min_callers()` knob. The duplication emit-once avoids is only real at ≥2 sites.
    Some(crate::infer::callee_call_site_count(db, callee) >= inline_min_callers())
}

/// Emit a call to a NAMED top-level def `callee` as a `Core::Call` — SPECIALIZING it (monomorphizing a
/// generic scheme / erasing a `const` param) when needed. This is the SHARED emit-once-and-call path used
/// by BOTH (a) a recursive call (`lower_recursive_call_or_decline`, which can't inline) and (b) an
/// `inline-never` def (which the author asked NOT to inline). Because both route here, an `inline-never`
/// GENERIC or `const`-dict def is specialized (polymorphism + dict erasure kept) AND emitted once + called
/// (inline avoided) — "avoid the inline but keep polymorphism", for free.
/// Eta-expand any ARGUMENT that is a bare operator used as a first-class function value, grounded by the
/// callee's parameter type at that position. Returns `None` when no argument needs rewriting (the common
/// case — the caller keeps the original slice, no allocation). See the call site in
/// [`emit_call_or_specialize`] for why: a bare operator has no runtime closure form.
///
/// Precise + conservative: an argument is rewritten ONLY when (a) the callee's parameter at that position
/// is a fully-GROUND arrow (`is_ground`, no `Any`) — so we know the exact closure signature and never touch
/// a generic/undetermined position — AND (b) the argument is a bare operation VALUE (`meta_apply_of` is a
/// `Prim`, and it is not itself an application). Every `Prim` is an operation (constructors dispatch through
/// `variant_disc_of`, not `meta_apply_of`), so this never eta-expands a constructor (those have their own
/// [`eta_ctor_closure`]) or an ordinary value. A normal call — no function-typed parameter — scans zero
/// candidate positions and returns `None` immediately.
pub(super) fn eta_operator_args(
    db: &mut Db,
    scheme: &crate::ty::Scheme,
    args: &[StructId],
) -> Option<Vec<StructId>> {
    // The callee's per-position parameter types (peel the declared arrow). A generic scheme's quantified
    // vars are `Ty::Var`, so those positions are not `is_ground` and are skipped below.
    let mut param_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = scheme.ty.clone();
    while let crate::ty::Ty::Fn(p, r) = cur {
        param_tys.push(*p);
        cur = *r;
    }
    // Cheap pre-scan: is there a GROUND-ARROW parameter fed a bare-operator argument? Most calls have no
    // function-typed parameter, so this returns None with no synthesis and no allocation.
    let needs = args.iter().enumerate().any(|(i, &a)| {
        param_tys
            .get(i)
            .is_some_and(|p| matches!(p, crate::ty::Ty::Fn(..)) && p.is_ground() && !p.has_any())
            && !matches!(resolved_of(db, a), Resolved::Apply { .. })
            && crate::eval::meta_apply_of(db, a).is_some()
    });
    if !needs {
        return None;
    }
    let rewritten = args
        .iter()
        .enumerate()
        .map(|(i, &a)| {
            let is_op_arg = param_tys.get(i).is_some_and(|p| {
                matches!(p, crate::ty::Ty::Fn(..)) && p.is_ground() && !p.has_any()
            }) && !matches!(resolved_of(db, a), Resolved::Apply { .. })
                && crate::eval::meta_apply_of(db, a).is_some();
            if is_op_arg {
                let arrow = param_tys[i].clone();
                synth_operator_eta(db, a, &arrow).unwrap_or(a)
            } else {
                a
            }
        })
        .collect();
    Some(rewritten)
}

/// Synthesize the eta lambda `(fn (__eta0 …) (<op_occ> __eta0 …))` for a bare operator `op_occ` at the
/// ground arrow `arrow` (the callee's parameter type), returning the lambda's occurrence id. The arrow's
/// parameter types seed the synthesized params so the lambda lowers without any inference context on the
/// synthesized nodes (mirrors [`eta_ctor_closure`], but the ground type comes from the call site rather
/// than the operator's own polymorphic scheme). `None` if `arrow` has no parameter or synthesis does not
/// classify as a lambda.
pub(super) fn synth_operator_eta(
    db: &mut Db,
    op_occ: StructId,
    arrow: &crate::ty::Ty,
) -> Option<StructId> {
    let mut param_tys: Vec<crate::ty::Ty> = Vec::new();
    let mut cur = arrow.clone();
    while let crate::ty::Ty::Fn(p, r) = cur {
        param_tys.push(*p);
        cur = *r;
    }
    if param_tys.is_empty() {
        return None;
    }
    let n = param_tys.len();
    let param_occs: Vec<StructId> = (0..n).map(|k| db.push_name(&format!("__eta{k}"))).collect();
    let mut body_children = Vec::with_capacity(1 + n);
    body_children.push(op_occ);
    for k in 0..n {
        body_children.push(db.push_name(&format!("__eta{k}")));
    }
    let body = db.push_list(body_children);
    let params_list = db.push_list(param_occs);
    let fn_head = db.push_name("fn");
    let lambda = db.push_list(vec![fn_head, params_list, body]);
    crate::resolve::resolve_subtree(db, lambda);
    let params = match resolved_of(db, lambda) {
        Resolved::Lambda { params, .. } => params,
        _ => return None,
    };
    for (k, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        db.param_types
            .entry(occ)
            .or_insert_with(|| param_tys[k].clone());
    }
    Some(lambda)
}

/// A PARTIALLY-applied binary OPERATOR `(+ 1)` / `(< 3)` as a first-class curried value: synthesize the
/// equivalent explicit lambda over the REMAINING operand, splicing the already-supplied operand into the
/// body so it is CAPTURED — `(fn (__eta0) (<op> <supplied> __eta0))`. This is exactly the shape a source
/// `(fn (b) (+ 1 b))` takes, which lowers+runs today (the binop twin of [`partial_ctor_eta_closure`]).
/// Operators curry (operator ruling), so `(+ 1)` is `\b. 1 + b`: the supplied operand is the FIRST, the
/// eta binds the SECOND. `head`/`supplied` are the caller's own occurrences, spliced verbatim (the original
/// `(op supplied)` apply this replaces is discarded, so the single-parent invariant holds — as in
/// `partial_ctor_eta_closure`). The eta param types as the SUPPLIED operand's type (a binop's two operands
/// share one type), with an unground Int width defaulted, so `lower_lambda_value` types it without needing
/// the operator's own polymorphic scheme (whose params are quantified vars). `(- e)` is UNARY NEGATION, a
/// distinct construct handled before this — Sub never reaches here.
pub(super) fn partial_binop_eta(db: &mut Db, head: StructId, supplied: StructId) -> Option<Core> {
    // The eta param's machine type = the supplied operand's type (both operands of a binop share it), with
    // an unground Int literal grounded to its default width — so `(+ 1)` binds an `Int64` second operand.
    let param_ty = match crate::infer::type_of(db, supplied) {
        crate::ty::Ty::Int(it) => crate::ty::Ty::Int(crate::ty::IntTy::fixed(
            it.ground_signed(),
            it.ground_width(),
        )),
        other => other,
    };
    let eta_binder = db.push_name("__eta0");
    let eta_ref = db.push_name("__eta0");
    // `(head supplied __eta0)` — reuse the caller's operator head + supplied operand verbatim (spliced,
    // like `partial_ctor_eta_closure`), the eta the fresh second operand.
    let body = db.push_list(vec![head, supplied, eta_ref]);
    let params_list = db.push_list(vec![eta_binder]);
    let fn_head = db.push_name("fn");
    let lambda = db.push_list(vec![fn_head, params_list, body]);
    crate::resolve::resolve_subtree(db, lambda);
    let params = match resolved_of(db, lambda) {
        Resolved::Lambda { params, .. } => params,
        _ => return None, // synthesis did not classify as a lambda — bail
    };
    let occ = crate::eval::param_name_occ(db, params[0]);
    db.param_types.entry(occ).or_insert(param_ty);
    match resolved_of(db, lambda) {
        Resolved::Lambda { params, body } => Some(lower_lambda_value(db, lambda, &params, body)),
        _ => None,
    }
}

pub(super) fn emit_call_or_specialize(
    db: &mut Db,
    head: StructId,
    callee: usize,
    args: &[StructId],
) -> Core {
    // RECORD "emitted as a real call": this call did NOT inline — it emits a `Core::Call` to a standalone
    // function (a recursive callee, an `inline-never` def, or — via `type_specialize` below — a
    // specialization). Keyed by the SOURCE `callee` (the user def), NOT the synthesized copy `type_specialize`
    // may return: so `fac` reads `emitted` even when the emitted loop is its accumulator/monomorphized twin
    // `fac$acc`/`fac#mono`. This is the `db.inlined` complement the `Instantiations` query reports as the
    // def's DISPOSITION. Recorded even if the signature is undetermined below (the AUTHOR wrote a call that
    // is meant to emit — reporting it as "emitted" is truer than "unreferenced" for a decline). No cost to a
    // plain compile: this path runs during lowering, which the query forces.
    db.called.insert(callee);
    // The callee must have a DETERMINED signature to be emitted (its params need machine valtypes). An
    // annotated recursive def qualifies (types by absorption); an unannotated one is solved by the
    // connected parameter solve (`solve_recursive_params`, A2) — it stays undetermined only when no use
    // in the body constrains a parameter (it grounds to `Any`), in which case the call still declines.
    let scheme = match crate::infer::def_scheme(db, callee) {
        Some(s) => s,
        None => {
            trace!(target: "rcdzc::lower", head = head.0, callee, "call: callee signature undetermined → decline (A2)");
            // `def_scheme` returned `None` for one of TWO reasons — distinguish them, because the actionable
            // advice is OPPOSITE. If every PARAMETER is already determined (annotated or solved), the
            // undetermined thing is the def's RESULT: the body never yields a concrete type because every
            // path recurses — a recursive function with NO BASE CASE. Telling that author to "add an explicit
            // annotation" on a parameter is wrong (the parameter — often ALREADY `(: n Int64)` — is fine); the
            // fault is the missing base case, and no parameter annotation fixes it.
            if crate::infer::recursive_def_params_all_determined(db, callee) {
                // A no-base-case recursion is a REJECTION of an invalid user program (every path recurses →
                // no value → undetermined result), not a capability decline — code it CDZ0204
                // (`NonProductiveRecursion`, the 02xx well-formedness band) so the fault carries a stable
                // code, not a bare codeless decline (seq-286). Distinct from the CDZ0999 nullary
                // reduce-bound decline in `lower/compute.rs`.
                return Core::Poison(Reject::coded(
                    crate::diag::Code::NonProductiveRecursion,
                    "this recursive function never returns a concrete value — every path calls itself, so \
                     its result type is undetermined and it has no machine representation. Add a BASE CASE \
                     that returns a value without recursing (e.g. an `if`/`match` arm that stops the \
                     recursion), so the result type is fixed",
                ));
            }
            // Otherwise a PARAMETER stayed undetermined — no use in the body pinned its type, so it grounds
            // to `Any` and has no machine width. TWO shapes reach here and the actionable advice DIFFERS, so
            // name both rather than always saying "annotate its parameters" (which OVERPROMISES: a generic
            // helper has no writable annotation).
            //   (a) A MONOMORPHIC recursive helper whose parameter is only ever passed through (never used
            //       in a width-fixing operation): an explicit annotation `(: p Int64)` DOES ground it.
            //   (b) A recursive-GENERIC helper — one THREADED from a generic producer, where the param's
            //       element type is a type VARIABLE tied to the caller's instantiation (`(def (go it acc f)
            //       …)` seeded by a `reduce`-shaped wrapper at ≥2 element types) — CANNOT be annotated: the
            //       element is polymorphic, and a lowercase `(GIter a)` in an annotation is an unbound type
            //       name (CDZ0101, no forall-binder), so `(: it GIter)` fails CDZ0203 (GIter needs a type
            //       argument). The real fix is the recursive-generic monomorphization tie (a tracked
            //       inference follow-up), not an annotation — so don't send the author down a dead end.
            return Core::Poison(Reject::decline(
                "this recursive function has a parameter whose type could not be inferred — no use in its \
                 body fixes the parameter's type, so it has no machine representation. If the parameter is \
                 a plain (monomorphic) value, add an explicit annotation, e.g. `(: p Int64)`. If it is a \
                 GENERIC helper threaded from a producer (its element type varies with the caller), an \
                 annotation cannot express the polymorphic element — this is the recursive-generic \
                 monomorphization tie, a known inference limitation, so use a single concrete element type \
                 or a non-delegating fold until it lands",
            ));
        }
    };
    // A bare OPERATOR passed as a first-class function VALUE — `(apply2 + 3 4)` passes `+` where the
    // callee's parameter is a function `(-> Int64 Int64 Int64)`. A bare operator has no runtime closure
    // form (it lowers to its operator RECORD, which is "not applyable" when the callee applies it), so
    // ETA-EXPAND each such argument to the equivalent lambda `(fn (a b) (+ a b))`, GROUNDED by the callee's
    // parameter type here (the ground arrow the bare operator's own `type_of` lacks — `def_scheme` supplies
    // it). Only a bare-operator arg at a ground-arrow parameter position is rewritten; every other argument
    // is unchanged (a normal call is byte-identical). Mirrors [`eta_ctor_closure`] (bare payload ctor).
    let eta_rewritten = eta_operator_args(db, &scheme, args);
    let args: &[StructId] = eta_rewritten.as_deref().unwrap_or(args);
    // RECURSIVE-GENERIC MONOMORPHIZATION. A GENERIC scheme (`ty_vars` non-empty — a parameter the body
    // only threads, generalized in `compute_def_scheme`) has no machine representation for its generic
    // params. Monomorphize THIS call: synthesize (or reuse) a copy of the callee whose generic params are
    // re-annotated with the concrete argument types at this site, and call THAT
    // (`DESIGN-recursive-generic-monomorphization-rcdzc.md`). The same def called at another type gets its
    // own copy; both emit as ordinary monomorphic functions. A MONOMORPHIC scheme (`ty_vars` empty) takes
    // the byte-identical `Core::Call { callee }` path below.
    //
    // So a generic definition is SPECIALIZED to each concrete type-argument set before it crosses the
    // component boundary (every emitted copy is monomorphic). And this is the SAME compile-time reduction
    // that specializes any def applied to compile-time-known arguments: a NON-recursive generic call
    // β-reduces (monomorphizes away, no `Call` at all — see the inline-reduce path), and a recursive one
    // takes a specialized copy here — not a distinct lowering path from ordinary compile-time application.
    //= spec/capabilities/type-system.md#a-generic-definition-is-monomorphized-before-the-component-boundary
    //# The compiler MUST monomorphize a generic definition — specialize it to each concrete set of type arguments it is used with, by compile-time reduction with those type-values bound — before the definition crosses a component boundary, consistent with the component ABI.
    //= spec/capabilities/type-system.md#a-generic-definition-is-monomorphized-before-the-component-boundary
    //# Monomorphization MUST be the same compile-time reduction by which the compiler specializes any definition applied to compile-time-known arguments, so that a generic instantiation is not a distinct lowering path from ordinary compile-time application.
    // This is also the component-ABI obligation that generics do not cross the boundary: every emitted copy
    // is monomorphic (concrete type arguments), so no generic definition appears in the component interface
    // — and an undetermined boundary type (a `Ty::Var`/`Ty::Any` that never got specialized) has no
    // `comp_valtype_of` and is rejected rather than emitted.
    //= spec/contracts/component-abi.md#generics-do-not-cross-the-boundary
    //# The compiler MUST monomorphize every exported and imported signature to concrete types before emitting the component interface.
    //= spec/contracts/component-abi.md#generics-do-not-cross-the-boundary
    //# A generic definition MUST NOT appear in a component's interface.
    // SPECIALIZE this call when either (a) the callee scheme is GENERIC (a type parameter to erase), or
    // (b) some argument is CONST-INLINABLE (a compile-time-known value — a constant or an ad-hoc-poly
    // DICTIONARY of functions — to inline + drop). Both go through `type_specialize`, which substitutes
    // the erased args into a specialized copy and returns the dropped positions. A monomorphic callee
    // called with only plain runtime args takes the byte-identical `Core::Call { callee }` path below.
    //
    // AD-HOC POLYMORPHISM with NO trait machinery: a "trait" is an ordinary record type whose fields are
    // its operations, an "instance" is an ordinary value of that record, and a def polymorphic over it
    // receives that dictionary as an ordinary EXPLICIT parameter (resolved by ordinary lexical binding —
    // there is no ambient/global trait-resolution engine, so which implementation a use site gets is the
    // explicit argument, visible at the call). A const-known dictionary is MONOMORPHIZED into the use site
    // here (inlined by `type_specialize`, the argument erased from the runtime call) — no runtime
    // dictionary lookup, no dispatch the manifest did not declare.
    //= spec/capabilities/type-system.md#ad-hoc-polymorphism-is-an-explicitly-passed-dictionary
    //# A trait MUST be an ordinary record type whose fields are the operations it declares, and an instance MUST be an ordinary value of that record type, so that ad-hoc polymorphism reuses records and first-class values rather than a separate trait construct.
    //= spec/capabilities/type-system.md#ad-hoc-polymorphism-is-an-explicitly-passed-dictionary
    //# A definition that is polymorphic over a trait MUST receive the instance as an ordinary explicit parameter, so that ad-hoc polymorphism is the existing type-valued-and-value-valued parameter mechanism rather than a separate resolution engine.
    //= spec/capabilities/type-system.md#ad-hoc-polymorphism-is-an-explicitly-passed-dictionary
    //# The compiler MUST NOT resolve a trait instance from ambient or global scope, so that which implementation a use site gets is visible at the call and no orphan rule or global-coherence assumption is needed for ad-hoc polymorphism to compose with content-addressed modules.
    //= spec/capabilities/type-system.md#ad-hoc-polymorphism-is-an-explicitly-passed-dictionary
    //# An explicitly passed instance MUST be monomorphized into the use site, so that a component carries no runtime dictionary lookup and no dispatch the manifest did not declare.
    // Does the callee declare any `const` parameter (an EXPLICIT compile-time parameter — Addendum 3)?
    // A `const` param is inlined + erased at instantiation; the AUTHOR declared it, so no heuristic
    // detection. Specialize this call when the scheme is GENERIC (a type param) OR the callee has a
    // `const` param. A non-`const` monomorphic call is byte-identical to the plain `Core::Call` below.
    let callee_params = db.defs[callee].params.clone();
    let has_const_param = callee_params.iter().any(|&p| {
        db.const_params
            .contains(&crate::eval::param_name_occ(db, p))
    });
    if !scheme.ty_vars.is_empty() || has_const_param {
        return match type_specialize(db, callee, args) {
            Some((spec, erased_positions)) => {
                trace!(target: "rcdzc::lower", head = head.0, callee, spec, "specialized call → monomorphized Core::Call (type/const args erased)");
                // DROP the ERASED arguments from the runtime call: a `(: t Type)` type-value OR a
                // `const` param's value (a dictionary / constant) is compile-time-only (substituted into
                // the copy's body), so it carries no runtime slot. The remaining args are the runtime
                // ones, in order — matching the specialized def's signature (which omits the erased params).
                let runtime_args: Vec<StructId> = args
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !erased_positions.contains(i))
                    .map(|(_, &a)| a)
                    .collect();
                Core::Call {
                    callee: spec,
                    args: runtime_args.into(),
                }
            }
            None => {
                // `type_specialize` returns `None` for TWO very different causes; name the one that
                // actually applies rather than the conflated message that sent iterator authors down a
                // dead end. (a) The callee declares a `const` param whose argument does not fold to a
                // compile-time value — a CONTRACT VIOLATION (Addendum 3). (b) The callee has NO `const`
                // param: it is a purely-inferred recursive-generic whose scheme carries an UNDETERMINED
                // type variable the monomorphizer cannot bind — the classic being a recursive-generic
                // PRODUCER (`from-list : List a -> Iter a`) whose result element var is NOT tied to its
                // argument's (inferred `∀a b. List a -> Iter b`), so a call at ≥2 element types has no
                // single value to bind `b` to. That is a known inference limitation, NOT a `const` issue;
                // pointing at `const` is actively misleading. Distinguish by whether the callee declares a
                // `const` param.
                let callee_has_const = db.defs[callee].params.iter().any(|&p| {
                    db.const_params
                        .contains(&crate::eval::param_name_occ(db, p))
                });
                // IDENTITY RE-PASS of the callee's OWN const param — a self-recursive call `f(step, …)` that
                // threads `f`'s const param UNCHANGED. In the STANDALONE lowering of `f`'s generic body the
                // const param is an unbound reference, so `type_specialize` can't bind it and returns `None`
                // — but this is NOT an ill-formedness: a const-param fn is specialize-at-each-call, and at
                // every CONCRETE call the param is bound to a value that this identity re-pass threads
                // through the recursion (the concrete `type_specialize` succeeds + fuses, e.g. a
                // devirtualized iterator driver). So this specific decline must be a plain DECLINE (not the
                // coded CDZ0201 well-formedness fault): the generic body simply isn't lowered standalone, its
                // callers specialize it. NARROW: only a BARE reference to THIS callee's own const-param
                // binder qualifies — a DERIVED const arg (`(compose step g)`, a shorter collection tail) is
                // not an identity re-pass, keeps the coded fault, and the collection-fold guard stays
                // independent (v-inference ACK 2026-07-18; witness + unsound-twin gate it).
                let callee_params = db.defs[callee].params.clone();
                let is_const_identity_repass = callee_has_const
                    && callee_params.iter().zip(args.iter()).any(|(&p, &a)| {
                        let occ = crate::eval::param_name_occ(db, p);
                        db.const_params.contains(&occ)
                            && matches!(
                                resolved_of(db, a),
                                Resolved::Ref { value } | Resolved::Param { binder: value }
                                    if value == occ
                            )
                    });
                if is_const_identity_repass {
                    trace!(target: "rcdzc::lower", head = head.0, callee, "const identity re-pass in standalone generic body — plain decline, not a fault (callers specialize)");
                    return Core::Poison(Reject::decline(
                        "a `const` parameter's self-recursive identity re-pass is specialized at each call, \
                         not in the standalone generic body",
                    ));
                }
                trace!(target: "rcdzc::lower", head = head.0, callee, callee_has_const, "specialization declined (const contract / undetermined generic type arg)");
                // A FORWARDED-PARAM const arg: the const argument is a BARE reference to an ENCLOSING
                // param/local binder (not a derived expression). This is the standalone-lowering false-
                // reject — a def like `fold(it: Iter, acc, g)` that forwards its own param `g` (or a
                // `step` destructured from an annotated `it: Iter`) into another def's `const` slot is
                // lowered STANDALONE (it is monomorphic, so not specialized at its call site), where the
                // forwarded param is an unbound runtime reference → this decline. Every REACHABLE call
                // DOES have a concrete arg, so the reject is a compile-time provability gap, not a runtime
                // bug — turn the cryptic message into an ACTIONABLE fix-it hint (concierge ruling, option
                // C; the real fix, backward const-propagation, is a separate operator-gated arc). A
                // DERIVED const arg (`fn(x)=>(step x)+1`, a composed closure) is NOT a bare param forward,
                // so it keeps the plain message — it is a genuine runtime-data dependency.
                let const_arg_is_forwarded_param = callee_has_const
                    && callee_params.iter().zip(args.iter()).any(|(&p, &a)| {
                        let occ = crate::eval::param_name_occ(db, p);
                        db.const_params.contains(&occ)
                            && db.ast.as_name(a).is_some()
                            && matches!(
                                resolved_of(db, a),
                                Resolved::Ref { .. } | Resolved::Param { .. }
                            )
                    });
                // Whether any ARGUMENT to this call is itself a call to a NULLARY generic producer (a def
                // taking ZERO params — `(empty)`), which has no argument to determine its result element and
                // so leaves the consumer's type var unbindable. Distinct from the arg-bearing producer/
                // transformer shapes: the actionable fix is annotating THAT argument, not a nested one.
                let arg_is_nullary_producer_call = args.iter().any(|&a| {
                    // An argument shaped `(name)` — a single-element application (a call with no args) whose
                    // head names a def taking ZERO parameters (`(def (empty) (GIter.Nil))`). A nullary def
                    // has no lambda wrapper, so match it by NAME (`def_by_name`) rather than `callee_def_index`
                    // (which resolves through a `Lambda` body a nullary def doesn't have).
                    match db.ast.get(a) {
                        crate::ast::Struct::List(items) if items.len() == 1 => items
                            .first()
                            .and_then(|&h| db.ast.as_name(h))
                            .and_then(|n| db.def_by_name(n))
                            .is_some_and(|d| db.defs[d].params.is_empty()),
                        _ => false,
                    }
                });
                let message = if const_arg_is_forwarded_param {
                    "an argument to a `const` parameter must be compile-time-known, but here it is a runtime \
                     parameter forwarded from an enclosing function — that function is lowered on its own \
                     (it is not specialized at its call sites), so the forwarded value is not known there. \
                     Fix it by keeping the enclosing function POLYMORPHIC (drop a type annotation on the \
                     intermediate parameter — e.g. `it` rather than `it: Iter` — so it specializes per call), \
                     or INLINE the forwarding into the caller so the concrete argument reaches the `const` \
                     parameter directly"
                        .to_string()
                } else if callee_has_const {
                    "an argument to a `const` parameter must be compile-time-known — it depends on runtime data"
                        .to_string()
                } else if arg_is_nullary_producer_call {
                    // One of this call's ARGUMENTS is itself a call to a NULLARY generic producer — `(empty) :
                    // ∀a. Iter a`, `(GIter.Nil)`-valued — that takes NO argument, so its result element var is
                    // determined ONLY by the context that consumes it. Here it flows into a generic recursive
                    // consumer (`count (empty)`) whose scheme can't pin the element either → this decline. The
                    // general message below names three shapes and advises annotating an argument / using a
                    // single concrete element type — but the load-bearing gap is the nullary producer arg with
                    // no element source. The workaround is to annotate THAT argument with a concrete element
                    // type (`(: (empty) (GIter Int64))` — VERIFIED it then runs), so name it specifically
                    // rather than sending the author to the general arg-based fixes.
                    "an argument to this generic function is a nullary generic producer (e.g. `(empty) : \
                     ∀a. Iter a`) whose result element type nothing here determines — it takes no argument to \
                     infer the element from, and neither this call nor its context pins it, so the monomorphizer \
                     has no concrete element type to bind. Annotate that argument with a concrete element type \
                     (e.g. `(: (empty) (Iter Int64))`), or give the producer a parameter / concrete return type \
                     that fixes its element"
                        .to_string()
                } else {
                    // The recursive-generic case: a type variable in the callee's scheme is not tied to any
                    // argument, so the monomorphizer has no concrete type to bind it to at this call. THREE
                    // known shapes hit this (all tracked inference follow-ups): (1) a PRODUCER whose RESULT
                    // element is inferred free of its argument (`List a -> Iter a` used at ≥2 element types);
                    // (2) a recursive TRANSFORMER threading a CLOSURE (`(map it f) = Cons (f h) (map rest f)`),
                    // instantiated at ≥2 DISTINCT element (closure-domain) types where at least one maps to an
                    // AGGREGATE result — the closure-domain var is tied to the element within one copy (so a
                    // SINGLE type, or several SAME-domain types, monomorphize fine — the closure-domain tie
                    // landed), but the per-copy tie is not preserved across distinct-domain monomorphizations
                    // when a result is a compound, so those copies collide on an untied var; (3) a recursive
                    // producer applied to an ARGUMENT whose type is itself the result of the SAME (or another
                    // in-flight) recursive-generic call — `(from-list (list (inner) (inner)))` where `(inner)`
                    // is a `from-list` call — so typing the argument re-enters the producer's own mid-solve
                    // scheme and the nested call's element reads as undetermined. VERIFIED workaround
                    // (tested): an explicit annotation on a nested generic-call argument (`(: (inner) (Iter
                    // Int64))`) OR on the whole result (`(: (from-list …) (Iter (Iter Int64)))`) grounds it
                    // and the program runs — a `let`-binding does NOT (it hits the same re-entry). Name the
                    // real cause + the workarounds an author can act on now; do NOT claim "more than one
                    // type" (shapes 2/3 fail at one), and do NOT blame `const` (this callee declares none).
                    "this generic function has a type variable this call cannot determine — a recursive-generic \
                     function whose scheme leaves a type variable untied to any argument cannot be \
                     monomorphized here. This happens with a PRODUCER whose result element is inferred free of \
                     its argument (`List a -> Iter a` used at more than one element type); with a recursive \
                     TRANSFORMER threading a closure (`(map it f) = Cons (f h) (map rest f)`) instantiated at \
                     more than one DISTINCT element type where a closure maps to a COMPOUND result (a single \
                     element type, or several of the same type, monomorphize fine); and with a producer \
                     applied to an argument whose type is itself another in-flight recursive-generic call's \
                     result (e.g. `(from-list (list (inner) (inner)))` where `inner` is a `from-list` call). \
                     Add an explicit annotation — on a nested generic-call argument or on the result — \
                     annotate the closure's parameter type, use a single concrete element type, or make the \
                     definition monomorphic"
                        .to_string()
                };
                Core::Poison(Reject::coded(Code::Malformed, message))
            }
        };
    }
    // LOCAL LAMBDA-LIFT: if the callee is a recursive do-local function that captures an enclosing
    // parameter, thread each captured param as an explicit trailing argument (and, once, as a trailing
    // callee param). EMPTY for every other callee → byte-identical to the plain call. This runs for BOTH
    // the external call and the recursive self-call (each reaches here), so the arity stays consistent.
    let extra = local_lift_extra_args(db, callee);
    trace!(target: "rcdzc::lower", head = head.0, callee, args = args.len(), lifted = extra.len(), "recursive call → Core::Call");
    let mut all_args = args.to_vec();
    all_args.extend(extra);
    Core::Call {
        callee,
        args: all_args.into(),
    }
}

/// LOCAL LAMBDA-LIFT for a recursive do-local function that CAPTURES its enclosing scope. If `callee` is
/// an internal (do-local) def whose body reads one or more ENCLOSING PARAMETERS, thread each captured
/// param as an explicit trailing parameter — append its signature entry to `defs[callee].params` (so
/// `def_params`/`select_function` slot it in the callee's frame and the body's otherwise slot-less
/// `Core::Param` resolves, NO body rewrite) — and return the capture REFERENCE occurrences to append as
/// the extra call arguments. Each such argument lowers to `Core::Param{binder}` (frame-agnostic in
/// `core_of`) and is resolved PER-FRAME at emit: the caller's slot at an external call, the callee's own
/// new slot at the recursive self-call. Computed ONCE per callee and MEMOIZED: recomputing after the
/// params were extended would see the captures as own-params and drop them, corrupting the self-call's
/// arity. Returns EMPTY (byte-identical; the CDZ0900 recursive-local-capture decline unchanged) for a
/// non-internal callee, one with no captures, or an UNSUPPORTED capture — a non-parameter binding (an
/// enclosing `let`-local, which has no reusable signature entry) or a capture the analysis refuses. Those
/// keep the honest interim decline until a later increment threads non-param captures too (all-or-nothing;
/// no partial lift).
fn local_lift_extra_args(db: &mut Db, callee: usize) -> Vec<StructId> {
    if let Some(occs) = db.local_lift_captures.get(&callee) {
        return occs.clone();
    }
    // Record an empty (unsupported / no-capture) result so the analysis runs at most once per callee.
    let unsupported = |db: &mut Db| -> Vec<StructId> {
        db.local_lift_captures.insert(callee, Vec::new());
        Vec::new()
    };
    // Only a do-local / module-member internal def can capture an enclosing scope; a top-level def has
    // none (and this keeps the plain-call path byte-identical for them).
    if !db.defs[callee].internal {
        return unsupported(db);
    }
    let Some(body) = db.defs[callee].body else {
        return unsupported(db);
    };
    // The callee's OWN params (name occurrences) — excluded from the capture set.
    let own_params: Vec<StructId> = db.defs[callee]
        .params
        .iter()
        .map(|&p| crate::eval::param_name_occ(db, p))
        .collect();
    let mut caps: Vec<StructId> = Vec::new();
    let mut refs: Vec<(StructId, usize)> = Vec::new();
    // `body` doubles as the lambda-id region for `is_within` (a callee-local `let` is "within" it).
    if !collect_captures(db, body, &own_params, body, &mut caps, &mut refs) {
        return unsupported(db);
    }
    if caps.is_empty() {
        return unsupported(db);
    }
    // INCREMENT-1 SCOPE: every capture must be an ENCLOSING PARAMETER — it then has a reusable `(: name T)`
    // (or bare-name) signature entry to append as the callee's new param. A captured non-parameter binding
    // (an enclosing `let`-local) has no such entry and needs real param synthesis (a later increment), so
    // keep the decline for it — all-or-nothing.
    let mut entries: Vec<StructId> = Vec::with_capacity(caps.len());
    for &cap in &caps {
        if crate::infer::def_of_param(db, cap).is_none() {
            return unsupported(db);
        }
        let entry = match db.parent_of(cap) {
            Some(parent) if db.ast.as_form(parent, ":").is_some() => parent,
            _ => cap,
        };
        entries.push(entry);
    }
    // The extra call arguments, in capture order: the FIRST reference occurrence recorded for each capture.
    let mut extra: Vec<StructId> = Vec::with_capacity(caps.len());
    for i in 0..caps.len() {
        match refs.iter().find(|(_, idx)| *idx == i).map(|(r, _)| *r) {
            Some(r) => extra.push(r),
            None => return unsupported(db), // unreachable: record_capture pushes a ref per capture
        }
    }
    // Thread the captures as explicit trailing params (positional order matches `extra`).
    for entry in entries {
        db.defs[callee].params.push(entry);
    }
    db.local_lift_captures.insert(callee, extra.clone());
    extra
}

/// A CANONICAL STRUCTURAL FINGERPRINT of the subtree at `id` — a string that is EQUAL for two subtrees of
/// identical shape+leaves regardless of their `StructId`s. The specialization memo needs a key stable
/// across an argument and its β-COPY (the self-call re-passes a copied node, a fresh `StructId`): node
/// identity would miss, re-specializing without end. For a CONST arg (a dictionary record of lambdas) this
/// fingerprint distinguishes `(record (op (fn (x) (+ x 10))))` from `(record (op (fn (x) (* x 2))))` (so
/// they get distinct copies) while collapsing two identical dicts (so they dedup). Bounded by the arg's
/// size (a const dict is small); walks the raw AST (leaves + child order), NOT resolutions.
pub(super) fn subtree_fingerprint(db: &Db, id: StructId, out: &mut String) {
    match db.ast.get(id) {
        crate::ast::Struct::Atom(l) => {
            use crate::ast::Leaf;
            match db.ast.leaf(*l) {
                Leaf::Name(n) => {
                    out.push('n');
                    out.push_str(n);
                }
                Leaf::Int { value, .. } => {
                    out.push('i');
                    out.push_str(&value.to_decimal_string());
                }
                Leaf::Float(d) => {
                    out.push('f');
                    out.push_str(&d.to_f64_bits().to_string());
                }
                Leaf::Str(s) => {
                    out.push('s');
                    out.push_str(s);
                }
                Leaf::Bool(b) => out.push(if *b { 'T' } else { 'F' }),
                Leaf::Sym(s) => {
                    out.push('y');
                    out.push_str(s);
                }
                Leaf::Char(c) => {
                    out.push('c');
                    out.push(*c);
                }
                Leaf::Bytes(bs) => {
                    out.push('b');
                    out.push_str(&bs.len().to_string());
                }
                other => {
                    // A defect leaf (BadChar/BadEscape/…) — a non-const arg would not reach here; fingerprint
                    // by its debug form defensively so two distinct defects do not collide.
                    out.push('?');
                    out.push_str(&format!("{other:?}"));
                }
            }
            out.push(';');
        }
        crate::ast::Struct::List(kids) => {
            out.push('(');
            for &k in kids {
                subtree_fingerprint(db, k, out);
            }
            out.push(')');
        }
    }
}

/// A BOUNDED structural node count of the arena subtree at `id`, saturating at `cap` — the crude
/// body-SIZE proxy the inline COST HEURISTIC (Addendum 4) compares against `INLINE_COST_THRESHOLD`. It
/// counts raw AST nodes (atoms + lists), NOT lowered Core ops, because the decision happens on the hot
/// lower path BEFORE `core_of` runs on the body; a raw-node count is a stable, cheap over-approximation
/// (a body that is large in nodes is large in emitted code). Saturating keeps a pathologically large body
/// O(cap), not O(body). This is a HEURISTIC input, so precision past the threshold is irrelevant.
pub(super) fn bounded_node_count(db: &Db, id: StructId, cap: u32) -> u32 {
    fn go(db: &Db, id: StructId, cap: u32, acc: &mut u32) {
        if *acc >= cap {
            return;
        }
        *acc += 1;
        if let crate::ast::Struct::List(kids) = db.ast.get(id) {
            for &k in kids {
                if *acc >= cap {
                    return;
                }
                go(db, k, cap, acc);
            }
        }
    }
    let mut acc = 0;
    go(db, id, cap, &mut acc);
    acc
}

/// Whether the argument subtree at `arg` CAPTURES A RUNTIME BINDING — a name resolving to a `Param`
/// (or a `Ref` to one) whose binder lies OUTSIDE `arg` itself. Such an argument depends on runtime data
/// (the captured param's per-call value), so it is NOT compile-time-known and a `const` parameter cannot
/// accept it — the specialization declines and the gate raises the coded "must be compile-time-known"
/// error. A capture of the arg's OWN internal lambda param (`(fn (x) (+ x 1))`'s `x`) is fine (it is bound
/// WITHIN `arg`); only a binder outside `arg` is a runtime capture. A reference to a top-level def /
/// prelude is not a param, so it is fine (re-resolves in the copy). This is the general closedness check
/// (replaces the earlier `own`-params-only test, which missed a capture of a CALLER's param).
pub(super) fn arg_captures_runtime_binding(db: &mut Db, arg: StructId) -> bool {
    fn go(db: &mut Db, id: StructId, root: StructId) -> bool {
        if db.ast.as_name(id).is_some()
            && let Resolved::Ref { value } | Resolved::Param { binder: value } = resolved_of(db, id)
            && !db.is_within(value, root)
        {
            // A param binder OUTSIDE the arg subtree — a runtime capture. (A `Ref` to a top-level def's
            // lambda body is also `!is_within`, but a def resolves to `Lambda`, not `Param`/`Ref`-to-param;
            // only a genuine param/local ref matches the pattern above.)
            return matches!(
                resolved_of(db, id),
                Resolved::Param { .. } | Resolved::Ref { .. }
            ) && is_param_or_local_binder(db, value);
        }
        if let crate::ast::Struct::List(kids) = db.ast.get(id) {
            return kids.clone().iter().any(|&k| go(db, k, root));
        }
        false
    }
    // Whether `binder` is a PARAMETER or a runtime `let`-local binder occurrence (a runtime binding),
    // as opposed to a top-level def / type / prelude node (compile-time-resolvable, re-resolves in a copy).
    fn is_param_or_local_binder(db: &mut Db, binder: StructId) -> bool {
        // A param/local binder is a user NAME node that is not a top-level def head. A top-level def's
        // body/name resolves to a `Lambda`, never reaching here as a bare `Param`/local `Ref` target.
        db.ast.as_name(binder).is_some() && db.def_index_by_body(binder).is_none()
    }
    go(db, arg, arg)
}

/// Whether a FUNCTION-TYPED `const` argument is LEXICALLY CLOSED (references no enclosing runtime binding),
/// judged by NAME rather than the arena parent chain — so it is immune to the β-substitution parent-THEFT
/// that makes `arg_captures_runtime_binding` FALSE-flag a genuinely closed lambda. The const-wrapper-chain
/// false-reject (`sum→fold→drive` forwarding `fn(a,x)=>a+x`) arises when a spliced source lambda's single
/// mutable parent pointer is stolen, so its OWN-param references look "outside" it to `is_within`. This
/// walk descends TOP-DOWN (theft corrupts a node's PARENT pointer, never its child structure) tracking
/// names BOUND at each point by SPELLING (the lambda's own params + nested `fn` binders). A bare name that
/// is neither bound NOR a resolvable global (a top-level def) — one resolving to a runtime Param/local-ref,
/// or to Poison (an orphaned free var in a β-copy) — is a genuine capture → NOT closed. This only RESCUES
/// an arg the primary `arg_captures_runtime_binding` already flagged; a self-recursive DIVERGING re-pass
/// (the derived-closure twin, closed post-substitution but whose arg fingerprint strictly EXTENDS the
/// prior spec's each depth) is caught separately by the fingerprint-extension backstop at the const-arg
/// gate, so this needn't prove termination itself.
pub(super) fn const_arg_is_lexically_closed(db: &mut Db, a: crate::ast::StructId) -> bool {
    if !matches!(crate::infer::type_of(db, a), crate::ty::Ty::Fn(_, _)) {
        return false;
    }
    !body_captures_free_runtime_name(db, a, &mut Vec::new())
}

/// The lexical free-name walk behind `const_arg_is_lexically_closed`. `bound` is the stack of names bound
/// by scopes enclosing `node` within the candidate lambda (its own params, then nested `fn` binders).
/// Returns true if some bare name is neither bound nor a resolvable global — a genuine enclosing capture.
/// Structural top-down descent only; never reads a parent pointer (`is_within`), so it is theft-immune.
pub(super) fn body_captures_free_runtime_name(
    db: &mut Db,
    node: crate::ast::StructId,
    bound: &mut Vec<String>,
) -> bool {
    if let Some(tail) = db.ast.as_form(node, "fn").map(<[_]>::to_vec)
        && let (Some(&params_occ), Some(&body)) = (tail.first(), tail.get(1))
        && let crate::ast::Struct::List(params) = db.ast.get(params_occ)
    {
        let param_names: Vec<String> = params
            .clone()
            .iter()
            .filter_map(|&p| {
                db.ast
                    .as_name(crate::eval::param_name_occ(db, p))
                    .map(str::to_string)
            })
            .collect();
        let added = param_names.len();
        bound.extend(param_names);
        let captured = body_captures_free_runtime_name(db, body, bound);
        bound.truncate(bound.len() - added);
        return captured;
    }
    if let Some(name) = db.ast.as_name(node).map(str::to_string)
        && !bound.contains(&name)
    {
        let captures = match resolved_of(db, node) {
            Resolved::Param { .. } => true,
            // A Ref to a top-level def body is a GLOBAL (re-resolves in any copy); a Ref to a param/local
            // binder is a capture.
            Resolved::Ref { value } => {
                db.ast.as_name(value).is_some() && db.def_index_by_body(value).is_none()
            }
            // An orphaned free var whose binding a β-copy lost — treat as a capture. A global never
            // resolves to Poison (it re-resolves by name).
            Resolved::Poison(_) => true,
            _ => false, // a prelude record / prim / literal / type — compile-time-stable globals
        };
        if captures {
            return true;
        }
    }
    if let crate::ast::Struct::List(kids) = db.ast.get(node) {
        return kids
            .clone()
            .iter()
            .any(|&k| body_captures_free_runtime_name(db, k, bound));
    }
    false
}

/// A COMPACT source-like rendering of an arena subtree, for the `Instantiations` query's description of
/// a `const`-inlined argument (a dictionary / constant) — so a report shows the concrete value an
/// ad-hoc-polymorphic call baked in, not an opaque fingerprint. A best-effort human view, NOT a
/// round-trippable printer: an atom renders its leaf text; a list renders `(head child…)`. A deep or
/// wide subtree is ELIDED to `…` past a small bound (a dictionary is small; a large inlined value would
/// bloat one line and is not the distinguishing data). Never enters `db` mutation — a pure read.
pub(super) fn render_arg_node(db: &Db, id: crate::ast::StructId) -> String {
    fn go(db: &Db, id: crate::ast::StructId, depth: usize, out: &mut String) {
        match db.ast.get(id) {
            crate::ast::Struct::Atom(_) => {
                if let Some(n) = db.ast.as_name(id) {
                    out.push_str(n);
                } else if let Some(s) = db.ast.as_str(id) {
                    out.push('"');
                    out.push_str(s);
                    out.push('"');
                } else if let Some(i) = db.ast.as_int(id) {
                    out.push_str(&i.to_decimal_string());
                } else if let Some(sym) = db.ast.as_sym(id) {
                    out.push_str("#\"");
                    out.push_str(sym);
                    out.push('"');
                } else {
                    // A float / char / bytes / bool / marker leaf — a rare const value; render a
                    // placeholder rather than reach for each leaf's spelling (this is a description, not
                    // the canonical printer, which lives in `cadenza-syntax`).
                    out.push('_');
                }
            }
            crate::ast::Struct::List(items) => {
                if depth >= 4 {
                    out.push('…');
                    return;
                }
                out.push('(');
                for (i, &c) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    if i >= 6 {
                        out.push('…');
                        break;
                    }
                    go(db, c, depth + 1, out);
                }
                out.push(')');
            }
        }
    }
    let mut out = String::new();
    go(db, id, 0, &mut out);
    out
}

/// Monomorphize a RECURSIVE GENERIC def `callee` for a call whose arguments have the concrete types at
/// this site: synthesize (once, memoized) a COPY of the def whose parameters are re-annotated with those
/// concrete types, and return the copy's `db.defs` index. The copy is an ordinary MONOMORPHIC def — its
/// `def_scheme`/`def_params` give it concrete machine valtypes, and `layout` emits it as its own function
/// (reached via the `Core::Call` this returns). A self-call inside the copied body re-resolves to the
/// original def by name and, threaded with the same-typed args, re-enters here and hits the memo — so the
/// recursion points at the specialization itself, exactly as `effects::specialize_recursive` closes its
/// own recursion. Keyed by `(callee-body, concrete-signature-string)` so the same def at the same types
/// reuses ONE function (byte-identical copies dedup by construction — same key). Returns `None` if an
/// argument type is undetermined (`Any`/a free `Var`) — the call declines rather than baking a loose
/// annotation. Modeled on `effects::specialize_recursive` (minus the trailing state params).
/// Whether `ty` is a runtime COLLECTION (List/Map/Set) — a value whose length/shape drives a fold's
/// recursion. Used to gate the `const`-collection-consumed-by-a-recursive-fold miscompile guard in
/// `type_specialize` (a const SCALAR / record / type is fine; only a collection folds down a spine).
pub(super) fn is_collection_ty(ty: &crate::ty::Ty) -> bool {
    matches!(
        ty,
        crate::ty::Ty::List(_) | crate::ty::Ty::Map(_, _) | crate::ty::Ty::Set(_)
    )
}

pub(super) fn type_specialize(
    db: &mut Db,
    callee: usize,
    args: &[StructId],
) -> Option<(usize, Vec<usize>)> {
    let orig_body = db.defs[callee].body?;
    let orig_params = db.defs[callee].params.clone();
    if orig_params.len() != args.len() {
        return None; // a partial/over-application never reaches a recursive-generic Core::Call
    }
    // Classify each argument. A TYPE-VALUED argument (its `type_of` is `Ty::Type` — the caller passed a
    // TYPE, e.g. `(len Int64 …)`) is compile-time-only: it is CONSUMED here (its concrete type-value is
    // substituted into the copy's body and it is dropped from both the specialized signature and the
    // runtime call). Every OTHER argument's concrete TYPE re-annotates its (kept) parameter. Both the
    // type-VALUE (for a type arg) and the type (for a value arg) must be DETERMINED — a loose `Any`/free
    // `Var` would mistype the copied body.
    // EXPLICIT `const` rule (operator, 2026-07-14; Addendum 3): a parameter the AUTHOR marked `const`
    // (recorded in `db.const_params`) is compile-time-known — its argument is inlined into the specialized
    // copy and the parameter ERASED from the runtime signature. No heuristic detection; the declaration is
    // the trigger. A `Ty::Type`-typed argument is the type-valued special case (a type is a const value),
    // erased the same way for back-compat even without an explicit `const`. Every OTHER argument is an
    // ordinary runtime value, kept + re-annotated with its concrete type. A `const` argument that does NOT
    // reduce to a closed compile-time value → `None` (the gate raises the coded "must be
    // compile-time-known" error). All arg/param types must be DETERMINED (a loose `Any`/`Var` mistypes the
    // copy).
    enum ArgKind {
        // A runtime value param, kept, re-annotated with this concrete type.
        Value(crate::ty::Ty),
        // A type-valued param: its concrete type-VALUE, substituted into the copy's body (param ERASED).
        TypeArg(crate::ty::Ty),
        // A `const` param's argument (a dictionary / constant): the arg's VALUE NODE is substituted into
        // the copy's body (so `(. d op)` folds to the concrete op) and the param ERASED. `String` = a
        // structural fingerprint of the arg (the memo/dedup key, stable across the arg and its β-copy).
        ConstArg(StructId, String),
    }
    let mut kinds: Vec<ArgKind> = Vec::with_capacity(args.len());
    for (&a, &p) in args.iter().zip(orig_params.iter()) {
        let is_const = db
            .const_params
            .contains(&crate::eval::param_name_occ(db, p));
        if matches!(crate::infer::type_of(db, a), crate::ty::Ty::Type) {
            // A type-valued argument — reduce it to the concrete type it names (`Int64` → `Ty::Int`).
            let tv = crate::eval::typeval_of(db, a)?;
            if matches!(tv, crate::ty::Ty::Any) || tv.has_free_var() {
                return None; // an undetermined type-value cannot monomorphize
            }
            kinds.push(ArgKind::TypeArg(tv));
        } else if is_const {
            // A `const` parameter — its argument MUST be a closed compile-time value (capturing no runtime
            // binding, from THIS def or an enclosing one). If it captures a runtime param, the contract is
            // violated → decline (the gate raises the coded error). Otherwise inline the arg's value node.
            if arg_captures_runtime_binding(db, a) && !const_arg_is_lexically_closed(db, a) {
                // The primary AST parent-chain walk flagged a capture. But its `is_within` is corrupted by
                // β-substitution parent-THEFT (a spliced source lambda's own params look "outside" it), so
                // cross-check with the theft-immune lexical free-name walk: only truly reject when the arg
                // is NOT lexically closed. This rescues the const-wrapper-chain false-reject
                // (`fn(a,x)=>a+x` forwarded through `sum→fold→drive`). A lexically-closed arg that
                // nonetheless DIVERGES on a self-recursive re-pass (the derived-closure twin
                // `fn(x)=>(step x)+1`, closed post-substitution but whose fingerprint strictly extends the
                // prior spec's each depth) is caught by the fingerprint-extension backstop below.
                return None; // the const arg depends on runtime data — not compile-time-known
            }
            // WARNING: MISCOMPILE GUARD: a `const` COLLECTION param (List/Map/Set) consumed by a SELF-RECURSIVE
            // fold cannot be specialized correctly here yet. The recursion passes a DERIVED shorter
            // collection (`(s t …)` where `t` is the rest of the const list) at each depth, but its arg
            // node is the SAME rest-binder occurrence every time, so the syntactic fingerprint collapses
            // ALL depths to ONE specialization → the recursion re-enters that spec → the tail-loop
            // transform emits a `loop { … br 0 }` whose exit test (the `(list)`-nil / length check) was
            // const-erased away ⇒ an INFINITE LOOP on a valid program (a value HANGS). A folded-value
            // fingerprint (to fully unroll) needs `core_of`, which memoizes into the arg node the
            // specialization then SPLICES into the copy — corrupting a `const`-DICTIONARY consumer's
            // lowering. Until the unroll is wired safely, DECLINE this composition (decline-don't-
            // miscompile): a coded compile error beats a runtime hang, and the RUNTIME-collection version
            // (no `const`) compiles + runs correctly. A NON-recursive const collection, or a const SCALAR /
            // DICTIONARY (unchanged through the recursion), is unaffected — only a const collection the
            // callee recursively folds over.
            if is_collection_ty(&crate::infer::type_of(db, a))
                && crate::eval::is_recursive(db, orig_body)
            {
                return None;
            }
            let mut fp = String::new();
            subtree_fingerprint(db, a, &mut fp);
            // A const CLOSURE argument's `subtree_fingerprint` is the AST of the ARG EXPRESSION — which for
            // a bare reference (`step`, a variant-payload binder threaded to a driver) is just the NAME
            // `nstep;`, IDENTICAL across two call sites that pass DIFFERENT closures (e.g. `sum` inlined
            // from a `map` chain vs a `filter` chain both call `drive(step, …)` where `step` names the same
            // parameter). That collapses two distinct specializations to one memo key → the second call
            // REUSES the first's spec → a WRONG-VALUE miscompile (module-scale: the 2nd export/@test runs
            // the 1st's closure). Disambiguate by the arg's RESOLVED closure identity: when `core_of(a)` is
            // a lifted `Core::Closure { code, captures }`, append the lifted-function `code` (distinct per
            // distinct lambda) + a fingerprint of each captured value (two closures of the same `code` but
            // different captures — e.g. `(fn (x) (+ x k))` at k=1 vs k=2 — must also differ). This keys the
            // memo on WHICH closure is passed, not the syntactic name, so `map`'s and `filter`'s driver
            // specializations are distinct.
            // SELF-RECURSIVE const re-pass INSIDE a specialized copy: if the arg is a bare re-pass of this
            // callee's OWN const param (by name) and a spec of THIS def has recorded that param's fingerprint
            // (`db.const_repass_fp`, keyed by `(orig_body, name)`), INHERIT the recorded fingerprint verbatim
            // rather than probing `core_of`. On a MIXED-match recursive arm (a recursive arm beside a
            // value-returning sibling) the re-passed `step` is UNBOUND in the orphan copy, so `core_of(step)`
            // is a Poison → no `|clos` suffix → a DIFFERENT memo key from the enclosing spec → a divergent
            // second spec whose body carries the unbound `step` (the const-param-drop / CDZ0101 bug
            // v-iterators' filter-map hit — a recursive const re-pass on a mixed-match arm). Inheriting the
            // recorded fingerprint makes this recursive call's key MATCH the enclosing spec's, so the
            // recursion closes on ONE spec (re-enters the working instance) and the orphaned body is never
            // lowered. Keyed by NAME (the copy preserves the source name); only fires for a genuine self-repass
            // of one of THIS def's const params for which a fingerprint was recorded.
            // Inherit the recorded fingerprint ONLY when this arg is a self-recursive const re-pass that
            // RESOLVES TO POISON — the orphan-copy case (a mixed-match recursive arm where the erased const
            // param is unbound, so `core_of` would yield a divergent key). A CLEANLY-RESOLVING recursive
            // re-pass (the module-scale `drive(step,…)` where `step` still resolves to its distinct wrapped
            // closure) must NOT inherit: it computes its OWN correct `|clos<code>`, and inheriting from the
            // `(orig_body,name)`-keyed record would cross-pollute between two specs of the SAME driver at
            // DIFFERENT closures (em's map-step vs ef's filter-step → the collision `5dee1c97f` fixed). So
            // the record is a FALLBACK only for the unbound orphan, never an override of a good resolution.
            let arg_name = db.ast.as_name(a).map(|s| s.to_string());
            let arg_is_poison = matches!(resolved_of(db, a), Resolved::Poison(_));
            let inherited = if arg_is_poison {
                arg_name.as_ref().and_then(|n| {
                    let is_own_const_param = orig_params.iter().any(|&pp| {
                        let occ = crate::eval::param_name_occ(db, pp);
                        db.const_params.contains(&occ) && db.ast.as_name(occ) == Some(n.as_str())
                    });
                    if is_own_const_param {
                        db.const_repass_fp.get(&(orig_body, n.clone())).cloned()
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            // Whether `subtree_fingerprint` produced a BARE NAME (`nstep;`) rather than a compound AST (a
            // lambda literal `(nfn;…)`, an application, a record). A bare name is the AMBIGUOUS case the
            // `|clos` closure-identity augmentation exists for: two call sites passing DIFFERENT closures via
            // the SAME parameter name (`drive(step,…)` from a map chain vs a filter chain) fingerprint
            // identically as `nstep;` and would collide (`5dee1c97f`). A COMPOUND fp (a lambda literal) is
            // ALREADY self-identifying — its full AST distinguishes distinct lambdas — AND appending `|clos`
            // there is HARMFUL: `core_of` lifts a FRESH `code` per copy, so a const-lambda arg re-passed on a
            // recursive edge gets `|clos2` in the copy vs `|clos1` at the main call → key divergence → the
            // divergent-second-spec CDZ0101 (the both-const `f` case of the const-param-drop bug). So append
            // `|clos` ONLY for a bare-name fp; a compound fp needs no augmentation and is stable across copies.
            let fp_is_bare_name = db.ast.as_name(a).is_some();
            if let Some(inh) = inherited {
                fp = inh;
            } else if fp_is_bare_name
                && matches!(crate::infer::type_of(db, a), crate::ty::Ty::Fn(_, _))
                // ONLY probe `core_of` when the arg is FUNCTION-TYPED — calling `core_of` on a compound that
                // CONTAINS a lambda (a `const` dictionary `(record (op (fn …)))`) would SPECULATIVELY LIFT
                // that lambda (`lower_lambda_value`) and pollute `db.captured_ref`, corrupting the
                // dictionary's own lowering (the same hazard `should_keep_binding` guards against). Combined
                // with the bare-name gate above, this fires only for a directly-function-typed BARE-NAME arg
                // (the driver-`step` module-scale collision case): its `subtree_fingerprint` is the ambiguous
                // `nstep;`, so disambiguate by the arg's RESOLVED closure identity — append the lifted `code`
                // (distinct per lambda) + a fingerprint of each capture, keying the memo on WHICH closure is
                // passed (`5dee1c97f`).
                && let Core::Closure { code, captures } = core_of(db, a)
            {
                fp.push_str("|clos");
                fp.push_str(&code.to_string());
                // DELIMIT the numeric code with a non-digit terminator: the termination backstop below
                // detects a diverging re-pass by SUBSTRING containment (`fp.contains(prev)`), and an
                // UN-delimited code lets a shorter code be a substring of a longer one — `clos2` ⊂ `clos25`
                // — so a lower-numbered lambda-lift `code` FALSE-fires the backstop against a higher-numbered
                // one whenever the whole-program lambda-lift ORDER assigns codes with that prefix relation
                // (e.g. a `let`-destructure vs `match` twin shifts the lift order 2/25 → the fold const-arg
                // was spuriously declined CDZ0201 — §8.1 DESIGN-binding-patterns-rcdzc). The `;` cannot occur
                // in a decimal code, so `clos2;` is not a substring of `clos25;`, while a GENUINELY nested
                // re-pass fp still contains the prior verbatim (delimiters and all) — the real divergence
                // guard is preserved.
                fp.push(';');
                for &cap in captures.iter() {
                    fp.push(':');
                    subtree_fingerprint(db, cap, &mut fp);
                }
            }
            // TERMINATION BACKSTOP (decline-don't-hang): a self-recursive def re-passing a closure DERIVED
            // from its own const param (the twin `fn(x)=>(step x)+1`) nests the prior depth's closure, so
            // its fingerprint STRICTLY EXTENDS the immediately-preceding spec's for THIS def (contains it +
            // longer): 22→46→70→… unbounded → a non-terminating spec unroll that HANGS. A genuine
            // const-closure consumer (even a map/filter/fold fusion pipeline) passes DISTINCT unrelated
            // closures to a driver — successive fps do NOT extend each other (probed across the whole
            // iterators suite: zero extensions). So decline a re-pass whose fp strictly extends the last
            // spec's for this `orig_body` — HERE, at the gate, before the divergent (malformed) spec builds,
            // so the reject is the coded CDZ0201 not a downstream "no local slot". Keyed per orig_body on
            // the IMMEDIATELY-preceding spec (not any-pair containment, which false-fires when a later
            // `nstep;|clos1` contains an earlier `nstep;`). Per-compile-scoped (fresh `Db` per compile).
            if matches!(crate::infer::type_of(db, a), crate::ty::Ty::Fn(_, _))
                && crate::eval::is_recursive(db, orig_body)
            {
                if let Some(prev) = db.last_const_closure_fp.get(&orig_body)
                    && fp.len() > prev.len()
                    && fp.contains(prev.as_str())
                {
                    return None; // diverging derived-closure re-pass — decline rather than unroll forever
                }
                db.last_const_closure_fp.insert(orig_body, fp.clone());
            }
            kinds.push(ArgKind::ConstArg(a, fp));
        } else {
            let mut t = crate::infer::type_of(db, a);
            // A BARE CLOSURE argument types `(-> Any … R)` — its unannotated params contribute `Any`
            // (`type_of`'s Lambda arm), so the specialized copy would annotate this parameter with a type
            // carrying nested `Any`, which `encode_ty` renders as `Unit` — a spurious `(-> Unit … Int64)`
            // that then conflicts CDZ0203 when the copy applies the closure. The closure's OWN body DOES
            // determine its params (`(fn (x a) (+ a x))` → both Int64), the same solve `lower_lambda_value`
            // runs; run it HERE so the specialized annotation is concrete. Only fires when the arg is a
            // lambda AND its type still has an `Any` hole (a fully-annotated / already-solved closure is
            // untouched — byte-identical). If a hole SURVIVES the solve (a genuinely unconstrained param),
            // DECLINE rather than annotate an unrepresentable `Unit` — a coded decline beats a miscompiled
            // annotation. A NON-lambda arg keeps the prior behavior (its nested `Any`, if any, is untouched).
            if t.has_any()
                && let (Some(lam_params), Some(lam_body)) = (
                    crate::eval::lambda_params_of(db, a),
                    crate::eval::lambda_body(db, a),
                )
            {
                match crate::infer::solved_lambda_arrow(db, &lam_params, lam_body) {
                    Some(solved) if !solved.has_any() => t = solved,
                    // The body-only solve left a hole — a PASS-THROUGH closure whose result is determined
                    // only by its DOMAIN (`(fn (s) s)` identity). Recover the closure's expected arrow from
                    // the callee's INSTANTIATED parameter type at this position (the domain tie pins it, e.g.
                    // `gmap`'s `f` at a `Iter String` instantiation is `(-> String ?b)`), then solve the body
                    // UNDER that domain so the pass-through result takes the domain's type. This is the
                    // monomorphization-copy half of the transformer closure tie (the call-site half is in
                    // `apply_scheme_to_args`); both are needed so the copy's closure annotation is concrete at
                    // EACH instantiation. Only a fully-concrete recovered arrow is accepted; else decline.
                    _ => {
                        let recovered = args
                            .iter()
                            .position(|&x| x == a)
                            .and_then(|pos| {
                                crate::infer::instantiated_param_arrow(db, callee, args, pos)
                            })
                            .and_then(|arrow| {
                                crate::infer::solved_lambda_arrow_under(
                                    db,
                                    &lam_params,
                                    lam_body,
                                    &arrow,
                                )
                            });
                        match recovered {
                            Some(solved) if !solved.has_any() && !solved.has_free_var() => {
                                t = solved
                            }
                            _ => return None,
                        }
                    }
                }
            }
            if matches!(t, crate::ty::Ty::Any) || t.has_free_var() {
                return None;
            }
            kinds.push(ArgKind::Value(t));
        }
    }
    // The positions dropped from the runtime call — a type arg OR a const-inlined arg (both erased).
    let type_arg_positions: Vec<usize> = kinds
        .iter()
        .enumerate()
        .filter(|(_, k)| matches!(k, ArgKind::TypeArg(..) | ArgKind::ConstArg(..)))
        .map(|(i, _)| i)
        .collect();

    // MEMO KEY: the def's body occurrence + the rendered concrete signature (EVERY arg's type-value or
    // type contributes — so `len` at `Lst Int64` and `Lst String` are distinct, and a type arg's VALUE
    // `Int64` distinguishes it from `String`). Two calls at the same instantiation share ONE specialization.
    let key: String = kinds
        .iter()
        .map(|k| match k {
            ArgKind::Value(t) => t.render_name(&db.name_ctx()),
            ArgKind::TypeArg(tv) => format!("@{}", tv.render_name(&db.name_ctx())), // `@` marks a type ARG slot
            ArgKind::ConstArg(_, fp) => format!("#{fp}"), // `#` marks a const-inlined slot
        })
        .collect::<Vec<_>>()
        .join(",");
    let memo_key = (orig_body, key);
    if let Some(&idx) = db.type_specializations.get(&memo_key) {
        return Some((idx, type_arg_positions));
    }

    // Each parameter's NAME (a bare name `p` or the inner name of `(: p T)`). A non-name binder declines.
    let mut param_names: Vec<String> = Vec::with_capacity(orig_params.len());
    for &p in &orig_params {
        let name = match db.ast.as_name(p) {
            Some(n) => n.to_string(),
            None => match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => db.ast.as_name(name_occ)?.to_string(),
                None => return None,
            },
        };
        param_names.push(name);
    }

    // The specialized NAME — unique per (def, instantiation). `#` makes it unspellable in source (no user
    // collision); the def-count suffix keeps distinct specializations distinct.
    let base = db.defs[callee].name.clone();
    let spec_name = format!("{base}#mono{}", db.defs.len());

    // Build the specialized signature `(spec (: p T)…)` — a VALUE param is kept and annotated with its
    // concrete type; a TYPE-VALUED param is OMITTED entirely (compile-time-only, no runtime slot). The
    // type-valued param's references in the body are substituted with its concrete type expression below,
    // so the omitted param needs no binder. `arg_of` collects those substitutions (keyed on the original
    // param's NAME occurrence, the identity a body reference resolves to).
    let spec_name_atom = db.push_name(&spec_name);
    let mut sig_children = vec![spec_name_atom];
    let mut arg_of: crate::fxhash::FxHashMap<StructId, StructId> =
        crate::fxhash::FxHashMap::default();
    for ((name, kind), &orig_p) in param_names.iter().zip(kinds.iter()).zip(orig_params.iter()) {
        match kind {
            ArgKind::Value(ty) => {
                let name_atom = db.push_name(name);
                let ty_expr = crate::eval::encode_typeval(db, ty);
                let colon = db.push_name(":");
                sig_children.push(db.push_list(vec![colon, name_atom, ty_expr]));
            }
            ArgKind::TypeArg(tv) => {
                // Substitute the CONCRETE type expression for the type param's references (`(Lst t)` →
                // `(Lst Int64)`): map the param's NAME occurrence → an `encode_typeval` node of its
                // type-value. The param itself is dropped from the signature (erased).
                let name_occ = crate::eval::param_name_occ(db, orig_p);
                let ty_expr = crate::eval::encode_typeval(db, tv);
                arg_of.insert(name_occ, ty_expr);
            }
            ArgKind::ConstArg(arg_node, _) => {
                // Substitute the arg's VALUE NODE for the param's references (`d` → `(record (op (fn …)))`),
                // so a body `(. d op)` projects the const record → the concrete lambda → and its application
                // β-folds to a direct op (no `call_indirect`, no runtime record). The param is dropped from
                // the signature (erased). The substituted node is the caller-side arg subtree, spliced into
                // the copy — the same mechanism `beta_reduce` uses for a substituted lambda argument.
                let name_occ = crate::eval::param_name_occ(db, orig_p);
                arg_of.insert(name_occ, *arg_node);
            }
        }
    }
    let sig = db.push_list(sig_children.clone());
    let spec_params: Vec<StructId> = sig_children[1..].to_vec();

    // RESERVE the def NOW (body filled after the copy) + MEMOIZE — so the recursive self-call inside the
    // copied body, re-entering `type_specialize` with the same instantiation, hits the memo and names THIS
    // spec (closing the recursion at the specialization, not the generic original).
    let spec_index = db.defs.len();
    db.push_specialized_def(crate::db::Def {
        name: spec_name.clone(),
        sig_occ: sig,
        params: spec_params.clone(),
        body: None,
        internal: false,
    });
    db.type_specializations.insert(memo_key, spec_index);

    // Record this DISTINCT instantiation for the `Instantiations` query — one human-readable entry per
    // parameter, in signature order (a kept runtime param as `name: TYPE`, an erased compile-time param
    // as `const name = VALUE`). Appended HERE, at the memo MISS, so the list holds one record per
    // synthesized specialization and never double-counts a call site that reuses an instance. The
    // const-argument description renders the inlined SOURCE (`render_arg_node`), not the memo's opaque
    // fingerprint — so a report can show which concrete dictionary an ad-hoc-polymorphic call baked in.
    let inst_args: Vec<String> = param_names
        .iter()
        .zip(kinds.iter())
        .map(|(name, kind)| match kind {
            ArgKind::Value(ty) => format!("{name}: {}", ty.render_name(&db.name_ctx())),
            ArgKind::TypeArg(tv) => format!("const {name} = {}", tv.render_name(&db.name_ctx())),
            ArgKind::ConstArg(arg_node, _) => {
                format!("const {name} = {}", render_arg_node(db, *arg_node))
            }
        })
        .collect();
    db.instantiations.push(crate::db::Instantiation {
        orig_body,
        spec_index,
        args: inst_args,
    });

    // Copy the body structurally (fresh occurrences that re-resolve against the new def's scope): a body
    // reference to a VALUE param re-resolves by name to the new annotated binder; a TYPE-VALUED param's
    // references are SUBSTITUTED with its concrete type expression (`arg_of`); a self-call `(callee …)`
    // re-resolves by name to the ORIGINAL def and, lowered with the same-typed args, re-enters here → memo.
    // `own_params` (the original params' name occurrences) copy + re-resolve against the specialized sig;
    // every OTHER free reference (a self-call, or a do-local sibling whose lexical scope the copy escapes)
    // is pinned + SHARED (without the pin a do-local generic's self-call re-resolved unbound, CDZ0101).
    let own_params: Vec<StructId> = orig_params
        .iter()
        .map(|&p| crate::eval::param_name_occ(db, p))
        .collect();
    // RECORD this spec's const-param fingerprints, keyed by `(orig_body, param NAME)`, so a SELF-RECURSIVE
    // const re-pass of THIS def's const param — lowered later, demand-driven — inherits the identical
    // fingerprint (see `const_repass_fp`), keeping the recursive call's memo key equal to this spec's so the
    // recursion closes on ONE spec. Recorded before the body copy (the recursive call may re-enter
    // `type_specialize` during the copy OR during a later demand-driven lower of the spec body). A second
    // spec of the same def at a DIFFERENT instantiation overwrites its own `(orig_body,name)` entry with the
    // matching fingerprint — correct, since a recursive re-pass belongs to whichever spec is currently being
    // lowered and every spec threads the SAME const closure through its own recursion.
    for (name, kind) in param_names.iter().zip(kinds.iter()) {
        if let ArgKind::ConstArg(_, fp) = kind {
            db.const_repass_fp
                .insert((orig_body, name.clone()), fp.clone());
        }
    }
    let spec_body = crate::eval::copy_structural_pub(db, orig_body, &own_params, &arg_of);

    // Wrap in a real `(def (spec params…) body)` arena node so the parent index links param → sig → def
    // (`is_param_occurrence` walks that chain; `binder_in` resolves a body reference against the sig).
    let def_head = db.push_name("def");
    let _def_form = db.push_list(vec![def_head, sig, spec_body]);

    db.fill_specialized_def(spec_index, spec_params, spec_body);
    // Register any do-local recursive functions the copy introduced (the copy-time twin of load-time
    // registration), so a nested recursive call in the copied body lowers to a `Core::Call`.
    db.register_reduced_callables(spec_body);
    Some((spec_index, type_arg_positions))
}

/// The `db.defs` index of the top-level def an application head names, if any — following a `Ref` to a
/// `Lambda` whose body matches a def's body occurrence. Returns `None` for a head that is not a named
/// top-level def (a `let`-bound lambda, a computed head).
pub(super) fn callee_def_index(db: &mut Db, head: StructId) -> Option<usize> {
    // The head resolves to a `Lambda { body }` for a named function (a top-level def, or a MODULE MEMBER
    // via Case R) or a `Ref` chain to one; match its body back to the def index. A `Member` projection
    // `(. m f)` reduces to the field lambda via `member_value` (WITHOUT the general `lambda_of`
    // β-reduction, which would inline a deep non-recursive chain — an exponential cost on the hot lower
    // path). So a recursive MODULE MEMBER called through the projection chain finds its registered
    // internal def (`modules::register_callable`), exactly as a bare-named recursive def finds its
    // top-level def, at no cost to an ordinary call.
    match resolved_of(db, head) {
        Resolved::Lambda { body, .. } => db.def_index_by_body(body),
        Resolved::Ref { value } => callee_def_index(db, value),
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(v) => callee_def_index(db, v),
            _ => None,
        },
        _ => None,
    }
}

/// Per-binding use facts for a whole `let`, collected in ONE walk of the binding region (all initializer
/// values + the body) rather than an O(scope) walk PER binding. For each `let`-bound initializer occurrence
/// it records: `count` — total references (the `uses_in` sum), and `escapes_whole` — whether any reference
/// is a WHOLE-VALUE use (not merely a projection operand, the `ref_escapes_whole` predicate). `let*`
/// scoping means a reference to `init_k` can only appear in the LATER inits + body (an earlier init cannot
/// name a later binding), so counting across the WHOLE region set gives each init its exact in-scope
/// count — no over-count. This replaces `should_keep_binding`'s per-binding `uses_in`/`binding_escapes_whole`
/// scope walks (each O(scope)), which made a wide `let` O(bindings × body) = O(N²) (the fix-16/18 class: a
/// per-item predicate walking the whole body per item → collect the facts in ONE walk).
#[derive(Default)]
pub(crate) struct BindingUses {
    count: crate::fxhash::FxHashMap<StructId, u32>,
    escapes_whole: crate::fxhash::FxHashSet<StructId>,
    /// Bindings referenced as a DIRECT APPLICATION HEAD via a bare `Ref` — `(f1 d)` where `f1` names the
    /// binding. Such a use is a CALL (it β-folds, or `call_indirect`s a kept runtime closure), NOT a
    /// whole-value escape — so it is recorded here and NOT in `escapes_whole` (which then means "escapes
    /// as a value in a NON-called position": stored into a compound/collection, passed as an argument,
    /// returned). Used by `should_keep_binding` to detect the adv-50 CALL-BOTH-WAYS shape (a capturing
    /// lambda whose handle both `escapes_whole` AND is `called_direct`), which must be force-kept.
    called_direct: crate::fxhash::FxHashSet<StructId>,
    /// Bindings that ESCAPE WHOLE inside a SIBLING do-local value-def's INITIALIZER — `(def bag1 #list(f))`
    /// stores `f` into bag1's cell. The `do`-arm SKIPS a `(def …)` form (its own name-refs are collected at
    /// the def's by-name uses), so such an escape is NOT in `escapes_whole`/`count`. Recorded SEPARATELY here
    /// (not folded into `escapes_whole`/`count`, which feed many keep/copy-propagate decisions — broadening
    /// them regresses broadly, the reverted 3713-case attempt) and read ONLY by `should_keep_binding`'s
    /// Lambda CALL-BOTH-WAYS force-keep: a capturing lambda that escapes into a sibling def's cell AND is
    /// called directly must be MATERIALIZED once, or it is beta-substituted (a fresh `Core::Closure` per use)
    /// → the residual direct call mis-resolves its cell → invalid wasm (breaker: `#list(f)` + a residual `(f 2)`).
    escapes_in_def_init: crate::fxhash::FxHashSet<StructId>,
}

/// Walk `node` once, recording into `out` a use (and, unless `proj_operand`, a whole-value escape) for
/// every `Resolved::Ref { value }` — and a use for every `SumPayload`/`BinField`/`MapField` reading `init`
/// directly. `proj_operand` marks that `node` sits in a PROJECTION-operand position (`(. □ i)`): a bare ref
/// there is a piece-read, not a whole-value escape (mirrors `ref_escapes_whole`'s projection arm), but it
/// STILL counts as a use (mirrors `uses_in`, which counts every ref). The combined walk reproduces both
/// `uses_in` (the `count`) and `ref_escapes_whole` (the `escapes_whole` set) in a single traversal.
pub(super) fn collect_binding_uses(
    db: &mut Db,
    node: StructId,
    proj_operand: bool,
    out: &mut BindingUses,
) {
    #[cfg(test)]
    crate::db::COLLECT_BINDING_USES_VISITS.with(|c| c.set(c.get() + 1));
    // A `(do S… tail)` block resolves to `Ref{last}` (`resolve_do` collapses it), which would hide the
    // NON-FINAL statements — a discarded intermediate like `(Map.insert m 1 f1)` never reaches the
    // recursion, so `f1`'s use there goes UNCOUNTED and its whole-value ESCAPE unrecorded. That matters
    // for the adv-50 force-keep: a capturing lambda STORED into a discarded intermediate is still LIFTED
    // (the discarded statement is lowered by `subtree_reaches_host_call`, poisoning the shared capture
    // occurrence), so its handle DOES escape even though the store's value is dropped — the escape must
    // be seen here for the CALL-BOTH-WAYS force-keep to fire. Walk EVERY form directly — the TAIL (the
    // last form, the do's value) is analyzed by its own iteration here, which is MORE precise than the
    // collapsed `resolved_of(do) = Ref{last}` would be, so we `return` afterward rather than falling
    // through (the tail is NOT missed — PR #1245 amazon-q). A pure do (no discarded store) records the
    // same facts either way, so this only ADDS the previously-hidden intermediate uses — never drops one.
    if let Some(forms) = db.ast.as_form(node, "do").map(<[_]>::to_vec) {
        let last_ix = forms.len().saturating_sub(1);
        for (i, f) in forms.iter().enumerate() {
            // A do-local `(def …)` is a binding, not a value statement — its refs are collected where the
            // def is used (by name), not as a statement here; skip it (matching `compute`'s `do` split).
            if db.ast.head_name(*f) != Some("def") {
                // The TAIL (last form) IS the do's value, so it inherits the caller's `proj_operand`: a
                // `do` sitting in a projection/member operand position projects its tail, so a bare `Ref`
                // tail is a piece-read, NOT a whole-value escape. Hard-coding `false` there would spuriously
                // flag it `escapes_whole` and flip keep/copy-propagation (PR #1245 Copilot). The NON-FINAL
                // statements are sequenced (their value discarded) → never a projection operand → `false`.
                let po = if i == last_ix { proj_operand } else { false };
                collect_binding_uses(db, *f, po, out);
            } else if let Some(v) = crate::resolve::do_value_def_value(db, *f) {
                // A do-local VALUE-def `(def b <init>)` is skipped above as a binding, but its INITIALIZER
                // can ESCAPE an ENCLOSING binding whole — `(def bag1 #list(f))` stores `f` into bag1's cell.
                // Collect the init's whole-value escapes into a THROWAWAY `BindingUses` and merge ONLY its
                // `escapes_whole` into the SEPARATE `escapes_in_def_init` set — deliberately NOT touching the
                // main `count`/`escapes_whole` (those feed many keep/copy-propagate decisions; broadening them
                // regressed 3713 cases — reverted). `escapes_in_def_init` is read ONLY by the Lambda
                // force-keep, so a capturing lambda that escapes into a sibling def AND is called directly is
                // materialized once (else beta-substituted → invalid wasm), with zero effect on any other
                // binding's keep decision. A FUNCTION-def has no value-init → `None` → still skipped.
                let mut sub = BindingUses::default();
                collect_binding_uses(db, v, false, &mut sub);
                out.escapes_in_def_init.extend(sub.escapes_whole);
            }
        }
        return;
    }
    match resolved_of(db, node) {
        Resolved::Ref { value } => {
            *out.count.entry(value).or_insert(0) += 1;
            if !proj_operand {
                out.escapes_whole.insert(value);
            }
        }
        // A projection/member reads a PIECE: its operand ref does not escape, but recurse the operand (a
        // deeper whole-value use can nest, `(. (f c) 0)`), flagging the operand as a projection position.
        Resolved::Proj { operand, .. } | Resolved::Member { operand, .. } => {
            collect_binding_uses(db, operand, true, out)
        }
        Resolved::If { cond, then_, else_ } => {
            collect_binding_uses(db, cond, false, out);
            collect_binding_uses(db, then_, false, out);
            collect_binding_uses(db, else_, false, out);
        }
        Resolved::And { lhs, rhs, .. } => {
            collect_binding_uses(db, lhs, false, out);
            collect_binding_uses(db, rhs, false, out);
        }
        Resolved::Not { operand } => collect_binding_uses(db, operand, false, out),
        // `(try e)` — the operand is used as a WHOLE value (it is matched/unwrapped), so it does not
        // count as a projection position; recurse with `proj_operand = false`.
        Resolved::Try { operand } => collect_binding_uses(db, operand, false, out),
        Resolved::Let { bindings, body } => {
            for (_, v) in bindings.iter() {
                collect_binding_uses(db, *v, false, out);
            }
            collect_binding_uses(db, body, false, out);
        }
        Resolved::Record { fields } => {
            for &v in fields.values() {
                collect_binding_uses(db, v, false, out);
            }
        }
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
            for &e in elems.iter() {
                collect_binding_uses(db, e, false, out);
            }
        }
        Resolved::Map { entries } => {
            for &(k, v) in entries.iter() {
                collect_binding_uses(db, k, false, out);
                collect_binding_uses(db, v, false, out);
            }
        }
        Resolved::Bin { segs } => {
            for s in segs.iter() {
                collect_binding_uses(db, s.slot, false, out);
                match &s.kind {
                    crate::resolved::SegKind::Bytes { size: Some(sz) } => {
                        collect_binding_uses(db, *sz, false, out)
                    }
                    crate::resolved::SegKind::Utf8 { size } => {
                        collect_binding_uses(db, *size, false, out)
                    }
                    _ => {}
                }
            }
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            collect_binding_uses(db, expr, false, out)
        }
        Resolved::Apply { head, args } => {
            // A bare-name application HEAD `(f1 …)` is a CALL of the binding `f1`, not a whole-value
            // escape of it: record it as `called_direct` (and count the use) but do NOT flag
            // `escapes_whole` — a called head β-folds or `call_indirect`s, it does not store/pass the
            // handle. A NON-`Ref` head (a nested `Apply` for a curried `((f1 a) b)`, a projection, a
            // prim) recurses normally; the curried case re-enters this arm at each level, so the bottom
            // bare-`Ref` head is still recorded as the direct call. This is what lets `should_keep_binding`
            // tell the adv-50 CALL-BOTH-WAYS shape (escapes AND called) from a pure call or a pure escape.
            if let Resolved::Ref { value } = resolved_of(db, head) {
                *out.count.entry(value).or_insert(0) += 1;
                out.called_direct.insert(value);
            } else {
                collect_binding_uses(db, head, false, out);
            }
            for &a in args.iter() {
                collect_binding_uses(db, a, false, out);
            }
        }
        Resolved::Match { scrutinee, arms } => {
            collect_binding_uses(db, scrutinee, false, out);
            for (_, body) in &arms {
                collect_binding_uses(db, *body, false, out);
            }
        }
        // A `SumPayload`/`BinField`/`MapField` reading `init` directly is a piece-read: a USE (counted) but
        // NOT a whole-value escape — exactly as `uses_in` counts it and `ref_escapes_whole` rejects it.
        Resolved::SumPayload { scrutinee, .. }
        | Resolved::BinField { scrutinee, .. }
        | Resolved::MapField { scrutinee, .. }
        | Resolved::RecordField { scrutinee, .. }
        | Resolved::RecordRest { scrutinee, .. }
        | Resolved::SetRest { scrutinee, .. } => {
            *out.count.entry(scrutinee).or_insert(0) += 1;
        }
        Resolved::Handle {
            init: seed,
            arms,
            body,
        } => {
            collect_binding_uses(db, seed, false, out);
            for arm in arms.iter() {
                collect_binding_uses(db, arm.body, false, out);
            }
            collect_binding_uses(db, body, false, out);
        }
        Resolved::Resume { value, next_state } => {
            collect_binding_uses(db, value, false, out);
            collect_binding_uses(db, next_state, false, out);
        }
        Resolved::Host { body, .. } => collect_binding_uses(db, body, false, out),
        Resolved::Int(_)
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
        | Resolved::Lambda { .. }
        | Resolved::Poison(_) => {}
    }
}

/// Whether the lambda occurrence `init` CAPTURES a free variable — references an enclosing runtime
/// binding (a `let` init / an enclosing parameter). Detected LIFT-FREE by reusing [`collect_captures`],
/// which reads only `resolved_of` (it never calls `core_of` / `lower_lambda_value`), so the check does
/// not trigger the very speculative lift the adv-50 force-keep guards against. A non-capturing lambda
/// (`(fn (x) (+ x 1))`) has an empty capture set and needs no env cell, so it is not subject to the
/// captured-occurrence poison and stays copy-propagated. Returns false for a non-lambda `init`.
/// Whether `init` REDUCES to a CAPTURING lambda — a factory-CALL head `(mk-adder k)` returning
/// `(fn (x) (+ x n))`, which is an `Apply` (not a `Resolved::Lambda`), so [`lambda_is_capturing`]
/// (which only inspects a LITERAL lambda) reports `false` for it. Reduces `init` to its lambda's
/// `(params, body)` via [`crate::eval::lambda_params_and_body`] (a β-reduction that does NOT lower —
/// the same reduce-not-lower family as the `lambda_body` gate that follows, so this never triggers the
/// speculative lift it exists to avoid) and runs the lift-free lexical capture walk
/// [`body_captures_free_runtime_name`] over the reduced body with the reduced params pre-bound. Used by
/// `should_keep_binding`'s reduces-to-lambda CALL-BOTH-WAYS force-keep (the adv-50 hazard one syntactic
/// step removed).
pub(super) fn reduced_lambda_is_capturing(db: &mut Db, init: StructId) -> bool {
    let Some((params, body)) = crate::eval::lambda_params_and_body(db, init) else {
        return false;
    };
    let mut bound: Vec<String> = params
        .iter()
        .filter_map(|&p| {
            db.ast
                .as_name(crate::eval::param_name_occ(db, p))
                .map(str::to_string)
        })
        .collect();
    body_captures_free_runtime_name(db, body, &mut bound)
}

pub(super) fn lambda_is_capturing(db: &mut Db, init: StructId) -> bool {
    let Resolved::Lambda { params, body } = resolved_of(db, init) else {
        return false;
    };
    let param_occs: Vec<StructId> = params
        .iter()
        .map(|&p| crate::eval::param_name_occ(db, p))
        .collect();
    let mut captures: Vec<StructId> = Vec::new();
    let mut capture_refs: Vec<(StructId, usize)> = Vec::new();
    // `collect_captures` returns false only on an unrepresentable capture (still a capture); its OUTPUT
    // `captures` set is what tells capturing from closed. A `false` return with a non-empty set is still
    // capturing — so gate on the set, not the return.
    let _ = collect_captures(
        db,
        body,
        &param_occs,
        init,
        &mut captures,
        &mut capture_refs,
    );
    !captures.is_empty()
}

/// Whether a `let` binding whose initializer is `init` should be KEPT as a named `Core::Let` binding
/// rather than copy-propagated. Kept iff (1) its value is a RUNTIME computation — its core is not a
/// constant/atom that folds away, so following it through would re-emit the computation — AND (2) its
/// name is used MORE THAN ONCE across `scope` (the later sibling initializers and the body — naming
/// pays for itself only when it avoids a recompute). A constant, a single-use binding, or a poison is
/// propagated (byte-neutral).
///
/// `uses` carries the per-binding use facts collected in ONE walk of the whole `let` region (see
/// [`BindingUses`]); the whole-value-escape and use-count queries are O(1) lookups into it rather than an
/// O(scope) walk per binding (the O(N²) a wide `let` otherwise pays).
pub(super) fn should_keep_binding(
    db: &mut Db,
    name_occ: StructId,
    init: StructId,
    uses: &BindingUses,
) -> bool {
    // A CAPTURING lambda whose HANDLE ESCAPES WHOLE (stored into a heap collection / sum payload / passed
    // / returned) AND is ALSO DIRECTLY CALLED — the adv-50 CALL-BOTH-WAYS shape — must be FORCE-KEPT as
    // ONE materialized runtime closure. This is the DUAL of the copy-propagate short-circuit just below.
    // The hazard: the ESCAPE lowers `f1` as a value (`lower_lambda_value` LIFTS it, recording the body's
    // capturing-reference occurrences in `db.captured_ref`), while the copy-propagate short-circuit would
    // let the DIRECT call `(f1 d)` β-fold to `(+ k d)` — REUSING the SAME captured occurrence `k`, now
    // memoized as a `Core::Captured` env-read in the ENCLOSING (env-less) scope → invalid wasm module /
    // rust `__cap0` unbound (E0425). Prior faces of this family AVOIDED the lift (propagate); here the
    // escape genuinely REQUIRES the lift, so instead AVOID THE FOLD: keep the binding as a `Core::Let`
    // slot holding the ONE lifted `Core::Closure`, and route the direct call through it via
    // `call_indirect` (`head_is_runtime_fn_value` recognizes a kept lambda binding as a runtime fn value).
    // Both uses then share the single materialized cell — no fold reuses a poisoned occurrence. Gated on
    // BOTH `escapes_whole` (a non-called whole-value escape → the lift is mandatory) AND `called_direct`
    // (a bare-`Ref` application head → the folding use that would reuse the occurrence): a pure escape
    // (no direct call) has no fold to poison and lifts fine unkept, and a pure direct call (no escape)
    // folds cleanly with no lift — only the intersection miscompiles. Detected lift-free via `resolved_of`
    // + the lexical `body_captures_free_runtime_name` walk (neither `core_of`-lowers, so the detection
    // itself never triggers the very lift it guards against). TUPLE/RECORD element storage is a
    // fixed-shape UNBOXED rep handled by the `compound_contains_*` arms below (they PASS today), so this
    // only engages the boxed-cell reps (list/set/map/sum-payload) the boundary sweep found broken.
    if matches!(resolved_of(db, init), Resolved::Lambda { .. })
        && (uses.escapes_whole.contains(&init) || uses.escapes_in_def_init.contains(&init))
        && uses.called_direct.contains(&init)
        && lambda_is_capturing(db, init)
    {
        return true;
    }
    // A LAMBDA-valued binding is NEVER kept as a runtime `let` slot — it is copy-propagated so its
    // applications fold (β-reduce) at each use. Short-circuit HERE, before `is_runtime_computation` calls
    // `core_of(init)` — which for a lambda runs `lower_lambda_value`, LIFTING it speculatively and (for a
    // capturing lambda) polluting `db.captured_ref` with the body's capturing-reference occurrences. Those
    // occurrences are SHARED with the fold's reduced body (`(g 5)` → `(+ 5 k)` reuses the original `k`
    // occurrence), so the stale `captured_ref` entry would then make the FOLDED `k` lower to a
    // `Core::Captured` env-read in the ENCLOSING scope (where there is no env) — a miscompile (reading an
    // uninitialized local). Checking `resolved_of` (not `core_of`) avoids triggering the lift. A lambda
    // that genuinely escapes to runtime is lifted at its USE site, not kept here.
    if matches!(resolved_of(db, init), Resolved::Lambda { .. }) {
        return false;
    }
    // The reduces-to-lambda CALL-BOTH-WAYS keep — the adv-50 hazard ONE SYNTACTIC STEP REMOVED. When the
    // init REDUCES to a CAPTURING lambda (a factory call `(mk-adder k)` returning `(fn (x) (+ x n))`) AND
    // its handle BOTH escapes-whole (into a heap collection / sum payload / passed / returned) AND is
    // directly called, the `Resolved::Lambda` keep above does NOT catch it (the init is an `Apply`, not a
    // literal lambda), so it falls to the propagate short-circuit just below — and copy-propagating it
    // MISCOMPILES exactly as the literal-lambda case would: the ESCAPE lifts the reduced closure (recording
    // its captured occurrences in `db.captured_ref`) while the DIRECT call β-folds `(f 2)` to `(+ 2 <cap>)`,
    // REUSING the captured occurrence — now memoized as a `Core::Captured` env-read in the ENCLOSING
    // (env-less) scope → invalid wasm module / rust `__cap0` unbound (E0425). Force-keep so the reduced
    // closure is materialized ONCE as a `Core::Let` slot and the direct call routes through it via
    // `call_indirect` (`head_is_runtime_fn_value`'s kept-binding arm), both uses sharing the single cell —
    // no fold reuses a poisoned occurrence. Gated IDENTICALLY to the literal keep (escapes_whole +
    // called_direct) PLUS a lift-free lexical capturing test on the REDUCED lambda: a NON-capturing reduced
    // lambda (`(mk-const)` → `(fn (x) x)`) has no occurrence to poison and folds cleanly, so it stays on
    // the propagate path below UNCHANGED (byte-neutral). Checked BEFORE the blanket `lambda_body`
    // short-circuit, which otherwise propagates every reduces-to-lambda init.
    if crate::eval::lambda_body(db, init).is_some()
        && (uses.escapes_whole.contains(&init) || uses.escapes_in_def_init.contains(&init))
        && uses.called_direct.contains(&init)
        && reduced_lambda_is_capturing(db, init)
    {
        return true;
    }
    // A binding whose value REDUCES to a lambda — a function returning a closure, `(mk n)` returning
    // `(fn (x) (+ n x))` — is the SAME hazard one syntactic step removed: it is not a `Resolved::Lambda`
    // (it is an `Apply`), so it slips past the check above, but `is_runtime_computation`'s `core_of` below
    // would still LIFT it (`lower_lambda_value`) and pollute `db.captured_ref` with the returned closure's
    // captured occurrences. Those occurrences are SHARED with the fold's reduced body — `(let ((f (mk n)))
    // (f 3))` copy-propagates `f` and β-reduces `((mk n) 3)` to `(+ n 3)`, reusing the ORIGINAL `n`
    // occurrence — so the stale `captured_ref` entry makes the FOLDED `n` lower to a `Core::Captured`
    // env-read in the ENCLOSING scope (which has no env) → invalid wasm ("expected i32, found i64"). Detect
    // it with `lambda_body` (which reduces but does NOT lower, so it does not pollute) and propagate, so the
    // application folds inline exactly as the (working) `((mk n) 3)` and HOF-argument forms do. (The
    // reduces-to-lambda CALL-BOTH-WAYS subset that MUST be kept is diverted just above; what reaches here is
    // a pure direct-call or a non-capturing reduced lambda, both of which fold correctly.)
    if crate::eval::lambda_body(db, init).is_some() {
        return false;
    }
    // An `if`/`match` whose ARMS are capturing lambdas — `(let ((f (if b (fn (x) (+ x n)) (fn (x) (* x
    // n))))) (f 10))` — is the FOURTH face of the same hazard, one control-flow step from the plain
    // lambda. The init is a `Resolved::If`/`Match`, not a `Resolved::Lambda` nor a value that
    // `lambda_body`-reduces to one (a conditional does not reduce to a SINGLE lambda), so it slips past
    // the two lambda short-circuits above — but the `is_runtime_computation` / host-call gates below all
    // call `core_of(init)`, which LOWERS the conditional and thereby SPECULATIVELY LIFTS each arm's
    // capturing lambda (`lower_lambda_value`), polluting `db.captured_ref` with the arm bodies'
    // capturing-reference occurrences. Those occurrences are SHARED with the fold of the selected-and-
    // called closure — `(f 10)` copy-propagates `f` and the CASE-OF-CASE rewrite β-reduces `((if b (fn
    // (x) (+ x n)) (fn (x) (* x n))) 10)` to `(if b (+ 10 n) (* 10 n))`, reusing the ORIGINAL `n`
    // occurrences — so the stale `captured_ref` entries make the folded `n` lower to a `Core::Captured`
    // env-read in the ENCLOSING scope (which has no closure env): wasm reads a nonexistent capture env
    // (garbage → wrong value or an overflow-guard trap) and rust references an unbound `__cap0` (E0425).
    // Detect it STRUCTURALLY (`reduce_to_if`/`reduce_to_match` + `lambda_body`, all of which reduce but
    // do NOT lower, so the detection never lifts) and propagate, so the selected arm folds inline through
    // the case-of-case / case-of-match rewrite exactly as the non-capturing join and the record-field-
    // held join (7fdb1dcb8) controls do. No escape guard (unlike the compound case): like the plain
    // lambda short-circuit above, a conditionally-chosen closure is ALWAYS copy-propagated so its
    // application folds — it is a lambda-valued binding, never a kept runtime slot.
    if if_or_match_selects_lambda(db, init) {
        return false;
    }
    // A COMPOUND (record/tuple/list) init that CONTAINS a lambda and is only PROJECTED — never used as a
    // whole value — is the THIRD face of the same hazard. It is neither a `Resolved::Lambda` nor a value
    // that `lambda_body`-reduces to one (it is a `Resolved::Record`/`Tuple`/`List`), so it slips past both
    // short-circuits above — but the `is_compound_value` / `is_runtime_computation` / host-call gates below
    // all call `core_of(init)`, which LOWERS the compound and thereby SPECULATIVELY LIFTS its contained
    // capturing lambda (`lower_lambda_value`), polluting `db.captured_ref` with the lambda body's
    // capturing-reference occurrences. Those occurrences are SHARED with the fold of the projected-and-
    // called closure — `(let ((r (record (f (fn (x) (+ x n)))))) ((. r f) 10))` copy-propagates `r` and
    // β-reduces `((. r f) 10)` to `(+ 10 n)`, reusing the ORIGINAL `n` occurrence — so the stale
    // `captured_ref` entry makes the folded `n` lower to a `Core::Captured` env-read in the ENCLOSING
    // scope (which has no closure env): the wasm emit passes the captured i64 where the env i32 handle
    // belongs (invalid module, `func … type mismatch`) and the rust emit references an unbound `__cap0`
    // (E0425). Detect it STRUCTURALLY (`resolved_of` + `lambda_body`, which reduce but do NOT lower, so the
    // detection never lifts) and propagate, so each projection folds through exactly as the inline-record
    // and direct-let controls do. A compound that ESCAPES whole (returned / passed / nested) genuinely
    // needs materialization and is LEFT to the lift+keep path below: no inline fold shares its lambda body
    // there (the projection happens at RUNTIME via `call_indirect`), so that lift is correct, not pollution.
    // A projection-only compound whose element is a CURRIED lambda `(fn (a) (fn (b) …))` is the ONE
    // sub-shape of `compound_contains_lambda` that MUST be KEPT (materialized), not folded: folding it
    // through the projection-apply β-reduces the outer lambda, yields the INNER lambda, and inlining that
    // returned lambda dangles its param (no closure frame) → an emit decline ("no local slot") though
    // check passes. Force-keeping materializes the compound so the curried element LIFTS to a runtime
    // closure and applies via `call_indirect` (head_is_runtime_fn_value's Proj arm + the emit routing).
    // Checked BEFORE the fold-through short-circuit below. A flat/single fn element (which folds correctly
    // today) is NOT matched by `compound_contains_curried_lambda`, so it still folds — no regression.
    if !uses.escapes_whole.contains(&init) && compound_contains_curried_lambda(db, init) {
        return true;
    }
    if !uses.escapes_whole.contains(&init) && compound_contains_lambda(db, init) {
        return false;
    }
    // A binding whose value PERFORMS A HOST EFFECT is ALWAYS kept — regardless of use count. A host call
    // has OBSERVABLE ORDER AND COUNT (`core-semantics.md` §Host Calls Are Ordered And Part Of Observable
    // Behavior), so copy-propagating it is UNSOUND both ways: at ≥2 uses it re-performs the effect once per
    // use (`let a = (E.op) in a + a` → two calls), and at 0 uses it DROPS the perform entirely (`let a =
    // (E.op) in 99` → the call vanishes). Naming it as a `Core::Let` binding evaluates it EXACTLY ONCE at
    // the `let`, in program order — the strict-`let` semantics the kept `Call`/`Arith` bindings already
    // rely on. This precedes the `is_runtime_computation` / use-count gates, which would otherwise inline
    // or drop it. (Checked AFTER the two lambda short-circuits, so `subtree_reaches_host_call`'s `core_of`
    // cannot speculatively lift a lambda init.) The soundness fix a `let`-bound generator/perform needs.
    //
    // EXCLUDE a fold-through COMPOUND init (a `Core::Tuple`/`Record`/`ListNew` whose ELEMENTS perform the
    // effect): the compound is not itself a perform — it CONTAINS them, and its own element lowering
    // already evaluates each element exactly once, left to right (`let t = (tuple (E.op) (E.op)) in …`
    // keeps the two calls in construction order without keeping `t`, so a projection-only tuple still folds
    // through). Keeping such a compound would instead force an unlowerable heap build. Only an init that IS
    // (or reduces to) the perform — not one that merely aggregates performs into a fold-through shape — is
    // force-kept here; the compound's elements are single-evaluated by the compound, not by naming it.
    // The host-reach test has TWO detectors. `subtree_reaches_host_call` (AST walk) catches a DIRECT perform
    // in the init. But adv-62b (HIGH wasm soundness): when the init is a CALL whose CALLEE BODY reaches a
    // host call — `(let ((r (mk))) …)` with `mk = (host (io) (let ((v (io.get))) (record …)))` — the AST
    // walk stops at the nullary-call node `(mk)` (its only child is the `mk` name ref) and MISSES the host
    // call in mk's body, so `r` was copy-propagated and each `(. r •)` re-inlined the `(host …)` block → the
    // host op fired once per use (double-fire trap). `core_reaches_host_call` follows the call into mk's
    // INLINED core (`core_of((mk))` is a `Core::Let` whose init IS the host call) and catches it, so `r` is
    // force-kept (materialized once; every projection reads the shared slot). Gated to a CALL init (`core_of`
    // is `Core::Call`, OR the init reduced to a `Core::Let`/compound wrapping the callee body — i.e. NOT
    // already caught by the AST walk) so the extra core-walk cost stays off the common direct-perform path.
    // The call-following `core_reaches_host_call` runs ONLY when the init is a CALL (`Resolved::Apply`) —
    // the one shape whose host call the AST walk can't see (it stops at the call node). A COMPOUND-literal
    // init (`Resolved::Record`/`Tuple`/`List` holding a capturing lambda) must NOT trigger it: walking the
    // compound's core `core_of`s its contained lambda, SPECULATIVELY LIFTING it (`lower_lambda_value`) and
    // polluting `db.captured_ref` → poisons the projected-closure fold (an INVALID module — the exact hazard
    // the lambda short-circuits above avoid; it regressed `a capturing closure stored in a let-bound
    // tuple/record is projected and applied`). Those compound inits reach no host call and are handled by the
    // `compound_contains_lambda` propagate-arm above anyway; only a genuine CALL init needs the deeper walk.
    let host_reaching = subtree_reaches_host_call(db, init)
        || (matches!(resolved_of(db, init), Resolved::Apply { .. })
            && core_reaches_host_call(db, init, &mut std::collections::HashSet::new()));
    if host_reaching
        && !matches!(
            core_of(db, init),
            Core::Tuple { .. } | Core::Record { .. } | Core::ListNew { .. }
        )
    {
        return true;
    }
    // FINDING-24: a per-dispatch `#st`-prefixed slot-state binding (the effects fold emits `#st{node}_{slot}`
    // for a GROWING threaded state — `(List.push pre t)` / `(Map.insert m k v)` / a `(let … growing)` wrapping
    // one — whose init transitively references the PRIOR dispatch's slot value) is FORCE-KEPT regardless of
    // use-count. Otherwise copy-prop re-inlines it PER DISPATCH — via the `is_compound_value` projection
    // propagate-arm below (force-propagates a projected Tuple/Record regardless of count) AND the `count < 2`
    // gate (an `#st` state is read once, by the next dispatch's init) — and the emitted Core/artifact is
    // O(k^N) again (rustc SIGSEGVs on the deeply-nested async emit). Naming it materializes the state ONCE per
    // dispatch and threads the name forward, so the fold is LINEAR in the dispatch count. Same
    // keep-regardless-of-count shape as the host-effect arm above, but for a compile-time SIZE reason.
    //
    // Keys on the `#st` PREFIX ALONE (collision-free — the effects fold's synthetic prefixes are
    // `#cv`/`#kv`/`#seed`/`#st`, and `#st` is emitted ONLY for a `next_state_is_growing_compound` state, so
    // the prefix is a sufficient, fold-guaranteed witness of the shape). Deliberately NOT gated on
    // `is_runtime_computation`: the F24 growing states lower to `Core::ListPush` / `Core::MapInsert` /
    // `Core::ListConcat` / `Core::Let`, NONE of which `is_runtime_computation` matches (it lists
    // Arith/Compare/Record/Tuple/ListNew/Call/Str*/BytesConcat only) — so an `is_runtime_computation` conjunct
    // would make this arm a NO-OP for exactly the list/map/let-wrapped shapes the finding is about (it would
    // fire ONLY for a scalar `(+ s 1)` state). The fold's own guard is the correctness gate; here the prefix
    // is the whole test. Placed BEFORE the `is_compound_value`/count propagate-arms so those cannot
    // re-propagate it.
    if db
        .ast
        .as_name(name_occ)
        .is_some_and(|n| n.starts_with("#st"))
    {
        return true;
    }
    // A value that folds to a constant / atom leaves no computation to share — always propagate.
    if !is_runtime_computation(db, init) {
        return false;
    }
    // A COMPOUND (tuple/record) binding that is ONLY ever PROJECTED — never used as a whole value —
    // need not be built on the heap at all: each projection folds straight through to the element's own
    // computation (a param `local.get`, a nested op, …), which is far cheaper than an `arr-alloc` +
    // per-field `box`/`arr-set` + `arr-get`/`get` + `drop` round-trip. Keeping it would build a heap
    // value only to read it back (or, when the projections fold, to drop it dead). So a projection-only
    // compound is NOT kept — it folds. A compound that ESCAPES as a whole (returned, passed as an arg,
    // nested into another compound) genuinely needs materialization and IS kept. (A non-compound
    // runtime binding — a shared scalar computation — keeps the multi-use rule below: naming avoids a
    // recompute.)
    if is_compound_value(db, init) && !uses.escapes_whole.contains(&init) {
        return false;
    }
    // A CONSTANT list `let`-bound and read ONLY through its element/rest PATTERN binders — `(let (((list a
    // b .. rest) (list 10 20 30))) (+ a b))` — need not be built on the heap: each binder resolves to a
    // `SumPayload{Elem(i)}`/`{RestFrom(k)}` that FOLDS straight through to the constant element/tail
    // (`fold_sum_path`'s `ListNew` arms), exactly as a constant tuple's projections fold. Keeping it would
    // build a `vec-empty`+`vec-push` chain only to read it back (or drop it dead) — pure waste for a value
    // that never varies. So a constant list NOT used as a WHOLE value (a `SumPayload` binder read is a
    // PIECE read, `ref_escapes_whole`→false, like a projection) is not kept — it folds. This closes the gap
    // where 2+ element binders tripped the ≥2-use rule below and materialized the list. (A RUNTIME list, or
    // a constant list that ESCAPES whole — returned/passed/nested — genuinely needs materialization and IS
    // kept: the old `ListNew`-is-always-whole reasoning still holds for those.)
    if matches!(core_of(db, init), Core::ListNew { .. })
        && is_constant_compound(db, init)
        && !uses.escapes_whole.contains(&init)
    {
        return false;
    }
    // Count references to this binding across its scope. Naming is worth it only at >= 2 uses.
    uses.count.get(&init).copied().unwrap_or(0) >= 2
}
