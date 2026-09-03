//! The handler REDUCER — `reduce_handle` and its inner-handle / performing-guard-match desugar plus
//! the hoist/distribute family (commuting conversions that float a pure/conditional wrapper so a
//! branch-perform's out-state threads). Extracted from the effects module to keep each file under
//! the source-size lint; behavior unchanged.

use super::*;
use crate::ast::{Struct, StructId};
use crate::db::Db;
use crate::fxhash::FxHashMap as HashMap;
use crate::resolve::resolved_of;
use crate::resolved::{HandleArm, Resolved};

/// Whether the subtree at `node` contains a CALL-SITE SPLAT `(.. t)` in argument position — a `(.. op)`
/// spread marker that is a non-head child of a non-construction application (`DESIGN-variable-arity-functions.md`
/// A.2/A.7). A structural, resolution-free walk over the AST, bounded by a node budget (a native-stack
/// backstop). Used by `reduce_handle` to DECLINE a handler body that call-splats, rather than miscompile it
/// (the fold does not yet expand splats). Does NOT flag a `..` in a collection PATTERN / construction (its
/// parent is a compound-ctor leaf) — only a call argument.
fn body_has_call_site_splat(db: &Db, node: StructId) -> bool {
    fn walk(db: &Db, node: StructId, budget: &mut u32) -> bool {
        if *budget == 0 {
            return false;
        }
        *budget -= 1;
        if let Struct::List(children) = db.ast.get(node) {
            // This node is an application `(head arg…)`. A non-head child that is a `(.. op)` marker, when the
            // head is not a compound constructor (`#tuple`/`#list`/… — those consume `..` as construction
            // spread, handled by v-ast-compound), is a call-site splat.
            let is_construction = db.ast.compound_ctor_leaf(node).is_some();
            if !is_construction {
                for (i, &ch) in children.iter().enumerate() {
                    if i > 0 && db.ast.spread_operand(ch).is_some() {
                        return true;
                    }
                }
            }
            for &ch in children.iter() {
                if walk(db, ch, budget) {
                    return true;
                }
            }
        }
        false
    }
    let mut budget = 65_536u32;
    walk(db, node, &mut budget)
}

/// are the resolved handle's children.
pub fn reduce_handle(
    db: &mut Db,
    init: StructId,
    arms: &[HandleArm],
    body: StructId,
    // CASE-1 EFFECT NON-LOCAL EXIT: TRUE when this handle is the WHOLE enclosing-function body (so
    // handle-result == function-result), known only by the lowering caller that has the handle node. When
    // TRUE, an abortive perform the tail-resumptive fold cannot lift is lowered to `Core::HandleAbort` (a
    // non-local return) rather than declined. FALSE (the safe default) for every internal/recursive caller.
    whole_fn_body: bool,
) -> Option<StructId> {
    // RE-ENTRY GUARD, held for the whole fold. `type_of` of a `handle` calls `reduce_handle` (E1c-2), and
    // `resume_result_type_ok`/`type_of(init)` below type nodes that can reach an enclosing handle — so
    // reducing a handle can re-enter this same `reduce_handle`. `db.enter_reduction()` bumps the shared
    // reduction-depth guard (the same backstop β-reduction uses); past the bound it returns `None` → we
    // decline. Held across the whole fold so every `type_of` inside is bounded. (The unbounded INLINE
    // loop — a recursive effectful callee β-reduced to a fresh non-self-referential copy each level —
    // is bounded separately by `THREAD_INLINE_LIMIT` in `thread_bounded`.)
    let mut guard = db.enter_reduction()?;
    let db = guard.db();
    // DECLINE-DON'T-MISCOMPILE (breaker): a call-site splat `(.. t)` lexically inside this handler BODY is
    // not yet supported — the tail-resumptive fold below types + lowers the body through its own path that
    // bypasses the shared `expand_call_splat_args` expansion, so the raw splat reached a backend as the WHOLE
    // tuple where a scalar element belongs, shipping an INVALID artifact on both targets
    // (accept-to-invalid-artifact, worse than a decline). Decline the whole handle fold if the body contains
    // such a splat (`cdz compile` then reports the clean "handler not reducible" decline) rather than
    // emitting a broken component. Follow-up: thread `expand_call_splat_args` through this fold, then lift.
    if body_has_call_site_splat(db, body) {
        return None;
    }
    // [cp4] Bind-once a multi-use nullary performing-factory let-local to the VERBATIM factory body (the
    // preserved capture-let = ca1m shape #3894 folds to 150), before the fold's per-use inline collapses
    // the creation-time capture into a per-application perform (silent 170). No-op unless the exact narrow
    // shape is present (see `bind_once_performing_factory`).
    let body = bind_once_performing_factory(db, body);
    // Build the operation→arm map, keyed by each arm's operation identity (read off the arm's op
    // projection's `(meta effect-op)`). An arm whose op is not an effect operation (a malformed arm) or
    // whose op the effect does not declare (CDZ0403 — reported elsewhere) makes the fold decline.
    // The set of ops THIS handle discharges — an op NOT in it, performed by an arm body, is FOREIGN (an
    // outer handler's effect). Computed up front so `hoist_next_state_foreign_perform` can classify a
    // next-state perform without the (not-yet-built) `HandlerCtx`.
    let mut discharged: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    for arm in arms {
        if let Some((d, i)) = crate::eval::effect_op_of(db, arm.op) {
            discharged.insert((d.0, i));
        }
    }
    let mut map = HashMap::default();
    for arm in arms {
        let (decl, idx) = crate::eval::effect_op_of(db, arm.op)?;
        // GUARD: the operation's declared RESULT type must be DETERMINED, and — for a tail resume — the
        // resume VALUE must agree with it. If the result type is undetermined (a malformed op arrow the
        // evaluator can't reduce, e.g. a single-arg `(-> T)`), the fold cannot safely substitute the
        // resume value as the perform's result — decline (a Todo), so the fold NEVER runs a program whose
        // resume value is unchecked against an unknown result type. A determined-but-MISMATCHED resume
        // value (`(resume true s)` for an Int64 op) is reported CDZ0201 by `check_resume_result_type`;
        // declining here as well keeps the fold from emitting the mistyped value (belt-and-suspenders —
        // the fault side rejects, the value side declines, so neither a wrong VALUE nor a wrong CODE ships).
        if !resume_result_type_ok(db, arm) {
            return None;
        }
        // E5 GENERAL / `ctl`-style arm: an arm that binds the continuation `k` explicitly (`cont: Some`).
        // STEP 2 (within-activation, lexical `k`): if `k` is used ONLY as an application head `(k v)` — never
        // bare, stored, or passed as an arg — the continuation does not escape, so `(k v)` is semantically
        // `(resume v)`; `ctl_arm_lexical_k_to_resume` rewrites the arm body accordingly, turning it into an
        // ordinary non-tail resumptive arm the existing pure-one-hole / two-hole folds serve (no new heap
        // rep). STEP 3 (deferred): a `k` that ESCAPES (bare / stored in a list-map / passed onward — the DES
        // scheduler's sleep/store/resume-later) needs the defunctionalized-frame `Ty::Cont` machinery; such
        // an arm still DECLINES cleanly here (a Todo, never a valid-but-wrong fold). Classified distinctly
        // from tail-resumptive (has `resume`, no `k`) and abortive (no `resume`, no `k`).
        let arm = if arm.cont.is_some() {
            match ctl_arm_lexical_k_to_resume(db, arm) {
                Some(rewritten_body) => HandleArm {
                    op: arm.op,
                    params: arm.params.clone(),
                    state: arm.state,
                    cont: None,
                    body: rewritten_body,
                },
                // `k` ESCAPES (passed onward / stored) — carry the arm through WITH `cont: Some` so the
                // pure-one-hole block below can REIFY `k` as a closure `(fn (#kv) C)` when `C` is pure
                // (step-3 inc-2a). If that block does not serve it, the fold declines at the end (never a
                // mis-fold — the pure-one-hole block is the ONLY consumer of a `cont: Some` arm).
                None => arm.clone(),
            }
        } else {
            arm.clone()
        };
        // FINDING-24 COVERAGE-GAP (sft1): canonicalize a single-bare-name-binder match over a COMPOUND
        // scrutinee in the arm body — `(match <compound> (h2 <body>))` → `(let ((h2 <compound>)) <body>)` —
        // so the resumptive fold binds the scrutinee ONCE per dispatch (the existing `#st` state-bind path)
        // instead of copying the compound scrutinee into every continuation copy (super-linear emit → invalid
        // wasm). A pure syntactic identity; see `hoist_single_binder_match_scrutinee`.
        let arm = match hoist_single_binder_match_scrutinee(db, arm.body) {
            Some(hoisted) => HandleArm {
                op: arm.op,
                params: arm.params.clone(),
                state: arm.state,
                cont: arm.cont,
                body: hoisted,
            },
            None => arm,
        };
        // NEXT-STATE FOREIGN-PERFORM HOIST (as2/as1/asb): an arm whose tail resume NEXT-STATE performs an
        // OUTER effect — `(resume t (+ t (A.get)))` — would drop/duplicate that perform if threaded as a
        // symbolic state expression (the `next_state_directly_performs_foreign` decline). Lift each next-state
        // foreign perform (and any resume-VALUE foreign perform, sequenced FIRST) to a dispatch-time `let`-init
        // before the resume — the proven value-equivalent form — so the ordinary resumptive fold serves it.
        // Byte-identical for a pure/served next-state (as3, the common case).
        let arm = match hoist_next_state_foreign_perform(db, arm.body, &discharged) {
            Some(hoisted) => HandleArm {
                op: arm.op,
                params: arm.params.clone(),
                state: arm.state,
                cont: arm.cont,
                body: hoisted,
            },
            None => arm,
        };
        map.insert((decl.0, idx), arm);
    }
    // This handle discharges ONE effect (all its arms share a decl — a handle's arms are for one effect),
    // so the context has ONE state slot. `state_ty_of_arms` derives the slot's state type from the init
    // seed joined with the arms' next-states. A NESTED handler whose shared recursive callee performs both
    // effects is MERGED into a multi-slot context by `thread_bounded`'s nested-`Handle` arm.
    let decl = arms
        .first()
        .and_then(|a| crate::eval::effect_op_of(db, a.op))
        .map(|(d, _)| d.0)?;
    let state_ty = state_ty_of_arms(db, init, arms);
    // CROSS-ARM COLLECTION-KEY PROPAGATION (v-inference, breaker tuple-key-map-rust-e0308 2026-08-09). The
    // initial-state seed's `type_of` bottoms an empty collection at OPEN vars — `Map.empty` is `Map(Var,Var)`,
    // the enclosing `(tuple n Map.empty)` `Tuple([Int64, Map(Var,Var)])`. A handler arm fixes the collection's
    // key/value/element (the `rec` arm's `(Map.insert m (tuple s …) …)` pins the key to `(Int64,Int64)`), and
    // `state_ty_of_arms` above JOINS that onto the slot type — but the join result is never written back onto
    // the INIT node's own type memo, and `beta_reduce` substitutes the init node (its id, reused not copied)
    // into the arm bodies, so emit reads `type_of(init map node) = Map(Var,Var)` and grounds it to the default
    // `BTreeMap<i64,i64>`. When the real key is a `(i64,i64)` tuple that is rust/rust-async E0308 (`__m.insert`
    // of a tuple key into an `<i64,i64>` map); wasm's tagless heap needs no spelled key type, so it is a
    // wasm-masked backend divergence, not a decline. Propagate the SOLVED collection type from the joined slot
    // back onto the init subtree's open-var collection nodes so `type_of(init)` reflects the real key/value.
    refine_init_collection_ty(db, init, arms);
    // WIDTH-CONSISTENCY GUARD (F1, corpus-bugfix/breaker 2026-07-28; refined for pyu8t1/pyu8r1 + the 14b
    // `two do-def-bound performs` case). A tail resume value is substituted into the op's RESULT position;
    // if its EMITTED width is narrower than the op result width, emit puts an i32 where i64 is demanded and
    // the module is INVALID ("expected i64, found i32"; rust widens and runs — a backend divergence). The
    // canonical hazard is a bare `(next (u) s (resume s (+ s x)))` with a UInt8 `x`: `s` infers UInt8 (i32)
    // via the next-state, the op result is Int64 (i64), so the state read is emitted i32 into an i64 slot.
    // DECLINE cleanly (a "not yet reducible" todo) rather than emit invalid wasm — the safe floor; a
    // widening-coercion fold is a later increment. See the emit-width computation below for the exact test.
    for arm in arms {
        if let Some(rv) = tail_resume_value_of(db, arm.body)
            && let Some(crate::ty::Ty::Int(rt)) = op_result_type(db, arm.op)
            && let crate::ty::Width::Fixed(rw) = rt.width
        {
            // The width the resume value is EMITTED at, compared to the op result width it is
            // substituted into: a narrower emit puts an i32 where i64 is demanded → invalid wasm.
            // The emit width is the value's OWN concrete fixed-int width when it has one — an
            // explicitly-widened answer `(resume (Int64.of s) …)` is Int64 = matches the result,
            // SAFE (pyu8t1); a narrow op-result whose answer is narrow-typed matches too (pyu8r1).
            // When the value type is NOT a concrete fixed int (an `Any`/deferred-width bare
            // `(resume s …)` state read whose type is not yet pinned), fall back to the STATE slot
            // width — the value IS the state, emitted at slot width. This supersedes the original
            // state-width-only test (which OVER-declined a widened narrow-state thread) and the
            // undetermined-value gate (which UNDER-declined a bare state read whose type is refined
            // to the concrete narrow state type BY THIS POINT → the 14b `two do-def-bound performs`
            // case regressed to an invalid module). Fires only on a determined width MISMATCH; a
            // matching width, a non-int result, or no width info on either side is untouched.
            let emit_w = match crate::infer::type_of(db, rv) {
                crate::ty::Ty::Int(vt) if matches!(vt.width, crate::ty::Width::Fixed(_)) => {
                    vt.width
                }
                _ => match &state_ty {
                    Some(crate::ty::Ty::Int(st)) => st.width,
                    _ => continue,
                },
            };
            if let crate::ty::Width::Fixed(ew) = emit_w
                && ew != rw
            {
                return None;
            }
        }
    }
    let slot = StateSlot { decl, state_ty };
    let ctx = HandlerCtx::new(db, map, vec![slot.clone()]);
    // DES multi-task reach: expose a DEFERRED resume that is stored in a compound and applied through a
    // helper — `(sleep (wake) s (unbox-apply (Box.Box (fn (_u) (resume unit wake)))))` where `unbox-apply(b)
    // = match b ((Box.Box th) (th unit))` — by reducing each arm body's store→match-extract→apply round-trip
    // (β-reduce + `eval::fold_ctor_match`, via `reduce_arm_deferred_resume`) to the resume-in-place form the
    // fold serves. Only an arm whose reduction EXPOSES a tail resume is rewritten; every other arm reduces to
    // itself (`==`), so tail/abortive/already-foldable arms are byte-identical. Rebuild the ctx only when some
    // arm changed. (v-discrete-event-sim inc-4's pqueue store→pop→apply reach; co-built with v-inference, who
    // owns `fold_ctor_match`'s SumPayload-aware substitution.)
    let exposed: HashMap<(u32, u32), HandleArm> = ctx
        .arms
        .iter()
        .map(|(&k, a)| {
            // ONLY a plain 4-part arm (no explicit continuation binder `k`): an escaping-k arm (`cont:
            // Some`, e.g. b2-min's `(a () s k (use-k k))`) is served by the escaping-k reify below, which
            // needs its body UN-reduced — β-reducing `(use-k k)` here to `(k 10)` breaks that reify ("value
            // is not applyable"). The deferred-resume-thunk shape this pass targets is a 4-part arm whose
            // resume is buried in a compound-stored closure, never a `cont: Some` arm.
            let rb = if a.cont.is_none() {
                reduce_arm_deferred_resume(db, a.body, &ctx)
            } else {
                a.body
            };
            let arm = if rb == a.body {
                a.clone()
            } else {
                HandleArm {
                    op: a.op,
                    params: a.params.clone(),
                    state: a.state,
                    cont: a.cont,
                    body: rb,
                }
            };
            (k, arm)
        })
        .collect();
    let ctx = if exposed
        .iter()
        .any(|(k, a)| ctx.arms.get(k).is_none_or(|o| o.body != a.body))
    {
        HandlerCtx::new(db, exposed, vec![slot])
    } else {
        ctx
    };
    // [pyfb3] Now that the handle BODY is available, compute this handler's per-op multi-dispatch set and
    // RE-EVALUATE collapse_enabled. The nullary-foreign-perform-let collapse candidate (pyfb3/pyfb1-let) fires
    // ONLY for an op drawn >=2 times in the body — a SINGLE dispatch folds the same shape strict via distribute
    // (as7) with no heap slot, so collapsing it there would re-land the 07e85af7c heap-collapse regression.
    // `HandlerCtx::new` decided collapse_enabled with an empty multi-set (body unavailable); redo it here.
    {
        let multi = ops_drawn_ge2(db, body, &ctx.arms);
        *ctx.multi_dispatch_ops.borrow_mut() = multi;
        let mdo = ctx.multi_dispatch_ops.borrow().clone();
        let ce = collapse_enabled_for(db, &ctx.arms, &mdo);
        ctx.collapse_enabled.set(ce);
    }
    // ABORTIVE (E4) TYPE-CONSISTENCY GUARD. An abortive arm materializes its BODY as the abort value, which
    // becomes the value of the position the perform occupied — a position the type checker typed by the
    // op's declared RESULT type (a perform types as its result, never as the arm value). If the arm body's
    // type differs from that result type (`bail : Int64 -> Bool` but the arm yields `n : Int64`), the abort
    // value does not fit where it lands: in a conditional it disagrees with the sibling branch and emits an
    // ill-typed `if` (invalid wasm). The checker misses this gap, so guard it in the fold — decline when any
    // abortive arm's body type does not match its operation's result type. (A tail-resumptive arm is already
    // covered: `resume_result_type_ok` checked its resume value against the result type above.)
    // The HANDLE BODY's type. An abort makes its arm value the WHOLE handle's value, so the abort value
    // must have this type. When an abort sits inside a COMPOUND-typed body — `(tuple 1 (Bail.bail 7))` or
    // `(tuple 1 (if c (Bail.bail 7) 5))` — a scalar abort value disagrees with the compound: the whole-
    // collapse path types the handle as the scalar (Int64) while the hoist/threading path substitutes the
    // scalar into the compound (a tuple `(1,7)`) — the two syntactic shapes even INFER different handle
    // types, and the hoist emits an ill-typed `if` (invalid wasm). The type checker misses the gap (a
    // perform types by its op result). Decline when an abortive arm's value type differs from the handle
    // body's type — the abort value simply doesn't fit where the handle promises to return it.
    let handle_body_ty = crate::infer::type_of(db, body);
    let abortive_keys: Vec<(u32, u32)> = ctx.abortive.iter().copied().collect();
    for (d, i) in abortive_keys {
        let arm_op = ctx.arms.get(&(d, i))?.op;
        let arm_body = ctx.arms.get(&(d, i))?.body;
        let body_ty = crate::infer::type_of(db, arm_body);
        let result_ty = op_result_type(db, arm_op);
        // Compare by COMPATIBILITY (`agrees_with`), NOT structural `==`: two `Int` types that differ only
        // in DEFERRED-vs-Fixed sign/width are compatible (an undetermined Int unifies with Int64), so `==`
        // would spuriously flag `(if (< (Bail.bail 7) 5) 1 2)` — the handle body infers `Int{Deferred}` and
        // the abort arm value `Int64{Fixed}`. Only a genuine MISMATCH (Int64 vs Bool, scalar vs tuple)
        // declines. An undetermined side (`Any`/var) never disqualifies. TWO checks: the abort value must
        // agree with (a) its operation's declared RESULT type, and (b) the HANDLE BODY's type.
        if let Some(rt) = result_ty
            && !undetermined_ty(&body_ty)
            && !undetermined_ty(&rt)
            && !body_ty.agrees_with(&rt)
        {
            return None;
        }
        if !undetermined_ty(&body_ty)
            && !undetermined_ty(&handle_body_ty)
            && !body_ty.agrees_with(&handle_body_ty)
        {
            return None;
        }
    }
    // DO-LOCAL VALUE-DEF → LET normalization. A handle body `(do (def v e) … rest)` binds `v` LOCAL to the
    // `do`. Several downstream folds (the abortive one-hole splice, the tail-resume thread's collapse-to-last)
    // DROP non-final do items and re-splice only the surviving expression — orphaning any `(def v e)` whose
    // `v` a LATER item references (notably a perform's ARGUMENT, `(Bail.bail v)` / `(Ask.ask v)`), which then
    // reads UNBOUND → spurious CDZ0101 (the do-def-in-perform-argument false-reject, corpus-bugfix 2026-07-24;
    // the `let`-twin never hit it because `let` rebuilds its scope). Rewrite a LEADING CHAIN of value defs
    // to nested `let`s wrapping the continuation up front — `(do (def v e) rest…)` ≡ `(let ((v e)) (do
    // rest…))`, recursively — so EVERY consumer below (abortive/pure-hole/thread) sees the scoped form.
    // Sound: `e` is evaluated once, in the SAME position and order it held in the `do` (its RHS may itself
    // perform — the `let` init runs exactly where the `do` item did); only `v`'s visibility widens to the
    // continuation. A FUNCTION def `(def (f p…) body)` (sig is a list, not a bare name) is left untouched —
    // it resolves to a lambda. (Rewrites only the LEADING chain, not every non-final def: a def AFTER a
    // non-def item is reached by re-threading the `do` tail, which normalizes it in turn.)
    // CAPTURE-ONCE normalization (finding #10 increment). A let-bound closure whose value is a
    // `(let ((a <perform>)…) LAMBDA)` — the capture `a` is a performing draw the returned lambda reads —
    // must discharge that draw ONCE and share it across every application; but a downstream β-reduction of
    // the closure's uses inlines `a`'s init at each use site, DUPLICATING the perform (multi-app ca1 read a
    // fresh draw per use). HOIST the performing init OUT of the closure-value-let to WRAP the binding here,
    // on the raw body BEFORE any use is reduced: `(let (… (f (let ((a P)…) LAMBDA)) …) BODY)` becomes
    // `(let ((a P)…) (let (… (f LAMBDA) …) BODY))`. Now `a` is a plain outer let-init the fold threads ONCE,
    // and every `(f v)` β-reduces against the PURE lambda closing over that single-threaded `a` (verified:
    // the hoisted single- and multi-app forms fold, sharing the one draw). Sound: the init runs once, before
    // the body, same order — only the binder's visibility widens. Fixpoint-loop so several such closures all
    // hoist; each hoist removes its own match, so it terminates.
    let mut body = body;
    while let Some(hoisted) = hoist_performing_capture_closure(db, body, &ctx) {
        // The hoist REUSES sub-nodes (the returned lambda, the let body) and re-parents them under the new
        // wrapping `let`s. A REUSED subtree can SHARE a load-time atom (e.g. a name occurrence) whose SINGLE
        // parent slot points into a stale/orphaned prior copy — so the exhaustive lexical-scope walk from such
        // a ref DEAD-ENDS before reaching the new binder → a false CDZ0101 "unbound" that no memo hygiene
        // (`forget`/`force`) can repair, because the parent CHAIN itself is broken (not just the memo). So
        // `deep_fresh_copy` the whole rewritten subtree: every node becomes a fresh synth id whose parent is
        // set correctly by `push_list`, giving a coherent parent chain for the exhaustive walk. Then
        // `forget_subtree` clears any stale memo and `force_structural_resolution_subtree` routes the
        // (now-synth) refs through the exhaustive walk against the CURRENT parents. This resolves a
        // nested-closure capture (cp1: `h` wrapping the hoisted `g`) correctly — no re-draw, no unbound.
        let hoisted = deep_fresh_copy(db, hoisted);
        crate::resolve::forget_subtree(db, hoisted);
        db.force_structural_resolution_subtree(hoisted);
        body = hoisted;
    }
    let body = lift_do_local_value_defs(db, body);
    // CAPTURE-AVOIDING HYGIENE (body side). Alpha-rename the handle body's LOCAL value binders (`let` pairs,
    // `do`-local `(def NAME v)`) to fresh `#`-names before the fold threads a resume VALUE into the perform
    // position inside this body. Without it, a body-local `(let ((x 7)) (+ x (E.get)))` (or its `do`-def
    // twin) CAPTURES a spliced-in free `x` carried from the arm — the arm's `(resume x s)` reads the OUTER
    // global x=100, but landing inside the body's `x=7` scope rebinds it (F2 → 14 not 107). `freshen_local_
    // binders` SHARES every free-name subtree (returns the node untouched when no local binder is inside), so
    // an enclosing-param / global reference the body legitimately reads keeps its pinned resolution — only
    // local binders and their in-scope references are rewritten. (breaker's silent-miscompile finding,
    // corpus-bugfix 2026-07-28.)
    let body = freshen_local_binders(db, body);
    // ABORTIVE-OUTER + NESTED-HANDLE PRE-REDUCTION (finding #11). When THIS handler is abortive and its body
    // contains a NESTED inner handle of a DIFFERENT effect that itself wraps a perform of THIS handler's op —
    // `(handle A abortive (+ (* 100 (handle B tail (if c (A.out n) n))) 7))` — the `A.out` is buried under
    // BOTH the inner `handle-internal B` node AND (here) a conditional. `hoist_conditional_abort` below and
    // the abort-reach guards do NOT descend into a nested handle node, so the buried `A.out` is invisible:
    // the inner B-handle later reduces during A's `thread` (the inside-out Handle arm), surfacing `(if c
    // (A.out n) n)` — but by then the hoist has already run, so the conditional abort is captured BRANCH-LOCAL
    // (yielding 9003 as the `if`'s value) and the enclosing `* 100`/`+ 7` continue → 900307, a silent
    // miscompile (finding #11: conditional foreign abort homes to the wrong inner boundary). REDUCE the inner
    // handles FIRST — B keeps `A.out` residual (foreign to B, `copy_pure`'d) and its wrapper is removed — so
    // the body becomes `(+ (* 100 (if c (A.out n) n)) 7)`, where `hoist_conditional_abort` CAN lift the
    // conditional abort to a branch tail A discharges. The UNCONDITIONAL case (abmin) already worked (the
    // inside-out thread reduces B then discharges the bare `A.out`); this makes the CONDITIONAL case take the
    // same homing path. GATED to abortive + a genuinely nested handle so the common body is untouched; sound
    // because reducing an inner handle is the same reduction the thread path performs, only sequenced earlier.
    let body = if !ctx.abortive.is_empty() && body_contains_nested_handle(db, body) {
        reduce_inner_handles(db, body)
    } else {
        body
    };
    // ABORTIVE (E4) NON-TAIL HOIST. An abort in a strict OPERAND under a conditional — `(+ 100 (if c
    // (Bail.bail 7) 50))` — is not directly foldable (the abort must escape the `+`). But an abort
    // ABANDONS the enclosing computation, so distributing the surrounding strict op INTO both `if` branches
    // is value-preserving: `(+ 100 (if c (Bail.bail 7) 50))` ≡ `(if c (+ 100 (Bail.bail 7)) (+ 100 50))`.
    // In the rewritten form the abort sits in an `if` BRANCH TAIL — the shape the per-branch fold
    // (`thread_branch_local_abort`) already handles (the abort branch's `(+ 100 (Bail.bail 7))` is an
    // unconditional-abort-in-a-branch, captured locally). Sound because the op's OTHER operands are pure
    // (the hoist requires it), so duplicating them across the two branches changes no observable effect.
    // Runs to a fixpoint (bounded) so a nested non-tail abort is lifted level by level; a shape it can't
    // lift is left as-is and the guard below declines it.
    let body = hoist_conditional_abort(db, body, &ctx);
    // DEF-BOUNDARY NON-LOCAL-EXIT (finding #11-B, 14:12783). A `(let ((a (helper (Foreign.op) …))) K)` whose
    // helper aborts in a MATCH arm: inline the helper and distribute K into the NON-ABORTIVE arms only, so the
    // abort homes to its boundary (the safe-floor guard below would otherwise DECLINE this — a naive fold
    // threads the abort into K = miscompile). Leaves any other shape unchanged.
    let body = hoist_match_abort_let(db, body, &ctx).unwrap_or(body);
    // PENDING-IN-HANDLE-BODY (adv-52, non-local-exit CC). An abortive (tail-)recursive callee called at a
    // NON-TAIL operand of a pure strict op in the handle body — `(+ (go 2) 999999)` — must ABANDON the pending
    // op on abort. The recursive-callee guard below (`body_has_unsound_abortive_perform` → the cross-fn arm)
    // would DECLINE this; instead fold it via the tagged CC + a handle-body tag short-circuit. v1: a
    // SHAREABLE-CONSTANT seed only (a heap seed's `#seed`-wrap for the tagged tuple is a follow-up). Declines
    // (falls through to the guard) for any shape `try_pending_in_handle_body` does not model — no miscompile.
    if !ctx.abortive.is_empty()
        && seed_is_shareable_constant(db, init)
        && let Some(folded) = try_pending_in_handle_body(db, body, &ctx, init)
    {
        reparent_under_handle_site(db, folded, body);
        return Some(folded);
    }
    // ABORTIVE (E4) SOUNDNESS GUARD. Two sound abort shapes are realized below: (1) an UNCONDITIONAL abort
    // collapses the whole handle to the arm value; (2) an abort in the TAIL of a tail-position `if` branch
    // folds per-branch (the branch-local abort restores the cell so the other branch survives — see
    // `thread_branch_local_abort`). An abort at any OTHER conditional position the hoist above could not
    // lift to a branch tail (e.g. under an EFFECTFUL sibling operand, or a short-circuit connective) still
    // fires on one path with no realizable shape — decline cleanly rather than miscompile.
    if !ctx.abortive.is_empty()
        && body_has_unsound_abortive_perform(db, body, &ctx, true, false, false)
    {
        // CASE-1 EFFECT NON-LOCAL EXIT: when this handle is the WHOLE function body, a SAME-FN unsound abort
        // (a direct perform the tail fold cannot lift) is now soundly lowered to `Core::HandleAbort` (a
        // non-local RETURN) during threading — so do NOT decline it. A CROSS-FN unsound abort (inside a
        // specialized recursive callee) STILL declines: a bare return there cannot abandon the CALLER's
        // pending continuation (the deferred tagged-return vertical). So decline iff the handle is not the
        // whole function body OR the unsoundness includes a cross-fn abort.
        let cross_fn = body_has_unsound_abortive_perform(db, body, &ctx, true, false, true);
        // SEED-SHAREABLE gate (first increment): only take the HandleAbort path for a SHAREABLE-CONSTANT seed.
        // A NON-shareable (heap) seed is threaded as `#seed`, so an abort value that reads the (advanced) state
        // carries a `#seed`/`#st` ref that the collapse path binds via `apply_seed_wrap`/`drain_and_wrap` on the
        // returned value — machinery the mid-body `Core::HandleAbort` does not yet receive, so its embedded
        // `#seed` reads UNBOUND (CDZ0101). Until the HandleAbort value gets that same seed-wrap/drain treatment
        // (a scoped follow-up), DECLINE the heap-seed case cleanly (it declined before this feature too — no
        // regression) and take HandleAbort only for the shareable-seed subset (the eab1 witness + scalar states).
        // FOREIGN-PERFORM gate (first increment): a FOREIGN perform on the strict spine BEFORE the abort (an
        // OUTER handler's op or a HOST call) has ALREADY committed its effect and must survive the abort — the
        // collapse path preserves it via its do-form foreign-advance rule, but the mid-body `Core::HandleAbort`
        // does NOT (it would emit the abort's non-local return without the pre-abort foreign dispatch, DROPPING
        // it → a host-call-sequence miscompile, e.g. expected ["io.fetch"] observed []). Until the HandleAbort
        // path carries the pre-abort foreign advance (a scoped follow-up), DECLINE a body that reaches a foreign
        // perform (it declined before this feature too — no regression). eab1's only performs are of THIS
        // handler's own effect (not foreign), so it still folds.
        if !whole_fn_body
            || cross_fn
            || !seed_is_shareable_constant(db, init)
            || body_reaches_foreign_perform(db, body, &ctx)
        {
            return None;
        }
        // else: RELAX — this handle is the whole function body and its only unsoundness is a same-fn abort
        // the fold cannot lift. Flip on HandleAbort mode so `thread`'s abortive handler emits a
        // `Core::HandleAbort` (non-local return) at the abort position instead of the collapse/decline.
        ctx.abort_as_handleabort.set(true);
    }
    // GUARD-CONDITION PERFORM. A perform inside a match-arm GUARD condition — `(match k ((guard p (E.op)) b)
    // …)` — is a position the distribution/fold walks do NOT descend (they route the scrutinee + arm bodies,
    // not the guard conds). For the SOUND, NARROW shape — a two-arm match whose first arm has an IRREFUTABLE
    // inner pattern guarded by a performing cond, and an irrefutable catch-all second arm — desugar it to an
    // `if` on the guard (`desugar_performing_guard_match`): the arm is selected iff the guard holds, so
    // `(match k ((guard <irrefutable> g) b) (<irrefutable> b2))` ≡ `(if g b b2)` (each binder let-bound to
    // the scrutinee). The guard becomes an `if` CONDITION — a strict-first position the if-condition fold
    // already routes through the enclosing handle. A shape this does NOT cover (multiple guarded arms, which
    // sequence handler state per arm-test — the per-branch-sees-the-seed distribution does not model that; or
    // a refutable guarded pattern) is left to DECLINE cleanly (the honest "not yet reducible" todo), never
    // the factually-wrong "no enclosing handler" error.
    let body = match desugar_performing_guard_match(db, body, &ctx) {
        Some(rewritten) => rewritten,
        None => {
            if body_has_performing_match_guard(db, body, &ctx) {
                return None;
            }
            body
        }
    };
    // RESUMPTIVE (tail-resume) NON-TAIL CONDITIONAL HOIST. A `perform` inside an `if`/`match` BRANCH
    // advances the handler state LOCALLY, but the `if` thread arm returns the post-CONDITION state as its
    // out-state — so the state advance is LOST to any CONTINUATION after the conditional (`(do (if c
    // (E.op) x) (E.op))` runs the second `(E.op)` against the pre-branch state). The branch-out-state is
    // genuinely a runtime PHI of the two branches, which the tail fold does not represent. Fix by the same
    // distribution the abortive hoist uses: lift the `if`/`match` OUT of the strict continuation into a
    // TAIL position, distributing the continuation into both branches — `(do (if c t e) k)` ≡ `(if c (do
    // t k) (do e k))`, `(+ (if c t e) rhs)` ≡ `(if c (+ t rhs) (+ e rhs))`. Then each branch threads its
    // own state THROUGH its copy of the continuation (the `if` is now in tail position, where the fold is
    // correct). Sound: the CONDITION stays the single `if` condition (evaluated exactly once, never
    // duplicated), and the continuation is duplicated across both branches but only one branch runs at
    // runtime — so every effect in it happens exactly once, in the same relative order, as long as no
    // EFFECTFUL sibling is evaluated BEFORE the `if` (that would jump the condition ahead of it); the
    // hoist requires every preceding sibling pure. Runs to a fixpoint (bounded). A shape it cannot lift
    // (a perform under a conditional the hoist could not raise to tail) is left as-is and declines below.
    let body = hoist_resumptive_conditional(db, body, &ctx);
    // E5 HANDLER DISTRIBUTION over a pure-conditioned tail conditional (a commuting conversion). When the
    // handle BODY is an `if`/`match` whose CONDITION/SCRUTINEE is strongly pure but a BRANCH / ARM BODY
    // performs a discharged op, the pure one-hole fold below declines (a branch perform is a NON-uniform
    // continuation — it runs only on the taken path). But the conditional IS the whole handle body (tail
    // position), so the handler distributes into each branch — `(handle E s arms (if c t e))` ≡ `(if c
    // (handle E s arms t) (handle E s arms e))`, `(handle E s arms (match k (p b)…))` ≡ `(match k (p (handle
    // E s arms b))…)`: the condition/scrutinee runs exactly ONCE (pure, evaluated first, advancing no
    // state), and each branch becomes a SMALLER handle body the fold already serves (only one runs at
    // runtime, seeing the seed state — nothing advanced it). Recurse `reduce_handle` on each; if any branch
    // is a shape the fold cannot serve, the whole distribution declines (`?`) and we fall through to the
    // ordinary decline. GATED to the NON-tail-resumptive regime (all arms non-tail, none abortive) — a
    // tail-resumptive branch perform is already handled by the threading path, and an abortive one by
    // `hoist_conditional_abort`; distributing there would only duplicate working paths.
    if ctx.abortive.is_empty()
        && ctx.arms.values().all(|a| tail_resume(db, a.body).is_none())
        && let Some(distributed) = distribute_handler_over_conditional(db, init, arms, body, &ctx)
    {
        return Some(distributed);
    }
    // NESTED-HANDLE PRE-REDUCTION (for a NON-tail-resumptive outer handler). The inside-out `thread` path
    // reduces a nested inner `handle` only while THREADING — which requires this (outer) handler's arms to
    // be tail-resumptive. When they are NOT, a body like `(handle B tail (+ (A.a) (B.b)))` reaches the E5
    // pure-one-hole check with the raw inner `handle` node still present (`pure_hole` sees a nested handle →
    // Impure → declines). Reduce the inner handle(s) FIRST so the body becomes `(+ (A.a) 20)` — a single
    // outer-effect perform in a pure one-hole context the E5 fold below serves (→ `(+ 1 (+ 20 10))` = 31).
    // GATED to the non-tail-resumptive, non-abortive regime (the tail path already reduces inner handles via
    // `thread`, and the merge path handles a recursive callee spanning both effects); only run it when the
    // body actually contains a nested handle, so the common no-nested-handle body is untouched.
    let body = if ctx.abortive.is_empty()
        && ctx.arms.values().all(|a| tail_resume(db, a.body).is_none())
        && body_contains_nested_handle(db, body)
    {
        reduce_inner_handles(db, body)
    } else {
        body
    };
    // APPLIED-LAMBDA PRE-REDUCTION. A handle body that wraps its perform in a lambda APPLICATION —
    // `((fn (x) (+ x (Amb.flip))) 100)` or a `let`-bound `(f 100)` where `f`'s body performs — reaches the
    // pure-one-hole classifier with the raw `Apply` node still present (`pure_hole` does not β-reduce a
    // lambda-head call, so it sees a non-uniform / effect-reaching call → declines). The `thread`/one-shot
    // path DOES inline such a call (its `call_reaches_discharged_effect` arm), so the one-shot case folds;
    // but the MULTI-shot path goes through `pure_hole` and declined. β-reduce these redexes FIRST so the
    // body becomes `(+ 100 (Amb.flip))` — a single perform in a pure one-hole context the E5 fold serves
    // under a multi-shot arm too. Same β-reduction the inline arm performs, only sequenced earlier; a
    // recursive callee is excluded by `call_reaches_discharged_effect` (specialized, not inlined). Gated on
    // a cheap syntactic check so the common no-such-redex body is untouched.
    let body = if body_contains_applied_performing_lambda(db, body, &ctx) {
        reduce_applied_lambdas(db, body, &ctx)
    } else {
        body
    };
    // RE-HOIST after inlining. `reduce_applied_lambdas` above β-reduces a performing helper CALL into its
    // body — surfacing a conditional that was HIDDEN behind the call (`(let ((a (demand 5 25))) cont)` →
    // `(let ((a (match (Db.get k) … (do (Db.put …) …)))) cont)`). The first `hoist_resumptive_conditional`
    // ran BEFORE that inline, so it saw only the opaque `(demand …)` call and could not lift the branch-
    // performing conditional now exposed in the `let` init. Without re-hoisting, the tail fold threads the
    // surfaced conditional's out-state as its post-scrutinee state — the branch's `put` advance is dropped
    // and a later `(Db.get k)` in the continuation reads the stale pre-branch state (the helper-call
    // out-state silent miscompile). Re-running the hoist lifts the exposed conditional to tail position (via
    // Site 4, the `let`-init distribution) where per-arm threading carries the advance through the
    // continuation. Idempotent + cheap: a no-op (returns `body` unchanged) when the inline surfaced no
    // branch-performing conditional, so a body with no such helper is untouched.
    let body = hoist_resumptive_conditional(db, body, &ctx);
    // adv-69 SAFE-DECLINE FLOOR (HIGH silent miscompile, breaker + corpus-bugfix 2026-08-04). The hoist above
    // lifts a branch-performing conditional that is DIRECTLY a `let`-init to tail position (Site 4), where
    // per-branch threading carries its state advance. But a conditional wrapped in a BLOCK (`(let ((v (let
    // ((b true)) (if b (E.op) x)))) cont)` — inner-let/do around the `if`/`match`) is opaque to Site 4: the
    // block's exit state reverts to block-ENTRY, so the branch perform's advance is DROPPED and a later
    // perform in `cont` resumes the stale pre-branch state → a WRONG VALUE, no error (worst class). Until the
    // full through-block distribution lands (a commuting conversion needing alpha-safe binder handling — a
    // separate careful increment), DECLINE this residual shape so it grades a clean Todo, never a silent
    // miscompile. Detects a `let`-init whose value is a block-wrapped branch-performing conditional the hoist
    // left in place. (The direct-init case the hoist DID lift no longer matches — it's now a tail conditional,
    // not a `let`-init block — so this never over-declines the working Site-4 path.)
    if body_has_block_wrapped_let_init_branch_perform(db, body, &ctx) {
        return None;
    }
    // adv-69 a3 sub-face: the SAME block-boundary out-state drop at a NESTED handle's arm RESUME-VALUE
    // position (performing THIS handler's op), which the let-init scan above does not reach (it stops at a
    // nested `Handle`). Decline cleanly rather than folding the dropped-advance wrong value (probe-a3 ran 33,
    // correct 34). Precisely keyed on `Resume{value}` so it never over-declines a threaded position the fold
    // serves (a position-agnostic block-wrapped-perform scan over-declines 5 working cases).
    if body_has_nested_arm_resume_value_block_wrapped_branch_perform(db, body, &ctx) {
        return None;
    }
    // adv-69 g3 + c3 sub-faces: the SAME block-boundary out-state drop at a MATCH-SCRUTINEE (g3) or a non-tail
    // `do`-STATEMENT (c3) consuming a block-wrapped branch perform — positions Site 5 / Site 1 lift only when
    // the conditional is DIRECT, not block-wrapped. Decline cleanly rather than folding the dropped-advance
    // wrong value (probe-g3 ran 33/34, probe-c3 ran 33/73). Keyed on the WRAPPED shape only, so a direct
    // conditional in either position (the passing d2/e1 twins) still folds.
    if body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, body, &ctx) {
        return None;
    }
    // finding #10 (breaker, MED-HIGH silent miscompile — the bind-once/closure-capture face). A `let`-bound
    // CLOSURE whose value is `(let ((a <perform>)) (fn … a …))` — the capture `a` is a PERFORMING inner-let
    // init — re-performs that draw PER APPLICATION: the closure re-derives its body from source through
    // `apply_lambda`/`beta_reduce` at each `(f x)`, discarding the once-evaluated draw and re-running it
    // inside the body (ca1c: single app 60 not 50; ca1: two apps 122 not 80). The `capture_subst` closes only
    // a STRONGLY-PURE init into the body; a performing init is left a live reference the re-derivation
    // re-fires. Correctly folding it (thread the draw ONCE before the closure value, close the closure over
    // the RESULT) is the eval-count/state-commitment capture-once increment; until then DECLINE the exact
    // shape (a silent wrong value → an honest "not yet reducible" todo). NARROW by construction (see
    // `body_has_closure_over_performing_capture`): only a let-bound closure whose init-let binds a discharged/
    // foreign perform referenced by the returned lambda — the direct handle-body captures (corpus-8688) and
    // the let-outside-the-closure workaround (d1) bind the draw in a PLAIN let, not the closure's init-let, so
    // they are untouched; a PURE inner init (d2fix) is not a perform, so it folds.
    if body_has_closure_over_performing_capture(db, body, &ctx) {
        // A performing-capture closure the early capture-once normalization did NOT hoist (a shape neither
        // FORM A — a let-bound closure-value-let — nor FORM B — an arg'd factory call — matches; e.g. a
        // conditionally-selected closure). DECLINE cleanly rather than fold a wrong value; the closure would
        // re-derive its body per application, re-firing the captured draw. (The common let-bound + arg'd
        // factory faces ARE hoisted above and never reach here.)
        return None;
    }
    // E5 PURE ONE-HOLE-CONTINUATION fold (general one-shot, the pure-continuation case). When the handle
    // BODY reaches EXACTLY ONE discharged perform `P` through STRICT, UNCONDITIONAL, effect-free positions
    // (`pure_hole`), its delimited continuation is the PURE one-hole context `C = body[P := □]`. Resuming
    // returns into that context, so `(resume v s)` yields `C[v]` — a copy of the body with `P` replaced by
    // `v`. The handle's value is then the arm body with every `(resume v s)` rewritten to `C[v]`:
    //   `(handle Amb 0 ((flip (u) s (+ 1 (resume 10 s)))) (+ 100 (Amb.flip)))`
    //     → arm `(+ 1 (resume 10 0))`, `C = (+ 100 □)` → `(+ 1 (+ 100 10))` = 111.
    // Because `C` is STRONGLY pure (no effect of any kind), a MULTI-shot arm may duplicate it safely:
    //   `(+ (resume 1 s) (resume 2 s))` over `(+ 100 (Amb.flip))` → `(+ (+ 100 1) (+ 100 2))` = 203.
    // The IDENTITY slice (`body` IS the bare perform, `C = □`, so `C[v] = v`) is the special case. A
    // perform under a CONDITIONAL (`if`/`match`/`and`/`or` — a non-uniform continuation), a SECOND
    // perform, a `resume`, a nested `handle`, or a non-primitive call disqualifies (`pure_hole` → this
    // block does not fire and threading declines below); those need the full captured-continuation
    // machinery (defunctionalized frames — a later increment).
    if let PureHole::Hole(perform) = pure_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        && !ctx.abortive.contains(&(decl, idx))
        && tail_resume(db, arm.body).is_none()
        // CONDITIONALLY-RESUMING ARM GUARD (corpus-bugfix/breaker 2026-07-28): decline an arm that resumes in
        // some branches but aborts (returns a bare value) in others — the reify below would mis-fold it (see
        // `arm_partially_resumes`). A `cont: Some` escaping-k arm is exempt (reified as a closure, not peeled).
        && (arm.cont.is_some() || !arm_partially_resumes(db, arm.body))
    {
        // Substitute the arm's params ↦ (pure-copied) perform args and its state binder ↦ the init seed
        // (nothing runs before the perform on a pure spine, so the state seen at the perform is the seed),
        // then rewrite every `(resume v s)` → `C[v]`. The perform's args are pure (`pure_hole` admits only
        // strongly-pure arguments), so evaluating them has no effect and they need no state threading.
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, copy_pure(db, a));
                }
            }
        } else if arm.params.len() == 1 && args.is_empty() {
            let p = arm.params[0];
            if !is_unit_param(db, p) {
                let unit = db.push_list(vec![]);
                subst.insert(p, unit);
            }
        } else {
            return None;
        }
        subst.insert(arm.state, init);
        // E5 ESCAPING-K REIFICATION (step-3 inc-2a): a general `ctl`-style arm that BINDS `k` (`cont: Some`)
        // and lets it ESCAPE (passes it to a fn / stores it — `ctl_arm_lexical_k_to_resume` returned None,
        // so the classifier carried it here). Over a PURE continuation `C` (`pure_hole` succeeded), reify
        // `k` as the CLOSURE `(fn (#kv) C)` where `C = body[perform := #kv]` — a lambda taking the resume
        // value `#kv` and running the delimited continuation. Substituting that lambda for the arm's `k`
        // binder makes the arm body carry `k` as an ordinary closure value; the runtime-closure machinery
        // lifts it (`Core::Closure`) and applies it (`Core::CallClosure`) at each `(k v)` — so `(use-k k)`
        // over `(+ 1 (A.f))` folds to `(use-k (fn (#kv) (+ 1 #kv)))` → 11. NO bespoke frame chain: a reified
        // continuation over a pure `C` IS a closure. (A `C` that re-performs the handled effect needs
        // handler-re-entry-at-apply, inc-2b — `pure_hole` fails on the second perform, so this never fires
        // for it.) The `#kv` binder is fresh (keyed on the perform node); the lambda captures `C`'s free
        // names, which the closure lift resolves into its env.
        let folded = if let Some(k_binder) = arm.cont {
            let kv_name = format!("#kv{}", perform.0);
            let kv_binder = db.push_name(&kv_name);
            let kv_ref = db.push_name(&kv_name);
            let cont_body = splice_context(db, body, perform, kv_ref);
            let fn_head = db.push_name("fn");
            let params_list = db.push_list(vec![kv_binder]);
            let k_lambda = db.push_list(vec![fn_head, params_list, cont_body]);
            let mut subst_k = subst.clone();
            subst_k.insert(k_binder, k_lambda);
            crate::eval::beta_reduce(db, arm.body, &subst_k)
        } else {
            // Pin the ARM BODY's enclosing captures BEFORE beta_reduce detaches them (mv-class arm-side).
            // A two-site resume arm `(+ (resume (+ n 1) s) (resume 2 s))` whose resume VALUE reads an
            // enclosing-fn param `n`: beta_reduce COPIES the arm body per the subst, and the copied `n` loses
            // its binding (ref_binder None) so each per-resume splice re-resolves it unbound. Pinning `n`
            // here (still parented to the def) records its resolution so beta_reduce's capture-share arm
            // preserves it. `lam_body = arm.body`, so the arm's own op-params/state binder (bound within) stay
            // unpinned + substitute normally. (breaker mv-class, arm-side/resume-value locus.)
            crate::eval::pin_free_vars(db, arm.body, arm.body, &[]);
            // adv-20: if the arm body is a pure LET-WRAPPED resume — `(let ((o (if (> s 100) … s))) (resume
            // (match o …) s))` — FLATTEN it (splice each pure let-init for its binder refs) so a let-init that
            // reads the arm state does not survive the `{arm.state → init}` subst below UNSUBSTITUTED (→
            // slotless `Core::Param{arm.state}` decline). The flatten PINS the arm's state/op-param uses first
            // so the flatten copy SHARES them (keeps `Ref{arm.state}`) rather than orphaning a bare sibling `s`
            // in the resume value; the subst below then substitutes the shared occurrences. Non-let arms fall
            // through unchanged. (v-inference-pinned locus; Option A.)
            let arm_binders: Vec<StructId> = arm
                .params
                .iter()
                .copied()
                .chain(std::iter::once(arm.state))
                .collect();
            let arm_body =
                flatten_pure_let_wrapped_resume(db, arm.body, &arm_binders).unwrap_or(arm.body);
            let substituted = crate::eval::beta_reduce(db, arm_body, &subst);
            // Also pin the SEED-derived captures now in `substituted`: when `n` is the handle SEED
            // (`(handle Amb n …)`), it enters the arm via the state-binder subst, so it appears in
            // `substituted` (not `arm.body`); pin it here so the per-resume splice shares it too.
            crate::eval::pin_free_vars(db, substituted, substituted, &[]);
            // CAPTURE-AVOIDING HYGIENE. Freshen the substituted arm body's LOCAL value binders before the
            // resume rewrite splices the continuation `C` (the handle body, carrying OUTER free names) into
            // its tail. Without this, an arm-local `(do (def x 5) (resume (+ x s) s))` CAPTURES `C`'s free `x`
            // (F1 → 10 not 105). Done AFTER `beta_reduce` (so the arm's op-param/state binder references are
            // already substituted away — freshening rebuilds the tree, which would otherwise orphan those
            // node-identity-keyed refs). (breaker's silent-miscompile finding, corpus-bugfix 2026-07-28.)
            let substituted = freshen_local_binders(db, substituted);
            // PIN `C`'S ENCLOSING CAPTURES before the (possibly multi-)splice. A MULTI-SHOT arm
            // (`(pick (u) s k (+ (k 1) (k 2)))`) has `k` applied ≥2×, so `rewrite_resume_to_context`
            // splices a FRESH copy of `C` per resume. When `C` (the handle body) reads an ENCLOSING-fn
            // param — `(let ((y 3)) (+ n (Amb.pick)))` free in `n` — each `splice_context` copy would
            // re-resolve `n` against its own orphan and report a false CDZ0101 "unbound n" (the reparent
            // at the fold tail only re-anchors the single folded root, not both C-copies). Pinning `C`'s
            // free captures here records their resolution so the per-resume copies SHARE the resolved
            // `n` — exactly the `apply_lambda_uncached` capture-share idiom. `C`'s own local binders (the
            // `let`'s `y`) are bound WITHIN `body`, so they stay unpinned and copy fresh. (breaker mv-class.)
            crate::eval::pin_free_vars(db, body, body, &[]);
            // Rewrite every `(resume v s)` → `C[v]` (the pure delimited continuation applied to the resume
            // value). The arm body's free names keep their pinned resolution through `beta_reduce`; `C`'s
            // free names resolve against the handle scope (the structural splice copy re-parents them). The
            // next-state is dead — nothing after the perform reads state on a pure spine.
            rewrite_resume_to_context(db, substituted, body, perform)
        };
        // Graft the synthesized `folded` UNDER the original handle's site BEFORE the type-check below, so a
        // FREE name inside it — an enclosing function's parameter or a match-arm binder used in the handle
        // body, `(handle … (+ x (Amb.flip)))` — resolves up the original lexical chain instead of reading
        // unbound. `push_list` leaves a synthesized root's parent `None`; the lowering site re-parents the
        // final result, but the `type_errors` guard here runs FIRST, so without this an outer-param body
        // would spuriously fault (unbound name) and OVER-DECLINE. Parent to the `handle` node itself (the
        // same discipline the lowering-site reparent uses — see lower.rs) so the scope walk ascends
        // folded → handle → … and a binder form above still recognizes the handle as its body child.
        reparent_under_handle_site(db, folded, body);
        // TYPE-CONSISTENCY GUARD (the resumptive analogue of the abortive guard above). A `resume` node
        // types as `Ty::Any` (infer.rs), so an arm body `(+ 1 (resume 10 s))` type-checks LENIENTLY — the
        // `+` accepts the `Any`-typed resume. But `(resume v s)` yields `C[v]`, which has the handle BODY's
        // type: for a Bool-typed body `(< (Amb.flip) 5)` the fold produces `(+ 1 (< 10 5))` — an integer
        // `+` over a Bool — which the ORIGINAL program was ill-typed to express (the arm consumes the
        // continuation result at a type the body cannot supply). `reduce_handle` runs at LOWERING, after
        // inference, so nothing re-checks the folded term — it would reach codegen as invalid wasm. Re-run
        // the type checker on the folded result and DECLINE if it faults, so an ill-typed composition is
        // rejected (the whole program errors — the handle has no valid fold), never miscompiled.
        let folded_faults = crate::infer::type_errors(db, folded);
        if !folded_faults.is_empty() {
            // Record the genuine type fault so the emit path surfaces CDZ0203 instead of the generic CDZ0900
            // (a `resume` types as `Ty::Any`, so this ill-typed composition is invisible at inference and
            // appears only in the folded term). The caller clears `fold_type_fault` before the fold and reads
            // it only on `None`, so a nested fold's fault never leaks.
            db.fold_type_fault = folded_faults.into_iter().next();
            return None;
        }
        return Some(folded);
    }
    // E5 ESCAPING-K over a RE-PERFORMING continuation (step-3 inc-2b / FACE-1 B2). A general `ctl`-style arm
    // that lets `k` ESCAPE (`cont: Some` carried here) whose delimited continuation `C` ITSELF re-performs
    // the handled effect — `(handle A 5 ((a () s k (use-k k))) (+ (A.a) (A.a)))`, where after the leading
    // `(A.a)` hole `C = (+ □ (A.a))` re-performs `A.a`. The pure-one-hole block above did NOT fire (`pure_
    // hole` fails on the second perform), and the two-hole refold below is keyed on a `resume` node this arm
    // has NONE of — so without this block it declines. It CANNOT be a compile-time in-place fold: `apply(k,
    // v)` runs `C[v]` in a SEPARATE activation, so the re-performed `A.a` in `C` must RE-ENTER the handler.
    // REIFY `k` as a SELF-RE-INSTALLING handler-wrapped closure: `k = (fn (#kv) H[perform := #kv])` where
    // `H` is the WHOLE handle node — splicing the leading perform inside `H` yields `(handle A 5 (arm) (+
    // #kv (A.a)))`, a copy of the handler wrapped around the continuation. Applying it (`(use-k k)` → `(k
    // 10)`) re-enters that handle, whose remaining `(A.a)` now has a home; the re-installed handle has ONE
    // FEWER perform, so `reduce_handle`'s natural re-entry folds it (recursively, N→N-1) — bottoming out at
    // the pure-one-hole reify when a single perform remains. So `(+ (A.a) (A.a))` → `(k 10)` = `(handle A 5
    // (arm) (+ 10 (A.a)))` → (+ 10 10) = 20. NO frame chain / `br_table` — a re-performing continuation is a
    // handler-wrapped closure; the recursion is the existing fold, bounded by the re-entry guard.
    //
    // SOUND SUBSET (this increment): the arm must be STATE-OBLIVIOUS — its body does not reference the state
    // binder `s`. Re-installing with the ORIGINAL `init` seed is only correct when the arm never advanced
    // the state (b2-min's `(use-k k)` never mentions `s`). A STATE-ADVANCING arm (the DES `sleep`'s `(at s
    // d)` wake computation) needs the advanced state threaded into the re-installed handle's seed — the
    // follow-on increment — so it still DECLINES cleanly here (never a wrong value). Also require exactly ONE
    // escaping application shape (single-value `k`), and the continuation must reach only THIS handler's
    // performs (no foreign/host perform in `C`, `body_reaches_foreign_perform` false) — a reified
    // continuation must not span a host boundary (§4.4), and a foreign perform in `C` has no home under the
    // re-installed same-effect handler.
    if let Some(perform) = do_aware_leading_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        // ESCAPING-K: the arm binds `k` and let it escape (`cont: Some` — carried past the classifier).
        && let Some(k_binder) = arm.cont
        && !ctx.abortive.contains(&(decl, idx))
        // EXACTLY TWO discharged performs in the body: the leading hole + ONE remaining in the continuation
        // `C`. The self-re-installing reify drives ONE re-entry (removing the leading perform), leaving a
        // single-perform handle the pure-one-hole fold bottoms out. A body with >2 performs would need
        // repeated re-installs this single-level reify does not complete (it produces a residual non-
        // applyable continuation), so decline cleanly there — the deeper-recursion increment's job.
        && count_discharged_performs(db, body, &ctx) == 2
        // STATE-OBLIVIOUS: the arm body must not read the state binder (else re-install with `init` is stale).
        && count_param_refs(db, arm.body, arm.state) == 0
        // The continuation must not span a foreign/host perform (host-composition invariant §4.4).
        && !body_reaches_foreign_perform(db, body, &ctx)
        // The whole handle node (H) must be reconstructable. By the time the fold runs, the canonical
        // `(handle E seed arms body)` has been desugared to the INTERNAL `(handle-internal seed arms body)`
        // (its head re-spelled, effect dropped) — that is `body`'s parent. Splicing the leading perform
        // inside it yields `(handle-internal seed arms C[#kv])`, the self-re-installing wrapped handle.
        && let Some(handle_node) = db.parent_of(body)
        && db.ast.head_name(handle_node) == Some(HANDLE_INTERNAL)
    {
        // Substitute the arm's params ↦ (pure-copied) leading-perform args and its state binder ↦ the init
        // seed (nothing runs before the leading perform on the strict spine, so the state there is the seed).
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, copy_pure(db, a));
                }
            }
        } else if arm.params.len() == 1 && args.is_empty() {
            let p = arm.params[0];
            if !is_unit_param(db, p) {
                let unit = db.push_list(vec![]);
                subst.insert(p, unit);
            }
        } else {
            return None;
        }
        subst.insert(arm.state, init);
        // Reify `k = (fn (#kv) H[leading-perform := #kv])`: splice the leading perform inside the WHOLE
        // handle node (not just the body) so the continuation carries the handler around itself.
        let kv_name = format!("#kv{}", perform.0);
        let kv_binder = db.push_name(&kv_name);
        let kv_ref = db.push_name(&kv_name);
        let cont_body = splice_context(db, handle_node, perform, kv_ref);
        let fn_head = db.push_name("fn");
        let params_list = db.push_list(vec![kv_binder]);
        let k_lambda = db.push_list(vec![fn_head, params_list, cont_body]);
        subst.insert(k_binder, k_lambda);
        let folded = crate::eval::beta_reduce(db, arm.body, &subst);
        reparent_under_handle_site(db, folded, body);
        // Type-consistency guard (as the pure-one-hole block) — the synthesized term is not re-checked
        // before codegen, so an ill-typed composition must decline, never miscompile.
        let folded_faults = crate::infer::type_errors(db, folded);
        if !folded_faults.is_empty() {
            // Record the genuine type fault so the emit path surfaces CDZ0203 instead of the generic CDZ0900
            // (a `resume` types as `Ty::Any`, so this ill-typed composition is invisible at inference and
            // appears only in the folded term). The caller clears `fold_type_fault` before the fold and reads
            // it only on `None`, so a nested fold's fault never leaks.
            db.fold_type_fault = folded_faults.into_iter().next();
            return None;
        }
        return Some(folded);
    }
    // E5 TWO-HOLE (general one-shot) fold: a NON-tail one-shot arm whose LEADING discharged perform sits on
    // the strict spine but whose continuation ITSELF performs (a second hole) — `(+ (Amb.flip) (Amb.flip))`
    // under `(flip (u) s (+ 1 (resume 10 s)))`. The pure one-hole block above declined it (`C` is not pure).
    // In a DEEP handler, `resume v s'` returns into `C[v]` WITH THE HANDLER STILL ACTIVE, so the second
    // perform in `C[v]` is handled too: `resume v s' = reduce_handle(s', arms, C[v])`. Each refold removes
    // one perform → terminates. GATED to a ONE-SHOT arm (`count_resumes == 1`): the resume value flows into
    // `C` exactly once, so the inner perform in `C` runs exactly once (a multi-shot arm would duplicate it —
    // the frame vertical's job). The leading perform's ARGS are strongly pure (`leading_strict_hole` checks),
    // so they need no state threading; the state at the leading perform is the seed (nothing runs before it).
    if let Some(perform) = do_aware_leading_hole(db, body, &ctx)
        && let Resolved::Apply { head, args } = resolved_of(db, perform)
        && let Some((decl, idx)) = is_perform(db, head, &ctx)
        && let Some(arm) = ctx.arms.get(&(decl, idx)).cloned()
        && !ctx.abortive.contains(&(decl, idx))
        // A tail-resumptive arm (bare OR do-wrapped interpose/forward) is served by the `thread` path — do
        // NOT steal it here (it would decline a forwarding arm whose resume value is a foreign perform).
        && !is_tail_resumptive_arm(db, arm.body)
        // A MATCH-SHAPED resumptive arm (`(match s ((Some n) (resume n s)) …)`) is served by the thread
        // path's match-shaped-resume-PEEL (`peel_resume_from_arm_body`), which folds a `do`-sequenced
        // multi-perform body over it. The `do`-aware leading-hole above would otherwise let THIS block reach
        // such a `do`-bodied case (`(do (St.get) (+ 1 (St.get)))` over a match-arm) and steal-then-decline it
        // (the refold does not serve a match-peel arm). So defer any peelable arm to the thread path — only a
        // NON-peelable arm (e.g. the DES deferred-resume-thunk `(set (w) s (run-thunk (fn (_u) (resume w w))))`)
        // is this block's. Without the `do`-aware finder this guard was unnecessary (the global finder never
        // reached a `do` body); it becomes load-bearing exactly because `do_aware_leading_hole` now does.
        && peel_resume_from_arm_body(db, arm.body).is_none()
        // MULTI-SHOT is sound only when the continuation `C` (re-reduced per resume) reaches NO FOREIGN
        // perform — i.e. only THIS handler's discharged ops, which the refold folds away into pure code.
        // A ONE-SHOT arm splices `C` once, so any foreign perform in it runs once (sound). But a MULTI-shot
        // arm would re-run a foreign/HOST perform in `C` once per resume — the host-composition invariant
        // (DESIGN §4.4: a reified continuation must not span a host call) forbids that, so require the body
        // to be free of any undischarged (foreign/host) perform when the arm resumes more than once.
        && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, &ctx))
        // CONDITIONALLY-RESUMING ARM GUARD (twin of the pure-one-hole block's): decline a partial-resume arm
        // (`(if cond ABORT (resume …))`) — the refold would rewrite only the resuming branch and mis-splice.
        && !arm_partially_resumes(db, arm.body)
    {
        // SILENT-MISCOMPILE GUARD (breaker pyth1). A nested closed HANDLE in the POST-RESUME TOLL position —
        // `(+ (resume v s') (handle E 40 … (+ (E.tick) 2)))` — is NOT reduced to its value by the refold; its
        // dispatches leak into the outer fold instead of folding to a self-contained 42, so a closed handle
        // = 42 in the toll produced 1414 not the correct 196 (uniform wasm+rust+rust-async, distinct-effect
        // identical → a genuine VALUE bug, not routing). The referentially-equal literal 42 in the toll folds
        // correctly. DECLINE (reject-not-miscompile) when the arm body reaches a nested handle OUTSIDE a
        // resume's own args — the resume subtree is skipped because its ANSWER value folds correctly via
        // `splice_context` (pyre6, landed) and its NEXT-STATE is guarded separately in
        // `rewrite_resume_to_refolded_context` (pyre3). Narrow: only a toll/context-position handle trips it.
        if arm_toll_reaches_nested_handle(db, arm.body) {
            return None;
        }
        let mut subst: HashMap<StructId, StructId> = HashMap::default();
        if arm.params.len() == args.len() {
            for (&p, &a) in arm.params.iter().zip(args.iter()) {
                if !is_unit_param(db, p) {
                    subst.insert(p, copy_pure(db, a));
                }
            }
        } else if arm.params.len() == 1 && args.is_empty() {
            let p = arm.params[0];
            if !is_unit_param(db, p) {
                let unit = db.push_list(vec![]);
                subst.insert(p, unit);
            }
        } else {
            return None;
        }
        subst.insert(arm.state, init);
        let substituted = crate::eval::beta_reduce(db, arm.body, &subst);
        // PIN the substituted arm body's resolution before the refold rebuilds it (breaker ts1, false-CDZ0101
        // unbound body-free-var). A two-site arm whose op-ARG is an enclosing binder — the body performs
        // `(St.feed n)`, `n` = main's param, substituted for `v` into the arm's `(if (> v 10) …)` CONDITION —
        // carries that `n` in the condition. `rewrite_resume_to_refolded_context` rebuilds the `if`-shaped arm
        // via `push_list` (recursing every child, incl. the condition), which OVERWRITES the shared `n` node's
        // parent to the fresh `>` → detaches `n` from main (single-parent arena) → the rebuilt `(if (> n 10) …)`
        // is a root, `n`'s chain dead-ends → spurious CDZ0101 'unbound n' on a VALID program. Anchor
        // `substituted` under the handle site FIRST so `n` resolves up the live chain to main, THEN
        // `resolve_subtree` memoizes every node's resolution against that CURRENT position — so the rebuild
        // can't change how `n` resolves (the apply_lambda pin-before-copy idiom, resolve.rs; the same fix
        // v-inference used for the guard-desugar arm-copy sibling ag5). Idempotent → no hang; a constant-arg
        // arm (no enclosing-binder ref) is byte-identical.
        reparent_under_handle_site(db, substituted, body);
        crate::resolve::resolve_subtree(db, substituted);
        // Rewrite the arm's single `(resume v s')` to `reduce_handle(s', arms, C[v])` — the re-reduced
        // continuation (a further discharged perform in `C` is folded by the recursive call). Declines
        // cleanly (`?`) if any recursive refold cannot be served.
        let folded = rewrite_resume_to_refolded_context(db, substituted, body, perform, arms)?;
        reparent_under_handle_site(db, folded, body);
        // Same type-consistency guard as the pure one-hole block — the synthesized term is not otherwise
        // re-checked before codegen.
        let folded_faults = crate::infer::type_errors(db, folded);
        if !folded_faults.is_empty() {
            // Record the genuine type fault so the emit path surfaces CDZ0203 instead of the generic CDZ0900
            // (a `resume` types as `Ty::Any`, so this ill-typed composition is invisible at inference and
            // appears only in the folded term). The caller clears `fold_type_fault` before the fold and reads
            // it only on `None`, so a nested fold's fault never leaks.
            db.fold_type_fault = folded_faults.into_iter().next();
            return None;
        }
        return Some(folded);
    }
    // CALLER-OBSERVED OUT-STATE (task #15). Before threading, scan the handle body for a recursive-effectful
    // call whose FINAL out-state a LATER spine item observes (`(do (run-ops …) (Prim.run 0))` — the trailing
    // perform reads the state `run-ops` advanced). Record each such callee so `specialize_recursive` emits it
    // in MULTI-VALUE mode (return `(value, out-state)`); the single-return convention drops the advance and
    // silently miscompiles the observer to the pre-recursion state. The mode decision reads only the callee's
    // OWN body, so this caller-side observation must be recorded up front. Purely additive: it only UPGRADES a
    // multi-value-threadable callee — a non-threadable one stays single-return.
    mark_caller_observed_outstate(db, body, &ctx);
    // RECURSION-BOUNDARY caller-observed out-state (finding #19): a recursive-effectful callee whose out-state
    // flows into an enclosing recursive def's self-call argument must thread across the recursion. Scans the
    // reachable recursive-effectful defs' bodies (not just the handle body) and marks both the callee and the
    // enclosing def multi-value. Additive: gated by `multivalue_leaves_threadable` at the mode decision.
    mark_recursion_boundary_observed_outstate(db, body, &ctx);
    // CALLER-OBSERVED OUT-STATE via a same-effect ABORT arm (breaker sr5). The task-#15 machinery above
    // upgrades a recursive-effectful callee to MULTI-VALUE mode when a LATER spine perform observes its
    // out-state — which threads correctly to a RESUMING observer (sr4 → 2). But when the observing perform's
    // arm is ABORTIVE (a `(fin (u) s s)` with no `resume`), the abort COLLAPSE materializes the arm value
    // (reading the state binder `s`) against the pre-recursion SEED slot rather than the callee's threaded
    // out-state — so `(do (def _g (grow k)) (Acc.fin))` reads 0 instead of the advanced 2: a SILENT
    // MISCOMPILE on both backends (breaker sr5, HIGH). Folding it correctly needs the abort collapse to read
    // the multi-value out-state the recursion advanced (a later increment, same family as the resuming
    // thread). Until then DECLINE cleanly (→ Todo) rather than emit the wrong value. NARROW: fires only when
    // a recursive-effectful callee advancing THIS handler's state is followed on the body spine by an
    // ABORTIVE same-handler perform that reads state — the resuming observer (sr4), the observer as the
    // recursion BASE CASE (sr2), and a plain same-op observer (sr1) all thread/fold correctly and are NOT
    // flagged.
    // sr5: FORMERLY a blanket decline of a recursive-advance-observed-by-abort. Now the multi-value path threads
    // the recursion's out-state to the abort observer (specialize_recursive's caller-observed multi-value mode,
    // extended to the abortive-observer case) when the callee is threadable — so the fold serves it. A callee the
    // multi-value machinery cannot bind still DECLINES cleanly, at specialize_recursive's caller-observed abortive
    // floor (not here). So this pre-reduction blanket guard is removed; the threadable case folds to the advanced
    // state, the non-threadable case declines downstream.
    // SEED LET-LIFT for a NON-CONSTANT init carrying a live CAPTURE. The `thread` perform arm splices the
    // threaded state (which starts as `init`) at every `s` reference and `deep_fresh_copy`s each splice to
    // break value/next-state sharing (the resume(a,a) bug). That fresh copy re-pushes each leaf UNPINNED, so
    // an internal state param re-resolves against the specialized def's sig — correct — but a leaf that was
    // RESOLVE-PINNED to a LIVE enclosing binder (a caller runtime arg `k` substituted into the seed by an
    // inlining `(f k)`, arm body `(resume s (+ s 1))` reading `s` twice) loses its pin and re-resolves against
    // the folded orphan → a spurious CDZ0101 "unbound k" (the let-wrapped-handle-seed bug). Rather than teach
    // `deep_fresh_copy` to tell a live capture from a to-be-respecialized param (a fragile global change), bind
    // the seed ONCE here to a fresh INTERNAL name and thread THAT: each `s` splice is then a fresh UNPINNED
    // `#seed` occurrence (nothing to lose on copy) that re-resolves to the wrapping `let`, and the capture `k`
    // sits in exactly ONE place — the let-init, grafted under the handle site below so it resolves up the live
    // chain. GATED to a non-constant seed (a bare int/bool/etc. leaf is scope-independent, shared safely, and
    // stays byte-identical — the common `(handle St 0 …)` corpus case is untouched); the wrap fires only for
    // the rare runtime-arg seed the bug needs.
    let seed_wrap: Option<(StructId, StructId)> = if !seed_is_shareable_constant(db, init) {
        let nm = format!("#seed{}", init.0);
        let binder = db.push_name(&nm);
        let sref = db.push_name(&nm);
        Some((binder, sref))
    } else {
        None
    };
    let thread_seed = seed_wrap.map(|(_, r)| r).unwrap_or(init);
    // Apply the SEED LET-LIFT to a to-be-returned value: `(let ((#seed init)) value)` when the seed was
    // let-bound (non-constant), else the value unchanged. Every `#seed` occurrence threaded into `value`
    // re-resolves to this binding. Used by BOTH the resumptive return (below) AND the abortive returns: an
    // abort arm that READS the state binder — `(halt (u) s (* 1000 (+ (Map.len s) a)))` over a HEAP seed
    // `Map.empty` — carries `#seed` refs (the state binder was threaded as `#seed`), so the collapsed abort
    // value must be wrapped in the same `let` or `#seed` reads UNBOUND (CDZ0101 — the abort × seed-let-lift
    // seam; scalar seeds are shareable constants → no `#seed`, so they never hit it; heap seeds do). breaker
    // heap-abort-state issue 2026-08-04.
    // Use `binder`/`init` DIRECTLY (not copies): `init` is the ORIGINAL seed node — it may carry a free
    // capture (an enclosing fn's param, `(handle St k …)` where `k` is main's param), and the whole `let` is
    // grafted under the handle site by the caller's `reparent_under_handle_site`, restoring `init`'s lexical
    // chain. A `copy_pure(init)` would re-push it DETACHED, orphaning that capture → CDZ0101. Only ONE return
    // path fires per `reduce_handle` call, so reusing `binder`/`init` in one place is safe (no arena aliasing).
    let apply_seed_wrap = |db: &mut Db, value: StructId| -> StructId {
        if let Some((binder, _)) = seed_wrap {
            let let_head = db.push_name("let");
            let pair = db.push_list(vec![binder, init]);
            let bindings = db.push_list(vec![pair]);
            db.push_list(vec![let_head, bindings, value])
        } else {
            value
        }
    };
    // Thread the INIT state through the body in evaluation order. The handle's value is the body's
    // value (the accumulated state is observable only through the operations), so we return the
    // rewritten body; the final threaded state is discarded (the body never reads it directly).
    let (rewritten, _final_states) = thread(db, body, vec![thread_seed], &ctx)?;
    // ABORTIVE (E4): if an abortive perform fired during threading, the handle's value is that arm's
    // value — the surrounding computation was abandoned, so the threaded body is dead. (Unconditional
    // strict abort only; a conditional abort was declined above.)
    if let Some(abort) = ctx.abort_value.get() {
        // ABORT-FOLD, do-SHAPE (preserve a pre-abort FOREIGN advance). An abort collapses the handle to the
        // arm value — but a FOREIGN perform (an OUTER handler's effect, or a host op) evaluated on the strict
        // spine BEFORE the abort has ALREADY committed its state advance to the ENCLOSING computation and must
        // survive (the inner abort is the inner handle's control; it cannot roll back an outer effect's
        // committed step). For a `(do <foreign…> <abortive-perform>)` body, the do-arm above ALREADY keeps the
        // pre-abort foreign items and appends the abort value as the tail — so `rewritten` is a sound
        // `(do <foreign…> <abort-value>)` whose foreign prefix the ENCLOSING fold discharges (advancing the
        // outer state), then yields the abort value. Returning it (instead of the BARE abort) preserves that
        // advance: `(do (A.tick) (B.bail 99))` under B → `(do (A.tick) 99)`, so an outer `(A.get)` reads the
        // advanced state (110, not the 109 the bare-abort collapse produced — breaker ao1-ao4, incl. heap ao3
        // + multi-advance ao4; the do-arm's `kept` already threads the FULL prefix trace per ao4).
        //
        // GATED to `rewritten` being a `do`-FORM. This is what SEPARATES the sound do-shape from the STILL-
        // AMBIGUOUS strict-operand shape (`(+ (A.tick) (B.bail 99))` → `rewritten` `(+ (A.tick) 99)`, a `+`
        // form): a bare-abort `+` case that is CORRECT-because-UNOBSERVED (the corpus case "an abortive
        // perform under THREE nested handlers abandons the two resumptive frames above it", `(+ (A.a) (+ (B.b)
        // (Bail.bail 99)))` = 99) is indistinguishable at THIS (inner) handler from the miscompiling one (the
        // difference lives in the OUTER continuation, invisible here) — so the strict-op lift is a SEPARATE
        // increment. Only the do-form, where the do-arm already produced the sound for-effect sequencing, is
        // safe to return now. The foreign-reach check reads the ORIGINAL `body` (parented — no orphan
        // resolve-pin poison, unlike `rewritten` whose tail is the orphan abort value). ZERO corpus regressions
        // (a `+`/`let`/`match`-shaped abort body is untouched; only a `do` body with a kept foreign prefix
        // flips todo→pass).
        if db.ast.as_form(rewritten, "do").is_some() && body_reaches_foreign_perform(db, body, &ctx)
        {
            // DRAIN before returning (github-liaison review on #2002, HIGH-class). The normal path below
            // runs `drain_and_wrap` to wrap `rewritten` in the binding-`let`s for any pending MULTIVALUE
            // self-call temps (a `f#ctx` self-call arm pushes `#t` to `ctx.pending` and returns `(. t 0)`;
            // the temp is bound here). This do-form early-return must drain TOO: a multivalue self-call in
            // the KEPT pre-abort foreign prefix — `(do (relabel-self…) (A.tick) (B.bail 99))` — would leave
            // its `(. t 0)` referencing an unbound `#t` if we returned `rewritten` undrained → spurious
            // CDZ0101 / no-machine-representation. `drain_and_wrap` is a no-op when nothing is pending (the
            // common case — a plain `(do (A.tick) (B.bail 99))` has no self-call temp), so this is a strict
            // hardening: byte-identical when `ctx.pending` is empty, correct when it is not.
            // ONLY the `#st` drained inits (the FINDING-24 per-dispatch state binds) — NOT every pending
            // entry. `ctx.pending` ALSO holds self-call `#t` temps (a `f#ctx` self-call arm pushes `(#t,
            // call_node)`); forgetting THOSE re-resolves a `call_node` subtree that can hold a BODY binder
            // (a match/let binder like `root`) → CDZ0101 unbound `root` (the compiler-ml self-host reject:
            // db-query-diff where NO `#st` fires, only self-call temps). The forget is FOR the `#st` seed-memo
            // only; filter by the `#st` name prefix so a non-`#st` temp is never touched.
            let do_pending_inits: Vec<StructId> = ctx
                .pending
                .borrow()
                .iter()
                .filter(|(name, _)| name.starts_with("#st") || name.starts_with("#fa"))
                .map(|(_, init)| *init)
                .collect();
            let drained = drain_and_wrap(db, &ctx, 0, rewritten);
            // Wrap in the seed let-lift (heap seed → the do-prefix / abort value may read `#seed`).
            let drained = apply_seed_wrap(db, drained);
            reparent_under_handle_site(db, drained, body);
            // FINDING-24 stale-memo clear (see the resumptive-return twin below): forget ONLY the drained `#st`
            // init subtrees so their `#seed`/`#st` refs re-resolve against the grafted chain (a blanket
            // `forget_subtree(drained)` regressed a live pin — see the twin). Empty unless an `#st` bind drained.
            for &init in &do_pending_inits {
                crate::resolve::forget_subtree(db, init);
            }
            return Some(drained);
        }
        // Re-anchor the abort value under the handle site BEFORE returning — the SAME reparent the normal
        // path does below (line ~1848). The abort value is the arm body `copy_pure`d off the (now-dead)
        // resume/perform node, a synthesized orphan with parent `None`. When it is (or contains) a BARE
        // NAME referencing an ENCLOSING binder — the arm `(bail (n) s n)` returns `n`, bound to the perform
        // arg, which for a RUNTIME arg `(Bail.bail k)` is a reference to the caller's param `k` — the
        // orphan copy has no lexical chain, so `k` reads UNBOUND → Poison → the handle types `Any` and
        // lowering declines "return type has no machine representation" (wasm) while rust computes (a
        // backend split: corpus-bugfix 2026-07-18). A CONST arg `(Bail.bail 7)` folds to a literal (no free
        // name) so it never exhibited this — only a runtime arg leaves a free reference. Re-parenting under
        // the handle restores the chain abort → handle → def so `k` re-resolves to the param and types
        // Int64. (Mirrors the identity-arm pass-through reparent for the resumptive path.)
        // DRAIN before returning (sr5): the abort arm value may read a pending MULTIVALUE self-call temp. For
        // `(do (def _g (grow k)) (Acc.fin))` where `grow` is caller-observed multi-value, threading the pre-abort
        // `(grow k)` pushed `$t0` to `ctx.pending` and advanced `cur[slot]` to `(. t0 1)`; the abort arm
        // `(fin (u) s s)` reads that state, so the abort value IS `(. t0 1)` — which references the unbound `$t0`
        // unless we bind it here. `drain_and_wrap` wraps the abort value in the pending binding-`let`s
        // (`(let (($t0 (grow k))) (. t0 1))`), so the recursion RUNS (advancing state) and the abort reads the
        // advanced out-state. A no-op when nothing is pending (the common bare-abort case → byte-identical).
        // Filter the `#st` per-dispatch inits to FORGET (re-resolve `#seed`/`#st` refs against the grafted
        // chain) exactly as the do-form + resumptive twins do; a non-`#st` temp (the `$t0` self-call node) is
        // NOT forgotten (its `call_node` may hold a body binder → CDZ0101 if re-resolved).
        let abort_pending_inits: Vec<StructId> = ctx
            .pending
            .borrow()
            .iter()
            .filter(|(name, _)| name.starts_with("#st") || name.starts_with("#fa"))
            .map(|(_, init)| *init)
            .collect();
        let abort = drain_and_wrap(db, &ctx, 0, abort);
        // Wrap in the seed let-lift: an abort arm that READS a heap-typed state binder carries `#seed` refs
        // (the state was threaded as `#seed`); without the wrapping `let` they read unbound (CDZ0101).
        let abort = apply_seed_wrap(db, abort);
        reparent_under_handle_site(db, abort, body);
        for &init in &abort_pending_inits {
            crate::resolve::forget_subtree(db, init);
        }
        return Some(abort);
    }
    // MULTI-VALUE (repro-1): if the handle BODY was itself a self-call to a multi-value spec — `(handle …
    // (relabel tree))` — the self-call arm pushed a pending temp and returned `(. t 0)` (the value
    // projection); the temp is not yet bound. Drain any pending temps into wrapping `let`s so the handle
    // value is `(let ((t (f#ctx … init))) (. t 0))`. (The self-call arm already discards each spec's
    // OUT-state at the top level — the handle observes only the value.) Nothing pending → returns `rewritten`.
    // Capture the pending drain-init node ids BEFORE draining (FINDING-24 stale-memo clear, below): these are
    // the `#st` per-dispatch state binds, pre-existing resolved subtrees whose `#seed` refs need re-resolution
    // once the seed-wrap grafts the binder. Empty in the common case (nothing pending → no-op forget).
    // ONLY the `#st` drained inits — NOT every pending entry (see the do-form twin above): `ctx.pending`
    // also holds self-call `#t` temps whose `call_node` can transitively hold a body binder (`root`);
    // forgetting a non-`#st` temp re-resolves it → CDZ0101 unbound `root` (the self-host reject where no
    // `#st` fires). Filter by the `#st` name prefix.
    let pending_inits: Vec<StructId> = ctx
        .pending
        .borrow()
        .iter()
        .filter(|(name, _)| name.starts_with("#st"))
        .map(|(_, init)| *init)
        .collect();
    let wrapped = drain_and_wrap(db, &ctx, 0, rewritten);
    // Apply the SEED LET-LIFT decided above: `(let ((#seed init)) wrapped)`. Every `#seed` occurrence
    // threaded into the body re-resolves to this binding; `init` (carrying the live capture) is evaluated
    // once here, and the whole `let` is grafted under the handle site below so `init`'s free names resolve
    // up the original chain. `None` (a constant seed) leaves `wrapped` untouched — byte-identical to before.
    // (Same helper the abortive returns above use.)
    let wrapped = apply_seed_wrap(db, wrapped);
    // Graft the threaded result UNDER the original handle's site (the same re-anchoring the E5 pure-one-hole
    // and multi-shot blocks above do). The `thread` perform arm returns the resume VALUE as the perform's
    // result via `deep_fresh_copy` — a freshly-pushed subtree whose root parent is `None`. When that value is
    // a BARE NAME referencing an ENCLOSING binder — the IDENTITY-arm pass-through `(handle St k ((get (u) s
    // (resume s s))) (St.get))`, whose folded body is just `k` (main's param) — the copy has no lexical chain,
    // so the reference reads UNBOUND → `Poison` → the handle types as `Any` and lowering declines "return type
    // has no machine representation" (a check/compile divergence: infer's export solve grounds it Int64, but
    // `type_of(rewritten)` here reads `Any`). Re-parenting `wrapped` under the handle node restores the chain
    // wrapped → handle → def so `k` re-resolves to main's param and types Int64. (A folded body forced through
    // an arithmetic op — `(+ s 1)` or a compound body — already re-parents its operands, which is why those
    // faces did not exhibit the leak; the bare pass-through is the gap.)
    reparent_under_handle_site(db, wrapped, body);
    // FINDING-24 stale-memo clear: when the seed was let-lifted (`seed_wrap` present) AND the fold drained a
    // per-dispatch `#st` state bind (v-effects' fold half), that `#st` let-init is a PRE-EXISTING resolved
    // program subtree (e.g. `(List.push #seed22 t)`) whose `#seed` reference was memoized at THREADING time —
    // before `apply_seed_wrap` (above) minted the `#seed` binder. `resolved_of` memoizes against the position
    // at first resolve, so that `#seed` occurrence stays memoized UNBOUND and the just-grafted `#seed` let is
    // never consulted → CDZ0101 `unbound #seed`. Forget ONLY the drained `#st` init subtrees (`pending_inits`)
    // so the next `resolved_of` recomputes their `#seed`/`#st` references against the FINAL `#seed`-outer /
    // `#st`-inner chain this function just built. Same stale-memo-after-reparent class the copied-binder paths
    // clear via `forget_subtree`. TARGETED (not a blanket `forget_subtree(wrapped)`): a whole-tree forget
    // re-resolves correctly-pinned nodes too and regressed 1 case (the `il2` data-driven-interleave-width
    // handler — a live pin shifted). Forgetting only the drained inits clears exactly the stale `#st`/`#seed`
    // memos and leaves every other pin intact → the seed-wrap corpus stays byte-identical (verified 0-regress).
    // `pending_inits` is empty unless the fold drained an `#st` bind, so this is a no-op for every existing
    // handle (constant OR heap seed) — it engages only once v-effects' fold half emits an `#st` state bind.
    for &init in &pending_inits {
        crate::resolve::forget_subtree(db, init);
    }
    Some(wrapped)
}

/// Reduce every NESTED inner `handle` found in `node` to its folded form, IN PLACE (returning a rewritten
/// copy). An inner handle that reduces (`reduce_handle`) is replaced by its result; one that does not is
/// left untouched. This runs in `reduce_handle` BEFORE the E5 pure-one-hole check so an OUTER handler whose
/// arm is NON-tail-resumptive can serve a body that contains a reducible inner handle of a DIFFERENT
/// effect: the inside-out `thread` path reduces an inner handle only while threading (which needs the OUTER
/// arm tail-resumptive), so `(handle A non-tail (handle B tail (+ (A.a) (B.b))))` used to decline — B never
/// got reduced before A's E5 fold ran and saw the raw inner `handle` node (a non-uniform continuation).
/// Reducing B FIRST turns the body into `(+ (A.a) 20)`, a single A-perform in a pure one-hole context the
/// outer E5 fold folds to `(+ 1 (+ 20 10))` = 31. SOUND + frame-free: reducing the inner handle is the
/// same already-proven-safe reduction the threading path performs, only sequenced earlier. Only the
/// SHALLOWEST inner handles are reduced (`reduce_handle` recurses into its own body), and a handle that is
/// itself the whole `node` is NOT reduced here (the caller is mid-reducing it). Bounded by `reduce_handle`'s
/// own re-entry guard.
pub(crate) fn reduce_inner_handles(db: &mut Db, node: StructId) -> StructId {
    // A `handle` node: reduce IT (its body's own nested handles are reduced by that call recursing here),
    // and use the result. If it declines, fall through to a structural copy so the node is unchanged.
    if let Resolved::Handle { init, arms, body } = resolved_of(db, node)
        && let Some(reduced) = reduce_handle(db, init, &arms, body, false)
    {
        return reduced;
    }
    // Otherwise descend structurally, reducing any inner handle in a child.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| reduce_inner_handles(db, c))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => node,
    }
}

/// Whether `node` IS or CONTAINS a nested `handle` (of an inner effect). The gate for the nested-handle
/// pre-reduction — a cheap syntactic check so a body with no inner handle skips the pre-reduction pass. The
/// handle body may BE the inner handle directly (`(handle A … (handle B …))`) or contain it inside an
/// operator (`(+ (handle B …) x)`), so both `node` itself and its descendants count.
pub(crate) fn body_contains_nested_handle(db: &mut Db, node: StructId) -> bool {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children.iter().any(|&c| body_contains_nested_handle(db, c)),
        Struct::Atom(_) => false,
    }
}

/// Whether the handle body has a MATCH whose arm PATTERN performs a discharged op — i.e. a `(guard
/// <pattern> <cond>)` whose GUARD CONDITION performs (a plain pattern is a binder/literal that never
/// performs, so a perform in the PATTERN position is always a guard cond). The effect-routing/distribution
/// walks (`distribute_handler_over_conditional`, the fold) descend a match's SCRUTINEE and ARM BODIES but
/// NOT the arm guard conditions, so a perform in a guard is left unrouted and reaches lowering as a bare
/// perform → the misleading "performed with no enclosing handler here" (a handler DOES enclose it). Routing
/// a guard perform through the handler is a genuine extension (the guard runs before the arm, advancing
/// handler state per arm-test — a stateful sequencing the current per-branch-sees-the-seed distribution
/// does not model), so until it is wired, DECLINE cleanly here (`reduce_handle` → `None` →
/// `HANDLER_NOT_REDUCIBLE_DECLINE`, an honest "not yet reducible" todo) rather than letting the unrouted
/// perform surface the factually-wrong "no enclosing handler" error. Recurses under `let`/`if`/`do`/nested
/// `match` (a guard in any nested match under the handle body is equally unrouted). A NESTED `handle`'s own
/// body is NOT descended (its guards are its own handler's concern — reduced when that inner handle folds).
pub(crate) fn body_has_performing_match_guard(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return false; // an inner handle's guards belong to that handle's own reduction
    }
    if let Resolved::Match { arms, .. } = resolved_of(db, node)
        && arms.iter().any(|&(pat, _)| subtree_performs(db, pat, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_performing_match_guard(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `pat` is an IRREFUTABLE inner pattern — a bare name binder or `_` wildcard — which matches ANY
/// scrutinee. Used by the guard desugar: a guarded arm whose inner pattern is irrefutable is selected iff
/// its GUARD holds, so it desugars to an `if` on the guard (the pattern narrows nothing).
pub(crate) fn is_irrefutable_pattern(db: &mut Db, pat: StructId) -> bool {
    match db.ast.as_name(pat) {
        Some("_") => true,
        Some(_) => true, // a bare name binds the whole scrutinee — irrefutable
        None => false,   // a literal / compound / annotated pattern refutes
    }
}

/// FINDING-24 COVERAGE-GAP (breaker sft1, the min-heap-sift face). Canonicalize a SINGLE-ARM match whose
/// only arm is a bare-NAME binder over a COMPOUND scrutinee — `(match <compound> (h2 <body>))` — into the
/// equivalent `let`: `(let ((h2 <compound>)) <body>)`. A single bare-name arm is UNCONDITIONALLY irrefutable
/// (a nullary/payload constructor pattern is a 2-element LIST `(Ctor sub)`, an atom name never is — see
/// `eval::fold_ctor_match` slice 0/1), so the match always succeeds and binds `h2` to the whole scrutinee;
/// the `let` is a pure syntactic identity. WHY it matters: the resumptive fold threads a `match` by copying
/// the SCRUTINEE into every continuation copy (one per dispatch), so a compound scrutinee referencing handler
/// state duplicated per dispatch makes emit grow SUPER-LINEARLY — the same continuation-duplication class as
/// finding-24, exposed through a match-scrutinee instead of a `let`-init (sft1: exponential, 47015 locals in
/// one function → invalid wasm "too many locals"). Routing the scrutinee through a `let` binds it ONCE via
/// the existing per-dispatch `#st` state-bind machinery `drain_and_wrap` already covers, collapsing the
/// growth back to LINEAR (sft1: 1.47MB→38820 bytes, valid, byte-for-byte correct outputs). Gated to a
/// COMPOUND scrutinee (an atom scrutinee is already a single slot — case E in the diagnosis is LINEAR — so
/// leave it untouched to minimize churn) and to a SINGLE arm (a multi-arm match is a real runtime dispatch,
/// not an irrefutable bind). Walks structurally (the match may be nested under `if`/`let`/`do` in an arm
/// body); copy-on-change so an unaffected subtree keeps its node identity (the scrutinee-identity-keyed
/// `fold_ctor_match`/`SumPayload` passes must not see a needlessly-rebuilt spine).
pub(crate) fn hoist_single_binder_match_scrutinee(db: &mut Db, node: StructId) -> Option<StructId> {
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && arms.len() == 1
    {
        let (pat, body) = arms[0];
        // The arm pattern must be a bare NAME binder (not `_`, not a `(Ctor …)`/tuple/literal list pattern),
        // and the scrutinee must be COMPOUND (a list — an application/op/ctor expr), not an atom already in
        // one slot. A `_` wildcard is left alone: it binds nothing, so there is no continuation-copied use to
        // collapse, and the scrutinee (if pure) folds away regardless.
        if let Some(name) = db.ast.as_name(pat).map(|s| s.to_string())
            && name != "_"
            && matches!(db.ast.get(scrutinee), Struct::List(_))
        {
            // First hoist any single-binder match NESTED in the body, then rebuild this one as a `let`.
            let body = hoist_single_binder_match_scrutinee(db, body).unwrap_or(body);
            let name_atom = db.push_name(&name);
            let let_head = db.push_name("let");
            let binding = db.push_list(vec![name_atom, scrutinee]);
            let bindings = db.push_list(vec![binding]);
            return Some(db.push_list(vec![let_head, bindings, body]));
        }
    }
    // Recurse structurally: the match may be nested inside an `if`/`let`/`do` arm body.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut changed = false;
            let mut rebuilt = Vec::with_capacity(children.len());
            for c in children {
                match hoist_single_binder_match_scrutinee(db, c) {
                    Some(r) => {
                        rebuilt.push(r);
                        changed = true;
                    }
                    None => rebuilt.push(c),
                }
            }
            if changed {
                Some(db.push_list(rebuilt))
            } else {
                None
            }
        }
        Struct::Atom(_) => None,
    }
}

/// Rewrite a match with a PERFORMING GUARD into an `if` so the existing if-condition fold routes the guard
/// perform — the SOUND, NARROW case: a two-arm match `(match scrut ((guard <irrefutable> g) b) (<irrefutable>
/// b2))` where the FIRST arm's inner pattern is irrefutable (so the arm is selected exactly when `g` holds)
/// and the SECOND is an irrefutable catch-all. Such a match is equivalent to `(if g b b2)` with each arm's
/// binder let-bound to the scrutinee — the guard becomes an `if` CONDITION, a strict-first position the fold
/// serves. Returns the rewritten `(if …)` (wrapped in a `let` if the first arm binds a name), or `None` when
/// the body is not this exact shape (leaving the honest guard-perform decline for the general case: multiple
/// guarded arms sequence state per arm-test, which this narrow rewrite does not model). Walks structurally
/// so the match may be nested in the body. Does NOT descend into an inner `handle` (its guards are that
/// handle's concern).
pub(crate) fn desugar_performing_guard_match(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    if matches!(resolved_of(db, node), Resolved::Handle { .. }) {
        return None;
    }
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node)
        && arms.len() == 2
    {
        let (pat0, body0) = arms[0];
        let (pat1, body1) = arms[1];
        // First arm must be `(guard <irrefutable> <cond>)` with a PERFORMING cond; second an irrefutable
        // catch-all with NO guard.
        if let Some(g) = db.ast.as_form(pat0, "guard").map(|t| t.to_vec())
            && g.len() == 2
            && is_irrefutable_pattern(db, g[0])
            && subtree_performs(db, g[1], ctx)
            && db.ast.as_form(pat1, "guard").is_none()
            && is_irrefutable_pattern(db, pat1)
        {
            let cond = g[1];
            // REJECT-DON'T-MISCOMPILE (v-effects finding #9): this desugar binds the scrutinee to each named
            // arm binder via `copy_pure(scrutinee)` (below) — a fresh COPY per binder. When the scrutinee
            // itself PERFORMS and there are ≥2 named binders (arm-0's guard binder AND a named arm-1 fallback
            // binder, e.g. `((guard x cond) x)` + `(_o _o)`), each copy RE-EVALUATES the performing scrutinee:
            // `(match (St.next) ((guard x (> x (St.next))) …) (_o _o))` emits `(let ((x (St.next)) (_o
            // (St.next))) …)` → the fallback `_o` reads a RE-DRAWN scrutinee (breaker f1: 6 not 3; f2
            // dispatch-witness confirms the third hidden draw). Correctness is INPUT-DEPENDENT — the SAME
            // program is correct when the guard HITS (c2: -400) and wrong when it MISSES (f1) — so the whole
            // shape must reject. The correct fix (bind the performing scrutinee ONCE to a temp, then ALIAS
            // both binders to it — `(let ((k S)) (let ((x k) (_o k)) …))`) is the fresh-context bind-once /
            // let-threading arc (shared root with findings #8/lb/sh). A SINGLE named binder is sound (one copy
            // = one eval — c1's pure-guard shape never enters here, and a lone-binder performing scrutinee
            // folds correctly), so gate ONLY on ≥2 named binders over a performing scrutinee. Names the
            // `let`-lift workaround (c3: 3 — bind the draw once outside the match).
            let named_binder_count = [g[0], pat1]
                .iter()
                .filter(|&&p| db.ast.as_name(p).is_some_and(|n| n != "_"))
                .count();
            if named_binder_count >= 2 && subtree_performs(db, scrutinee, ctx) {
                // DECLINE (return None) rather than rewrite: the caller's `None` arm sees
                // `body_has_performing_match_guard` = true (this same performing-guard shape) and returns
                // `None` for the whole fold — the honest HANDLER_NOT_REDUCIBLE "not yet reducible" todo
                // decline, not the re-evaluation miscompile the copy-per-binder rewrite would produce. Flips to
                // a fold when the bind-once arc materializes the performing scrutinee once and aliases both
                // binders to it.
                return None;
            }
            let if_head = db.push_name("if");
            let if_node = db.push_list(vec![if_head, cond, body0, body1]);
            // Wrap in `let` bindings for any named (non-`_`) inner patterns, so the guard/bodies resolve them
            // to the scrutinee. A wildcard `_` binds nothing.
            let mut binders: Vec<StructId> = Vec::new();
            for &p in &[g[0], pat1] {
                if let Some(name) = db.ast.as_name(p).map(|s| s.to_string())
                    && name != "_"
                {
                    let name_atom = db.push_name(&name);
                    let scrut_copy = copy_pure(db, scrutinee);
                    binders.push(db.push_list(vec![name_atom, scrut_copy]));
                }
            }
            if binders.is_empty() {
                return Some(if_node);
            }
            let let_head = db.push_name("let");
            let bindings = db.push_list(binders);
            return Some(db.push_list(vec![let_head, bindings, if_node]));
        }
        // REFUTABLE first-arm pattern (a destructuring `(bin …)`/`(tuple …)`/ctor pattern) guarded by a
        // PERFORMING cond, with an irrefutable catch-all second arm. The irrefutable rewrite above (→ `(if g
        // b b2)`) is UNSOUND here: it drops the pattern-match, so a scrutinee that FAILS `P` would still run
        // the guard `g` and pick `b`/`b2` on `g` alone. The sound rewrite KEEPS the pattern match and hoists
        // the performing guard into an `if` INSIDE the matched arm: `(match k ((guard P g) b) (_ b2))` ≡
        // `(match k (P (if g b b2)) (_ b2))`. When `P` matches, `g` runs and selects `b` (guard holds) or
        // `b2` (guard fails); when `P` does not match, the catch-all yields `b2` WITHOUT running `g` — exactly
        // the guarded-match semantics. `g` is now an if-condition WITHIN a match arm, a strict-first position
        // the arm-body fold routes through the enclosing handle (the match-scrutinee/arm fold already
        // descends arm bodies). The catch-all `b2` is copied into the matched arm's else so a failed guard
        // falls through to the same value. Gated identically: second arm irrefutable + no guard, first-arm
        // guard cond performs. (breaker bg-family, refutable-pattern face.)
        if let Some(g) = db.ast.as_form(pat0, "guard").map(|t| t.to_vec())
            && g.len() == 2
            && !is_irrefutable_pattern(db, g[0])
            && subtree_performs(db, g[1], ctx)
            && db.ast.as_form(pat1, "guard").is_none()
            && is_irrefutable_pattern(db, pat1)
        {
            let inner_pat = g[0];
            let cond = g[1];
            // `(if g b0 <copy of b2>)` — the matched arm's body: guard holds → b0, else the catch-all value.
            let if_head = db.push_name("if");
            let b2_for_else = copy_pure(db, body1);
            let if_node = db.push_list(vec![if_head, cond, body0, b2_for_else]);
            // Rebuild the match: first arm keeps the REFUTABLE pattern with the guard-hoisted `if` body; the
            // catch-all is unchanged. `(match scrut (inner_pat if_node) (pat1 body1))`.
            let match_head = db.push_name("match");
            let scrut_copy = copy_pure(db, scrutinee);
            let arm0 = db.push_list(vec![inner_pat, if_node]);
            let arm1 = db.push_list(vec![pat1, body1]);
            return Some(db.push_list(vec![match_head, scrut_copy, arm0, arm1]));
        }
    }
    // Recurse structurally: the match may be nested inside the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut changed = false;
            let mut rebuilt = Vec::with_capacity(children.len());
            for c in children {
                match desugar_performing_guard_match(db, c, ctx) {
                    Some(r) => {
                        rebuilt.push(r);
                        changed = true;
                    }
                    None => rebuilt.push(c),
                }
            }
            if changed {
                Some(db.push_list(rebuilt))
            } else {
                None
            }
        }
        Struct::Atom(_) => None,
    }
}

/// β-reduce every APPLIED-LAMBDA REDEX in `node` whose callee reaches a discharged operation AND whose
/// arguments are all STRONGLY PURE — an `((fn (x) …(E.op)…) 100)` or a `let`-bound `(f 100)` where `f`'s
/// body performs — into its substituted body, recursively. Runs in `reduce_handle` BEFORE the pure-one-hole
/// classifier so a body that wraps its single perform in a lambda application reaches the fold in reduced
/// form: `((fn (x) (+ x (Amb.flip))) 100)` becomes `(+ 100 (Amb.flip))`, a single perform in a pure one-hole
/// context the E5 fold serves — folding under a MULTI-shot arm too (the `thread`/one-shot path already
/// inlines such a call via its `call_reaches_discharged_effect` arm, but the pure-one-hole/multi-shot path
/// does not). SOUND: this is a β-reduction with PURE arguments — substituting a pure argument (even into a
/// param used MANY times) duplicates no effect. A PERFORMING argument is NOT pre-reduced here (the
/// strongly-pure guard fails): β-substituting it into a multiply-used param would duplicate its effect
/// (`(mixed (Amb.flip))`, `mixed x = x + x + …` → `flip` per `x`, a miscompile), so it is left for the
/// `thread` path, which threads the argument's state exactly once before inlining. A RECURSIVE callee is
/// EXCLUDED (`call_reaches_discharged_effect` returns false — it is specialized, not inlined) so the
/// reduction terminates. A non-redex node is descended structurally. Bounded by `reduce_handle`'s re-entry
/// guard (each reduced body is re-walked once).
/// Reduce a handler-arm body that DEFERS its resume inside a closure STORED IN A COMPOUND and applied
/// through a helper — the DES multi-task pqueue's store→pop→apply shape: `(sleep (wake) s (unbox-apply
/// (Box.Box (fn (_u) (resume unit wake)))))` where `unbox-apply(b) = match b ((Box.Box th) (th unit))`.
/// The `resume` is buried behind the `Box.Box` constructor + `unbox-apply`'s match, so the fold's
/// classifiers see no TAIL resume and the arm declines. Compose reductions until it surfaces:
///  (a) β-reduce a one-shot non-recursive helper call [`apply_lambda`]; ONE-SHOT guard `count_param_refs ≤ 1`
///      so a resume-bearing closure argument is not DUPLICATED (v-discrete-event-sim CONFIRMED every stored
///      continuation applies EXACTLY ONCE — pop removes before apply — so this guard IS the DES contract);
///  (b) case-of-known-constructor fold a `match` over a visible ctor [`eval::fold_ctor_match`, v-inference's
///      SumPayload-aware substitution — the `(Ctor v)` binder's uses resolve to `SumPayload{scrutinee}` not
///      `Ref`, so a targeted resolve-time rewrite, not `beta_reduce`];
/// repeating until a tail `resume` surfaces or no progress. Unchanged body ⇒ returned as-is (byte-identical),
/// so tail/abortive/already-foldable arms are untouched. Bounded iteration. Conservative: any shape it
/// cannot cleanly reduce is left as-is (the arm then declines exactly as before — never a mis-fold).
/// A ONE-SHOT non-recursive helper call reducible by the deferred-resume fold: `head` is a non-recursive
/// lambda/def, arity matches, EACH param is used at most once in the body (so β-reduction never DUPLICATES a
/// deferred resume-closure — the one-shot continuation contract), and every arg is either strongly pure or a
/// lambda (a deferred resume-closure). Returns the β-reduced body (NOT yet re-resolved — the caller
/// `forget_subtree`s it), or `None` if `node` is not such a call. Shared by the deferred-resume fold's arm
/// (a) (the call is the whole term) and arm (a′) (the call is a match SCRUTINEE) so the two gates cannot
/// drift.
pub(crate) fn reduce_one_shot_helper_call(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && !crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        && params
            .iter()
            .all(|&p| count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1)
        && args.iter().all(|&a| {
            strongly_pure(db, a, ctx) || matches!(resolved_of(db, a), Resolved::Lambda { .. })
        })
    {
        return crate::eval::apply_lambda(db, head, &args).ok().flatten();
    }
    None
}

/// ESCAPED-CLOSURE-LEAK RECOVERY (cx5d). When `reduce_handle`'s output LEAKS a discharged perform — a
/// closure performing an arm op, passed to a NON-recursive ONE-SHOT helper that APPLIES it OUTSIDE the fold's
/// reach (the closure lifts standalone, its perform un-homed → `reduced_body_leaks_escaped_perform` fires) —
/// β-INLINE each such helper call in the ORIGINAL handle `body`, so the closure is applied INLINE, within
/// the fold's reach. A subsequent re-run of `reduce_handle` then threads the now-in-reach perform. Returns
/// the rewritten body if it inlined ≥ 1 such call (already re-resolved), else `None`.
///
/// CTX-FREE (matches the discharged ops directly from `arms`). SAFETY is the caller's: it re-runs
/// `reduce_handle` and RE-CHECKS the leak, using the result only if it now folds cleanly — so this can only
/// turn a decline into a fold, NEVER change a folding case (every case that folds today reaches no leak, so
/// the caller never invokes this) nor a value (one-shot: each helper param is used `<= 1` time, so the
/// inlined perform runs exactly once). A recursive / multi-use helper, or a non-lambda-non-simple arg, is
/// left alone (the honest decline stands, as before).
pub fn inline_escaped_one_shot_perform_call(
    db: &mut Db,
    body: StructId,
    arms: &[HandleArm],
) -> Option<StructId> {
    let arm_ops: Vec<(u32, u32)> = arms
        .iter()
        .filter_map(|a| crate::eval::effect_op_of(db, a.op).map(|(d, i)| (d.0, i)))
        .collect();
    if arm_ops.is_empty() {
        return None;
    }
    let (rewritten, changed) = inline_escaped_worker(db, body, &arm_ops);
    if changed {
        // `apply_lambda` copies the callee body (a copied binder ref keeps a stale memoized resolution) and
        // the `push_list` rebuild produces fresh unresolved parents — forget the subtree so the re-run
        // `reduce_handle`'s `resolved_of` recomputes every ref against the inlined structure (the same
        // re-resolve hygiene `reduce_arm_deferred_resume` applies per inlined call). KEEP-PINNED: a capture
        // the escaping closure carries (a free var `pin_free_vars` resolved + `apply_lambda` SHARED into the
        // reduced body — e.g. an outer `let ((a 7))` the closure reads) is a shared node with a STALE parent
        // (the dead original closure), so forgetting + force-structural-re-resolving it walks that dead chain
        // into an orphan → a spurious CDZ0101 (breaker sk4c). Its pinned memo is already correct, so preserve
        // it; only the freshly re-parented (non-pinned) refs need recompute.
        crate::resolve::forget_subtree_keep_pinned(db, rewritten);
        // A LOAD-TIME reference re-parented by the rebuild (e.g. a `let`-body ref when the inlined helper
        // call sat in the binding-init) keeps its stale load-time `scope_skip` entry, so the fast-path would
        // resolve it against its ORIGINAL scope (the pre-rebuild init) rather than the rebuilt one — leaving
        // a later hygiene rename to miss it and the ref dangling (a false CDZ0101, breaker iso-b). Force the
        // subtree's load-time nodes onto the exhaustive walk so every ref resolves against the CURRENT
        // (rebuilt) parent chain. `forget_subtree` alone is insufficient — it clears the resolution memo but
        // not the load-time skip coverage the recompute then re-consults.
        db.force_structural_resolution_subtree(rewritten);
        Some(rewritten)
    } else {
        None
    }
}

/// Recursive worker for [`inline_escaped_one_shot_perform_call`]: rewrite `node`, β-inlining an eligible
/// one-shot helper call that passes an escaping performing closure. Returns `(rewritten, changed)`.
pub(crate) fn inline_escaped_worker(
    db: &mut Db,
    node: StructId,
    arm_ops: &[(u32, u32)],
) -> (StructId, bool) {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && crate::eval::effect_op_of(db, head).is_none() // the head is not itself a perform
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && !crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        // DUPLICATION-SAFETY. Originally a strict one-shot bound (every param referenced `<= 1`), which
        // rejected a closure APPLIED MULTIPLE TIMES through the helper (`apply-twice(g) = (+ (g 1) (g 2))`,
        // pclos) — yet that duplication is exactly correct: each `(g i)` is an independent application, so
        // inlining to `(+ (Src.read 1) (Src.read 2))` reproduces the two per-application performs + state
        // advances the un-inlined program has. Relax it: a param may be referenced more than once when its
        // argument is safe to duplicate at each reference — a LAMBDA closure (each reference is an
        // application) or a SIMPLE-PURE name/atom (no effect to duplicate). The following clause already
        // constrains EVERY arg to exactly those two shapes, and the recovery's post-fold re-check (no
        // residual leak + non-poison core, `lower.rs`) is the correctness net that rejects any misfold.
        && params.iter().zip(args.iter()).all(|(&p, &a)| {
            count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1
                || arg_is_lambda_valued(db, a)
                || arg_is_simple_pure(db, a)
        })
        && args
            .iter()
            .all(|&a| arg_is_lambda_valued(db, a) || arg_is_simple_pure(db, a))
        && args.iter().any(|&a| lambda_body_reaches_op(db, a, arm_ops))
        && let Ok(Some(reduced)) = crate::eval::apply_lambda(db, head, &args)
    {
        // The inlined body may expose another such call (finite AST bounds the recursion).
        let (r, _) = inline_escaped_worker(db, reduced, arm_ops);
        return (r, true);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let mut new_children = Vec::with_capacity(children.len());
            let mut any = false;
            for c in children {
                let (rc, ch) = inline_escaped_worker(db, c, arm_ops);
                any |= ch;
                new_children.push(rc);
            }
            if any {
                (db.push_list(new_children), true)
            } else {
                (node, false)
            }
        }
        Struct::Atom(_) => (node, false),
    }
}

/// A syntactically-simple PURE argument: a bare name or an atom leaf (no perform to preserve/reorder). Used
/// by the escaped-closure recovery's one-shot gate for the NON-lambda args (the lambda arg is the closure).
pub(crate) fn arg_is_simple_pure(db: &Db, node: StructId) -> bool {
    db.ast.as_name(node).is_some() || matches!(db.ast.get(node), Struct::Atom(_))
}

/// Whether `node` is a LAMBDA VALUE — a `(fn …)`, SEEN THROUGH a type ANNOTATION `(: (fn …) (-> A B))`.
/// A one-shot helper's β-reduction (hop 1) carries the parameter's declared type onto the substituted
/// argument (`substituted_arg` → `(: closure (-> …))`), so at the NEXT hop the same closure arrives as an
/// `Annot`, not a bare `Lambda` — a raw `matches!(resolved_of, Lambda)` misses it and the multi-hop inline
/// stalls after one hop (cx6). `lambda_of` (which the reach/apply gates use) already sees through `Annot`;
/// this mirrors that so the arg-eligibility gate agrees, letting the recovery chain through every hop. Only
/// an annotated/bare lambda passes — an `Annot` wrapping a non-lambda does not (its `expr` recurses to
/// `false`), keeping the one-shot gate as tight as before for non-closure args.
pub(crate) fn arg_is_lambda_valued(db: &mut Db, node: StructId) -> bool {
    match resolved_of(db, node) {
        Resolved::Lambda { .. } => true,
        Resolved::Annot { expr, .. } => arg_is_lambda_valued(db, expr),
        _ => false,
    }
}

/// Whether `node` is a lambda whose BODY reaches a perform of one of `arm_ops` (a discharged op this handle
/// routes) — the escaping closure whose perform the recovery re-homes by inlining its applying helper.
pub(crate) fn lambda_body_reaches_op(db: &mut Db, node: StructId, arm_ops: &[(u32, u32)]) -> bool {
    let Some((_params, hbody)) = crate::eval::lambda_params_and_body(db, node) else {
        return false;
    };
    subtree_reaches_arm_op(db, hbody, arm_ops)
}

pub(crate) fn subtree_reaches_arm_op(db: &mut Db, node: StructId, arm_ops: &[(u32, u32)]) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some((d, i)) = crate::eval::effect_op_of(db, head)
        && arm_ops.contains(&(d.0, i))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_reaches_arm_op(db, c, arm_ops)),
        Struct::Atom(_) => false,
    }
}

/// Rebuild a `(match SCRUTINEE arm…)` node with a NEW scrutinee, preserving the arms. The match AST is
/// `(match <scrutinee> <arm>…)` — child 0 is the `match` head, child 1 is the scrutinee, the rest are arms
/// — so this replaces child 1. `arms` is passed for a shape assertion only (the arm children are copied from
/// the original node verbatim). Falls back to the original node if it is not a well-formed match list.
pub(crate) fn rebuild_match_scrutinee(
    db: &mut Db,
    node: StructId,
    _arms: &[(StructId, StructId)],
    new_scrutinee: StructId,
) -> StructId {
    if let Struct::List(children) = db.ast.get(node).clone()
        && children.len() >= 2
    {
        let mut new_children = children;
        new_children[1] = new_scrutinee;
        return db.push_list(new_children);
    }
    node
}

pub(crate) fn reduce_arm_deferred_resume(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> StructId {
    let mut cur = node;
    for _ in 0..32 {
        if tail_resume(db, cur).is_some() {
            break; // exposed — done
        }
        // (b) a match over a known ctor → its arm body with SumPayload-binder refs substituted.
        if let Some(folded) = crate::eval::fold_ctor_match(db, cur) {
            cur = folded;
            continue;
        }
        // (a) a one-shot non-recursive helper call whose args are pure or deferred resume-closures.
        if let Some(reduced) = reduce_one_shot_helper_call(db, cur, ctx) {
            // RE-RESOLVE the β-reduced body: `apply_lambda` COPIES the callee's arm bodies, and a copied
            // `(Ctor v)` binder ref keeps its memoized `SumPayload { scrutinee }` pointing at the callee's
            // ORIGINAL match scrutinee — so a following `fold_ctor_match` (whose gate is `s == scrutinee`,
            // node-identity) would skip the binder, leaving it unsubstituted (the stale-scrutinee skip
            // v-inference diagnosed for the recursion-unfold — the SAME hazard applies here when a helper
            // whose body matches its arg is inlined, e.g. `sched-step` popping the pqueue). Forgetting the
            // memoized resolution makes `resolved_of` recompute each binder ref against the copy's own
            // scrutinee occurrence — the `fuse_match_into_if` clone+re-resolve hygiene.
            crate::resolve::forget_subtree(db, reduced);
            cur = reduced;
            continue;
        }
        // (a′) the one-shot helper call sits in the SCRUTINEE of a `(match (helper …) arms…)` — the pop of a
        //      DIRECTLY-built entry constructed by a helper: `(sched-step (mk1 wake kb))` first reduces the
        //      outer `sched-step` (arm a) to `(match (mk1 wake kb) …)`, whose scrutinee is still the
        //      UNREDUCED `(mk1 …)` call, so `fold_ctor_match` (which needs a VISIBLE ctor) can't fire and the
        //      arm binders never surface (they resolve Poison). Reduce that nested scrutinee call in place —
        //      same one-shot predicate + re-resolve hygiene as arm (a) — turning the scrutinee into the
        //      helper's `(PQCons …)` body so the next iteration's `fold_ctor_match` pops it. Distinct from
        //      arm (c) (a RECURSIVE callee in the scrutinee, one-level-unfolded); this is the non-recursive
        //      direct-helper analogue, the shape v-inference isolated (a NON-recursive `mk1` reproduces the
        //      same decline the recursive `pins` does — the blocker is a nested-scrutinee call not reduced
        //      before the pop, NOT the recursion).
        if let Resolved::Match { scrutinee, arms } = resolved_of(db, cur) {
            // The scrutinee may carry a leading annotation `(: (helper …) T)` (the handler-arm value is
            // annotated at its declared type) — peel it to reach the call before testing/reducing.
            let call = match resolved_of(db, scrutinee) {
                Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => expr,
                _ => scrutinee,
            };
            if let Some(reduced_scrut) = reduce_one_shot_helper_call(db, call, ctx) {
                crate::resolve::forget_subtree(db, reduced_scrut);
                cur = rebuild_match_scrutinee(db, cur, &arms, reduced_scrut);
                continue;
            }
        }
        // (c) a RECURSIVE helper call whose concrete argument selects a NON-recursive (base) arm — the
        //     time-ordered pqueue's `pins` (sorted-insert): `(pins PQNil t kb)` matches `q` = the visible
        //     `PQNil` ctor, takes the base arm `(PQCons (tuple t kb PQNil))`, and never recurses. Such a call
        //     is typically NESTED in an argument (`(sched-step (pins PQNil …))`) — the pop applies to the
        //     insert's result — so rewrite the FIRST such nested call in place: unfold `pins` ONE level (the
        //     recursion-permitting variant, since `apply_lambda` refuses a recursive callee up front), let
        //     `fold_ctor_match` resolve its internal `(match q …)` on the now-visible ctor, and splice the
        //     base-arm result back. Turning the `pins` arg into a directly-constructed `PQCons` lets the
        //     existing arm (a) reduce the surrounding `sched-step` on the next iteration. ACCEPT the unfold
        //     only if the folded body has NO residual self-call to the callee (base arm taken); a non-base
        //     concrete arg (recursion actually needed) leaves the self-call and is DISCARDED — a clean
        //     decline. "Peek one level, accept only if base-arm" is the loop's recursion budget;
        //     `fold_ctor_match` stays a pure one-step fold (v-inference's co-owned split).
        if let Some(rewritten) = rewrite_recursive_base_arm_call(db, cur, ctx) {
            cur = rewritten;
            continue;
        }
        break; // no further reduction applies
    }
    cur
}

/// Find the FIRST call to a recursive helper whose concrete argument selects a NON-recursive base arm and
/// rewrite it to that arm's (folded) body in place, rebuilding the surrounding structure. Returns `Some`
/// with the rewritten `node` if such a call was found + folded, else `None` (no progress). The call is
/// usually nested in an argument — `(sched-step (pins PQNil t kb))` — so this walks structurally and
/// rewrites the innermost-first match. See the (c) block in [`reduce_arm_deferred_resume`] for the policy;
/// the accept gate is `!body_calls_def(folded, callee)` (base arm taken → no residual self-call).
pub(crate) fn rewrite_recursive_base_arm_call(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    // Try to rewrite THIS node if it is the recursive-base-arm call.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && let Some(callee) = callee_def_index_of(db, head)
        && let Some((params, hbody)) = crate::eval::lambda_params_and_body(db, head)
        && crate::eval::is_recursive(db, hbody)
        && params.len() == args.len()
        // ONE-SHOT guard, scoped to the params whose ARGUMENT is a resume-closure (a lambda). Such an arg
        // must not be DUPLICATED (a deferred continuation applies exactly once — pop removes before apply),
        // so its param may appear at most once in the callee body. A PURE arg (`PQNil`, `wake`) is freely
        // duplicable — a sorted-insert references its queue/element params multiply while rebuilding the
        // order, harmless for pure values; enforcing ≤1 on those would reject every real recursive insert.
        && params.iter().zip(args.iter()).all(|(&p, &a)| {
            if matches!(resolved_of(db, a), Resolved::Lambda { .. }) {
                count_param_refs(db, hbody, crate::eval::param_name_occ(db, p)) <= 1
            } else {
                true
            }
        })
        && args.iter().all(|&a| {
            strongly_pure(db, a, ctx) || matches!(resolved_of(db, a), Resolved::Lambda { .. })
        })
        && let Some(unfolded) = crate::eval::apply_lambda_one_level_recursive(db, head, &args)
            .ok()
            .flatten()
        // RE-RESOLVE the unfolded node before folding. `apply_lambda`'s β-reduce COPIES the callee's arm
        // bodies, but a copied `(Ctor v)` binder ref keeps its memoized `SumPayload { scrutinee }` pointing
        // at the callee's ORIGINAL match-scrutinee occurrence — not the fresh one in the unfolded copy. So
        // `fold_ctor_match`/`rewrite_sum_payload` (whose substitution gate is `s == scrutinee`, node-identity)
        // would silently skip those refs, leaving the binder (`kb`) unsubstituted and the inner `(match kb …)`
        // pointing at a stale scrutinee → the continuation never surfaces. Forgetting the memoized resolution
        // (both the `resolved` column and the `resolved_subtrees` walk-guard) makes `resolved_of` recompute
        // each binder ref against the copy's OWN scrutinee occurrence — the same clone+re-resolve hygiene
        // `fuse_match_into_if` (lower.rs) applies when it clones match arms into `if` branches.
        && {
            crate::resolve::forget_subtree(db, unfolded);
            true
        }
        // Unfolding a callee whose param carries a resume-closure LET-BINDS that arg once (eval-once), so
        // the unfolded body is `(let ((kb …)) (match q …))` — fold the match THROUGH the leading `let`
        // wrappers (kept around the folded arm, preserving the one-shot binding).
        && let Some(folded) = fold_ctor_match_through_lets(db, unfolded)
        // ACCEPT only if the base arm was taken — no residual self-call to the callee. A non-base concrete
        // arg (recursion actually needed) leaves the self-call in the folded body → DISCARD (clean decline).
        && !body_calls_def(db, folded, callee)
    {
        return Some(folded);
    }
    // Otherwise recurse into children, rebuilding the first branch that rewrites.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (i, &c) in children.iter().enumerate() {
            if let Some(rc) = rewrite_recursive_base_arm_call(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[i] = rc;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

pub(crate) fn reduce_applied_lambdas(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    // An application whose head is a (non-recursive) lambda/ref-to-lambda reaching the discharged effect,
    // AND whose arguments are all strongly pure (the soundness guard — see the doc comment): β-reduce it
    // (substitute args for params), then re-walk the reduced body for further nested redexes.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && args.iter().all(|&a| strongly_pure(db, a, ctx))
        && call_reaches_discharged_effect(db, head, ctx)
    {
        // Reduce each argument's own redexes first (a pure arg won't itself be a performing redex, but keep
        // the recursion uniform), then β-reduce the call. A parameterized callee substitutes; a nullary def
        // has no lambda wrapper (its name resolves straight to its body), so fall back to that body.
        let rargs: Vec<StructId> = args
            .iter()
            .map(|&a| reduce_applied_lambdas(db, a, ctx))
            .collect();
        let reduced = match crate::eval::apply_lambda(db, head, &rargs).ok().flatten() {
            Some(r) => r,
            None => match crate::eval::lambda_body_of_nullary(db, head) {
                Some(b) => b,
                None => return node, // not actually reducible — leave it
            },
        };
        return reduce_applied_lambdas(db, reduced, ctx);
    }
    // Otherwise descend structurally, reducing any redex in a child.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let rebuilt: Vec<StructId> = children
                .iter()
                .map(|&c| reduce_applied_lambdas(db, c, ctx))
                .collect();
            db.push_list(rebuilt)
        }
        Struct::Atom(_) => node,
    }
}

/// Whether `node` IS or CONTAINS an applied-lambda redex the pre-reduction will reduce — a call whose
/// callee reaches a discharged operation and whose arguments are all strongly pure. The gate for the
/// applied-lambda pre-reduction, so a body with no such redex skips the pass (the common case). Mirrors
/// `reduce_applied_lambdas`'s own guard (incl. the strongly-pure-args soundness condition) so the gate and
/// the reducer agree on exactly which redexes fire.
pub(crate) fn body_contains_applied_performing_lambda(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && args.iter().all(|&a| strongly_pure(db, a, ctx))
        && call_reaches_discharged_effect(db, head, ctx)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_contains_applied_performing_lambda(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Graft the synthesized `folded` body into the lexical slot the (post-hoist) handle `body` occupies, so a
/// free name inside `folded` — an enclosing function parameter or a match-arm binder the handle body
/// references — resolves up the SAME scope chain the original `body` did. `body`'s parent is the handle
/// node (or, in a distributed branch, the `if`/arm above it), which itself ascends to the enclosing
/// `def`/`fn`/`match`; parenting `folded` at `body`'s parent + child index reproduces exactly that chain.
/// This lets the `type_errors` guard in `reduce_handle` type the folded term with its free names bound
/// (else an outer-param body over-declines with a spurious unbound-name fault). A no-op if `body` is the
/// arena root (no parent) — a top-level handle body has no enclosing scope to reach anyway.
pub(crate) fn reparent_under_handle_site(db: &mut Db, folded: StructId, body: StructId) {
    let Some(parent) = db.parent_of(body) else {
        return; // a top-level handle body — no enclosing scope to reach
    };
    // When the handle body being reduced is the SECOND child of a 2-element `(x body)` pair — a `match`
    // ARM `(pattern body)` (the distribution case) or a `let` binding `(name init)` — the binder scope
    // check (`resolve::match_arm_binds`) reads the pair's RECORDED body child (`pb[1]`) and demands the
    // reference ascend from THAT node. Parenting `folded` directly under the pair leaves `pb[1]` as the
    // ORIGINAL body, so the ascended-from child (`folded`) would not match and a binder-referencing arm
    // body would over-decline. Rebuild a fresh `(x folded)` pair in the original pair's slot so `folded`
    // IS the recorded body child — then the binder resolves against the reduced body. (A non-pair parent —
    // the `handle` node itself, or an `if` node in a distributed branch — is reached by ascending THROUGH
    // it to the enclosing binder form, so it needs no rebuild: parent `folded` directly.)
    if let Struct::List(children) = db.ast.get(parent).clone()
        && children.len() == 2
        && children[1] == body
        && let Some(grandparent) = db.parent_of(parent)
    {
        let fresh_pair = db.push_list(vec![children[0], folded]);
        db.reparent(fresh_pair, Some(grandparent), db.child_ix_of(parent) as u32);
        return;
    }
    db.reparent(folded, Some(parent), db.child_ix_of(body) as u32);
}

/// HANDLER DISTRIBUTION over a pure-conditioned tail conditional (a commuting conversion). If `body` is an
/// `(if c t e)` or a `(match scrut arms…)` whose CONDITION/SCRUTINEE is strongly pure (runs once, advances
/// no state, so it need not thread through the handler) but whose fold otherwise declines because a
/// BRANCH / ARM BODY performs a discharged op, distribute the handler into each branch:
///   `(handle E s arms (if c t e))`     ≡ `(if c (handle E s arms t) (handle E s arms e))`
///   `(handle E s arms (match k (p b)…))` ≡ `(match k (p (handle E s arms b))…)`
/// Each branch / arm body is re-`reduce_handle`d with the SAME init/arms (only one runs at runtime, seeing
/// the seed state — the condition/scrutinee advanced nothing). Returns the rebuilt conditional, or `None`
/// if the body is neither shape, the condition/scrutinee is not strongly pure, or any branch's fold
/// declines (so the caller falls through to the ordinary decline — never a partial rewrite). A `match`
/// PATTERN is reused verbatim in the rebuilt arm, so its binder still scopes the (re-anchored) reduced arm
/// body — `reduce_handle`'s `reparent_under_handle_site` anchors the reduced body under the ORIGINAL arm
/// pair while it type-checks, so a binder-referencing arm body resolves; the final rebuild then re-parents
/// each reduced body under its new arm pair.
pub(crate) fn distribute_handler_over_conditional(
    db: &mut Db,
    init: StructId,
    arms: &[HandleArm],
    body: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    match resolved_of(db, body) {
        Resolved::If { cond, then_, else_ } => {
            // The condition must be strongly pure — it runs once, before either branch. A performing
            // condition is the `pure_hole` if-cond case (folds below) or a threading shape; distributing it
            // would risk moving a perform. Only distribute when a BRANCH performs (else the fold serves the
            // body directly).
            if !strongly_pure(db, cond, ctx) {
                return None;
            }
            if !subtree_performs(db, then_, ctx) && !subtree_performs(db, else_, ctx) {
                return None;
            }
            // Reduce each branch as its own handle body (init/arm occurrences are only READ + copied on
            // substitution, so sharing them across the branch reductions is safe). Either branch declining
            // makes the whole distribution decline — no partial rewrite.
            let then_r = reduce_handle(db, init, arms, then_, false)?;
            let else_r = reduce_handle(db, init, arms, else_, false)?;
            let if_head = db.push_name("if");
            Some(db.push_list(vec![if_head, cond, then_r, else_r]))
        }
        Resolved::Match {
            scrutinee,
            arms: match_arms,
        } => {
            // The SCRUTINEE must be strongly pure (evaluated once, before any arm). Only distribute when an
            // ARM BODY performs. A pattern is a binder position (no perform), so `subtree_performs` on the
            // arm bodies is the trigger.
            if !strongly_pure(db, scrutinee, ctx) {
                return None;
            }
            if !match_arms
                .iter()
                .any(|&(_, arm_body)| subtree_performs(db, arm_body, ctx))
            {
                return None;
            }
            // Rebuild `(match scrutinee (pat body')…)`: reduce each arm body under the same init/arms, reuse
            // each pattern verbatim (its binder scopes the reduced body — see `reparent_under_handle_site`).
            // Any arm's fold declining declines the whole distribution.
            let match_head = db.push_name("match");
            let mut children = vec![match_head, scrutinee];
            for &(pat, arm_body) in match_arms.iter() {
                let reduced = reduce_handle(db, init, arms, arm_body, false)?;
                children.push(db.push_list(vec![pat, reduced]));
            }
            Some(db.push_list(children))
        }
        _ => None,
    }
}

/// Whether applying `head` (a non-perform application head) reaches an abortive operation through the
/// callee's body FROM INSIDE A CONDITIONAL — the unsound cross-function case. Follows a NON-RECURSIVE
/// callee (the inline arm's target) and checks whether an abortive perform sits under an `if`/`match`/
/// short-circuit within it. Such an abort surfaces, after the inline arm β-reduces the callee, as a
/// non-tail conditional the hoist never saw (it was opaque behind the call) — and `thread`'s `if` arm
/// would then capture it PER-BRANCH as if the `if` were the handle's tail, dropping the enclosing op
/// (`(+ 10 (check -1))` → 109, a MISCOMPILE). An UNCONDITIONAL cross-fn abort (the callee's body is a bare
/// abort, `(+ 10 (boom 99))`) is NOT flagged — inlining yields a plain strict abort the E4-a machinery
/// collapses soundly. A recursive callee is not followed (not inlined). Depth-bounded.
pub(crate) fn call_reaches_conditional_abortive(
    db: &mut Db,
    head: StructId,
    ctx: &HandlerCtx,
) -> bool {
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    // NON-RECURSIVE callee: it is INLINED (`reduce_applied_lambdas`) and the RE-HOIST after inlining
    // (`hoist_conditional_abort`) lifts its conditional abort to a branch tail where the E4 machinery folds
    // it soundly — e.g. `(+ 10 (check -1))` with `check n = (if (< n 0) (Bail.bail 99) n)` folds to 99 (the
    // abort abandons the `+ 10`), NOT the 109 the pre-re-hoist code feared. Any post-inline shape the hoist
    // still cannot lift is caught by `body_has_unsound_abortive_perform` (the post-inline guard) → declines,
    // never miscompiles. So a non-recursive conditional-abort callee no longer needs blocking here. Only a
    // RECURSIVE callee (NOT inlined; its abort flows into the pending continuation — `(+ (go 2) 999999)` →
    // 1000506) still needs the br-out-of-handle non-local-exit and is flagged below.
    if !crate::eval::is_recursive(db, body) {
        return false;
    }
    // A RECURSIVE callee is not inlined but IS specialized (`specialize_recursive`), and that path returns
    // the callee's value as an ordinary return — so an abortive op inside a SELF-recursive callee, reached
    // through a NON-tail position in the handle body, has its abort value flow into the PENDING continuation
    // instead of abandoning it (`(+ (go 2) 999999)` where `go` bails → 500+999999 not 500 — adv-52, a silent
    // wrong value on all backends). The `specialize_recursive` abortive guard only refuses NON-tail
    // *recursion*; a TAIL-recursive callee like `go` passes it, so the unsoundness must be caught HERE: an
    // abortive op inside a recursive callee's body is opaque to the syntactic hoist exactly as a non-recursive
    // one is, so it is flagged the same way (denying the enclosing operand capturable-tail → `reduce_handle`
    // declines to the safe floor; a full fold needs the br-out-of-handle non-local-exit convention). We still
    // walk the body ONCE for a conditional abort (the self-call inside it is just another `Apply` node the
    // structural walk descends past — `subtree_has_conditional_abortive` never re-enters this fn, so a
    // recursive body is not a non-termination risk).
    subtree_has_conditional_abortive(db, body, ctx, false)
}

/// Whether `node`'s subtree contains an application whose head is an abortive RECURSIVE callee (the
/// `call_reaches_conditional_abortive` shape). Used to reject a sibling operand of a pending-in-handle-body op
/// that ALSO reaches such a callee (v1 folds only the single-recursive-operand shape). Walks `Apply` heads +
/// args structurally; a non-`Apply` leaf (literal/param) is never a match.
fn reaches_abortive_recursive_call(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, args } = resolved_of(db, node) {
        if is_perform(db, head, ctx).is_none() && call_reaches_conditional_abortive(db, head, ctx) {
            return true;
        }
        if reaches_abortive_recursive_call(db, head, ctx) {
            return true;
        }
        return args
            .iter()
            .any(|&a| reaches_abortive_recursive_call(db, a, ctx));
    }
    false
}

/// PENDING-IN-HANDLE-BODY fold (adv-52, non-local-exit CC). The handle body is a PURE strict op with exactly
/// ONE operand that is a direct call to an abortive (tail-)recursive callee — `(+ (go 2) 999999)` where `go`
/// bails at its base. An ordinary specialized return would let the pending op consume the abort value
/// (500 + 999999, a silent miscompile); the abort must instead ABANDON the pending op so the handle's value is
/// the abort/arm value (500 → +7 outside → 507). Force the callee into the tagged-abort CC (it returns
/// `#tuple(tag value)`) and short-circuit the pending op on the abort tag:
///   `(+ (go 2) K)` → `(let ((r (go#eff 2 seed))) (if (= (. r 0) 1) (. r 1) (+ (. r 1) K)))`.
/// Returns `None` (decline → the safe floor stands) for any shape v1 does not model: a performing/impure or
/// abortive-reaching sibling, >1 recursive operand, a non-pure op head, recursive-call args that perform, or a
/// callee the tagged path refuses to specialize (e.g. a state-reading abort arm).
fn try_pending_in_handle_body(
    db: &mut Db,
    body: StructId,
    ctx: &HandlerCtx,
    seed: StructId,
) -> Option<StructId> {
    let Resolved::Apply { head, args } = resolved_of(db, body) else {
        return None;
    };
    // The op head must be PURE — a primitive/operator, not a perform, not itself an abortive recursive call.
    if is_perform(db, head, ctx).is_some() || call_reaches_conditional_abortive(db, head, ctx) {
        return None;
    }
    // Exactly ONE operand is a DIRECT call to an abortive recursive callee; every other operand pure + free of
    // any abortive-recursive call.
    let mut rec_pos: Option<usize> = None;
    for (i, &a) in args.iter().enumerate() {
        let is_abortive_rec = matches!(resolved_of(db, a), Resolved::Apply { head: ah, .. }
            if is_perform(db, ah, ctx).is_none() && call_reaches_conditional_abortive(db, ah, ctx));
        if is_abortive_rec {
            if rec_pos.is_some() {
                return None; // >1 abortive-recursive operand — v1 declines.
            }
            rec_pos = Some(i);
        } else if subtree_performs(db, a, ctx) || reaches_abortive_recursive_call(db, a, ctx) {
            return None; // an impure / abortive-reaching sibling — v1 declines.
        }
    }
    let rpos = rec_pos?;
    let Resolved::Apply {
        head: rec_head,
        args: rec_args,
    } = resolved_of(db, args[rpos])
    else {
        return None;
    };
    // The recursive call's args must be pure (v1 — no perform threads through them).
    if rec_args.iter().any(|&a| subtree_performs(db, a, ctx)) {
        return None;
    }
    let callee_def = callee_def_index_of(db, rec_head)?;
    // FORCE the callee into the tagged CC — it is TAIL-recursive, so the ordinary non-tail trigger won't fire.
    // `specialize_recursive`'s tagged gate ORs in `force_tagged_abort` (relaxing ONLY the all-tail requirement).
    db.force_tagged_abort.insert(callee_def);
    let (call, spec) = build_spec_call(db, rec_head, &rec_args, &[seed], ctx)?;
    // If the tagged spec did NOT materialize (the gate still rejected the shape — a mutual / caller-observed /
    // state-reading-arm callee), decline cleanly rather than emit a raw-tuple call the pending op would misread.
    if !db.tagged_abort_specs.contains(&spec) {
        return None;
    }
    // Build `(let ((r call)) (if (= (. r 0) 1) (. r 1) (OP a0… (. r 1) …)))`.
    let k = ctx.temp_ctr.get();
    ctx.temp_ctr.set(k + 1);
    let tname = format!("{spec}$pc{k}");
    // The pending context: the op with the recursive operand replaced by the NORMAL result value `(. r 1)`.
    let rhead = copy_pure(db, head);
    let mut op_children = vec![rhead];
    for (i, &a) in args.iter().enumerate() {
        if i == rpos {
            op_children.push(tuple_proj(db, &tname, 1));
        } else {
            op_children.push(copy_pure(db, a));
        }
    }
    let pending = db.push_list(op_children);
    // `(if (= (. r 0) 1) (. r 1) <pending>)` — on the abort tag return the abort/arm value bare (abandon the
    // pending op); else apply the pending op to the normal result.
    let tag_proj = tuple_proj(db, &tname, 0);
    let one = tagged_int_lit(db, 1);
    let eq_head = db.push_name("=");
    let cond = db.push_list(vec![eq_head, tag_proj, one]);
    let abort_val = tuple_proj(db, &tname, 1);
    let if_head = db.push_name("if");
    let if_node = db.push_list(vec![if_head, cond, abort_val, pending]);
    let let_head = db.push_name("let");
    let tn_atom = db.push_name(&tname);
    let pair = db.push_list(vec![tn_atom, call]);
    let bindings = db.push_list(vec![pair]);
    Some(db.push_list(vec![let_head, bindings, if_node]))
}

/// finding #11-B (oamin4/oa3): whether `init` is a call `(helper arg…)` whose callee ABORTS INSIDE A MATCH
/// ARM and one of whose ARGUMENTS reaches a FOREIGN perform — the def-boundary conditional-abort shape that
/// slips past the syntactic hoist and the ordinary cross-fn guard (which walks `if`/`and` conditionals but
/// not `match` arms). The `unwrap`-shape `(unwrap (E.fetch) tag)`: `unwrap` aborts in its `(None) => Bail.out
/// tag` arm (a CONDITIONAL abort — the `(Some v) => v` arm returns a value that flows into the continuation),
/// and the `E.fetch` argument performs an OUTER handler's op. Under such a call the abort must home to the
/// Bail boundary, but the fold captures it per-branch and threads the abort value into the continuation (a
/// silent wrong value: single call → 10223, chained pair → 2667423). GATED on the foreign argument so the
/// PURE-argument controls (oamin1/oamin5: `(unwrap (if …) tag)`) — where the abort DOES home correctly —
/// keep folding. Narrow by construction: it fires only for a call carrying a foreign perform into a
/// match-aborting helper, the exact def-boundary shape the non-local-exit convention will later fold.
pub(crate) fn init_is_foreign_arg_match_abort_call(
    db: &mut Db,
    init: StructId,
    ctx: &HandlerCtx,
) -> bool {
    let Resolved::Apply { head, args } = resolved_of(db, init) else {
        return false;
    };
    if is_perform(db, head, ctx).is_some() {
        return false; // a direct perform, not a helper call
    }
    let Some(body) = crate::eval::lambda_body(db, head)
        .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
    else {
        return false;
    };
    if crate::eval::is_recursive(db, body) {
        return false; // a recursive callee is specialized, not inlined — covered elsewhere
    }
    // The callee aborts inside a match arm (a conditional abort the `if`-only hoist/guard cannot see), AND an
    // ARGUMENT of the call DIRECTLY performs a foreign op (the outer `E.fetch` — the sound-capture blocker).
    // Test the ARGUMENTS narrowly (`next_state_directly_performs_foreign` = a literal `(Outer.op …)` not
    // followed through user calls), NOT `body_reaches_foreign_perform`, which follows the callee and would
    // OVER-report on the helper's own match/pattern heads (flagging the pure-arg controls oamin1/oamin5).
    if !callee_aborts_in_match_arm(db, body, ctx) {
        return false;
    }
    args.iter()
        .any(|&a| next_state_directly_performs_foreign(db, a, ctx))
}

/// Whether `node` contains an abortive perform sitting inside a `match` ARM BODY — the match-arm analogue of
/// the `if`-branch conditional abort `subtree_has_conditional_abortive` already detects. Kept SEPARATE from
/// that walk so the existing cross-fn guard sites (which correctly fold a pure-scrutinee match-arm abort per
/// branch) are unperturbed; only the finding #11-B let-init gate consults this.
pub(crate) fn callee_aborts_in_match_arm(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Match { arms, .. } = resolved_of(db, node)
        && arms
            .iter()
            .any(|&(_, arm_body)| subtree_has_abortive_perform(db, arm_body, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| callee_aborts_in_match_arm(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `node` contains an abortive perform reached from UNDER a conditional (`if`/`match`/short-circuit
/// connective) — `under_cond` tracks whether we have descended into a conditional position. A bare abort
/// not under any conditional is NOT reported (it is the sound unconditional-collapse case).
pub(crate) fn subtree_has_conditional_abortive(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    under_cond: bool,
) -> bool {
    if under_cond
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    // A conditional marks its branches / shielded operand `under_cond`.
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return subtree_has_conditional_abortive(db, cond, ctx, under_cond)
            || subtree_has_conditional_abortive(db, then_, ctx, true)
            || subtree_has_conditional_abortive(db, else_, ctx, true);
    }
    if let Resolved::And { lhs, rhs, .. } = resolved_of(db, node) {
        return subtree_has_conditional_abortive(db, lhs, ctx, under_cond)
            || subtree_has_conditional_abortive(db, rhs, ctx, true);
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_has_conditional_abortive(db, c, ctx, under_cond)),
        Struct::Atom(_) => false,
    }
}

/// Whether the subtree at `node` contains a perform of an ABORTIVE operation (one in `ctx.abortive`) —
/// the trigger for the non-tail hoist. A structural walk mirroring `subtree_performs`, narrowed to
/// abortive ops only (a tail-resumptive perform is threaded, not hoisted).
pub(crate) fn subtree_has_abortive_perform(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| subtree_has_abortive_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Whether `t` is an UNDETERMINED type (an `Any` or a free `Var`) — an abort-type comparison over an
/// undetermined side is inconclusive, so callers treat it as "allow" rather than a definite mismatch.
pub(crate) fn undetermined_ty(t: &crate::ty::Ty) -> bool {
    matches!(t, crate::ty::Ty::Any | crate::ty::Ty::Var(_))
}

/// The VALUE TYPE an abortive perform in `node` collapses to — the type of the abortive arm's BODY (which
/// becomes the abort value). Returns `Some(ty)` when `node` contains exactly one abortive op whose arm
/// body type is determinable; `None` if it finds none (or the arm/type is unavailable). Used by the hoist
/// to check that distributing an enclosing op keeps the produced `if` branches type-consistent.
pub(crate) fn abortive_perform_value_ty(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<crate::ty::Ty> {
    if let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        let arm_body = ctx.arms.get(&id)?.body;
        return Some(crate::infer::type_of(db, arm_body));
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .find_map(|&c| abortive_perform_value_ty(db, c, ctx)),
        Struct::Atom(_) => None,
    }
}

/// DEF-BOUNDARY NON-LOCAL-EXIT (finding #11-B, 14:12783). Fold the shape `(let ((a <helper-call>)) K)` where
/// `<helper-call>` is a non-recursive helper that ABORTS in a MATCH arm and carries a FOREIGN-performing
/// argument (`(let ((a (unwrap (E.fetch) 11))) (* a 2))`, `unwrap o tag = (match o ((Some v) v) ((None)
/// (Bail.out tag)))`). INLINE the helper and DISTRIBUTE the continuation `K` into the match's NON-ABORTIVE
/// arms only — `(Some v) → (let ((a v)) K)` — leaving the abortive arm's bare escape (`(None) → (Bail.out
/// tag)`) UNTOUCHED, so the abort HOMES to its handler boundary instead of the naive per-branch fold
/// threading the abort value into `K` (the 10223-vs-5113 miscompile the safe-floor guard prevents). The
/// resulting distributed match is the shape the reducer already folds correctly (proven: 43/5113). Returns
/// the rewritten match, or `None` if the shape does not match (the guard then declines, as before). Gated by
/// `init_is_foreign_arg_match_abort_call` — the exact def-boundary shape the safe-floor otherwise rejects.
pub(crate) fn hoist_match_abort_let(
    db: &mut Db,
    body: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    let form = db.ast.as_form(body, "let").map(|t| t.to_vec())?;
    if form.len() != 2 {
        return None;
    }
    let (bindings_occ, k) = (form[0], form[1]);
    let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
        return None;
    };
    // A SINGLE binding `(a INIT)` — the def-boundary let-init shape. (Multi-binding lets are left to the
    // guard; the continuation-distribution below assumes one binder threading into K.)
    if pairs.len() != 1 {
        return None;
    }
    let Struct::List(kv) = db.ast.get(pairs[0]).clone() else {
        return None;
    };
    if kv.len() != 2 {
        return None;
    }
    let (a_binder, init) = (kv[0], kv[1]);
    if !init_is_foreign_arg_match_abort_call(db, init, ctx) {
        return None;
    }
    // INLINE the helper call (β-reduce its body with the call args substituted) → its `(match …)` body.
    let Resolved::Apply { head, args } = resolved_of(db, init) else {
        return None;
    };
    let inlined = crate::eval::apply_lambda(db, head, &args).ok().flatten()?;
    let mform = db.ast.as_form(inlined, "match").map(|t| t.to_vec())?;
    if mform.len() < 2 {
        return None;
    }
    let scrut = mform[0];
    // Rebuild `(match scrut <arm>…)`: an ABORTIVE arm keeps its bare escape; a NON-ABORTIVE arm gets the
    // continuation spliced `(pat (let ((a <arm-body>)) K))`. K is shared across arms (its `a` refs re-resolve
    // to the nearest enclosing `a` binder — the per-arm inner let — by scope). At least ONE arm must abort
    // (else there is no non-local-exit to home; leave it to the ordinary fold).
    let mut any_abort = false;
    let match_head = db.push_name("match");
    let let_head = db.push_name("let");
    let mut children = vec![match_head, scrut];
    for &arm in &mform[1..] {
        let Struct::List(pb) = db.ast.get(arm).clone() else {
            return None;
        };
        if pb.len() != 2 {
            return None;
        }
        let (pat, arm_body) = (pb[0], pb[1]);
        if subtree_has_abortive_perform(db, arm_body, ctx) {
            any_abort = true;
            children.push(db.push_list(vec![pat, arm_body]));
        } else {
            let pair = db.push_list(vec![a_binder, arm_body]);
            let binds = db.push_list(vec![pair]);
            let inner_let = db.push_list(vec![let_head, binds, k]);
            children.push(db.push_list(vec![pat, inner_let]));
        }
    }
    if !any_abort {
        return None;
    }
    Some(db.push_list(children))
}

/// Rewrite a NON-TAIL conditional abort into a branch-tail one by distributing the enclosing strict op
/// into both `if` branches, to a fixpoint. `(+ 100 (if c (Bail.bail 7) 50))` → `(if c (+ 100 (Bail.bail
/// 7)) (+ 100 50))`. Sound because an abort ABANDONS the enclosing computation (so pushing the op into
/// the aborting branch, where it never completes, changes nothing) and the op's OTHER operands are PURE
/// (required below — duplicating a pure operand across the two branches is observably identical). After
/// the lift the abort sits in an `if` branch tail, the shape `thread_branch_local_abort` already folds.
///
/// Only a genuine application `(op a…)` that is NOT itself a perform / special form is a hoist site, and
/// only when EXACTLY the `if`-argument's subtree holds the abortive perform while every sibling operand
/// is perform-free. A shape that does not match is left unchanged (the guard then declines it). Bounded
/// by a rewrite budget so a pathological input cannot loop.
pub(crate) fn hoist_conditional_abort(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> StructId {
    let mut cur = node;
    // A generous fixpoint bound: each pass lifts at least one `if` one level, and a body has far fewer
    // than this many nested strict operands. Prevents any accidental non-convergence.
    for _ in 0..256 {
        match hoist_once(db, cur, ctx) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// One rewrite step of [`hoist_conditional_abort`]: find the FIRST (pre-order) application `(op a…)` with
/// an `if` operand carrying an abortive perform and all other operands pure, distribute the op into the
/// branches, and return the rewritten WHOLE tree. `None` if no such site exists (fixpoint reached).
pub(crate) fn hoist_once(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> Option<StructId> {
    // Is THIS node a hoist site? A plain application `(op a0 … ak)` — head not a perform — with one arg an
    // `if` holding an abortive perform and every OTHER arg perform-free.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
    {
        // The op must be a strict primitive/operator whose operands all evaluate — an `if`/`let`/`match`
        // head is a special form handled by its own thread arm, not a distributable op. `resolved_of` of
        // an operator head is a prim; we simply require the head itself to be perform-free and rebuild the
        // same head. (Being conservative: we only distribute when the head is perform-free.)
        if !subtree_performs(db, head, ctx) {
            for (i, &a) in args.iter().enumerate() {
                if let Resolved::If { cond, then_, else_ } = resolved_of(db, a)
                    && subtree_has_abortive_perform(db, a, ctx)
                {
                    // Every OTHER operand (and the head) must be perform-free, so distributing them into
                    // both branches duplicates only pure code. The `if`'s CONDITION must also be pure (it
                    // evaluates once before the branch; duplicating it is fine only if pure — an effectful
                    // condition would perform twice). If any sibling or the condition performs, this is not
                    // a sound hoist site — leave it (the guard declines).
                    let others_pure = args
                        .iter()
                        .enumerate()
                        .all(|(j, &b)| j == i || !subtree_performs(db, b, ctx));
                    let cond_pure = !subtree_performs(db, cond, ctx);
                    // TYPE-SAFE distribution. After the hoist, the aborting branch `(op … (Bail v) …)`
                    // COLLAPSES to the abort value (its type = the abortive arm's body type), while the
                    // other branch `(op … e …)` keeps the op's RESULT type. Those must AGREE or the produced
                    // `if` is ill-typed → invalid wasm (`(tuple 1 (if c (Bail.bail 7) 5))`: op result is a
                    // tuple, abort value is Int64). Only distribute when the op's result type equals the
                    // abort value type; otherwise leave it (the guard declines). Undetermined either side →
                    // allow (the scalar cases where inference hasn't ground both, matching prior behavior).
                    let op_result = crate::infer::type_of(db, node);
                    let abort_ty = abortive_perform_value_ty(db, a, ctx);
                    // A DEFERRED-width/sign numeric (an `Int64`-literal arm value not yet ground in a
                    // specialized arena — 13175's `(bail (u) s 99)` → `Int{Deferred,Deferred}`) is not
                    // structurally distinct from a fixed numeric of the same kind: it unifies to the required
                    // width on recompile, so distributing the enclosing op keeps the `if` branches consistent.
                    // Treat it as undetermined FOR THIS CHECK (a genuinely-fixed cross-width mismatch — both
                    // fully solved — still requires exact equality, so a real tuple-vs-int / Int8-vs-Int64
                    // mismatch still blocks the hoist).
                    let undet = |t: &crate::ty::Ty| {
                        undetermined_ty(t)
                            || (matches!(t, crate::ty::Ty::Int(_) | crate::ty::Ty::Float(_))
                                && !t.is_fully_solved())
                    };
                    let types_agree = match &abort_ty {
                        Some(at) => undet(&op_result) || undet(at) || op_result == *at,
                        None => true, // no single abort value type found — fall through, guard decides
                    };
                    if others_pure && cond_pure && types_agree {
                        // Build `(op a0 … <branch> … ak)` for each branch, then `(if cond then' else')`.
                        let rebuild = |db: &mut Db, branch: StructId| -> StructId {
                            let children: Vec<StructId> = std::iter::once(head)
                                .chain(
                                    args.iter()
                                        .enumerate()
                                        .map(|(j, &b)| if j == i { branch } else { b }),
                                )
                                .collect();
                            db.push_list(children)
                        };
                        let new_then = rebuild(db, then_);
                        let new_else = rebuild(db, else_);
                        let if_head = db.push_name("if");
                        return Some(db.push_list(vec![if_head, cond, new_then, new_else]));
                    }
                }
            }
        }
    }
    // A strict application `(op … operand …)` whose OPERAND is a `(let (binds) body)` — pure bindings — whose
    // BODY carries an abortive perform (typically `(let ((d n)) (if c (A.out n) n))`, the LET-WRAPPED-PREDICATE
    // face of finding #11: the conditional abort sits under a `let`, so the `if`-operand hoist above does not
    // see it). LIFT the `let` OUT to wrap the whole application: `(op … (let (b) body) …)` ≡ `(let (b) (op …
    // body …))`. The next hoist pass then sees `(op … body …)` with the `if`-abort DIRECT in the operand and
    // distributes it. Sound when the `let`'s binding inits AND every OTHER operand (and the head) are pure —
    // then the lifted bindings run in the same observable order (nothing effectful is reordered), and the
    // bindings scope over the rebuilt application (they were only used inside `body`). Only fires when the let
    // body carries an abort (a plain value-let operand threads normally). Distinct from the let-INIT-carries-if
    // case below (there the `if` is a binding init; here it is the let BODY, one level out).
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && !subtree_performs(db, head, ctx)
    {
        for (i, &a) in args.iter().enumerate() {
            if let Some(letform) = db.ast.as_form(a, "let").map(|t| t.to_vec())
                && letform.len() == 2
                && subtree_has_abortive_perform(db, letform[1], ctx)
                && let Struct::List(pairs) = db.ast.get(letform[0]).clone()
            {
                let binds_pure = pairs.iter().all(|&p| match db.ast.get(p).clone() {
                    Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                    _ => true,
                });
                let others_pure = args
                    .iter()
                    .enumerate()
                    .all(|(j, &b)| j == i || !subtree_performs(db, b, ctx));
                if binds_pure && others_pure {
                    // `(op a0 … (let (b) body) … ak)` → `(let (b) (op a0 … body … ak))`.
                    let inner_app: Vec<StructId> = std::iter::once(head)
                        .chain(
                            args.iter()
                                .enumerate()
                                .map(|(j, &b)| if j == i { letform[1] } else { b }),
                        )
                        .collect();
                    let app = db.push_list(inner_app);
                    let let_head = db.push_name("let");
                    return Some(db.push_list(vec![let_head, letform[0], app]));
                }
            }
        }
    }
    // A SHORT-CIRCUIT connective `(and lhs rhs)` / `(or lhs rhs)` whose RIGHT operand carries an abort is
    // itself a conditional in disguise — `rhs` runs on only one value of `lhs`. Desugar it to the
    // equivalent `if` so the abort lands in a branch tail the per-branch capture folds: `(and lhs rhs)` ≡
    // `(if lhs rhs false)` (rhs runs only when lhs is true), `(or lhs rhs)` ≡ `(if lhs true rhs)` (rhs runs
    // only when lhs is false). `lhs` becomes the `if` CONDITION — evaluated exactly once either way, so no
    // duplication and no purity constraint on it (a later hoist pass lifts any abort inside `lhs`, which is
    // an unconditional strict position — the connective always evaluates lhs). Only fires when `rhs` holds
    // an abortive perform (a plain short-circuit with no abort threads unchanged). The desugared `if`'s
    // aborting branch materializes the abort VALUE opposite a Bool constant, so it must be Bool-typed —
    // guaranteed by the TYPE-CONSISTENCY guard at the top of `reduce_handle` (an abortive arm whose body
    // type ≠ its op's result type declines before reaching here; a connective operand's op result IS Bool,
    // so a surviving abortive arm yields Bool). A plain short-circuit with no abort is not a site.
    if let Resolved::And { lhs, rhs, is_and } = resolved_of(db, node)
        && subtree_has_abortive_perform(db, rhs, ctx)
    {
        let if_head = db.push_name("if");
        let (then_, else_) = if is_and {
            let false_lit = db.push_atom(Leaf::Bool(false));
            (rhs, false_lit) // (and lhs rhs) ≡ (if lhs rhs false)
        } else {
            let true_lit = db.push_atom(Leaf::Bool(true));
            (true_lit, rhs) // (or lhs rhs) ≡ (if lhs true rhs)
        };
        return Some(db.push_list(vec![if_head, lhs, then_, else_]));
    }
    // A `(let (b0… (k (if c t e)) b1…) body)` whose binding INIT is an `(if …)` carrying an abort: the init
    // is a NON-tail strict position (its value feeds the binder `k`), so the per-branch capture can't reach
    // it in place. But an abort ABANDONS everything, so lift the `if` OUT of the let, distributing the whole
    // let into each branch with the init replaced by that branch's value: `(let (… (k (if c t e)) …) body)`
    // ≡ `(if c (let (… (k t) …) body) (let (… (k e) …) body))`. Sound when `c` and the PRECEDING binding
    // inits (b0…) are pure — they are duplicated across the two branches (a later binding sees earlier ones,
    // so we cannot reorder past an effectful earlier init). The aborting branch's `(k t)` init is then an
    // unconditional abort the fold collapses; the other branch keeps the let. Only the FIRST such init is
    // lifted per pass (the fixpoint handles the rest).
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        let (bindings_occ, body_occ) = (form[0], form[1]);
        if let Struct::List(pairs) = db.ast.get(bindings_occ).clone() {
            for (bi, &pair) in pairs.iter().enumerate() {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                    && let Resolved::If { cond, then_, else_ } = resolved_of(db, kv[1])
                    && subtree_has_abortive_perform(db, kv[1], ctx)
                {
                    // `cond` and every PRECEDING init must be pure (duplicated across the two branches; an
                    // effectful earlier binding cannot be run twice). A later init / the body may perform —
                    // it is copied whole into each branch unchanged (run once per branch, as before).
                    let cond_pure = !subtree_performs(db, cond, ctx);
                    let preceding_pure = pairs[..bi].iter().all(|&p| match db.ast.get(p).clone() {
                        Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                        _ => true,
                    });
                    if cond_pure && preceding_pure {
                        // Rebuild the whole `let` with binding `bi`'s init replaced by `branch`.
                        let rebuild = |db: &mut Db, branch: StructId| -> StructId {
                            let name = kv[0];
                            let new_pair = db.push_list(vec![name, branch]);
                            let new_pairs: Vec<StructId> = pairs
                                .iter()
                                .enumerate()
                                .map(|(j, &p)| if j == bi { new_pair } else { p })
                                .collect();
                            let new_bindings = db.push_list(new_pairs);
                            let let_head = db.push_name("let");
                            db.push_list(vec![let_head, new_bindings, body_occ])
                        };
                        let new_then = rebuild(db, then_);
                        let new_else = rebuild(db, else_);
                        let if_head = db.push_name("if");
                        return Some(db.push_list(vec![if_head, cond, new_then, new_else]));
                    }
                }
            }
        }
    }
    // An `(if COND t e)` whose CONDITION is itself an `(if c2 t2 e2)` carrying an abort — the shape the
    // connective desugar above LEAVES when an `and`/`or` with an abortive rhs is an ENCLOSING if's condition:
    // `(if (and b (Bail 7)) 100 200)` → (connective desugar) → `(if (if b (Bail 7) false) 100 200)`. The
    // abort is now buried in the condition's branch, where neither the operand-distribution nor the
    // branch-tail capture reaches it. Distribute the OUTER if THROUGH its condition: `(if (if c2 t2 e2) t e)`
    // ≡ `(if c2 (if t2 t e) (if e2 t e))`. Sound: `c2` is evaluated once either way (no duplication); the
    // outer branches `t`/`e` are duplicated into each inner branch, so they must be PURE (an effectful `t`/`e`
    // would run on a path it shouldn't, or twice). After the lift, an aborting `t2`/`e2` sits as an inner
    // `if`'s CONDITION `(if (Bail 7) t e)` — an abort in a strict-first (condition) position the next pass /
    // the `if`-condition thread handles (an abort in a condition abandons before branching, already folded).
    // Only fires when the condition-if carries an abort (a pure nested condition-if is left alone).
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node)
        && let Resolved::If {
            cond: c2,
            then_: t2,
            else_: e2,
        } = resolved_of(db, cond)
        && subtree_has_abortive_perform(db, cond, ctx)
        && !subtree_performs(db, then_, ctx)
        && !subtree_performs(db, else_, ctx)
    {
        let mk_if = |db: &mut Db, c: StructId, t: StructId, e: StructId| -> StructId {
            let if_head = db.push_name("if");
            db.push_list(vec![if_head, c, t, e])
        };
        // Copy the duplicated outer branches per inner arm (single-parent arena — each use needs its own node).
        let t_a = copy_pure(db, then_);
        let e_a = copy_pure(db, else_);
        let t_b = copy_pure(db, then_);
        let e_b = copy_pure(db, else_);
        let inner_then = mk_if(db, t2, t_a, e_a);
        let inner_else = mk_if(db, e2, t_b, e_b);
        return Some(mk_if(db, c2, inner_then, inner_else));
    }
    // Not a site here — recurse into children, rebuilding with the FIRST rewritten child. A special form
    // (`if`/`let`/`match`) is descended structurally too: a non-tail abort nested in a `let` init or an
    // `if` branch's operand is lifted within that sub-position, then the enclosing thread arm folds it.
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (k, &c) in children.iter().enumerate() {
            if let Some(new_c) = hoist_once(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[k] = new_c;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

/// Whether `body` is served by the E5 ONE-SHOT refold (the pure-one-hole OR two-hole fold in `reduce_handle`)
/// — a positive servability check SHARED with the Site-5 `#cv`-lift so the two never contend (specificity
/// ordering, concierge-steered 2026-08-04). The refold serves a body whose leading discharged perform sits on
/// a strict spine and whose arm is a NON-tail-resumptive one-shot resumptive arm (a tail-resumptive arm is
/// served by the `thread` path, NOT the refold). Mirrors the two-hole refold's gate (`reduce_handle`, the
/// `do_aware_leading_hole` block): a leading hole + a discharged, non-abortive, NON-tail-resumptive,
/// non-peelable, non-partially-resuming arm. The DECISIVE conjunct separating the ao10 shape from the refold
/// cases is `is_tail_resumptive_arm`: ao10's outer arm `(tick (u) s (resume s (+ s 1)))` IS tail-resumptive
/// (refold declines → the `#cv`-lift is the right transform → 111), while the refold-test arm `(+ 1 (resume 10
/// s))` is NON-tail (refold serves → don't lift, defer → 1/13). When this returns true, the Site-5 5b lift
/// stands down. (`pure_hole`/`do_aware_leading_hole` alone don't separate them — both shapes have a leading
/// hole; the arm shape is the separator.)
pub(crate) fn body_served_by_oneshot_refold(db: &mut Db, body: StructId, ctx: &HandlerCtx) -> bool {
    let Some(perform) = do_aware_leading_hole(db, body, ctx) else {
        return false;
    };
    let Resolved::Apply { head, .. } = resolved_of(db, perform) else {
        return false;
    };
    let Some((decl, idx)) = is_perform(db, head, ctx) else {
        return false;
    };
    let Some(arm) = ctx.arms.get(&(decl, idx)).cloned() else {
        return false;
    };
    !ctx.abortive.contains(&(decl, idx))
        && !is_tail_resumptive_arm(db, arm.body)
        && peel_resume_from_arm_body(db, arm.body).is_none()
        && !arm_partially_resumes(db, arm.body)
        // EXACT-MATCH the refold's ONE-SHOT/foreign conjunct (reduce_handle, the two-hole block): the refold
        // only serves a MULTI-shot arm when the continuation reaches NO foreign perform (a one-shot arm is
        // always fine — it splices `C` once). Without this, `body_served_by_oneshot_refold` was STRICTLY
        // LOOSER than the refold: for a multi-shot arm reaching a foreign perform it returned true (Site-5 5b
        // stood down "refold serves it") while the refold DECLINED → NEITHER transform ran → the lost-advance
        // ao10 exists to prevent could recur for that shape (github-liaison/Copilot #2147 review, source-
        // verified). Adding it makes the shared predicate EXACTLY the refold's gate, so the `#cv`-lift and the
        // refold can never both stand down. `body` is the whole conditional the lift/refold sees (same node
        // the refold's `body_reaches_foreign_perform(body, ctx)` inspects).
        && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, ctx))
}

/// Lift a branch-performing conditional out of its strict continuation into TAIL position, one level per
/// pass, to a fixpoint (see `hoist_resumptive_once` for the per-site transforms). Called from the perform
/// call site in `reduce_handle` for the value-preservation argument. `None`-free (returns the rewritten
/// tree, or the input unchanged when no site is found).
pub(crate) fn hoist_resumptive_conditional(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> StructId {
    let mut cur = node;
    // Each pass lifts at least one conditional one level toward tail position; a body nests far fewer
    // than this many strict positions. The bound prevents any accidental non-convergence.
    for _ in 0..256 {
        match hoist_resumptive_once(db, cur, ctx) {
            Some(next) => cur = next,
            None => break,
        }
    }
    cur
}

/// Whether `node` is an `if`/`match` whose SELECTED-branch position (a branch of an `if`, or an arm body
/// of a `match`) contains a perform of an op this handler discharges — the shape whose branch-local state
/// advance the tail fold drops to a continuation. The condition/scrutinee performing is irrelevant here
/// (that threads fine); only a BRANCH/ARM-BODY perform is the hoist trigger.
pub(crate) fn conditional_branch_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        Resolved::If { then_, else_, .. } => {
            subtree_performs(db, then_, ctx) || subtree_performs(db, else_, ctx)
        }
        Resolved::Match { arms, .. } => arms
            .iter()
            .any(|&(_, body)| subtree_performs(db, body, ctx)),
        _ => false,
    }
}

/// Whether `node` is a BLOCK (`let`/`do`) whose TAIL VALUE — through nested `let`/`do` wrappers — is a
/// branch-performing conditional that Site 4's hoist does NOT reach. Site 4 (`conditional_branch_performs`)
/// only lifts a conditional that is DIRECTLY the `let`-init value; a conditional wrapped in a block
/// (`(let ((v (let ((b true)) (if b (E.op) x)))) cont)`) is opaque to it, so the block's exit state falls
/// back to the block-ENTRY state and the branch perform's advance is DROPPED at the block boundary — a
/// silent wrong-value (adv-69, HIGH: `+ (* 10 v) (E.op)` reads the stale pre-branch state). Detecting this
/// residual shape (AFTER the hoist ran, so a liftable direct-init case is already in tail position) lets
/// `reduce_handle` DECLINE cleanly (honest Todo) rather than fold a wrong value. Peels `let`/`do` block
/// wrappers to reach the tail conditional. (The `let` arm peels its BODY regardless of whether the binding
/// PERFORMS — the `..` discards the init: a performing binding is threaded by the ordinary `let` arm, so it
/// does not itself trigger this decline, but this scanner does not need to distinguish it — declining on the
/// tail conditional is sound either way; a performing binding just means the peel keeps going to the body.)
/// Peel nested PURE `let` wrappers off `node`, returning `(collected wrapper binding-pairs, innermost tail)`
/// — the through-block commuting-conversion peel for adv-69 Site 6. `(let (w0…) (let (w1…) tail))` yields
/// `([w0…, w1…], tail)`. Returns `None` if `node` is not a `let` OR any wrapper binding INIT PERFORMS (an
/// effectful wrapper binding cannot be floated earlier in the enclosing `let` — reordering it past a later
/// perform would change effect order/count; leave those to the ordinary `let` thread arm / a later increment).
/// The peel stops at the first non-`let` tail; the caller checks that tail is a branch-performing conditional.
/// A pure wrapper binding is safe to float because the enclosing `let`'s bindings evaluate sequentially and a
/// PURE init commutes freely earlier in that sequence (it reads only already-bound names, which stay in order).
pub(crate) fn peel_pure_let_wrapper(
    db: &mut Db,
    node: StructId,
) -> Option<(Vec<StructId>, StructId)> {
    let mut collected: Vec<StructId> = Vec::new();
    let mut cur = node;
    while let Some(form) = db.ast.as_form(cur, "let").map(|t| t.to_vec()) {
        if form.len() != 2 {
            break;
        }
        let Struct::List(pairs) = db.ast.get(form[0]).clone() else {
            break;
        };
        // Every wrapper binding init must be PURE (no perform of ANY effect) to float it out safely.
        for &p in &pairs {
            match db.ast.get(p).clone() {
                Struct::List(pkv) if pkv.len() == 2 => {
                    if reaches_any_perform(db, pkv[1]) {
                        return None;
                    }
                }
                _ => return None, // malformed binding pair — do not peel
            }
        }
        collected.extend(pairs);
        cur = form[1];
    }
    if collected.is_empty() {
        return None; // `node` was not a `let` wrapper at all
    }
    Some((collected, cur))
}

pub(crate) fn block_wrapped_branch_performs(db: &mut Db, node: StructId, ctx: &HandlerCtx) -> bool {
    match resolved_of(db, node) {
        // A `let` block: its value is the body; recurse through it. (A performing binding is threaded by
        // the ordinary `let` arm — this drop only bites when the wrapper's BODY is the branch conditional.)
        Resolved::Let { body, .. } => {
            conditional_branch_performs(db, body, ctx)
                || block_wrapped_branch_performs(db, body, ctx)
        }
        // A `do` block resolves to a `Ref` at its last form (its value); recurse through it.
        Resolved::Ref { value } => {
            conditional_branch_performs(db, value, ctx)
                || block_wrapped_branch_performs(db, value, ctx)
        }
        _ => false,
    }
}

/// adv-69 **a3 sub-face** (breaker `probe-a3`, block-outstate battery). A block-wrapped branch-performing
/// conditional in a NESTED handle's arm RESUME-VALUE position, performing the OUTER (this) handler's
/// discharged op: `(handle St x ((get …)) (handle Up 0 ((ask (u) t (resume (let ((b true)) (if b (St.get)
/// 99)) t))) …))`. The outer `St` handler threads its state through the inner `Up` handle, but the block
/// boundary INSIDE the inner arm's resume-VALUE drops the outer `St.get`'s state advance (runs 33, correct
/// 34) — the same class of block-boundary out-state drop as the let-init face, but at a DIFFERENT position
/// (`Resume{value}`) that the let-init scanner deliberately does not reach (it stops at a nested `Handle`).
/// Detect a `Resume` whose VALUE is a branch-performing conditional performing THIS handler's op — whether
/// DIRECT (`(resume (if b (St.get) 99) t)`) or BLOCK-WRAPPED (`(resume (let ((b true)) (if b (St.get) 99))
/// t)`) — so `reduce_handle` declines cleanly (honest Todo) instead of folding the dropped-advance wrong
/// value. BOTH forms drop the advance at this position (verified: dropping the direct-conditional disjunct
/// makes `(resume (if true (St.get) 99) t)` miscompile to 33, not fold to 34) — unlike the let-init face,
/// where a DIRECT init is lifted by Site 4 and only the block-wrapped residue remains; a `resume`-value is
/// never hoisted (it lives in a nested handle's arm the outer reduction doesn't rewrite), so a direct
/// conditional there is a genuine miscompile too. PRECISELY POSITIONAL — keyed on the `Resume{value}` slot,
/// NOT a position-agnostic branch-perform scan (that over-declines 5 working cases where the perform sits in
/// a threaded position the fold DOES serve). The through-block fold that flips a3 → PASS is the same deferred
/// commuting conversion as the let-init face. Related:
/// [[adv69-block-wrapped-branch-perform-drops-state-advance-at-block-boundary]].
pub(crate) fn body_has_nested_arm_resume_value_block_wrapped_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A `Resume` whose VALUE is a branch perform of THIS handler's op — direct (`conditional_branch_performs`)
    // OR block-wrapped (`block_wrapped_branch_performs`) — is the a3 drop; both forms revert the out-state at
    // this position. (A `Resume` reached while scanning the outer handle's BODY belongs to a nested handle's
    // arm — the outer body itself has no bare resume; the outer handler's own arms are not in `body`.)
    if let Resolved::Resume { value, .. } = resolved_of(db, node)
        && (conditional_branch_performs(db, value, ctx)
            || block_wrapped_branch_performs(db, value, ctx))
    {
        return true;
    }
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_nested_arm_resume_value_block_wrapped_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Scan `node` for a `let` whose ANY binding init is a BLOCK-WRAPPED branch-performing conditional (the
/// adv-69 shape the hoist cannot lift). Walks the whole subtree via `child_ids` — the miscompiling `let`
/// may sit anywhere the fold would thread. Returns true iff such a `let`-init exists, so `reduce_handle`
/// declines cleanly rather than folding the dropped-advance wrong value.
pub(crate) fn body_has_block_wrapped_let_init_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A NESTED handle: its ARMS belong to THAT inner handler's reduction (a block-wrapped perform in an arm
    // resume-value is the a3 guard's territory), so don't scan them here. But its INIT and BODY both run UNDER
    // this (outer) handler — a block-wrapped branch perform of THIS handler's OUTER op in a `let`-init in the
    // inner body OR the inner handle's INIT (the init is evaluated as part of the handle expression, in the
    // outer extent — `eval.rs` passes `init` to `reduce_handle` alongside `body`) drops the outer advance
    // exactly like the top-level face (the intervening inner handle does not rewrite an outer-effect perform),
    // and the outer reduction's scan previously stopped dead at the inner `Handle` and MISSED it (a silent
    // 33-vs-34 miscompile, v-effects self-probe 2026-08-04; the INIT position was the a4-init sub-face,
    // liaison/Copilot on merged #1933). Descend into BOTH the inner init and body, keeping THIS outer `ctx`:
    // `block_wrapped_branch_performs` is ctx-keyed, so it fires only on a perform of the OUTER discharged op —
    // an inner-effect perform never matches, so this cannot over-decline the inner handler's own shapes.
    if let Resolved::Handle { init, body, .. } = resolved_of(db, node) {
        // The inner handle's INIT may ITSELF be a block-wrapped branch perform (`(handle B (let ((k true))
        // (if k (A.ga) 9)) …)` — the block IS the init, not a let-binding within it), so check the init node
        // directly too; then recurse into both init and body for a `let`-init nested anywhere inside them.
        return block_wrapped_branch_performs(db, init, ctx)
            || body_has_block_wrapped_let_init_branch_perform(db, init, ctx)
            || body_has_block_wrapped_let_init_branch_perform(db, body, ctx);
    }
    // A `let` at THIS node: check each init for the block-wrapped branch-performing shape.
    if let Some(parts) = db.ast.as_form(node, "let").map(<[_]>::to_vec)
        && parts.len() == 2
        && let Struct::List(pairs) = db.ast.get(parts[0]).clone()
    {
        for pair in pairs {
            if let Struct::List(kv) = db.ast.get(pair).clone()
                && kv.len() == 2
                && block_wrapped_branch_performs(db, kv[1], ctx)
            {
                return true;
            }
        }
    }
    // Recurse structurally — the miscompiling `let` may be nested anywhere in the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_block_wrapped_let_init_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// adv-69 **g3 + c3 sub-faces** (breaker block-outstate battery, post-floor 39-probe run). The SAME block-
/// boundary out-state drop as the let-init floor, but at two more consuming positions the hoist does NOT
/// reach:
///   * **g3 — MATCH-SCRUTINEE**: `(match (let ((b true)) (if b (St.get) 99)) (v (+ (* 10 v) (St.get))))` — a
///     block-wrapped branch perform in the scrutinee. Site 5 lifts a scrutinee that is DIRECTLY a branch-
///     performing conditional, but a block wrapper is opaque to it, so the scrutinee's out-state reverts to
///     entry (ran 33, correct 34).
///   * **c3 — DO-STATEMENT (non-last, discarded)**: `(do (let ((x true)) (if x (St.put 7) unit)) (+ (* 10
///     (St.get)) x))` — a block-wrapped branch perform as a non-tail `do` item. Site 1 hoists a non-last item
///     that is DIRECTLY a branch-performing conditional, but a block wrapper defeats its `conditional_branch_
///     performs` match, so the statement's `put` advance is dropped (ran 33, correct 73; the minimal twins
///     d2/e1 — a BARE `if` in the statement, or a def-bound cond — hoist fine and PASS).
///
/// Both key on `block_wrapped_branch_performs` (the WRAPPED shape ONLY): a DIRECT `if`/`match` in either
/// position is lifted by Site 1/5 and still folds — so this never over-declines the working hoist paths.
/// Until the through-block fold lands (the deferred commuting conversion), DECLINE these residual shapes so
/// they grade a clean Todo, never the silent wrong value. Related:
/// [[adv69-block-wrapped-branch-perform-drops-state-advance-at-block-boundary]].
pub(crate) fn body_has_block_wrapped_scrutinee_or_statement_branch_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> bool {
    // A NESTED handle: its ARMS are the inner handler's concern (a3's `Resume{value}` scanner covers the
    // nested-arm position), but its BODY runs UNDER this (outer) handler — a block-wrapped OUTER-effect perform
    // in a match-scrutinee or non-tail do-statement in the inner body drops the outer advance exactly like the
    // top-level face, and a scan that stopped at the inner `Handle` MISSED it (the a4-class miscompile, but for
    // the scrutinee/do-statement positions instead of the let-init: v-effects self-probe 2026-08-04, 33 vs 73).
    // Descend into the inner body ONLY, keeping THIS outer `ctx` — `block_wrapped_branch_performs` is ctx-keyed,
    // so only an OUTER-effect perform fires (an inner-effect perform never matches → no over-decline of the
    // inner handler's own shapes). Recurses through depth-N nesting via this same descent.
    if let Resolved::Handle { body, .. } = resolved_of(db, node) {
        return body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, body, ctx);
    }
    // g3: a MATCH whose SCRUTINEE is a block-wrapped branch-performing conditional.
    if let Resolved::Match { scrutinee, .. } = resolved_of(db, node)
        && block_wrapped_branch_performs(db, scrutinee, ctx)
    {
        return true;
    }
    // c3: a `do` with a NON-LAST item that is a block-wrapped branch-performing conditional (a discarded
    // statement whose branch performs — its advance is dropped at the block boundary). The LAST item is the
    // do's value (tail) — a block-wrapped conditional there is the let-init/body shape the other scanners /
    // hoist handle; only the non-tail statement position is this face.
    if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec)
        && items.len() >= 2
    {
        for &it in &items[..items.len() - 1] {
            if block_wrapped_branch_performs(db, it, ctx) {
                return true;
            }
        }
    }
    // Recurse structurally — the miscompiling position may be nested anywhere in the body.
    match db.ast.get(node).clone() {
        Struct::List(children) => children
            .iter()
            .any(|&c| body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, c, ctx)),
        Struct::Atom(_) => false,
    }
}

/// Rebuild `node`'s branch/arm-body positions by mapping each through `f` (which wraps a branch in the
/// distributed continuation). The condition/scrutinee is left UNTOUCHED (evaluated once, not duplicated).
pub(crate) fn map_conditional_branches(
    db: &mut Db,
    node: StructId,
    mut f: impl FnMut(&mut Db, StructId) -> StructId,
) -> StructId {
    match resolved_of(db, node) {
        Resolved::If { cond, then_, else_ } => {
            let nt = f(db, then_);
            let ne = f(db, else_);
            let if_head = db.push_name("if");
            db.push_list(vec![if_head, cond, nt, ne])
        }
        Resolved::Match { scrutinee, arms } => {
            let match_head = db.push_name("match");
            let mut children = vec![match_head, scrutinee];
            for (pat, body) in arms {
                let nbody = f(db, body);
                children.push(db.push_list(vec![pat, nbody]));
            }
            db.push_list(children)
        }
        _ => node,
    }
}

/// One rewrite step of [`hoist_resumptive_conditional`]. Finds the FIRST (pre-order) strict position
/// holding an `if`/`match` whose branch performs, distributes the enclosing strict context into the
/// branches, and returns the rewritten WHOLE tree. Two site shapes (mirroring the abortive hoist):
///   * a NON-TAIL `do` item `(do a0 … (if c t e) … an)` → `(do a0 … (if c (do t … an) (do e … an)))`,
///     the conditional and everything AFTER it moved into each branch (it was a strict item, so it and
///     its continuation always run). Every item BEFORE it must be pure (they run before the condition;
///     duplicating them is not needed — they stay in the outer `do` prefix — but the conditional must
///     become the do's tail, so items before it that PERFORM would still be threaded strictly, which is
///     fine; the real constraint is only that we don't reorder — handled by keeping the prefix intact).
///   * a strict application operand `(op a0 … (if c t e) … ak)` → `(if c (op … t …) (op … e …))`, with
///     every operand BEFORE the conditional pure (else the condition would jump ahead of an earlier
///     perform) and the head perform-free. Operands AFTER may perform (duplicated, run once per taken
///     branch, in order).
pub(crate) fn hoist_resumptive_once(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
) -> Option<StructId> {
    // Site 1: a `do` whose a NON-LAST item is a branch-performing conditional. Move that item and every
    // following item into each branch as a `(do <branch> rest…)`; keep the preceding items as the do's
    // prefix. Reduces to the tail-position shape one conditional at a time.
    if let Some(items) = db.ast.as_form(node, "do").map(|t| t.to_vec())
        && items.len() >= 2
    {
        for (i, &it) in items.iter().enumerate() {
            if i + 1 < items.len() && conditional_branch_performs(db, it, ctx) {
                let rest: Vec<StructId> = items[i + 1..].to_vec();
                // Wrap a branch in `(do <branch> rest…)` — the continuation after the conditional.
                let wrap = |db: &mut Db, branch: StructId| -> StructId {
                    let do_head = db.push_name("do");
                    let mut ch = vec![do_head, branch];
                    ch.extend_from_slice(&rest);
                    db.push_list(ch)
                };
                let new_cond = map_conditional_branches(db, it, wrap);
                // The rewritten `do` keeps items[..i] as a prefix, with the distributed conditional as its
                // new tail. If i==0 there is no prefix and the conditional IS the whole do's value.
                if i == 0 {
                    return Some(new_cond);
                }
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend_from_slice(&items[..i]);
                ch.push(new_cond);
                return Some(db.push_list(ch));
            }
            // Site 1 THROUGH-BLOCK (adv-69 c3): a NON-LAST do-item that is a PURE-let-WRAPPED branch-
            // performing conditional — `(let ((x true)) (if x (St.put 7) unit))`. Direct Site 1 above misses
            // it (the item is a `let`, not an `if`), so it fell to the block-wrapped-statement decline. FRESHEN
            // the wrapper's local binders first (alpha-rename to fresh names) so distributing `rest` into the
            // conditional branches cannot CAPTURE a free name in `rest` that collides with a wrapper binder
            // (c3's `x` = the enclosing fn param, distinct from the block's `x`); then PEEL the pure wrapper,
            // distribute `rest` into the inner conditional, and RE-WRAP in the (freshened) pure `let`. The
            // wrapper bindings are pure (`peel_pure_let_wrapper` gate), so re-wrapping the distributed
            // conditional preserves evaluation order + effect count — the branch perform's state advance now
            // threads through the continuation exactly like the direct case.
            if i + 1 < items.len() && block_wrapped_branch_performs(db, it, ctx) {
                let it_fresh = freshen_local_binders(db, it);
                if let Some((wrapper_pairs, inner_cond)) = peel_pure_let_wrapper(db, it_fresh)
                    && !wrapper_pairs.is_empty()
                    && conditional_branch_performs(db, inner_cond, ctx)
                {
                    let rest: Vec<StructId> = items[i + 1..].to_vec();
                    let wrap = |db: &mut Db, branch: StructId| -> StructId {
                        let do_head = db.push_name("do");
                        let mut ch = vec![do_head, branch];
                        ch.extend_from_slice(&rest);
                        db.push_list(ch)
                    };
                    let distributed = map_conditional_branches(db, inner_cond, wrap);
                    // Re-wrap the distributed conditional in the freshened pure `let` wrapper.
                    let bindings = db.push_list(wrapper_pairs);
                    let let_head = db.push_name("let");
                    let rewrapped = db.push_list(vec![let_head, bindings, distributed]);
                    if i == 0 {
                        return Some(rewrapped);
                    }
                    let do_head = db.push_name("do");
                    let mut ch = vec![do_head];
                    ch.extend_from_slice(&items[..i]);
                    ch.push(rewrapped);
                    return Some(db.push_list(ch));
                }
            }
        }
    }
    // Site 2: a strict application `(op a0 … ak)` — head not a perform — with a branch-performing
    // conditional operand and every PRECEDING operand pure. Distribute the op into the branches.
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && !subtree_performs(db, head, ctx)
    {
        for (i, &a) in args.iter().enumerate() {
            if conditional_branch_performs(db, a, ctx) {
                // Every operand BEFORE the conditional must be pure — else distributing moves the `if`
                // condition ahead of an earlier perform, reordering effects. Operands AFTER may perform
                // (they are duplicated into each branch but only one branch runs, preserving order/count).
                let preceding_pure = args[..i].iter().all(|&b| !subtree_performs(db, b, ctx));
                if preceding_pure {
                    let rebuild = |db: &mut Db, branch: StructId| -> StructId {
                        let children: Vec<StructId> = std::iter::once(head)
                            .chain(
                                args.iter()
                                    .enumerate()
                                    .map(|(j, &b)| if j == i { branch } else { b }),
                            )
                            .collect();
                        db.push_list(children)
                    };
                    return Some(map_conditional_branches(db, a, rebuild));
                }
            }
        }
    }
    // Site 3: a SHORT-CIRCUIT connective `(and lhs rhs)` / `(or lhs rhs)` whose RHS performs is itself a
    // conditional in disguise — `rhs` runs only on one value of `lhs`, so its perform's state advance is a
    // branch-local one that the strict two-operand threading would drop to a continuation. Desugar it to
    // the equivalent `if` (`(and lhs rhs)` ≡ `(if lhs rhs false)`, `(or lhs rhs)` ≡ `(if lhs true rhs)`)
    // so the next pass sees a branch-performing `if` and lifts it out of its continuation (Site 1/2). `lhs`
    // becomes the `if` CONDITION — evaluated exactly once either way, so no duplication and no purity
    // constraint on it. Only fires when `rhs` performs (a pure connective threads/copies wholesale).
    if let Resolved::And { lhs, rhs, is_and } = resolved_of(db, node)
        && subtree_performs(db, rhs, ctx)
    {
        let if_head = db.push_name("if");
        let (then_, else_) = if is_and {
            let false_lit = db.push_atom(Leaf::Bool(false));
            (rhs, false_lit) // (and lhs rhs) ≡ (if lhs rhs false)
        } else {
            let true_lit = db.push_atom(Leaf::Bool(true));
            (true_lit, rhs) // (or lhs rhs) ≡ (if lhs true rhs)
        };
        return Some(db.push_list(vec![if_head, lhs, then_, else_]));
    }
    // Site 4: a `let` whose BINDING INIT is a branch-performing conditional — the shape an inlined helper
    // leaves (`reduce_applied_lambdas` turns `(let ((a (demand 5 25))) cont)` into `(let ((a (match (Db.get
    // k) … (do (Db.put …) c)))) cont)`). The `let`-init threading (the `let` arm in `thread_bounded`) threads
    // the init and takes its out-state as the post-INIT state — but for a branch-performing conditional init
    // that out-state is the post-SCRUTINEE state (the `Match`/`If` thread arms drop each branch's advance),
    // so the branch's `put` advance never reaches the following bindings / body: a later `(Db.get k)` reads
    // the stale pre-branch state (the helper-call out-state silent miscompile). Distribute the CONTINUATION
    // (this binder + every following binding + the body) INTO each branch, keeping the PRECEDING bindings as
    // an outer `let` prefix so the conditional's own condition/scrutinee still sees them:
    //   `(let (p… (nk (if c t e)) r…) body)`
    //     ≡ `(let (p…) (if c (let ((nk t) r…) body) (let ((nk e) r…) body)))`
    // Now the conditional is in TAIL position of the outer `let` and each branch binds `nk` to the branch
    // value and threads the continuation UNDER the branch-advanced state (the per-branch `let` threading
    // carries the advance). Sound: the preceding inits stay in place and in order (a performing one is still
    // threaded once, before the condition — same order as the original); the condition/scrutinee is evaluated
    // exactly once (it is the single distributed `if`/`match` head); the continuation is duplicated across the
    // branches but only one runs at runtime, so every effect in it happens exactly once, in order. Only the
    // FIRST branch-performing init is distributed per pass (the fixpoint loop lifts a second one next pass).
    if let Some(pairs_form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && pairs_form.len() == 2
        && let Struct::List(pairs) = db.ast.get(pairs_form[0]).clone()
    {
        let body_occ = pairs_form[1];
        for (k, &pair) in pairs.iter().enumerate() {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                continue;
            };
            if kv.len() == 2 && conditional_branch_performs(db, kv[1], ctx) {
                let binder = kv[0];
                let init = kv[1];
                // The continuation = this binder (bound to the branch value) + the remaining bindings +
                // the body, rebuilt as a `let` around each branch.
                let rest_pairs: Vec<StructId> = pairs[k + 1..].to_vec();
                // `map_conditional_branches` calls `wrap` ONCE PER BRANCH (both `if` arms, or every `match`
                // arm), so the continuation — the binder, the remaining bindings, and the body — is spliced
                // into EACH branch. The arena is single-parent (`push_list` overwrites `parent`/`child_ix`),
                // so reusing the SAME `binder`/`rest_pairs`/`body_occ` nodes across branches would parent
                // them to whichever branch is built LAST, orphaning the earlier branch's copy — the "one leaf
                // under two parents" class `deep_fresh_copy` guards (same as the If/Match thread arms copying
                // their state-refs per branch). DEFENSIVE: today this sharing is latent-benign — the `thread`
                // pass runs after this hoist and re-processes each branch (rebuilding via `push_list`/
                // `copy_pure`), re-freshing the shared nodes before resolution matters (verified: the fold
                // yields the same value with or without this copy). But relying on a later pass to launder a
                // structurally-invalid shared subtree is fragile, so give each branch its OWN fresh copy of
                // every continuation node here — making the "continuation duplicated across branches" comment
                // structurally true and robust to a future thread-pass change. (Copilot flag on PR #534.)
                let wrap = |db: &mut Db, branch: StructId| -> StructId {
                    let let_head = db.push_name("let");
                    let binder_c = deep_fresh_copy(db, binder);
                    let this_pair = db.push_list(vec![binder_c, branch]);
                    let mut bindings = vec![this_pair];
                    for &rp in &rest_pairs {
                        let rp_c = deep_fresh_copy(db, rp);
                        bindings.push(rp_c);
                    }
                    let bindings_list = db.push_list(bindings);
                    let body_c = deep_fresh_copy(db, body_occ);
                    db.push_list(vec![let_head, bindings_list, body_c])
                };
                let distributed = map_conditional_branches(db, init, wrap);
                // Preceding bindings (pure or performing — both stay in place, in order) become the outer
                // `let` prefix. With none, the distributed conditional IS the whole value.
                if k == 0 {
                    return Some(distributed);
                }
                let let_head = db.push_name("let");
                let prefix_bindings = db.push_list(pairs[..k].to_vec());
                return Some(db.push_list(vec![let_head, prefix_bindings, distributed]));
            }
        }
    }
    // Site 5: an `if`/`match` whose CONDITION/SCRUTINEE is ITSELF a branch-performing conditional — the shape
    // a connective in the condition leaves after Site 3 desugars it: `(if (and b (> (St.tick) 0)) t e)` →
    // Site 3 → `(if (if b (> (St.tick) 0) false) t e)`, an outer `if` whose CONDITION is a branch-performing
    // `if`. The `If`/`Match` thread arms take the post-CONDITION state as the whole conditional's out-state
    // (they do not observe a per-branch advance in the CONDITION), so the condition's `tick` advance is
    // DROPPED — the outer branches thread against the pre-condition seed and a branch `(St.tick)` reads the
    // stale state (the connective-in-scrutinee silent miscompile: `→ 1` where `→ 2` is correct). Bind the
    // performing condition/scrutinee to a fresh `let` so the outer conditional reads a plain scalar and the
    // performing conditional becomes a `let`-INIT — exactly the shape Site 4 distributes (which threads each
    // branch's advance through the continuation). One binding away is the WORKING let-bound form (verified:
    // a hand let-bound connective threads correctly). The bound name is `#cv{…}` (a `#`-prefixed name is
    // unbindable in source — CDZ0210 — so it can't be captured). Only fires
    // when the condition/scrutinee itself performs in a branch (a pure condition needs no lift).
    {
        let cond_scrut = match resolved_of(db, node) {
            Resolved::If { cond, .. } => Some(cond),
            Resolved::Match { scrutinee, .. } => Some(scrutinee),
            _ => None,
        };
        // 5a: the condition/scrutinee is itself a branch-performing conditional (always lift).
        // 5b (ao10): the condition performs AND a branch performs AND the one-shot REFOLD does NOT serve this
        // body. SPECIFICITY ORDERING (concierge-steered 2026-08-04): the E5 refold is the MORE-SPECIFIC
        // transform for a performing condition (it re-reduces the leading hole, constant-folding the cond to
        // select a branch), and wins where it applies; the `#cv`-lift is the general branch-advance-
        // preservation fallback. Gate 5b on `body_served_by_oneshot_refold` being FALSE (the shared
        // servability predicate, so the two transforms can never both fire): a refold-served body like `(if (<
        // (Amb.flip) 5) (+ 1 (Amb.flip)) 0)` (non-tail arm `(+ 1 (resume 10 s))`) → refold serves → DON'T lift
        // → 1/13; ao10's `(if (> (A.tick) 5) (do (A.tick) 99) 5)` (tail-resumptive outer arm `(resume s (+ s
        // 1))`) → refold declines → LIFT → 111. The DECISIVE separator is `is_tail_resumptive_arm` inside that
        // predicate. Without this gate, 5b `#cv`-lifted the refold cases before the refold ran, breaking their
        // leading-hole logic (4 lib-test regression). Pass the WHOLE `node` (the conditional) as the body — the
        // refold checks the leading hole through it.
        let refold_serves = body_served_by_oneshot_refold(db, node, ctx);
        if let Some(cs) = cond_scrut
            && (conditional_branch_performs(db, cs, ctx)
                || (subtree_performs(db, cs, ctx)
                    && conditional_branch_performs(db, node, ctx)
                    && !refold_serves))
        {
            // `(if CS t e)` ≡ `(let ((#cv CS)) (if #cv t e))`; `(match CS arms)` ≡ `(let ((#cv CS)) (match
            // #cv arms))`. Rebuild the conditional with the condition/scrutinee replaced by a fresh `#cv`
            // reference, wrapped in a `let` binding `#cv` to the original (performing) condition/scrutinee.
            // The next fixpoint pass sees the `let`-init branch-performing conditional and Site 4 distributes
            // it. The bound name is a fresh `#cv{node}` — unbindable in source (a `#`-name is CDZ0210 in binder
            // position) so it cannot be captured by a user binder.
            let cv_name = format!("#cv{}", node.0);
            let cv_binder = db.push_name(&cv_name);
            let cv_ref = db.push_name(&cv_name);
            let rebuilt = match resolved_of(db, node) {
                Resolved::If { then_, else_, .. } => {
                    let if_head = db.push_name("if");
                    db.push_list(vec![if_head, cv_ref, then_, else_])
                }
                Resolved::Match { arms, .. } => {
                    let match_head = db.push_name("match");
                    let mut children = vec![match_head, cv_ref];
                    for (pat, body) in arms {
                        children.push(db.push_list(vec![pat, body]));
                    }
                    db.push_list(children)
                }
                _ => unreachable!("cond_scrut is Some only for If/Match"),
            };
            let pair = db.push_list(vec![cv_binder, cs]);
            let bindings = db.push_list(vec![pair]);
            let let_head = db.push_name("let");
            return Some(db.push_list(vec![let_head, bindings, rebuilt]));
        }
    }
    // Site 6 (adv-69 through-block commuting conversion): a `let` binding whose INIT is a branch-performing
    // conditional WRAPPED in pure `let` blocks — `(let (… (v (let (w…) (if c (E.op) x))) …) body)`. Site 4
    // only fires on a DIRECT conditional init (`conditional_branch_performs(kv[1])` is false when `kv[1]` is a
    // `let`, not an `if`/`match`), so a block-wrapped init reached `block_wrapped_branch_performs`'s safe-floor
    // decline instead of folding — the branch's state advance dropped at the block boundary (adv-69: 33 not
    // 34). FLOAT the inner pure wrapper bindings `w…` OUT into the enclosing `let`, leaving the conditional as
    // `v`'s DIRECT init: `(let (… w… (v (if c (E.op) x)) …) body)`. This is a commuting conversion — sound
    // because the wrapper bindings are PURE (no perform: `wrapper_is_pure` below), so hoisting them earlier in
    // the same `let`'s (sequential, non-recursive) binding list changes no evaluation order or effect count;
    // `v`'s init now evaluates the conditional against the same values. Site 4 then fires on the now-direct
    // conditional init on the next fixpoint pass (threading each branch's advance through the continuation).
    // Only the wrapper is peeled here (one binding, first found); Site 4 does the actual branch distribution.
    // GATED: only when the inner conditional performs THIS handler's discharged op in a branch (the adv-69
    // trigger) — a pure-wrapped pure conditional needs no lift. A `do`-wrapper is left to Site 1 (it peels
    // do-statements); this handles the `let`-wrapper commuting conversion the direct-init Site 4 can't reach.
    if let Some(let_form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && let_form.len() == 2
        && let Struct::List(outer_pairs) = db.ast.get(let_form[0]).clone()
    {
        let outer_body = let_form[1];
        for (k, &pair) in outer_pairs.iter().enumerate() {
            let Struct::List(kv) = db.ast.get(pair).clone() else {
                continue;
            };
            if kv.len() != 2 {
                continue;
            }
            // The init must be a `let`-block whose (through-nested-pure-lets) tail is a branch-performing
            // conditional, with every peeled wrapper binding PURE.
            if let Some((wrapper_pairs, inner_cond)) = peel_pure_let_wrapper(db, kv[1])
                && !wrapper_pairs.is_empty()
                && conditional_branch_performs(db, inner_cond, ctx)
            {
                // Rebuild: outer prefix `p…` + the floated wrapper bindings `w…` + `(v inner_cond)` + the
                // remaining outer bindings `r…`, under the same body. Fresh-copy the floated + rebound nodes
                // (single-parent arena) so nothing is shared across the rebuild.
                let mut new_pairs: Vec<StructId> =
                    Vec::with_capacity(outer_pairs.len() + wrapper_pairs.len());
                for &p in &outer_pairs[..k] {
                    new_pairs.push(deep_fresh_copy(db, p));
                }
                for &wp in &wrapper_pairs {
                    new_pairs.push(deep_fresh_copy(db, wp));
                }
                let binder_c = deep_fresh_copy(db, kv[0]);
                let cond_c = deep_fresh_copy(db, inner_cond);
                let v_pair = db.push_list(vec![binder_c, cond_c]);
                new_pairs.push(v_pair);
                for &p in &outer_pairs[k + 1..] {
                    new_pairs.push(deep_fresh_copy(db, p));
                }
                let new_bindings = db.push_list(new_pairs);
                let body_c = deep_fresh_copy(db, outer_body);
                let let_head = db.push_name("let");
                return Some(db.push_list(vec![let_head, new_bindings, body_c]));
            }
        }
    }
    // Site 7 (adv-69 a4-init through-block): a NESTED handle whose SEED (init) is a pure-`let`-wrapped
    // branch-performing conditional that performs an OUTER (this-ctx) op — `(handle-internal (let ((k true))
    // (if k (A.ga) 9)) arms body)`. The seed is evaluated in THIS handler's extent (eval passes it to
    // `reduce_handle` alongside the body), so the block boundary drops the outer perform's advance (a4-init:
    // 33 not 34) and the fold declines (CDZ0900). FLOAT the pure `let` wrapper OUTSIDE the whole handle —
    // `(let ((k true)) (handle-internal (if k (A.ga) 9) arms body))` — leaving the conditional as a DIRECT
    // seed the outer fold threads (the direct-seed control folds → 34). FRESHEN the wrapper binders first so
    // floating past the handle's arms/body cannot capture a free name there; the wrapper is pure (peel gate),
    // so the commuting conversion preserves evaluation order + effect count. Keyed on the OUTER ctx via
    // `block_wrapped_branch_performs`, so an INNER-op-only seed never fires (no over-float of the inner
    // handler's own shapes).
    if db.ast.head_name(node) == Some(HANDLE_INTERNAL)
        && let Some(hi) = db.ast.as_form(node, HANDLE_INTERNAL).map(<[_]>::to_vec)
        && hi.len() == 3
        && block_wrapped_branch_performs(db, hi[0], ctx)
    {
        let seed_fresh = freshen_local_binders(db, hi[0]);
        if let Some((wrapper_pairs, inner_cond)) = peel_pure_let_wrapper(db, seed_fresh)
            && !wrapper_pairs.is_empty()
            && conditional_branch_performs(db, inner_cond, ctx)
        {
            let hi_head = db.push_name(HANDLE_INTERNAL);
            let new_handle = db.push_list(vec![hi_head, inner_cond, hi[1], hi[2]]);
            let bindings = db.push_list(wrapper_pairs);
            let let_head = db.push_name("let");
            return Some(db.push_list(vec![let_head, bindings, new_handle]));
        }
    }
    // A NESTED `handle-internal`'s ARM LIST is opaque to THIS outer hoist WHEN no arm reaches an operation
    // the OUTER handler discharges — those arms are the INNER handler's concern, folded under ITS ctx by the
    // inside-out `reduce_handle(inner)`. Recursing into such an arm treated its shape `((. B op) (p) s (match
    // …))` as an ordinary strict application `(op a0 a1 <match>)` and let Site 2 distribute the arm-op HEAD
    // into the `match` branches — inverting the arm so the `match` sat in the op-slot, `effect_op_of` → None,
    // the inner op-map went EMPTY, and the nested fold declined (nv1f). BUT an inner arm that itself performs
    // the OUTER effect — the arg-arm `(cut (b) t (do … (set-ty …) …))` where `set-ty` is the OUTER DbState
    // handler's own op demanded across `compute-type` — MUST stay in this recursion, because the outer hoist
    // legitimately needs to lift its branch-performing conditional (skipping it re-broke the arg-arm:
    // wasm-unreachable, the ec113e84e over-scope). So skip the arms child ONLY when EVERY arm body is free of
    // an outer-ctx perform (`subtree_performs(.., ctx)` — ctx here is the OUTER handler, and that predicate is
    // exactly "reaches an op THIS ctx discharges"). nv1f's inner B arm performs only B (inner) → skipped; the
    // arg-arm's set-ty reaches the outer op → recursed. (Mirrors `thread_bounded`, which treats a nested
    // handle as opaque; this is the finer-grained rule the hoist needs since an inner arm can re-perform the
    // outer effect.)
    let arms_to_skip = if db.ast.head_name(node) == Some(HANDLE_INTERNAL) {
        db.ast
            .as_form(node, HANDLE_INTERNAL)
            .and_then(|t| t.get(1).copied())
            .filter(|&arms| {
                // arms = `((op (params) state body)…)`; each arm's BODY is its last child. Skip the arms
                // list only if NONE of the arm bodies reaches an outer-ctx perform.
                let arm_bodies: Vec<StructId> = match db.ast.get(arms) {
                    Struct::List(arm_nodes) => arm_nodes
                        .iter()
                        .filter_map(|&a| match db.ast.get(a) {
                            Struct::List(parts) => parts.last().copied(),
                            Struct::Atom(_) => None,
                        })
                        .collect(),
                    Struct::Atom(_) => Vec::new(),
                };
                // Use `subtree_reaches_discharged_op` (a perform of an op the OUTER ctx discharges), NOT
                // `subtree_performs`: the latter treats a bare `resume` as effectful, so an inner arm's own
                // `(resume …)` would spuriously read as "performs the outer effect" and defeat the skip. We
                // want ONLY "reaches a perform of the OUTER handler's op" — the arg-arm's `set-ty`, not B's
                // inner resume.
                !arm_bodies
                    .iter()
                    .any(|&b| subtree_reaches_discharged_op(db, b, ctx))
            })
    } else {
        None
    };
    // A `match`: recurse into the SCRUTINEE and each ARM BODY as EXPRESSIONS — NEVER descend into an arm
    // `(pattern body)` PAIR as a whole. A two-element arm pair `(pattern body)` resolves as an
    // `Apply { head: pattern, args: [body] }`, so the generic child-recursion below would hand the pair to
    // Site 2 (strict-application distribution), which reads the PATTERN as a function head and distributes
    // it into the body's branches — corrupting a well-formed `(match #cv (P (match S (Q b))))` into the
    // malformed `(match #cv (match S (Q (P b))))`, whose arm slot now holds a bare `(match …)` instead of a
    // `(pattern body)` pair (resolve_match then Poisons it → the nested-match-scrutinee eg1 CDZ0900). Recurse
    // the arm BODY only and rebuild `(pat new_body)`, keeping the pattern intact. (Site 5 above already
    // handles a performing SCRUTINEE at the match level; this is the fallthrough that lifts a conditional
    // nested inside the scrutinee or an arm body.)
    if let Resolved::Match { scrutinee, arms } = resolved_of(db, node) {
        if let Some(new_scrut) = hoist_resumptive_once(db, scrutinee, ctx) {
            return Some(rebuild_match_scrutinee(db, node, &arms, new_scrut));
        }
        if let Struct::List(children) = db.ast.get(node).clone() {
            // Arm pairs are children [2..]; child 0 is the `match` head, child 1 is the scrutinee.
            for (k, &arm) in children.iter().enumerate().skip(2) {
                if let Struct::List(pb) = db.ast.get(arm).clone()
                    && pb.len() == 2
                {
                    if let Some(new_body) = hoist_resumptive_once(db, pb[1], ctx) {
                        let new_arm = db.push_list(vec![pb[0], new_body]);
                        let mut new_children = children.clone();
                        new_children[k] = new_arm;
                        return Some(db.push_list(new_children));
                    }
                    continue; // do NOT recurse into the arm pair as an expression
                }
                // A non-pair arm child (malformed match — resolve would have Poisoned it): recurse
                // generically, harmless since resolve_match rejects it anyway.
                if let Some(new_c) = hoist_resumptive_once(db, arm, ctx) {
                    let mut new_children = children.clone();
                    new_children[k] = new_c;
                    return Some(db.push_list(new_children));
                }
            }
        }
        return None;
    }
    // Not a site here — recurse into children, rebuilding with the FIRST rewritten child (so a
    // conditional nested inside a `let` init / branch / arm is lifted within that sub-position, then the
    // enclosing pass lifts it further if needed).
    if let Struct::List(children) = db.ast.get(node).clone() {
        for (k, &c) in children.iter().enumerate() {
            if Some(c) == arms_to_skip {
                continue; // an inner handler's outer-effect-free arms fold under their own reduce_handle
            }
            if let Some(new_c) = hoist_resumptive_once(db, c, ctx) {
                let mut new_children = children.clone();
                new_children[k] = new_c;
                return Some(db.push_list(new_children));
            }
        }
    }
    None
}

/// Whether `node` contains an abortive perform (of an op in `ctx.abortive`) the fold CANNOT realize
/// soundly — run AFTER `hoist_conditional_abort`, so every abort the hoist could lift to a branch tail
/// already has been. What remains SOUND: (1) an UNCONDITIONAL abort (`(+ 1 (Bail.bail 7))` — E4-a
/// collapses the whole handle to the arm value); (2) an abort anywhere inside a TAIL-position `if`/`let`
/// branch reached through only PURE STRICT OPS — `thread_branch_local_abort` captures the branch's abort
/// regardless of how deep the pure nesting is (`(if c (+ 100 (Bail.bail 7)) …)` folds: the branch value
/// IS the abort). UNSOUND (flagged → `reduce_handle` declines): an abort the hoist could not lift and the
/// per-branch capture cannot intercept — (a) under a SHORT-CIRCUIT connective `and`/`or` right operand
/// (threaded strictly, collapses the whole handle across a branch), (b) alongside an EFFECTFUL sibling
/// operand (the hoist requires pure siblings to distribute, so a perform-bearing sibling blocks the lift),
/// or (c) in a conditional's CONDITION. `tail` = value flows to the handle value along a pure-strict path
/// from the nearest enclosing branch; `under_cond` = descended into a conditional branch. Flag = an abort
/// at `under_cond && !tail`.
pub(crate) fn body_has_unsound_abortive_perform(
    db: &mut Db,
    node: StructId,
    ctx: &HandlerCtx,
    tail: bool,
    under_cond: bool,
    // When TRUE, the SAME-FN direct-abort case is NOT flagged (it is now soundly lowered to Core::HandleAbort
    // under a whole-def-body handle); ONLY a CROSS-FN unsound abort is reported. FALSE = the full check.
    cross_fn_only: bool,
) -> bool {
    // An abort reached under a conditional at a non-tail (non-capturable) position is the unsound shape.
    if !cross_fn_only
        && under_cond
        && !tail
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && let Some(id) = is_perform(db, head, ctx)
        && ctx.abortive.contains(&id)
    {
        return true;
    }
    // A CROSS-FUNCTION call reaching a CONDITIONAL abort in a NON-TAIL position is unsound. The abort lives
    // inside the callee's body under an `if`/`match`/connective (opaque here — the hoist lifts only
    // SYNTACTIC conditionals, so it cannot see it), so after the inline arm β-reduces the callee the abort
    // surfaces as a non-tail conditional the hoist never lifted — and `thread`'s `if` arm would capture it
    // PER-BRANCH as if the `if` were the handle's tail, dropping the enclosing op (`(+ 10 (check -1))` →
    // 109 instead of 99, a MISCOMPILE). Flag it so `reduce_handle` DECLINES: this needs the non-local-exit
    // calling convention (a later vertical). An UNCONDITIONAL cross-fn abort (`(+ 10 (boom 99))`, the
    // callee is a bare abort) is NOT flagged — inlining yields a plain strict abort E4-a collapses soundly.
    if !tail
        && let Resolved::Apply { head, .. } = resolved_of(db, node)
        && is_perform(db, head, ctx).is_none()
        && call_reaches_conditional_abortive(db, head, ctx)
    {
        return true;
    }
    // An `if`: the CONDITION evaluates BEFORE branching, so an abort there abandons the whole computation
    // regardless of which branch would run. When the `if` is on a TAIL path (`tail=true` — it is the handle
    // body, or the tail of a branch the per-branch capture intercepts), that condition-abort IS capturable:
    // `thread`'s `if` arm threads the condition first, the abort sets the cell, and `reduce_handle` (body) /
    // `thread_branch_local_abort` (branch) takes the abort value — the enclosing computation is dead, so
    // nothing is dropped. This is what lets a connective-in-condition abort fold after the hoist distributes
    // the outer `if` through it (`(if (and b (Bail 7)) 100 200)` → `(if b (if (Bail 7) 100 200) …)`, the
    // condition-abort in the tail branch). A NON-tail `if` (`tail=false` — an operand `(+ 1 (if (Bail 7) …))`
    // the hoist could not lift) keeps the condition non-capturable → flagged. So the condition inherits the
    // `if`'s own `tail`. Each BRANCH is a conditional position (`under_cond=true`, `tail` carried).
    if let Resolved::If { cond, then_, else_ } = resolved_of(db, node) {
        return body_has_unsound_abortive_perform(db, cond, ctx, tail, under_cond, cross_fn_only)
            || body_has_unsound_abortive_perform(db, then_, ctx, tail, true, cross_fn_only)
            || body_has_unsound_abortive_perform(db, else_, ctx, tail, true, cross_fn_only);
    }
    // A `(let ((n init)…) body)`: the let's VALUE is the BODY's value, so the body inherits THIS position's
    // tail-ness + `under_cond`. Each INIT is a strict operand. KEY (post-hoist, mirroring the generic-op
    // case): an UNCONDITIONAL abort in an init INSIDE a tail branch is CAPTURABLE — when the init aborts the
    // whole let collapses to that value, which `thread_branch_local_abort` takes per-branch — so init `bi`
    // is capturable-tail iff we are on a `tail` path AND every PRECEDING init is perform-free (an effectful
    // earlier init runs before the abort and cannot be dropped; a conditional abort in an init was already
    // lifted out of the let by the hoist, so a surviving one is unconditional). The BODY inherits the let's
    // tail-ness + `under_cond`.
    if let Some(form) = db.ast.as_form(node, "let").map(|t| t.to_vec())
        && form.len() == 2
    {
        let (bindings_occ, body_occ) = (form[0], form[1]);
        if let Struct::List(pairs) = db.ast.get(bindings_occ).clone() {
            for (bi, &pair) in pairs.iter().enumerate() {
                if let Struct::List(kv) = db.ast.get(pair).clone()
                    && kv.len() == 2
                {
                    let preceding_pure = pairs[..bi].iter().all(|&p| match db.ast.get(p).clone() {
                        Struct::List(pkv) if pkv.len() == 2 => !subtree_performs(db, pkv[1], ctx),
                        _ => true,
                    });
                    // finding #11-B (oamin4/oa3): a def-boundary conditional abort with a foreign-performing
                    // argument — `(let ((a (unwrap (E.fetch) tag))) …)` where `unwrap` aborts in a match arm and
                    // `E.fetch` performs an outer op. The abort must home to the handler boundary, but the fold
                    // captures it per-branch and threads the abort value into the let continuation (a silent
                    // wrong value). It is opaque to the `if`-only hoist AND to the ordinary cross-fn guard (which
                    // walks `if`/`and`, not `match` arms). Flag it here so `reduce_handle` declines to the safe
                    // floor. GATED on the foreign argument so the PURE-arg controls (oamin1/oamin5) still fold.
                    if init_is_foreign_arg_match_abort_call(db, kv[1], ctx) {
                        return true;
                    }
                    let init_tail = tail && preceding_pure;
                    if body_has_unsound_abortive_perform(
                        db,
                        kv[1],
                        ctx,
                        init_tail,
                        under_cond,
                        cross_fn_only,
                    ) {
                        return true;
                    }
                }
            }
        }
        return body_has_unsound_abortive_perform(
            db,
            body_occ,
            ctx,
            tail,
            under_cond,
            cross_fn_only,
        );
    }
    // Generic descent over a strict application `(op a0 … ak)`. KEY (post-hoist): an abort nested under a
    // PURE strict op INSIDE a tail branch is CAPTURABLE — `thread_branch_local_abort` takes the branch's
    // abort value regardless of the pure `op` wrapping it — so we PRESERVE `tail` for such an operand
    // rather than lowering it. An operand is capturable-tail iff (i) we are already on a capturable path
    // (`tail`), (ii) the op is NOT a short-circuit connective, and (iii) every OTHER operand (and the head)
    // is PERFORM-FREE (an effectful sibling would run before/after the abort and cannot be duplicated or
    // dropped — the hoist declines to lift across it, so its abort stays flagged). A short-circuit right
    // operand (index >= 2 of `and`/`or`) is conditional AND non-capturable (threaded strictly), so it is
    // marked `under_cond` and `tail=false`.
    match db.ast.get(node).clone() {
        Struct::List(children) => {
            let head_name = children
                .first()
                .and_then(|&c| db.ast.as_name(c).map(str::to_string));
            let is_short_circuit = matches!(head_name.as_deref(), Some("and") | Some("or"));
            // Whether every element (head + operands) OTHER than `i` is perform-free — so the abort in
            // operand `i` is the sole effect and its enclosing pure op is transparent to the per-branch
            // capture (the branch collapses to the abort value, discarding the pure wrapper).
            let siblings_pure = |db: &mut Db, i: usize| -> bool {
                (0..children.len()).all(|j| j == i || !subtree_performs(db, children[j], ctx))
            };
            (0..children.len()).any(|i| {
                let c = children[i];
                if i == 0 {
                    // The head: descend as a strict, non-capturable operand (a head rarely performs).
                    return body_has_unsound_abortive_perform(db, c, ctx, false, under_cond, cross_fn_only);
                }
                let sc_right = is_short_circuit && i >= 2;
                // Capturable-tail: on a tail path, not a short-circuit right operand, siblings all pure.
                // BUT a CROSS-FUNCTION CONDITIONAL-abortive operand is NOT capturable even so — its abort is
                // opaque to the hoist (hidden under an `if` in the callee), so after the inline arm surfaces
                // it the per-branch capture would wrongly drop THIS enclosing op (`(+ 10 (check -1))` →
                // 109). Deny it capturable-tail so the cross-fn-reach check above flags it (`!tail`).
                let cross_fn_abort = matches!(resolved_of(db, c), Resolved::Apply { head, .. }
                    if is_perform(db, head, ctx).is_none() && call_reaches_conditional_abortive(db, head, ctx));
                let capturable = tail && !sc_right && !cross_fn_abort && siblings_pure(db, i);
                let child_tail = capturable;
                let child_cond = under_cond || sc_right;
                body_has_unsound_abortive_perform(db, c, ctx, child_tail, child_cond, cross_fn_only)
            })
        }
        Struct::Atom(_) => false,
    }
}
