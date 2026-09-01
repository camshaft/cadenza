//! The tail-resumptive continuation THREADER — `thread`/`thread_bounded` and the multi-value
//! `thread_returning_tuple`, plus the FINDING-24 growing-collection-state guards. Extracted from
//! the effects module to keep each file under the source-size lint; behavior unchanged.

use super::*;
use crate::ast::{CompoundCtor, Leaf, Struct, StructId};
use crate::db::Db;
use crate::fxhash::FxHashMap as HashMap;
use crate::resolve::resolved_of;
use crate::resolved::Resolved;

/// Thread `body` in MULTI-VALUE mode (repro-1): the synthesized `f#ctx` returns `(value, out-state-per-slot)`
/// at every tail, so a caller's self-call can thread the recursion's advanced state to a LATER sibling. The
/// walk descends through `if`/`match` (each branch/arm body is its own tail, producing its own tuple under
/// its own out-state); at a LEAF tail expression it threads with `thread_bounded` (self-calls there push
/// pending temps via the multi-value self-call arm), then drains those temps into wrapping `let`s and
/// packages `("tuple" value out-states…)`. Returns `None` (clean decline) for a leaf whose self-call sits
/// under a conditional — a position v1 does not bind soundly. `ctx.multivalue` must be set by the caller.
pub(crate) fn thread_returning_tuple(
    db: &mut Db,
    body: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    callee_def: usize,
) -> Option<StructId> {
    match resolved_of(db, body) {
        // An `if`: thread the CONDITION (a perform there threads state to both branches; a self-call there
        // is unconditional and drained around the whole `if`), then each branch is its OWN tail — recurse,
        // giving each a fresh copy of the post-condition state (single-parent-arena discipline, as the
        // ordinary `if` thread arm does). Rebuild `(if rcond then-tuple else-tuple)`, wrapping any
        // condition-level self-call temps around it.
        Resolved::If { cond, then_, else_ } => {
            let mark = ctx.pending.borrow().len();
            let (rcond, cur) = thread_bounded(db, cond, states, ctx, 0)?;
            let then_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let else_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
            let rthen = thread_returning_tuple(db, then_, then_states, ctx, callee_def)?;
            let relse = thread_returning_tuple(db, else_, else_states, ctx, callee_def)?;
            let if_head = db.push_name("if");
            let if_node = db.push_list(vec![if_head, rcond, rthen, relse]);
            Some(drain_and_wrap(db, ctx, mark, if_node))
        }
        // A `match`: thread the SCRUTINEE, then each arm BODY is its own tail — recurse under a fresh copy of
        // the post-scrutinee state. The PATTERN is a binder position (copied structurally). Rebuild `(match
        // rscrut (rpat body-tuple)…)`, wrapping any scrutinee-level self-call temps around it.
        Resolved::Match { scrutinee, arms } => {
            let mark = ctx.pending.borrow().len();
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, 0)?;
            let match_head = db.push_name("match");
            let mut children = vec![match_head, rscrut];
            for (pat, arm_body) in arms {
                let rpat = copy_pure(db, pat);
                let arm_states: Vec<StructId> = cur.iter().map(|&s| copy_pure(db, s)).collect();
                let rbody = thread_returning_tuple(db, arm_body, arm_states, ctx, callee_def)?;
                children.push(db.push_list(vec![rpat, rbody]));
            }
            let match_node = db.push_list(children);
            Some(drain_and_wrap(db, ctx, mark, match_node))
        }
        // A `(let ((n init)…) dispatch)` whose BODY is itself a dispatch (`if`/`match`) holding the tail
        // self-call — finding #12 (`walk k = let d = E.next in (if … (walk (+ k 1))))`). The leaf arm below
        // would treat the whole `let` as one leaf and `selfcall_under_conditional` would see the self-call
        // under the body's `if` → decline, dropping the recursion to single-return (the trailing-observer
        // reads the PRE-call state, 705 vs 708). Instead THREAD the inits (each may perform — advancing
        // state), then recurse `thread_returning_tuple` on the body under the post-init state, and rebuild the
        // `let` around the body's tuple so the binders stay in scope. Only descend when the body is a
        // dispatch (the shape this arm handles); a plain-leaf `let` body still routes through the leaf arm
        // below (a self-call directly in a leaf `let` is bound by `thread_bounded`, unchanged). The inits are
        // threaded with `thread_bounded` exactly as the ordinary `let` arm does; a self-call IN an init is on
        // the unconditional strict spine and pushes a pending temp drained by the wrapping `drain_and_wrap`.
        _ if db.ast.as_form(body, "let").map(|t| t.len()) == Some(2) && {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            matches!(
                resolved_of(db, form[1]),
                Resolved::If { .. } | Resolved::Match { .. }
            ) || db.ast.as_form(form[1], "let").map(|t| t.len()) == Some(2)
                || db.ast.as_form(form[1], "do").is_some()
                // A `(let ((d (E.op))) (f … recursive …))` whose BODY carries the tail re-entrant call —
                // the cross-def recursion-boundary shape (nr0: `(let ((d (S.depth))) (outer (- k 1) (+ acc
                // (inner d 0))))`). The leaf arm below would `thread_bounded` the WHOLE let and DROP the
                // let-init perform's binder (`d` rides into the self-call arg UNBOUND — a CDZ0101 the escape
                // validator catches). Route it here so the init is threaded (binding `d` to the resume value,
                // advancing the state) and the body is recursed as its own tail, rebuilding the `let` so `d`
                // stays in scope.
                || contains_recursive_call(db, form[1], callee_def)
        } =>
        {
            let form = db.ast.as_form(body, "let").unwrap().to_vec();
            let (bindings_occ, body_occ) = (form[0], form[1]);
            let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
                return None;
            };
            let mark = ctx.pending.borrow().len();
            let mut cur = states;
            let mut rpairs = Vec::with_capacity(pairs.len());
            for pair in pairs {
                let Struct::List(kv) = db.ast.get(pair).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                let name_copy = copy_pure(db, kv[0]);
                let (rinit, next) = thread_bounded(db, kv[1], cur, ctx, 0)?;
                cur = next;
                rpairs.push(db.push_list(vec![name_copy, rinit]));
            }
            // The body dispatch is the let's tail — thread it returning a tuple under the post-init state.
            let rbody = thread_returning_tuple(db, body_occ, cur, ctx, callee_def)?;
            let let_head = db.push_name("let");
            let bindings = db.push_list(rpairs);
            let rlet = db.push_list(vec![let_head, bindings, rbody]);
            Some(drain_and_wrap(db, ctx, mark, rlet))
        }
        // A `(do stmt… tail)` whose tail is a dispatch (`if`/`match`) or a nested `let`/`do` chain — breaker
        // #14 ra6 (`(let ((a (E.next))) (do (E.next) (if … (race …) k)))`, a let-body-do with a PERFORMING
        // discarded head). The leaf arm below would treat the whole `do` as one leaf and `selfcall_under_
        // conditional` would see the tail self-call under the `if` → single-return, dropping the out-state the
        // discarded head advanced (the trailing observer reads pre-recursion state). Thread the leading stmts
        // (each may PERFORM — advancing state, its value discarded) with `thread_bounded`, recurse
        // `thread_returning_tuple` on the tail under the post-stmt state, and rebuild the `do`. Mirrors the
        // `let`-dispatch arm above (the do-analogue). Only when the tail is threadable-shaped; a plain-leaf do
        // tail still routes through the leaf arm.
        _ if db
            .ast
            .as_form(body, "do")
            .map(|t| t.len() >= 2)
            .unwrap_or(false) =>
        {
            let items = db.ast.as_form(body, "do").unwrap().to_vec();
            let (tail, stmts) = items.split_last().unwrap();
            let (tail, stmts) = (*tail, stmts.to_vec());
            let mark = ctx.pending.borrow().len();
            let mut cur = states;
            let mut rstmts = Vec::with_capacity(stmts.len());
            for st in stmts {
                // A leading stmt is evaluated for effect (value discarded); thread its state forward.
                let (rst, next) = thread_bounded(db, st, cur, ctx, 0)?;
                cur = next;
                rstmts.push(rst);
            }
            // The tail is the do's value — thread it returning a tuple under the post-stmt state.
            let rtail = thread_returning_tuple(db, tail, cur, ctx, callee_def)?;
            let do_head = db.push_name("do");
            let mut children = vec![do_head];
            children.extend(rstmts);
            children.push(rtail);
            let rdo = db.push_list(children);
            Some(drain_and_wrap(db, ctx, mark, rdo))
        }
        // A LEAF tail expression — an operator/operand spine, a `let`, a tuple/ctor, a bare value, or a
        // perform. Thread it: any self-call on its unconditional strict spine pushes a pending temp (the
        // multi-value self-call arm), and a later sibling threads against that temp's `(. t 1)` out-state.
        // A self-call GATED behind a conditional here is unhandled in v1 — decline cleanly.
        _ => {
            if selfcall_under_conditional(db, body, callee_def) {
                return None;
            }
            let mark = ctx.pending.borrow().len();
            let (value, out_states) = thread_bounded(db, body, states, ctx, 0)?;
            let tuple = build_value_state_tuple(db, value, &out_states);
            Some(drain_and_wrap(db, ctx, mark, tuple))
        }
    }
}

pub(crate) fn thread(
    db: &mut Db,
    node: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
) -> Option<(StructId, Vec<StructId>)> {
    thread_bounded(db, node, states, ctx, 0)
}

/// Thread an `if` BRANCH, capturing an abortive perform as LOCAL to the branch (E4): if threading the
/// branch set the whole-handle `abort_value` cell, the branch's value is that arm value and the cell is
/// CLEARED (so the abort does not collapse the OTHER branch or the whole handle — a branch-tail abort
/// just yields the arm value for that branch, since the enclosing `if` is the handle body's value).
/// Returns the branch's rewritten value (the abort value if it aborted, else the threaded rewrite). The
/// branch's out-state is discarded (nothing after an `if` in the tail-fold shape reads it).
/// Whether `node` references a synthesized `#cv{…}` name — the condition-value binding the performing-
/// conditional hoist introduces (`(if C t e)` ≡ `(let ((#cv C)) (if #cv t e))`). Such a name is bound by a
/// `let` that wraps only its own conditional; a branch out-state carrying a `#cv` ref cannot be re-used in
/// the merged If-arm state position (it would resolve out of scope → CDZ0101). The If-arm branch-out-state
/// merge skips a slot whose branch out-state contains one. A cheap name-prefix scan (a `#cv`-prefixed name is
/// unbindable in source — CDZ0210 in binder position — so a user can't introduce a colliding one and the match
/// is unambiguous).
pub(crate) fn contains_cv_ref(db: &Db, node: StructId) -> bool {
    any_name(db, node, |nm| nm.starts_with("#cv"))
}

/// Whether `node`'s subtree mentions a NAME node equal to `name` (a purely syntactic scan). Used by the
/// `do`-peel's effectful-stmt guard: at the arm-return thread site a do-`def` binder resolves unreliably
/// (a body binder shows as `Poison(Unbound)`), so a resolution-based ref count can't be trusted — a stable
/// name-string match is the safe witness for "the next-state reads this effectful def's binder".
pub(crate) fn subtree_mentions_name(db: &Db, node: StructId, name: &str) -> bool {
    any_name(db, node, |nm| nm == name)
}

/// Whether any NAME atom in `node`'s subtree satisfies `pred` — a purely syntactic DFS (`as_name`/`get` only,
/// no resolution). The shared walker behind `contains_cv_ref` (`#cv`-prefix) and `subtree_mentions_name`
/// (exact-name match); collapse the two identical recursions (v-code-cleanliness dedup lead).
pub(crate) fn any_name(db: &Db, node: StructId, pred: impl Fn(&str) -> bool + Copy) -> bool {
    if let Some(nm) = db.ast.as_name(node) {
        return pred(nm);
    }
    match db.ast.get(node) {
        Struct::List(children) => children.iter().any(|&c| any_name(db, c, pred)),
        Struct::Atom(_) => false,
    }
}

/// Whether `prim` is an ACCUMULATING collection operation — one that takes a prior collection as its FIRST
/// operand and produces a new collection embedding it (`prelude-and-resolution.md`; the set v-rust-backend
/// root-caused as the O(k^N) shapes). Each re-substitutes the prior slot-state per dispatch, so a fold that
/// threads its result blows the Core up exponentially unless the state is per-dispatch let-bound. This is the
/// GENERIC axis — the compiler's own `Prim` intrinsics, NOT operator spelling (a user op coincidentally named
/// `push` resolves to no `Prim` and is excluded, per the no-hard-coded-capabilities rule). EXCLUDES fresh
/// constructors (`ListNew`/`MapNew`/`SetOf` — don't embed a prior), reads (`ListAt`/`MapLookup`/`*Len`),
/// removes (`MapRemove`/`SetRemove` — the shrink twin, not the shipping blow-up surface), and every scalar/
/// plain-compound op (`Add`/`Record`/`Tuple`/…): those keep the state O(1) and stay byte-identical.
pub(crate) fn is_accumulating_collection_prim(prim: crate::resolved::Prim) -> bool {
    use crate::resolved::Prim;
    matches!(
        prim,
        Prim::ListPush
            | Prim::ListPrepend
            | Prim::ListConcat
            | Prim::ListUpdate
            | Prim::MapInsert
            | Prim::SetInsert
            | Prim::SetUnion
            | Prim::SetIntersection
            | Prim::SetDifference
            | Prim::BytesConcat
    )
}

/// FINDING-24 guard: does a threaded slot-state GROW a collection off the prior state — i.e. does evaluating
/// it PRODUCE the result of an accumulating collection op (`(List.push pre t)`, `(Map.insert m k v)`, a
/// `Bytes.concat`), possibly THROUGH a `let`/`match`/`if`/name that forwards to one? Only such a state
/// re-embeds the prior slot per dispatch (each dispatch the body reads state k times, each read re-substitutes
/// the growing expr → O(k^N) Core, `core_of`'s memo can't collapse the StructId-distinct `deep_fresh_copy`
/// trees → rustc SIGSEGVs the async emit). Those states get the per-dispatch `#st` sibling let-bind (the fix);
/// EVERY OTHER shape — a bare name / prior `#st` ref, a tuple PROJECTION `(. t 1)`, a scalar counter `(+ s 1)`,
/// a plain record/tuple, a fresh-constant collection — keeps the state O(1) and stays byte-identical.
///
/// NARROWED (was: fire on ANY compound) to the accumulating-collection `Prim` set — matching a `Prim`, not a
/// spelling, is the generic axis (`is_accumulating_collection_prim`) and drops the benign compounds (counter
/// folds, projections, records, plain calls) that the broad guard over-fired on. The mk21 shape threads the
/// grown collection through a `let`-bound `match` (`(let ((m2 (match … (Map.insert m k v) …))) m2)`), so the
/// reach follows `Ref`/`Let`-body/`Match`-arms/`If`-branches to find the producing op — a top-level shape
/// match alone would miss it.
///
/// EXCLUDES a next-state that REACHES A PERFORM (`reaches_any_perform`): a host/foreign perform in the
/// next-state slot must be discharged IN PLACE by the enclosing fold (it is the effect the dispatch runs) —
/// hoisting it into an `#st` let-init reorders/elides it (breaker: `(host (ask) …)` next-state call trapped
/// "host-call ask.ask" + a final-dispatch state-call elided). Binding is for a PURE growing state only; a
/// perform-bearing state stays where it is regardless of size (correctness over the size win).
/// Correctness-safe either way for the pure case: a false "not growing" only leaves the exponential in place
/// (no miscompile); a false "growing" only adds a redundant kept binding (behavior-preserving, same value +
/// eval order). Keeps the common corpus byte-identical — the bind fires ONLY on a growing-collection state.
pub(crate) fn next_state_is_growing_compound(db: &mut Db, next_state: StructId) -> bool {
    // A perform-bearing state must NOT be hoisted into a let-init (it would reorder/elide the effect).
    // NOTE: a state carrying a `#seed` ref IS a valid F24 target (the growing states ARE seed-derived, e.g.
    // `(List.push #seed22 t)`); it binds correctly BECAUSE the #st let nests INNER to the #seed wrap and the
    // single reparent_under_handle_site resolves the whole tree in one pass (seed-graft contract, v-rb) — the
    // #st init must be registered BY REFERENCE to the original node (attached #seed), never deep_fresh_copy'd.
    if reaches_any_perform(db, next_state) {
        return false;
    }
    next_state_produces_accumulating_op(db, next_state, 0)
}

/// Recursion bound for the accumulating-op reach — a threaded state forwards through at most a few
/// `let`/`match`/`if`/name hops before its producing op; well above any real fold shape (mk21 is depth 3:
/// `let` → binder `Ref` → `match` arm). A deeper chain declines to fire (leaves the exponential in place —
/// the safe floor, never a miscompile).
pub(crate) const F24_REACH_LIMIT: u32 = 16;

/// Does evaluating `node` PRODUCE the result of an accumulating collection op (`is_accumulating_collection_prim`)?
/// Follows the value-producing spine only — an application HEAD that is an accumulating `Prim`, or a
/// `Ref`/`Let`-body/`Match`-arm/`If`-branch that forwards to one — so a growing op reached through the mk21
/// `let`-bound `match` fires while a benign compound (arith, record, projection, a plain call whose callee we
/// don't follow) does not. Bounded by `F24_REACH_LIMIT`.
/// Does an accumulating op's ELEMENT arg reference a name bound to a PRIOR DISPATCH RESULT — a body
/// let/do-binding the fold FRESHENED to `#{userletter}{id}` (`freshen_walk`, effects.rs ~7043/7076, which
/// renames every body-lifted binder to `#{name}{arena-id}` as it inlines the interposing let/do)? Such a
/// binding's value came from an EARLIER dispatch of this handler (mk2: `a = (Reg.touch n)`, referenced by
/// dispatch-2's op-arg `(+ #a64 1)`), so the growth is DATA-CHAINED on a prior dispatch's OUTPUT, not a
/// straight-line accumulator: it does not multiplicatively re-embed (mk2 baseline 4838 bytes, no blowup),
/// and firing the `#st` bind ORPHANS that resume-value binding when the drain re-scopes the body (CDZ0101
/// `#a…`).
///
/// SYNTACTIC signal (resolution is unavailable — at the arm-return thread site a body binder resolves to
/// `Poison(Unbound)`, indistinguishable from an outer param, so a resolve/reach test can't separate them):
/// a `#`-prefixed name that is NOT one of the KNOWN accumulation synthetics (`#st` slot bind, `#seed` handle
/// seed, `#cv` op-arg lift, `#kv` perform-result lift) is a `freshen_walk`-renamed BODY binding = a prior
/// dispatch result. A plain outer PARAM (`n`) stays a BARE name (never `#`-prefixed), so a real grower whose
/// op-arg reads `n` (bf1 `(+ 10 n)`) still fires — and its free-var aliasing is handled by the selective
/// copy. Every prelude/user op name (`+`, `UInt8`, `wrap`, `bin`) is bare too. Bounded by `F24_REACH_LIMIT`.
pub(crate) fn element_arg_reaches_prior_dispatch_result(
    db: &mut Db,
    node: StructId,
    depth: u32,
) -> bool {
    if depth > F24_REACH_LIMIT {
        return false; // too deep — do NOT over-exclude (a missed exclusion only leaves a benign case firing;
        // over-exclusion would disable the fix on a real accumulator — the worse failure)
    }
    if let Some(nm) = db.ast.as_name(node) {
        // Two `#`-prefixed shapes in an ELEMENT-arg position (elem/val/key — NOT the collection operand
        // args[0], which the caller skips) mean the growth is NOT a straight-line accumulator, so decline:
        //   (a) a `#seed` — a HANDLE SEED threaded as DATA. In a straight-line accumulator the only `#seed`
        //       is the collection operand (args[0], the accumulation itself — pfxM/f24-list `(Map.insert
        //       #seed30 …)`). A `#seed` in an ELEMENT arg is a CROSS-HANDLER seed woven into the added value
        //       (xh1: an inner `put` dispatched inside an outer `S` arm, `(Map.insert #st (+ #seed103 1)
        //       #seed103)` — `#seed103` is the OUTER S-handler's seed). The inner `#st` embeds the outer
        //       `#seed` whose binder is grafted only when the OUTER handler returns (AFTER the inner drain), so
        //       the bind orphans it (CDZ0101 `#seed`); and the bind does not even linearize the nested shape
        //       (still exponential) — trunk emits xh1 correctly (219KB, no rustc-SIGSEGV), so declining loses
        //       nothing. `#cv`/`#kv` (op-arg / perform-result lifts) are likewise not straight-line accumulation.
        //   (b) any OTHER `#`-prefixed name — a `freshen_walk`-renamed BODY binding (`#a64`, mk2's `a` = a prior
        //       dispatch's resume value), the data-chained-dispatch case.
        // The ONLY `#`-name that is NOT a decline signal here is `#st` (a PRIOR slot bind threaded forward as
        // the accumulation — but that too is normally the collection operand; in an element arg it would be a
        // slot read, still fine to keep). So: decline on any `#`-name EXCEPT `#st`.
        return nm.starts_with('#') && !nm.starts_with("#st");
    }
    match db.ast.get(node).clone() {
        Struct::Atom(_) => false,
        Struct::List(children) => children
            .iter()
            .any(|&c| element_arg_reaches_prior_dispatch_result(db, c, depth + 1)),
    }
}

pub(crate) fn next_state_produces_accumulating_op(db: &mut Db, node: StructId, depth: u32) -> bool {
    if depth > F24_REACH_LIMIT {
        return false;
    }
    match resolved_of(db, node) {
        // An accumulating op is applied as `(Map.insert m k v)` etc. Its HEAD is a member access
        // (`Map.insert` → `Resolved::Member`), whose intrinsic is read off the `(meta apply)` channel by
        // `meta_apply_of`; a direct `Prim` head (a desugared intrinsic) is read by `prim_of`. Consult both,
        // exactly as `lower.rs`'s application dispatch does (`meta_apply_of(head).or_else(prim_of(head))`).
        Resolved::Apply { head, args } => {
            let is_accum = crate::eval::meta_apply_of(db, head)
                .or_else(|| crate::eval::prim_of(db, head))
                .is_some_and(is_accumulating_collection_prim);
            // DATA-CHAINED-DISPATCH EXCLUSION (mk2): if an ELEMENT arg (any arg after the first — the
            // collection operand) reaches a name whose BINDING is a PRIOR DISPATCH RESULT (a resume-value
            // let-binding of THIS handler — its init reaches a discharged perform), the growth is data-chained
            // on a prior dispatch's OUTPUT, not a straight-line accumulator: (i) it does not multiplicatively
            // re-embed (mk2 baseline 4838 bytes, no blowup), and (ii) firing the `#st` bind orphans that
            // resume-value binding when the drain re-scopes the body (CDZ0101 `#a…`). EXEMPT: the collection
            // operand `args[0]` (it IS the accumulation — the slot/seed), a plain outer PARAM like main's `n`
            // (bf1's `(+ 10 n)` — `n` has no init/no perform, so a real bytes-frame grower still FIRES + is
            // fixed by the selective-copy), and any arm-inner pure let (`kk`=`Map.len`, `t`=`prev+v` — no
            // perform behind them). The signal is precise: a PERFORM behind an element-arg name = a prior
            // dispatch's resume value.
            is_accum
                && !args
                    .iter()
                    .skip(1)
                    .any(|&a| element_arg_reaches_prior_dispatch_result(db, a, 0))
        }
        // A bare name / prior `#st` ref → follow to its bound value (mk21's `m2` → the `match`).
        Resolved::Ref { value } => next_state_produces_accumulating_op(db, value, depth + 1),
        // A `let` forwards its VALUE from the body; the body is typically the binder name, resolved via `Ref`.
        Resolved::Let { body, .. } => next_state_produces_accumulating_op(db, body, depth + 1),
        // A `match`/`if` produces a grown collection if ANY reachable branch does — but ONLY when reached
        // DEEPER (depth > 0), i.e. inside a `let`-binding init whose RESULT is bound and threaded as a name
        // (mk21: `(let ((m2 (match … (Map.insert …) …))) m2)` — the match is bound to `m2` atomically). A
        // TOP-LEVEL conditional-valued next-state `(if c (List.push s v) s)` / `(match … (grow) (s))` is the
        // two-hole IF/MATCH-PEEL shape (`peel_resume_from_arm_body`): it already threads correctly, and the
        // refold REBUILDS its condition/scrutinee via push_list — so registering it BY REFERENCE for the `#st`
        // bind detaches a shared op-arg node (the `(> v 10)`, `v`↦body-free-var `n`) → a false CDZ0101 unbound
        // (breaker ts1, #2336). Declining to fire on a top-level conditional leaves the (small, non-blown-up)
        // peel shape byte-identical — the safe floor (a top-level `if` where BOTH branches grow would miss the
        // size win, but no such shape blows up in the corpus, and a false "not growing" is never a miscompile).
        Resolved::Match { arms, .. } if depth > 0 => arms
            .iter()
            .any(|&(_, body)| next_state_produces_accumulating_op(db, body, depth + 1)),
        Resolved::If { then_, else_, .. } if depth > 0 => {
            next_state_produces_accumulating_op(db, then_, depth + 1)
                || next_state_produces_accumulating_op(db, else_, depth + 1)
        }
        // SEE-THROUGH a TUPLE PROJECTION `(. t idx)` into the tuple's element `idx`. The multi-resume peel
        // may thread the next-state as a projection of a let-bound `(tuple <value> <state>)` — bound once
        // in scope so a value AND a threaded next-state share one slot (the tpwJ cross-scope collapse). A
        // GROWING state (`(Map.insert m k v)` / `(List.push s v)`) wrapped as that tuple's `idx` element
        // would otherwise be HIDDEN from this detection (a `Proj` matches none of the arms above → `false`),
        // so the `#st` per-dispatch bind would NOT fire and the FINDING-24 exponential blowup would RETURN
        // for a growing-state handler. Resolve the projected operand to its `(tuple …)` construction (a
        // direct tuple form, or a `Ref`/`Let` chain ending at one) and recurse into element `idx`, so a
        // growing element inside a threaded tuple is still detected and `#st` still fires. `None` if the
        // operand does not reach a tuple with that element (a non-tuple projection is not an accumulator).
        Resolved::Proj { operand, index } => tuple_elem_of(db, operand, index)
            .is_some_and(|elem| next_state_produces_accumulating_op(db, elem, depth + 1)),
        _ => false,
    }
}

/// The `index`-th element expression of the tuple `(tuple e0 e1 …)` the node at `operand` reaches — a DIRECT
/// tuple construction, or one reached through a `Ref` (a `let`-binder to a tuple init) or a `Let` body.
/// `None` if `operand` does not reach a tuple, or the tuple has no such element. Used by the tuple-projection
/// see-through arm of the growing-state detection so a `(. t idx)` next-state is inspected at its real
/// element rather than treated as opaque (which would hide a growing state from the `#st` bind — F24).
pub(crate) fn tuple_elem_of(db: &mut Db, operand: StructId, index: usize) -> Option<StructId> {
    // Follow a `Ref` (let-binder → its init) / `Let` (→ body) chain to the tuple construction, bounded.
    let mut cur = operand;
    for _ in 0..=F24_REACH_LIMIT {
        // A direct tuple form — `compound_form_of` reads the native `#tuple` ctor-leaf head too (not only the
        // `("tuple" …)` name/string alias), so M3-nativized tuples are recognized (dc02-class hardening, #5340).
        if let Some(elems) = db
            .ast
            .compound_form_of(cur, crate::ast::CompoundCtor::Tuple)
        {
            return elems.get(index).copied();
        }
        match resolved_of(db, cur) {
            Resolved::Ref { value } => cur = value,
            Resolved::Let { body, .. } => cur = body,
            _ => return None,
        }
    }
    None
}

/// Thread a conditional BRANCH / match ARM body, returning its rewrite AND its OUT-STATE (per slot) — the threaded
/// state each slot has after the branch runs. The `If`/`Match` arms use this to MERGE per-branch out-states
/// into a conditional-valued out-state so a SIBLING that reads state after the conditional (a recursive-call
/// operand, a `do`-continuation) sees the advance a branch perform made — fixing the recursive-branch-perform
/// self-recursive faces + the through-block fold (the If-arm previously returned the pre-branch post-condition
/// state, dropping branch advances). On a branch-local abort, the out-state is the incoming `states`
/// unchanged (the abort discards the continuation, so no advance is observable past it).
pub(crate) fn thread_branch_local_abort_with_out(
    db: &mut Db,
    branch: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, Vec<StructId>)> {
    let before = ctx.abort_value.get();
    let states_in = states.clone();
    let (rbranch, out) = thread_bounded(db, branch, states, ctx, inline_depth)?;
    let after = ctx.abort_value.get();
    // A NEW abort fired while threading THIS branch → it is local to the branch: use the abort value as
    // the branch's rewrite and restore the cell to its prior state (so a sibling branch / the handle is
    // not collapsed). Its out-state is the incoming state (the abort abandons the continuation). If no new
    // abort fired, keep the ordinary threaded rewrite + its threaded out-state.
    if after != before
        && let Some(abort) = after
    {
        ctx.abort_value.set(before);
        // ABORT-FOLD do-SHAPE inside a BRANCH / match-ARM body (the branch/arm-body face of the abort-outer-
        // advance fix — this helper threads both `if` branches AND `match` arm BODIES; sibling faces landed
        // separately: direct-handle-body do-shape #2002, strict-operand #2010, match-SCRUTINEE #2017 — a
        // perform in a match SCRUTINEE is on the strict spine and threaded normally, NOT here). A FOREIGN
        // perform (an OUTER handler's effect) on the branch's strict spine BEFORE the abort has committed
        // its advance and must survive — but the branch-local collapse returns the BARE `abort` value,
        // discarding `rbranch` (which the do-arm already built as the sound `(do <foreign…> <abort-value>)`).
        // `(if c (do (A.tick) (B.bail 99)) …)` / `(match k (_ (do (A.tick) (B.bail 99))))` under B then reads
        // the outer `(A.get)` at the pre-advance state (109 vs 110). Use `rbranch` (a `do`-form reaching a
        // foreign perform) as the branch rewrite so the ENCLOSING fold discharges the prefix; its out-state
        // is still `states_in` (the abort abandons THIS branch's continuation, so no advance is observable
        // PAST the conditional — only the foreign prefix's own effect escapes, carried inside the `do`). Same
        // do-form gate + parented-foreign-reach as the reduce_handle do-shape fold; a non-do `rbranch`
        // (a bare `(B.bail 7)` / a `(+ 100 (B.bail 7))` collapsed to `7`) keeps the bare-abort value.
        if db.ast.as_form(rbranch, "do").is_some() && body_reaches_foreign_perform(db, branch, ctx)
        {
            Some((rbranch, states_in))
        } else {
            Some((abort, states_in))
        }
    } else {
        Some((rbranch, out))
    }
}

/// Bound on cross-function INLINE depth during threading. A handled body that inlines callees deeper
/// than this DECLINES rather than unrolling without end (`reference-compiler.md` §An Unbounded Handler
/// Context Declines) — the safe backstop against a RECURSIVE effectful callee slipping past the
/// `is_recursive` exclusion (β-reduction produces a fresh non-self-referential copy each inline, so a
/// naive `is_recursive` on the copy reads false and would unroll forever). E3's proper per-context
/// specialization handles a recursive callee; anything the specialization does not cover, this bound
/// declines. Set well above any real non-recursive inline chain in the corpus.
pub(crate) const THREAD_INLINE_LIMIT: u32 = 64;

pub(crate) fn thread_bounded(
    db: &mut Db,
    node: StructId,
    states: Vec<StructId>,
    ctx: &HandlerCtx,
    inline_depth: u32,
) -> Option<(StructId, Vec<StructId>)> {
    if inline_depth > THREAD_INLINE_LIMIT {
        return None; // an unbounded inline chain — decline (a recursive callee the spec path missed)
    }
    match resolved_of(db, node) {
        // A PERFORM `(E.op args…)` of a discharged operation: resolve to its arm, substitute the arm's
        // params ↦ (rewritten) args and its state binder ↦ the CURRENT state OF THAT OP'S SLOT, and rewrite
        // the arm body's TAIL resume to the resume VALUE, threading the resume's next-STATE forward IN THAT
        // SLOT (the other slots pass through unchanged). This is what lets nested handlers over one
        // recursive callee thread each effect's state independently.
        Resolved::Apply { head, args } if is_perform(db, head, ctx).is_some() => {
            let (decl, idx) = is_perform(db, head, ctx).unwrap();
            let arm = ctx.arms.get(&(decl, idx))?.clone();
            let slot = ctx.slot_of(decl)?;
            // Thread state through each argument left-to-right (an argument may itself perform).
            let mut cur = states;
            let mut rewritten_args = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rewritten_args.push(ra);
                cur = next;
            }
            // EFFECT-DUPLICATION GUARD. The arm body β-reduces by SUBSTITUTING each argument for its
            // parameter, so a param used more than once COPIES its argument. If that argument reaches a
            // perform — of THIS handler's op OR a FOREIGN one (`arg_reaches_any_perform`) — duplicating it
            // would run the effect once per use: `(E.op (tuple (A.get) (A.get)))` whose arm reads `(. p 0)`
            // AND `(. p 1)` would thread four gets, not two (a miscompile). Decline instead — this fold
            // cannot represent it without a let-binding the tail surface does not model. A param whose arg
            // reaches NO perform (a literal, a name, a PURE compound like `(record (a 3) (b 4))`) duplicates
            // no effect, so multi-use is fine; a single-use param likewise runs its arg exactly once. This
            // mirrors the applied-lambda pre-reduction's pure-args soundness guard. (`arg_reaches_any_perform`
            // is used, NOT `body_reaches_foreign_perform`, because the latter over-reports a record literal's
            // field-pair as an unresolvable call — spuriously declining a pure record argument.)
            // OP-ARG LET-LIFT (cross-handler inline-arg-position completeness fix, breaker xh1/xh2). A
            // MULTI-USE param whose arg PERFORMS would duplicate the effect on substitution (the guard below).
            // For a FOREIGN perform arg — an op THIS handler does not discharge, e.g. `(B.put (A.get))` under
            // B where `A.get` homes to the enclosing A — the sound rewrite is to BIND it once to a fresh
            // `#cv` let and duplicate the pure ref, exactly the Site-5 hoist for performing conditions (verified
            // by xh2: the hand-let-bound spelling `(let ((x (A.get))) (B.put x))` FOLDS while the inline arg
            // declines). Lift each such arg BEFORE building the arm body: replace the arg with a `#cv` ref and
            // remember the binding, then wrap the whole arm result in the `let`s (the perform runs once, in
            // arg-evaluation order, then B's arm reads the pure `#cv` twice). Gated to FOREIGN performs (a
            // perform of THIS handler's OWN op as a multi-use arg still declines — it needs threading, not a
            // plain let-bind; a distinct, harder shape). A single-use or pure arg is untouched.
            let mut arg_lifts: Vec<(StructId, StructId)> = Vec::new(); // (#cv binder, arg)
            if arm.params.len() == rewritten_args.len() {
                // Collect indices of multi-use params whose arg performs; decide lift-vs-decline per arg.
                let mut to_lift: Vec<usize> = Vec::new();
                for (i, &p) in arm.params.iter().enumerate() {
                    let a = rewritten_args[i];
                    let refs = count_param_refs(db, arm.body, p);
                    if is_unit_param(db, p) || !arg_reaches_any_perform(db, a, ctx) || refs == 1 {
                        // A unit param, a PURE arg (nothing to preserve — a dropped pure arg stays elided,
                        // core-semantics.md #A Trap Occurs Only Where Its Computation Is Observed / the
                        // unused-parameter elision in 09-functions), or a SINGLE-use performing arg (naive
                        // substitution places it once → the perform runs exactly once, no lift needed).
                        continue;
                    }
                    // A performing arg used a number of times OTHER than once — `refs == 0` (the arm IGNORES
                    // its param, so substitution DROPS the arg) or `refs >= 2` (the arm reads it more than
                    // once, so substitution DUPLICATES it). Naive substitution would LOSE the perform (dropped)
                    // or RE-RUN it (duplicated). If the arg performs a FOREIGN op (none of it is THIS handler's
                    // discharged op that would need threading), LET-LIFT it so the perform runs EXACTLY ONCE
                    // for its effect: a DROPPED foreign perform still threads its state advance to the outer
                    // handler (an unread op-arg's foreign perform must not vanish — its declared operation is
                    // observable, capabilities-and-effects.md #A Handler Threads State; the same class as the
                    // do-discarded-foreign-perform preservation below), and a DUPLICATED one is bound once and
                    // read via the pure `#cv` ref (xh1/xh2). A perform of THIS handler's discharged op that
                    // would be DUPLICATED (`refs >= 2`) needs threading, not a plain let — decline (safe floor).
                    // (A DROPPED (`refs == 0`) discharged-op arg is left untouched here — out of scope; the
                    // enclosing fold's own threading covers the single-handler case, and a cross-level drop of
                    // a discharged-op arg is a separate, harder arc.)
                    // We already established `arg_reaches_any_perform` TRUE via the guard above, so test only
                    // the discharged-op half here — one traversal per candidate (github-liaison/Copilot #2120).
                    if !subtree_reaches_discharged_op(db, a, ctx) {
                        to_lift.push(i);
                    } else if refs >= 2 {
                        return None;
                    }
                }
                for i in to_lift {
                    let a = rewritten_args[i];
                    // `#cv{StructId}` is a globally-unique fresh binder. Collision-safety rests on TWO facts
                    // (github-liaison #2156 corrected the earlier "unspellable" claim): (1) the StructId is the
                    // arena index of the arg node `a` — a MONOTONIC, never-reused counter (`push_atom`/
                    // `push_list` = `StructId(structure.len())`), so two lift sites never share a `#cv{…}` name
                    // (the Site-5 performing-condition hoist keys on its own distinct `if`/`match` node id). (2)
                    // A user can never introduce a `#cv{N}` BINDER to capture/shadow a live lift site: a
                    // `#`-leading name is a refutable CONSTRUCTOR pattern, illegal in binding position (rejected
                    // CDZ0210). So `#cv…` is UNBINDABLE, not lexically unspellable — a backtick `` `#cv0` `` DOES
                    // lex as a REFERENCE, but reference position can't capture; only a binder could, and a
                    // `#cv…` binder is rejected. Pinned by `the_op_arg_lift_cv_binder_namespace_cannot_be_
                    // captured_by_a_user_binder`. No centralized gensym needed — arena-id monotonicity + binder-
                    // position rejection guarantee it (github-liaison/Copilot #2120/#2156 reviews).
                    let cv_name = format!("#cv{}", a.0);
                    let cv_binder = db.push_name(&cv_name);
                    let cv_ref = db.push_name(&cv_name);
                    arg_lifts.push((cv_binder, a));
                    rewritten_args[i] = cv_ref;
                }
            }
            // The arm binds its params to the args and its state binder to THIS SLOT's current state.
            // Substitute both into the arm body (a capture-safe arena substitution), then extract the tail
            // resume.
            let mut subst: HashMap<StructId, StructId> = HashMap::default();
            if arm.params.len() == rewritten_args.len() {
                for (&p, &a) in arm.params.iter().zip(&rewritten_args) {
                    // The `()` placeholder param of a nullary op binds nothing — skip it.
                    if !is_unit_param(db, p) {
                        subst.insert(p, a);
                    }
                }
            } else if arm.params.len() == 1 && rewritten_args.is_empty() {
                // A NULLARY perform `(E.op)` for a `(-> Unit T)` op: the arm's single parameter is the
                // ELIDED unit argument — either written `()` (binds nothing) or a named binder `u` (binds
                // to unit). Supply a synthesized `unit` so a named unit binder resolves.
                let p = arm.params[0];
                if !is_unit_param(db, p) {
                    let unit = db.push_list(vec![]); // `()` — the unit value
                    subst.insert(p, unit);
                }
            } else {
                // Any other arity mismatch — decline.
                return None;
            }
            // A FINDING-24 `#st{node}_{slot}` growing-state bind (set at the resume path below) is in scope
            // ONLY inside the RESUME continuation's `drain_and_wrap`. An ABORTIVE arm that READS the state
            // gets `cur[slot]` = that `#st` NAME substituted for its state binder, but the strict-op abort
            // collapse emits the abort value OUTSIDE that scope and drains the `#st` binds for `do`-form
            // bodies only — so a `+`-form abort's `#st` reference LEAKS unbound → a spurious CDZ0101 on a
            // well-formed program (breaker abx3/ab4). Decline HONESTLY (`reduce_handle` → None →
            // HANDLER_NOT_REDUCIBLE todo) rather than emit the leaked `#st`. NARROW: fires only when the
            // state IS a `#st` name (a prior resume grew it), this arm is abortive, AND its body references
            // the state — an abort-only handle (abx5: state is the seed, not `#st`) or a state-ignoring abort
            // arm (abx4) is untouched. The FULL fold (drain the `#st` into the strict-op abort value + the
            // outer-observation soundness) is a separate increment.
            if ctx.abortive.contains(&(decl, idx))
                && db
                    .ast
                    .as_name(cur[slot])
                    .is_some_and(|n| n.starts_with("#st"))
                && count_param_refs(db, arm.body, arm.state) > 0
            {
                return None;
            }
            subst.insert(arm.state, cur[slot]);
            let arm_body = crate::eval::beta_reduce(db, arm.body, &subst);
            // ABORTIVE arm (E4): the arm never resumes, so performing it ABANDONS the surrounding
            // computation — the arm body value becomes the handle's value. Record it in the ctx's abort
            // cell (interior-mutable) and return it as this node's value; the enclosing strict context is
            // dead. Two callers read the cell: `reduce_handle` (an UNCONDITIONAL abort collapses the whole
            // handle to this value) and `thread_branch_local_abort` (a TAIL abort in an `if` branch is
            // local — it uses this value for the branch and restores the cell). An UNSOUND conditional
            // abort (non-tail branch, condition) was declined by `body_has_unsound_abortive_perform` in
            // `reduce_handle` before threading, so it never reaches here. State does not thread (abandoned).
            if ctx.abortive.contains(&(decl, idx)) {
                // FIRST abort wins. Threading proceeds in EVALUATION ORDER (left-to-right), so the first
                // abortive perform to fire is the leftmost — and it ABANDONS the rest of the computation,
                // including any later abortive perform on the same strict spine. If the cell is ALREADY set
                // (an earlier operand aborted), that earlier value stands: do NOT overwrite it, and return
                // it so this dead position carries the surviving value. `(+ (Bail.bail 7) (Bail.bail 9))`
                // must yield 7 (the first), never 9 — without this guard the second perform overwrote the
                // cell as threading continued past the first abort (a miscompile).
                if let Some(existing) = ctx.abort_value.get() {
                    return Some((existing, cur));
                }
                // OP-ARG LET-LIFT on the ABORT value too (strict-fold #17 face-4 RESIDUAL, corpus-bugfix
                // 2026-08-11). When the aborting arm IGNORES its param, its unread arg — a FOREIGN perform
                // `(B.tick)` this handler does not discharge — was dropped by the `beta_reduce` above, so the
                // outer handler never threaded its state advance (a later `(B.tick)` read stale state:
                // `(A.bail (B.tick))` under B → 55003 not 55004). The RESUMPTIVE path already wraps its result
                // in `arg_lifts` (below); the abort path returned the bare arm value, skipping them. Wrap the
                // abort value in the SAME `#cv` lets so the foreign perform still runs ONCE for effect before
                // the abort collapses the continuation — the abort discards the CONTINUATION, not an
                // already-committed foreign advance (the do-shape abort-outer-advance rule, same spirit as the
                // `(do <foreign…> <abort-value>)` preservation in `reduce_handle`). A resumptive arg drop and an
                // abortive arg drop are the SAME face; both must thread the dropped foreign dispatch. Nothing
                // lifted (a pure/unread-pure arg, the common case) → `arm_body` unchanged, byte-identical.
                let aborted = if arg_lifts.is_empty() {
                    arm_body
                } else {
                    let mut wrapped = arm_body;
                    for &(binder, arg) in arg_lifts.iter().rev() {
                        let let_head = db.push_name("let");
                        let pair = db.push_list(vec![binder, arg]);
                        let bindings = db.push_list(vec![pair]);
                        wrapped = db.push_list(vec![let_head, bindings, wrapped]);
                    }
                    wrapped
                };
                let copied = copy_pure(db, aborted);
                // CASE-1 EFFECT NON-LOCAL EXIT (v-effects). When this handle is the WHOLE function body
                // (`whole_fn_body`) AND we are NOT threading a specialized recursive callee
                // (`!in_recursive_specialize` — an abort there lands in the callee's frame, where a bare
                // wasm return cannot abandon the CALLER's pending continuation; that case stays declined as
                // the deferred tagged-return vertical), lower the abort to a `Core::HandleAbort` embedded AT
                // THIS position instead of the whole-handle COLLAPSE. The collapse (`abort_value` → the bare
                // value as the ENTIRE handle result) is sound only for an UNCONDITIONAL abort; for a
                // CONDITIONAL abort the tail fold cannot lift (e.g. under an effectful sibling), it would
                // wrongly drop the other path. `HandleAbort` instead KEEPS the surrounding structure and does
                // a non-local RETURN at runtime — sound because the handle IS the whole def body, so the
                // returned value is exactly the function result. `handle_id` is unused by the CASE-1 emit
                // (`emit(value); Return`); it is filled with `copied` as a valid placeholder occurrence.
                if ctx.abort_as_handleabort.get() && !ctx.in_recursive_specialize.get() {
                    let ha_ty = crate::infer::type_of(db, copied);
                    let ha = crate::lower::synth_core(
                        db,
                        crate::core::Core::HandleAbort {
                            value: copied,
                            handle_id: copied,
                        },
                        ha_ty,
                    );
                    return Some((ha, cur));
                }
                ctx.abort_value.set(Some(copied));
                return Some((copied, cur));
            }
            // The arm body must reduce to a TAIL `(resume value next-state)` — the value becomes the
            // perform's result; the next-state threads forward IN THIS SLOT. Two shapes:
            //   * a bare `(resume v s)` — the value is `v`.
            //   * a `(do stmt… (resume v s))` — an INTERPOSING/FORWARDING arm that runs side-effecting
            //     STATEMENTS (a perform of ANOTHER effect — an outer handler's, or a host op — recorded
            //     before forwarding) then resumes. The statements must RUN, so the perform's result is a
            //     `(do stmt… v)`: the statements sequenced (folded later under their own enclosing
            //     handler / emitted as host calls), then the resume value. This is what lets an inner
            //     handler INTERPOSE on a delegated effect — count it via an outer effect, then forward.
            // [tpwJ, breaker 2026-08-16] CROSS-SCOPE shared-`let` arm: when a `let` binder is referenced in
            // BOTH a resume value AND its next-state, the DISTRIBUTE peel below splits it into two separate
            // matches/ifs whose two copies land in different emit scopes — a multi-use binding kept in one,
            // copy-propagated away in the other → cross-scope "no local slot". Serve it via the COLLAPSED path
            // instead: rewrite the arm's tail resumes to `(tuple v s)` IN PLACE (structure intact, the binder
            // stays bound once in its own arm with its match-pattern binders live), bind that single
            // `(value, state)` tuple ONCE to a fresh `#st`-prefixed name (materialized by `drain_and_wrap`
            // OUTSIDE both, force-kept via the `should_keep_binding` `#st` carve-out), and PROJECT
            // value = `(. t 0)`, next_state = `(. t 1)`. Both projections read the ONE shared slot → no
            // cross-scope orphan. Falls back to the distribute peel if the collapse does not model the shape.
            // OPTION A (v-inference-confirmed): fire the COLLAPSE only when the combined is CLOSED at the
            // DRAIN level — i.e. every op-arg substituted into it is drain-safe (a literal / an outer param /
            // an already-`#st`-bound state ref), NOT a HANDLE-BODY or ARM-LOCAL `let` binding. The collapsed
            // tuple is materialized by `drain_and_wrap` OUTSIDE the arm, so a substituted op-arg that is a
            // body-bound name (which the fold LIFTS to a `#`-name bound near the perform site) would resolve
            // out of scope there → CDZ0101 (neu1's `#big70`). A body-bound-op-arg arm STAYS on the distribute
            // path (where it already folds correctly), so this narrowing closes the literal-op-arg family
            // (tpwJ + the tpw ladder) with ZERO regression and defers the body-bound-op-arg cross-scope class
            // as an honest follow-up. `Resolved::Ref` is exactly a `let`-binding reference; a `Param`/literal/
            // member/prim is not, so it is the precise drain-safe discriminator.
            // OPTION A (drain-closed): fire the COLLAPSE only when the built `combined` is HOISTABLE to the
            // drain level — it must reference no LIFTED `#`-name that is bound NEAR the perform site (a `#cv`
            // op-arg/condition lift, a `#big…`/freshen-walk body-let lift), since `drain_and_wrap` materializes
            // the combined OUTSIDE the arm and such a name would resolve out of scope there → CDZ0101 (neu1's
            // `#big70`). `#st`/`#seed` names ARE bound at the drain level (the per-dispatch state bind and the
            // handle-seed lift), so a threaded `(. #st… 1)` incoming state is fine — that is what keeps a
            // uniformly-literal-arg handler (tpwJ) ALL on the collapse path across dispatches. A mixed handler
            // whose combined reaches a body-lifted name (neu1) stays ALL on the distribute path (where it
            // already folds), so this narrowing closes the literal-op-arg family with ZERO regression and
            // defers the body-lifted class as a follow-up. Structural (not resolution-based): a lifted name's
            // resolution is unreliable pre-materialization, but its `#`-prefix is an exact, stable witness.
            // A-TIGHT GUARDS (v-inference-confirmed): the collapse's per-dispatch `#st`-bind + tuple-projection
            // is a STRAIGHT-LINE-thread transform, sound ONLY for a single-slot, non-recursive-driver handler
            // whose threaded state is not a growing collection. Fire it only there; the diverse remainder stays
            // on the proven distribute path (named follow-up classes: recursive-driver / growing-state /
            // cross-handler shared-let). (1) SINGLE-SLOT — a merged multi-slot ctx (a cross-handler shape,
            // xh1) threads several states; the single-name projection does not model that. (2) NOT a recursive
            // driver — `thread_returning_tuple` infers the recursive result type from the threaded shape, and a
            // tuple-projected next-state leaves it undetermined (rq3). (3) NOT a growing-collection next-state —
            // a `(. t 1)` projecting a `Map.insert`/`List.push` state would need the base's see-through arm
            // AND re-introduces the F24 surface here (lru1/rrb1); defer it.
            let mut arm_next_states: Vec<StructId> = Vec::new();
            let arm_state_grows = arm_resume_next_states(db, arm_body, &mut arm_next_states)
                .is_some()
                && arm_next_states
                    .iter()
                    .any(|&s| next_state_is_growing_compound(db, s));
            // GROWING-STATE was originally excluded from the collapse (F24 caution: a `(. #st_vs 1)` projecting
            // a `List.push`/`Map.insert` next-state re-introduces the F24 exponential + a see-through arm). But
            // for the mid-arm-FOREIGN-PERFORM shared-let shape (xhsGrow) the DISTRIBUTE fallback MISCOMPILES (the
            // shared-binder divergence), and the collapse's per-dispatch `#st_vs` bind IS the F24 fix — so
            // collapsing it is LINEAR (verified: a 6-dispatch xhsGrow compiles in ~0.4s, no blowup) AND correct
            // (differential-confirmed variant==inline-control at 2 and 6 dispatches). So ALLOW the collapse for a
            // growing-state arm that has a foreign-perform-with-args let-init — the correct fold, superseding the
            // safe-floor decline below (which still fires when the frozen combined is non-hoistable). A
            // growing-state arm WITHOUT such a perform (gws1, lru1/rrb1) stays excluded (distribute-correct).
            let collapse_ok = ctx.collapse_enabled.get()
                && ctx.slots.len() == 1
                && !ctx.in_recursive_specialize.get()
                && (!arm_state_grows || arm_has_let_init_reaching_arg_perform(db, arm_body));
            let mut collapsed: Option<StructId> = None;
            let is_candidate = {
                let mdo = ctx.multi_dispatch_ops.borrow().clone();
                arm_is_collapse_candidate(db, arm_body, (decl, idx), &mdo, &ctx.arms)
            };
            if collapse_ok
                && is_candidate
                && let Some(combined0) = peel_tuple_value_state(db, arm_body)
            {
                // FREEZE any mid-arm FOREIGN-perform argument into a kept `#st`-name computed against the
                // INCOMING state (xhs1). Bind-once of the shared binder is necessary but NOT sufficient: when
                // the OUTER handler later folds the embedded foreign perform, it RE-THREADS the perform's arg
                // against its own pass, so a state-dependent arg expression is re-derived (wrong incoming
                // state) — the resume copy stays I-frozen (right) while the arg copy diverges (wrong). Freezing
                // the arg to a force-kept `#st` slot at THIS (inner) fold gives the outer handler an opaque
                // frozen name, not a re-derivable expression. (v-inference's foreign-perform-arg analogue of
                // the `#st` state-bind.)
                let freeze_mark = ctx.pending.borrow().len();
                let combined = freeze_foreign_perform_args(db, combined0, ctx);
                if combined_hoistable_to_drain(db, combined) {
                    collapsed = Some(combined);
                } else {
                    // The freeze speculatively pushed drain-level `#fa` binds; the distribute fallback does not
                    // use the frozen combined, so roll them back (else an unreferenced `#fa` bind materializes).
                    ctx.pending.borrow_mut().truncate(freeze_mark);
                }
            }
            // SAFE-FLOOR (xhsGrow): a GROWING-STATE arm with a mid-arm foreign-perform-with-args let-init has
            // the collapse EXCLUDED by `arm_state_grows`, so it would fall to the distribute peel — which wraps
            // the foreign-perform let-init around BOTH resume slots and mis-threads the shared binder (the xhs1
            // divergence), a SILENT x3 miscompile (43078123 vs the correct 44071111). Decline until the collapse
            // extends to growing-state (the F24 surface). breaker-confirmed no over-fire: keyed on a
            // foreign-perform-with-args let-init + `arm_state_grows`, which no landed corpus case combines (gws1
            // grows but has NO foreign perform; xhs1-D perform but NON-growing → stay collapse-correct).
            if collapsed.is_none()
                && arm_state_grows
                && arm_has_let_init_reaching_arg_perform(db, arm_body)
            {
                return None;
            }
            let (value, next_state) = if let Some(combined) = collapsed {
                let name = format!("#st{}_vs", node.0);
                ctx.pending.borrow_mut().push((name.clone(), combined));
                let value = tuple_proj(db, &name, 0);
                let next_state = tuple_proj(db, &name, 1);
                (value, next_state)
            } else {
                peel_resume_from_arm_body(db, arm_body)?
            };
            // SAFE-DECLINE: a FOREIGN perform DIRECTLY in the threaded NEXT-STATE (as2/as1, breaker
            // 2026-08-05; both-perform gap closed per github-liaison/Copilot #2289). An arm `(resume t (+ t
            // (A.get)))` performing an OUTER effect `A.get` in its next-state expr is unsound as a fold: the
            // next-state threads forward as a state EXPRESSION, so the embedded `A.get` is either DROPPED (a
            // single perform discards the final slot state — as2: `(+ 10 0) + seed = 5`, should be 6) or
            // DUPLICATED across dispatches (multi-perform re-splices the state expr — as1: 63, fits no model).
            // Both are SILENT wrong values, both backends, O0-O3. Decline whenever the RAW resume next-state
            // performs a foreign op — INCLUDING the both-perform `(resume (A.get) (A.get))` where the VALUE
            // also performs one (Copilot #2289: the next-state foreign is dropped REGARDLESS of the value —
            // asb compiled to 56 vs the correct 57 — so a `&& !value-performs` clause let it slip past; that
            // clause is dropped). The proven-correct forms STAY folding because their next-state has NO direct
            // foreign perform: as3 `(resume (+ t (A.get)) t)` — foreign in the VALUE, next-state the bare `t`;
            // as7 `(let ((x (A.get))) (resume t (+ t x)))` — foreign is the let-INIT, resume next-state `(+ t
            // x)` is pure; the interposing `(do (A.tick) (resume v s))` — foreign is a do-STATEMENT, next-state
            // `s`. `arm_resume_next_states` reads the RAW resume child (descending do/let/match WITHOUT wrapping
            // the surrounding binder into it) — unlike `peel_resume_from_arm_body`, which WRAPS the `let`-init/
            // `do`-stmt into both slots and would make as7/interpose look unsound. Runs over the ORIGINAL
            // PARENTED `arm.body` (NOT the post-peel orphans, whose dead parent chain would POISON spec-name
            // resolution — `effect_op_of` resolves the decl-literal; resolving an orphan leaked a
            // `loop#eff3$s1` CDZ0101 into the recursive-fold surface). The structural (non-call-following)
            // `next_state_directly_performs_foreign` distinguishes as2's literal `(A.get)` from a recursive
            // fold that threads an outer effect through a self-call/specialized CALLEE body (never a direct
            // perform in the arm's next-state), so the recursive-fold suite stays folding. The correct FOLD
            // (run the next-state foreign once at dispatch, thread its pure result — the inline analogue of
            // as7's let-lift) is a deeper eval-order arc; decline is the safe floor.
            {
                let mut raw_next_states = Vec::new();
                if arm_resume_next_states(db, arm.body, &mut raw_next_states).is_some()
                    && raw_next_states
                        .into_iter()
                        .any(|s| next_state_directly_performs_foreign(db, s, ctx))
                {
                    return None;
                }
            }
            // `value`/`next_state` are the resume node's own CHILDREN, so their `parent_of` still points at
            // that (now-discarded) `resume` node — an orphan whose parent chain does NOT reach the threaded
            // body's enclosing `(def …)`. If either carries a NAME reference (e.g. a state-threading arm's
            // `(resume s (+ s 1))`, where the substituted state `s` is a reference to the specialization's
            // `$s{k}` state param), splicing it elsewhere leaves that reference resolving against the dead
            // resume node's scope → a spurious CDZ0101 leaking the internal `walk#eff2$s0` name. COPY them
            // (a re-parenting structural copy) so each is detached from the dead resume and receives fresh
            // parentage when spliced into the value/next-state position. (The `do`-wrapped `value` is
            // already a fresh `push_list`; copying it again is a harmless re-parent.)
            //
            // DISTINCT DEEP copies are load-bearing when `value` and `next_state` are the SAME node — a
            // `resume(a, a)` arm (dispatch/done handing the op's arg back AND as the next state) has both
            // children equal, AND when that shared child (or a leaf within it) is a RESOLVE-PINNED bare name
            // (`a` substituted by an inlined helper's arg — `turn(fuel)` → `a`↦`fuel`, a pinned occurrence),
            // `copy_pure`/`beta_reduce` SHARES it (its pinned-name fast-path returns the node as-is to avoid
            // exponential re-resolution on deep inline chains). Sharing puts ONE node in TWO positions (the
            // `+` operand AND the self-call's trailing state arg) — a single-parent-arena orphan: `parent_of`
            // points at only one, so the other occurrence resolves against a foreign scope → CDZ0101 unbound
            // (the effectful-helper-in-a-self-call-arg bug, v-agent-harness Inc-3). `deep_fresh_copy` re-pushes
            // EVERY node fresh (no shared leaf), so each splice gets its own subtree that re-resolves against
            // the specialized def's sig (which carries the driver's own params).
            let value = deep_fresh_copy(db, value);
            // FINDING-24 (exponential fold-lowering): when the perform advances a slot's threaded state to a
            // GROWING COMPOUND ((List.push pre t) / (Map.insert m k v) / (+ s 1)), the NEXT dispatch's init +
            // the body SUBSTITUTE that state expr, and each dispatch re-embeds the prior → O(k^N) Core
            // (`deep_fresh_copy` bakes N distinct fresh trees; `core_of` memo can't collapse StructId-distinct
            // copies; reproduced N=3..7 ~3x/dispatch, rustc SIGSEGVs the async emit). FIX (per-dispatch state
            // let-bind — the #seed-graft principle per dispatch): bind the grown state to a fresh
            // `#st{node}_{slot}` name (into `ctx.pending`, materialized once by `drain_and_wrap`, INNER to the
            // `#seed` wrap at reduce_handle:2736) and thread the NAME forward via `cur[slot]`, so the next
            // dispatch substitutes the SMALL name, not the growing expr → Core LINEAR. NOTE the #st bind must
            // be force-KEPT through emit (should_keep_binding #st-prefix carve-out, v-rust-backend's lower.rs
            // piece) — else copy-prop re-inlines the single-use binding and the blow-up returns; this fold-side
            // emission + that keep co-land as a co-gated pair.
            //
            // SEED-GRAFT CONTRACT (v-rb): the growing state may carry a `#seed` ref (a heap handle-seed
            // let-lift, bound OUTER by apply_seed_wrap; e.g. `(List.push #seed22 t)`). Register the state
            // BY REFERENCE to the ORIGINAL (pre-`deep_fresh_copy`) node so its `#seed` ref stays ATTACHED —
            // a `deep_fresh_copy` here would re-push the `#seed` ref DETACHED (orphan → CDZ0101 unbound
            // `#seed`). Do NOT resolve the #st init now; the single `reparent_under_handle_site` (2748)
            // resolves the whole wrapped tree (#st lets inner + #seed wrap outer) in one pass, so the #st
            // init's #seed ref binds up exactly like the body's own #seed refs. The threaded `#st` NAME is
            // single-use (one occurrence in the next dispatch), so it needs no fresh-copy sharing guard — the
            // deep_fresh_copy the pre-fix code did on next_state is only needed on the NON-bound path.
            let next_state =
                if ctx.bind_growing_state.get() && next_state_is_growing_compound(db, next_state) {
                    let sname = format!("#st{}_{slot}", node.0);
                    let sref = db.push_name(&sname);
                    // SELECTIVE copy: fresh-copy the init so an outer FREE-VAR the op-arg carries (bf1/bf2/bf3:
                    // `v`↦`(+ 10 n)`) is not ALIASED into two arena positions (→ CDZ0101 unbound `n`), while
                    // PRESERVING the `#seed` ref by-reference so a grafted handle-seed (incl a NESTED handler's,
                    // which piece-3's forget snapshot does not reach — xh1) stays attached and resolves as before.
                    // A full deep_fresh_copy fixes the free-var but re-orphans a nested `#seed`; the by-reference
                    // original fixes `#seed` but aliases the free-var — `deep_fresh_copy_keep_seed` does both.
                    let init = deep_fresh_copy_keep_seed(db, next_state);
                    ctx.pending.borrow_mut().push((sname, init));
                    sref
                } else {
                    deep_fresh_copy(db, next_state)
                };
            cur[slot] = next_state;
            // Wrap the perform's result VALUE in the op-arg `#cv` let-lifts (innermost binding = last lifted
            // arg, so bindings evaluate in arg order): `(let ((#cv (A.get))) <value with v↦#cv>)`. The foreign
            // perform now runs ONCE as the let-init (the enclosing fold discharges it), and B's arm reads the
            // pure `#cv` however many times. Nothing lifted → `value` unchanged (byte-identical common case).
            let value = if arg_lifts.is_empty() {
                value
            } else {
                let mut wrapped = value;
                for (binder, arg) in arg_lifts.into_iter().rev() {
                    let let_head = db.push_name("let");
                    let pair = db.push_list(vec![binder, arg]);
                    let bindings = db.push_list(vec![pair]);
                    wrapped = db.push_list(vec![let_head, bindings, wrapped]);
                }
                wrapped
            };
            Some((value, cur))
        }
        // A `do` sequence — `(do e0 e1 … en)`. Evaluate each in EVALUATION ORDER, threading state; the
        // sequence's value is the LAST expression's value, its state the last's next-state. (A `do` is
        // grammar; the resolver does not model a NESTED `do` as an expression at all — it declines — so we
        // cannot rebuild a `do` node. Instead we return the LAST item's rewrite: after folding, each
        // earlier item is a PURE value-expression (its effect was folded into the threaded state), so its
        // value being discarded is exactly `do`'s semantics — evaluate for effect, yield the last. If an
        // earlier item did NOT fold to a pure expression it would carry an unresolved perform and the
        // whole handler would already have declined; and a `do` item that is a bare literal/constant has
        // no effect to preserve. So dropping the earlier rewrites is sound for the tail-resumptive surface
        // this fold serves. (A `do` item with a residual RUNTIME trap is out of scope here — that needs
        // nested-`do` value support, an orthogonal feature.)
        _ if db.ast.as_form(node, "do").is_some() => {
            let items: Vec<StructId> = db.ast.as_form(node, "do").unwrap().to_vec();
            if items.is_empty() {
                return None;
            }
            // LET-LIFT over the continuation. A NON-FINAL do item that inlines a cross-fn effectful helper
            // whose body is a `(let ((x e)) lbody)` — the memoize combinator `store(k) = (let v = … in
            // (put; v))` — binds `x` LOCAL to that item. But `x` is referenced by the perform's OUT-STATE
            // (`put`'s next-state `Map.insert(s, k, x)`) + its substituted arg, which the `do` threads FORWARD
            // to the LATER items. Spliced there, `x` is OUTSIDE its `let` → a spurious CDZ0101 "unbound x"
            // (the sequenced-memoize leak; a FINAL-position such item is fine — nothing threads its state on).
            // Lift the `let` to wrap the whole continuation: `(do (let ((x e)) lbody) rest…)` ≡ `(let ((x e))
            // (do lbody rest…))`, so `x` scopes over `lbody` AND `rest` (whose out-state references it).
            // Sound: the init runs once, before the body + rest, in the same order — only `x`'s visibility
            // widens. Detect via an inline-PREVIEW (β-reduce the item's callee body with its args) so we see
            // the `let` the inline WILL produce; rewrite + re-thread the whole node (fixpoint lifts a second).
            for (i, &it) in items.iter().enumerate() {
                if i + 1 >= items.len() {
                    break; // the FINAL item's state is not threaded on — no escape, no lift needed
                }
                if let Some((binds, lbody)) = inlined_let_of_do_item(db, it, ctx) {
                    let do_head = db.push_name("do");
                    // `(do lbody rest…)` — the let body followed by the remaining items.
                    let mut cont = vec![do_head, lbody];
                    cont.extend_from_slice(&items[i + 1..]);
                    let cont_do = db.push_list(cont);
                    let let_head = db.push_name("let");
                    let lifted = db.push_list(vec![let_head, binds, cont_do]);
                    // Keep the items BEFORE `i` as the outer do prefix (they run first, in order); if `i==0`
                    // the lifted `let` IS the whole node.
                    let rewritten = if i == 0 {
                        lifted
                    } else {
                        let do_head2 = db.push_name("do");
                        let mut ch = vec![do_head2];
                        ch.extend_from_slice(&items[..i]);
                        ch.push(lifted);
                        db.push_list(ch)
                    };
                    return thread_bounded(db, rewritten, states, ctx, inline_depth);
                }
            }
            let mut cur = states;
            let mut last = None;
            // NON-FINAL items that, after rewriting, STILL reach a FOREIGN perform (an effect THIS handler
            // does not discharge) must be PRESERVED — dropping them loses a residual side effect. The
            // collapse-to-last shortcut is sound ONLY for an item that folded to a PURE expression (its
            // discharged effect went into the threaded state). But an INNER handler folding a body
            // `(do (Outer.bump) (Outer.get))` sees `Outer.bump` as FOREIGN: it does not fold it into any
            // slot, so the rewrite leaves the perform intact — and dropping the non-final one silently
            // erases its state advance the moment the OUTER handler folds it (the do-sequenced-outer-perform-
            // under-inner-handle miscompile: `(A.bump)` discarded under an inner `B` reads back stale). Keep
            // each such survivor and rebuild a `do` over them; the enclosing fold re-threads it (or, for a
            // host-delegated foreign op, it lowers as a host-call sequence). A pure-folded item is still
            // dropped — byte-identical to before for the single-handler-depth surface.
            let items_len = items.len();
            let mut kept: Vec<StructId> = Vec::new();
            let abort_before_do = ctx.abort_value.get();
            for (i, it) in items.into_iter().enumerate() {
                let (r, next) = thread_bounded(db, it, cur, ctx, inline_depth)?;
                cur = next;
                // A NON-FINAL item that ABORTS ends the sequence: everything AFTER it is DEAD (never runs at
                // runtime — the abort abandons the continuation). Its rewrite `r` is the abort value, which
                // becomes the do's value. Stop here — do NOT thread the dead suffix (threading it would run
                // its performs / push pending self-call temps for code that never executes, and letting the
                // loop continue would OVERWRITE `last` with the dead FINAL item's rewrite, dropping the abort
                // value → `(do (A.tick) (B.bail 99) (A.tick))` under B mis-yielded the trailing dead `(A.tick)`
                // as the do-value instead of 99, forcing the dead tick: 23/34 instead of 110). A non-final
                // abort at the LAST item falls through to the `i+1==items_len` arm normally.
                if i + 1 < items_len && ctx.abort_value.get() != abort_before_do {
                    last = Some(r);
                    break;
                }
                if i + 1 == items_len {
                    last = Some(r);
                } else if ctx.abort_value.get().is_none()
                    && body_reaches_foreign_perform(db, r, ctx)
                {
                    // KEEP a non-final residual-foreign-perform item — BUT only while no abort has fired.
                    // An ABORTIVE non-final item (`(do (Halt.stop n) …)`, `stop` never resumes) sets the
                    // abort cell and returns the abort VALUE as `r` — a `copy_pure` of the arm body, an
                    // ORPHAN (parent `None`) not yet reparented, carrying live names (the arm `(list v v v)`
                    // with `v`↦ the perform's runtime arg `n`). `body_reaches_foreign_perform` runs
                    // `resolved_of` over it, which RESOLVE-PINS `n` as unbound against the orphan scope —
                    // and that pin outlives `reduce_handle`'s later `reparent_under_handle_site`, surfacing a
                    // spurious CDZ0101 "unbound n" (the abortive-heap-list regression). Once an abort has
                    // fired the sequence is ABANDONED anyway — `reduce_handle` returns the abort value and
                    // discards this do-arm's result — so there is nothing to preserve. Skip the check.
                    kept.push(r);
                }
            }
            let last = last.unwrap();
            if kept.is_empty() {
                Some((last, cur))
            } else {
                kept.push(last);
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend(kept);
                Some((db.push_list(ch), cur))
            }
        }
        // A CROSS-FUNCTION perform: `(f args…)` where `f` is a NON-RECURSIVE function whose body reaches
        // A RECURSIVE effectful call `(f args…)` — `f` recurses AND reaches a discharged op, so it cannot
        // be inlined (it would not terminate). SPECIALIZE it: emit `f#ctx` once per handler context
        // (`DESIGN-effects-rcdzc.md` §4.3), threading this context's state as a trailing parameter, and
        // emit a call `(f#ctx args… <current-state>)`. Checked BEFORE the cross-function inline arm below,
        // because a β-reduced copy of `f`'s body is NOT self-referential (its self-call names the original
        // `f`), so `is_recursive` on the copy reads false — if the inline arm ran first it would unroll
        // the recursion one level per inline (bounded only by the depth backstop). Catching recursion
        // here first routes `f` to specialization, never to the unbounded inline.
        Resolved::Apply { head, args }
            if ctx.has_state() && recursive_call_reaches_discharged(db, &head, ctx) =>
        {
            // Thread state through the args first (they evaluate before the call), then the call takes the
            // current threaded state OF EVERY SLOT as its trailing arguments (in slot order — the order
            // `specialize_recursive` lays the trailing state params out in).
            let mut cur = states;
            let mut rargs = Vec::with_capacity(args.len() + ctx.slots.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rargs.push(ra);
                cur = next;
            }
            // ACCUM-COPY REDIRECT (rn post-observer fix, increment 1). When `head` names a seed-wrapper this
            // merged context redirects to its accum COPY `f$acc` (which has extra accumulator params), the
            // wrapper's own call `(f 1)` supplied only the ORIGINAL args — so append the accumulator SEEDS
            // (read from the wrapper body `(f$acc orig… seed…)`) here, positioned right after the original
            // args and BEFORE captures/states, matching `f$acc`'s sig layout `[orig… seed… captures… states…]`.
            // A self-call INSIDE `f$acc` names `f$acc` directly (not the wrapper), so `accum_seed_redirect`
            // returns `None` for it and it passes its own accumulator arg — no double-seed. Empty (no-op) for
            // every non-redirected call.
            if let Some(head_def) = callee_def_index_of(db, head)
                && let Some((_acc, seeds)) = accum_seed_redirect(db, head_def, ctx.slots.len())
            {
                rargs.extend(seeds);
            }
            let spec = specialize_recursive(db, head, ctx)?;
            // CAPTURED enclosing-fn params come AFTER the original args and BEFORE the state args (the sig
            // layout). Each is passed as a fresh bare-name reference: inside `f#ctx` it resolves to the new
            // capture param; at the INITIAL call from the handle body it resolves to the enclosing fn's param
            // (`run-with`'s `tool`). They are CONSTANT across the recursion, so the same name is passed every
            // call — no threading. This one arm handles both the internal self-calls and the initial call.
            if let Some(captures) = db.effect_spec_captures.get(&spec).cloned() {
                for name in captures {
                    rargs.push(db.push_name(&name));
                }
            }
            // One trailing state arg per slot, in slot order — each a FRESH copy of the incoming state node.
            // The state node `cur[slot]` is ALSO returned as this call's out-state (single-return: `cur`
            // unchanged), so a LATER position that threads against that out-state — a mutual-partner call's
            // let-body performing the discharged op, whose perform arm β-reduces the state node into its
            // next-state — RE-PARENTS the shared node in the single-parent arena, orphaning THIS call's
            // trailing-arg occurrence and leaking the internal `f#eff{n}$s{k}` name in a CDZ0101 (the
            // mutual-cycle `compute#eff5$s0` leak). A fresh copy per call arg keeps the call's embedded state
            // distinct from the out-state that flows forward. `deep_fresh_copy` (not `copy_pure`) for the
            // same reason as the `if`/`match`/perform arms: a resolve-pinned `$s{k}` ref must be re-pushed
            // unpinned so it re-resolves against the specialized def's sig, not shared as a pinned node.
            for &s in cur.iter() {
                let fresh = deep_fresh_copy(db, s);
                rargs.push(fresh);
            }
            // Build the call `(<spec-name> args… state…)`. The specialized def is named, so a name atom
            // resolves to it (via `def_by_name`), and the ordinary recursive `Core::Call` + reachability
            // path emits it.
            let name_atom = db.push_name(&spec);
            let mut call = vec![name_atom];
            call.extend(rargs);
            let call_node = db.push_list(call);
            if db.multivalue_specs.contains(&spec) {
                // MULTI-VALUE MODE (repro-1): `f#ctx` returns `(value, out-state-per-slot)`. The call's
                // OUT-state is a RUNTIME value (not a symbolic expr of the incoming state, as a perform's
                // is), so it must be LET-BOUND before it can be projected and threaded forward. Bind the
                // call to a fresh temp `t`, register the binding in `ctx.pending` (a leaf tail-expr drains
                // it into wrapping `let`s), and RETURN `(. t 0)` as the call's VALUE with `[(. t 1)…]` as
                // the NEW state per slot. This is what makes a LATER sibling self-call / perform thread
                // against THIS call's advanced out-state (`(. t 1)`), not the un-advanced incoming state.
                let k = ctx.temp_ctr.get();
                ctx.temp_ctr.set(k + 1);
                let tname = format!("{spec}$t{k}");
                ctx.pending.borrow_mut().push((tname.clone(), call_node));
                let value = tuple_proj(db, &tname, 0);
                let new_states: Vec<StructId> = (0..cur.len())
                    .map(|slot| tuple_proj(db, &tname, (slot + 1) as u32))
                    .collect();
                return Some((value, new_states));
            }
            // The call's VALUE is the specialized fn's result; the states after it are not observed (the
            // corpus never reads post-recursion state — the single-return shape).
            Some((call_node, cur))
        }
        // A CROSS-FUNCTION perform: `(f args…)` where `f` is a NON-RECURSIVE function whose body reaches
        // an operation this handler discharges (`DESIGN-effects-rcdzc.md` §3, the new inline trigger). The
        // handler must be present in the callee, so INLINE `f` into the handled region — β-reduce the call
        // (substitute args for params) and thread state through the reduced body, exactly as if the
        // callee's body were written inline. This is what makes `(handle … (gen))` work when `gen`
        // performs the discharged op. A RECURSIVE such callee is caught by the specialization arm ABOVE.
        Resolved::Apply { head, args } if call_reaches_discharged_effect(db, head, ctx) => {
            // Thread state through the arguments FIRST (they evaluate before the call, in order), then
            // inline the callee and thread its (reduced) body.
            let mut cur = states;
            let mut rargs = Vec::with_capacity(args.len());
            for &a in args.iter() {
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                rargs.push(ra);
                cur = next;
            }
            // A PARAMETERIZED callee β-reduces (substitute args for params); a NULLARY def `(gen)` has no
            // lambda wrapper — its name resolves straight to its body, so `apply_lambda` yields nothing
            // and we thread the body directly.
            let reduced = match crate::eval::apply_lambda(db, head, &rargs).ok().flatten() {
                Some(r) => r,
                None => crate::eval::lambda_body_of_nullary(db, head)?,
            };
            // DEEP-FRESH the reduced body before threading it. `apply_lambda`/`beta_reduce` SUBSTITUTES the
            // (threaded) args for the callee's params by returning each arg node AS-IS (its pinned-name fast
            // path), so a substituted arg that is a RESOLVE-PINNED reference to a DRIVER param — the caller's
            // `acc` spliced into an inlined helper `turn(a, acc) = acc + …` — keeps its pin to the caller's
            // (now-dead) scope. When this inline happens INSIDE a recursive self-call's arg, the reduced body
            // lands in the synthesized `f#ctx` def, where that pinned `acc` no longer resolves → CDZ0101
            // unbound (v-agent-harness Inc-3, the helper-references-a-driver-param sub-case). A fully-fresh
            // copy drops the stale pins so every name re-resolves against the specialized def's sig (which
            // carries the driver's params). Harmless for the non-nested inline (a fresh copy of a body whose
            // refs already resolve re-resolves to the same bindings).
            let reduced = deep_fresh_copy(db, reduced);
            thread_bounded(db, reduced, cur, ctx, inline_depth + 1)
        }
        // An `(if cond then else)` — the condition is evaluated (thread state through it), then BOTH
        // branches see the post-condition state (only one runs, but each is rewritten under the same
        // incoming state — a perform in a branch reads that state). The branch STATES are not merged
        // (the corpus never observes post-`if` state across a perform-in-a-branch), so the `if`'s
        // out-state is the post-CONDITION state — sound for the single-return shape (a recursive call
        // in a branch takes the branch-local threaded state as its argument; nothing after the `if`
        // reads state). This is what threads countdown's `(if (= (tick) 0) 0 (+ 1 (loop)))`.
        Resolved::If { cond, then_, else_ } => {
            let (rcond, cur) = thread_bounded(db, cond, states, ctx, inline_depth)?;
            // Each BRANCH is threaded under the post-condition state. An ABORTIVE perform in a branch's
            // TAIL is LOCAL to that branch: the branch's value is the arm value (the abort discards up to
            // the handle, but the `if` IS the handle body's value, so per-branch the abort just yields the
            // arm value). Capture it per-branch — thread the branch, and if it set the whole-handle abort
            // cell, take that value as the branch's rewrite and CLEAR the cell so the OTHER branch (and the
            // handle) are not collapsed. (Sound when the `if` is in the handle's tail position — a NON-tail
            // conditional abort is declined by `body_has_unsound_abortive_perform` in `reduce_handle`
            // before threading, so an `if` reached here is safe to fold per-branch.)
            //
            // Each branch gets its OWN FRESH COPY of the incoming state-ref nodes. Both branches EMBED the
            // state (a perform substitutes it into a resume value; a recursive/mutual call appends it as a
            // trailing state arg), and the arena is single-parent — so sharing one state-ref node across
            // both branches orphans whichever is parented second, leaking the internal `f#ctx$s0` name in a
            // CDZ0101 (the mutually-recursive-effect case where the perform is in one branch and the mutual
            // call in the other; and the effectful-helper-with-a-conditional-perform-in-a-self-call-arg case,
            // where the inlined helper's `if` threads the state into both branches). `deep_fresh_copy` (NOT
            // `copy_pure`) is required: `copy_pure` = `beta_reduce`, whose pinned-name fast path returns a
            // RESOLVE-PINNED state-ref (`f#ctx$s{k}`, pinned when the inlined helper body was resolved) AS-IS
            // — so both branches share the one pinned node, re-orphaning it. `deep_fresh_copy` re-pushes the
            // leaf fresh (unpinned), giving each branch a genuinely independent node that re-resolves against
            // the specialized def's sig (which declares `$s{k}`).
            let then_states: Vec<StructId> = cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
            let else_states: Vec<StructId> = cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
            let (rthen, then_out) =
                thread_branch_local_abort_with_out(db, then_, then_states, ctx, inline_depth)?;
            let (relse, else_out) =
                thread_branch_local_abort_with_out(db, else_, else_states, ctx, inline_depth)?;
            let if_head = db.push_name("if");
            let rif = db.push_list(vec![if_head, rcond, rthen, relse]);
            // MERGE the per-branch out-states. Only ONE branch runs at runtime, so the `if`'s out-state for
            // each slot is that branch's out-state, selected by the SAME condition: `(if cond then_out
            // else_out)`. This lets a SIBLING that reads state AFTER the conditional (a recursive-call operand,
            // a `do`-continuation) observe a branch perform's advance — the recursive-branch-perform fix
            // (self-recursive rw1/rw3/rw5 + the through-block fold; operator-prioritized). When NEITHER branch
            // advanced a slot (`then_out[i] == else_out[i] == cur[i]` — the common no-branch-perform case, e.g.
            // countdown), the merge collapses to `cur[i]` unchanged, so those working cases are byte-identical.
            // GATED on a PURE condition (`!subtree_performs(cond)`): only a pure `cond` is safely re-usable in
            // the state selector. A PERFORMING condition is lifted to `(let ((#cv COND)) (if #cv t e))` — its
            // threaded `rcond` is a `#cv` reference bound by a `let` that wraps ONLY the value-if, so re-using
            // it in the state position (outside that `let`) leaks the `#cv` name (CDZ0101). Those
            // performing-condition cases (the short-circuit `or`/`and` connectives desugared to an `if`) were
            // ALREADY correct with the post-condition `cur` out-state (the condition's own performs advanced
            // `cur`; the branches there are pure `true`/`false`/value literals), so skipping the merge for them
            // is both necessary (avoid the leak) and sound (no branch advance to propagate). The condition is
            // pure here, evaluated once as the value-if's head — re-using it in the state selector duplicates
            // no effect (the same soundness the `match` branch-dependent-next-state peel relies on).
            let cond_pure = !subtree_performs(db, cond, ctx) && !contains_cv_ref(db, rcond);
            let merged: Vec<StructId> = cur
                .iter()
                .zip(then_out.iter().zip(else_out.iter()))
                .map(|(&c, (&t, &e))| {
                    // SAFE to merge this slot only when: (1) the condition is PURE (a performing cond is
                    // lifted to `(let ((#cv COND)) …)`, so `rcond` = a `#cv` ref bound by a `let` wrapping only
                    // the value-if — re-using it in the state position leaks the name); AND (2) NEITHER branch
                    // out-state references a synthesized `#cv` (a branch whose sub-position performs is itself
                    // `#cv`-lifted INSIDE the branch, so its out-state carries a branch-local `#cv` that is
                    // out of scope in the merged state position — the leak the connective-desugar hit). A
                    // clean branch out-state (rw1's `(if true (St.get) 0)` → then_out is a plain resume
                    // next-state, no `#cv`) merges safely. When unsafe or when neither branch advanced the
                    // slot, keep `cur` (byte-identical to the pre-fix behavior — no regression).
                    //
                    // A BRANCH MUST ACTUALLY PERFORM to have advanced state (breaker en1 fix). The
                    // `(t != c || e != c)` test alone is a NODE-IDENTITY compare — but the branches were
                    // threaded over `deep_fresh_copy`(cur) (5140-5141), so `then_out`/`else_out` are FRESH
                    // node ids ALWAYS distinct from `c` even when the branch performed NOTHING and its
                    // out-state is semantically just `cur`. Without the perform gate, a pure branching
                    // conditional in the handle-body tail — the `(if (>= r 100) r 0)` LET-BODY of an inlined
                    // performing helper `(let ((r (+ x (St.bump)))) (if …))` — spuriously builds the state
                    // selector `(if cond r 0)` from its VALUE branches (`r`,`0`), which carry NO state advance.
                    // That value-`if` then rides forward as the slot's next-STATE (cur[slot] = the if), and a
                    // later dispatch substitutes it back as its resume value → the let-body `if` lands in the
                    // let's BINDINGS, the binder `r` referenced in the if-cond is structurally unreachable →
                    // false CDZ0101 "unbound r". Requiring an actual branch perform restores the invariant
                    // "state out of a pure conditional = cur unchanged": neither branch performs → no advance
                    // → keep `c` (en1 folds; the pure analog already did). A genuine branch perform (rw1/rw3:
                    // `(if c (St.get) 0)`) still merges — its out-state is a real threaded resume next-state.
                    let branch_advanced =
                        subtree_performs(db, then_, ctx) || subtree_performs(db, else_, ctx);
                    let mergeable = cond_pure
                        && branch_advanced
                        && (t != c || e != c)
                        && !contains_cv_ref(db, t)
                        && !contains_cv_ref(db, e);
                    if mergeable {
                        let ifh = db.push_name("if");
                        let condc = deep_fresh_copy(db, rcond);
                        let tc = deep_fresh_copy(db, t);
                        let ec = deep_fresh_copy(db, e);
                        db.push_list(vec![ifh, condc, tc, ec])
                    } else {
                        c
                    }
                })
                .collect();
            Some((rif, merged))
        }
        // A `(match scrutinee (pattern body)…)` — the analogue of `if` for the pattern engine. Thread the
        // SCRUTINEE (a perform there reads/threads state, `(match (Get.next) …)`), then rewrite each arm:
        // the PATTERN is a binder position (copied structurally, never threaded — like a `let` binder), the
        // BODY is threaded under the post-scrutinee state (only one arm runs, so each sees the same incoming
        // state, mirroring the `if` branches). An abortive perform in an arm BODY tail is branch-local — the
        // `match` IS the handle body's value, so per-arm the abort yields the arm value — captured by
        // `thread_branch_local_abort` (which restores the cell so a sibling arm / the handle is not
        // collapsed). The out-state MERGES the per-arm out-states into a match-valued out-state `(match
        // scrut (pat arm-out)…)` — the `Match` analogue of the `if` per-branch out-state merge below — so a
        // SIBLING reading state after the match (a recursive-call operand) observes an arm perform's advance
        // (the recursive-branch-perform self-recursive miscompile's match-arm face, #1993); when no arm
        // advanced a slot the merge collapses to the incoming state unchanged. Rebuild the same `(match
        // rscrut (pat rbody)…)` form so the pattern engine lowers it by the ordinary path.
        Resolved::Match { scrutinee, arms } => {
            let abort_before_scrut = ctx.abort_value.get();
            let (rscrut, cur) = thread_bounded(db, scrutinee, states, ctx, inline_depth)?;
            // SCRUTINEE ABORT collapses the whole match (the abort-outer-advance class, scrutinee face). The
            // scrutinee is evaluated BEFORE any arm; if it ABORTS — `(match (do (A.tick) (B.bail 99)) …)` under
            // B — no arm runs, so the match's value is the scrutinee's rewrite. `thread_bounded` already
            // produced the sound `(do (A.tick) 99)` (the do-arm kept the pre-abort foreign `A.tick`); return
            // it directly so the ENCLOSING fold discharges the foreign prefix (advancing the outer state)
            // rather than wrapping the aborted scrutinee in a dead `(match … arms)` whose bare-abort collapse
            // would drop `A.tick` (109 vs 110). Only when a NEW abort fired threading the scrutinee (cell
            // flipped) — a non-aborting scrutinee falls through to the ordinary per-arm threading below.
            if ctx.abort_value.get() != abort_before_scrut {
                return Some((rscrut, cur));
            }
            let match_head = db.push_name("match");
            let mut children = vec![match_head, rscrut];
            // Collect each arm's (pattern, out-state) so the arm out-states can be MERGED into a match-valued
            // out-state (the `Match` analogue of the `if` per-branch out-state merge — see that arm). An
            // arm-body perform advances the state, and a sibling reading state after the match (a recursive
            // call operand) must see that advance; without the merge the match returns the post-SCRUTINEE
            // state and the advance is dropped (the recursive-branch-perform self-recursive miscompile's
            // match-arm face). Each arm's out-state is captured alongside its rewrite.
            let mut arm_outs: Vec<Vec<StructId>> = Vec::with_capacity(arms.len());
            let mut arm_pats: Vec<StructId> = Vec::with_capacity(arms.len());
            for (pat, body) in arms.iter() {
                let (pat, body) = (*pat, *body);
                // The pattern binds names for the arm body (a binder position) — copy it structurally so it
                // is self-contained, exactly as a `let` binder name is copied (never substituted/threaded).
                let rpat = copy_pure(db, pat);
                // Each arm gets its OWN FRESH COPY of the incoming state-refs — the same single-parent-arena
                // reason as the `if` branches: an arm body EMBEDS the state (a perform substitutes it into a
                // resume value; a recursive/mutual call appends it as a trailing state arg), so sharing one
                // state-ref node across arms orphans whichever is parented second, leaking the internal
                // `f#ctx$s0` name (a mutual group dispatched by `match` with the perform in one arm and the
                // mutual call in another; and the conditional-perform-helper-in-a-self-call-arg case).
                // `deep_fresh_copy` (NOT `copy_pure`) for the same reason as the `if` arm above — `copy_pure`
                // shares a resolve-pinned `$s{k}` ref across arms; `deep_fresh_copy` gives each an unpinned
                // fresh node that re-resolves against the spec sig.
                let arm_states: Vec<StructId> =
                    cur.iter().map(|&s| deep_fresh_copy(db, s)).collect();
                let (rbody, arm_out) =
                    thread_branch_local_abort_with_out(db, body, arm_states, ctx, inline_depth)?;
                children.push(db.push_list(vec![rpat, rbody]));
                arm_pats.push(pat);
                arm_outs.push(arm_out);
            }
            let rmatch = db.push_list(children);
            // MERGE the per-arm out-states into a `(match scrut (pat arm-out)…)`-valued out-state per slot —
            // the `Match` analogue of the `if` merge. Only ONE arm runs, selected by the SAME (pure, already-
            // evaluated) scrutinee, so the slot's out-state is that arm's out-state under the matching pattern.
            // GATED identically to the `if` arm: a PURE scrutinee (a performing one is `#cv`-lifted, whose ref
            // can't be re-used in the state position) with `#cv`-free arm out-states; collapses to `cur[i]`
            // when NO arm advanced the slot (byte-identical to the pre-fix post-scrutinee behavior — working
            // cases unchanged, no regression). The scrutinee is re-used in the state selector, duplicating no
            // effect (pure) — the same soundness the `match` branch-dependent-next-state peel relies on.
            let scrut_pure = !subtree_performs(db, scrutinee, ctx) && !contains_cv_ref(db, rscrut);
            let merged: Vec<StructId> = (0..cur.len())
                .map(|i| {
                    let c = cur[i];
                    let advanced = arm_outs.iter().any(|o| o[i] != c);
                    let cv_clean = arm_outs.iter().all(|o| !contains_cv_ref(db, o[i]));
                    if scrut_pure && advanced && cv_clean {
                        let mh = db.push_name("match");
                        let sc = deep_fresh_copy(db, rscrut);
                        let mut ch = vec![mh, sc];
                        for (a, &pat) in arm_pats.iter().enumerate() {
                            let pc = copy_pure(db, pat);
                            let oc = deep_fresh_copy(db, arm_outs[a][i]);
                            ch.push(db.push_list(vec![pc, oc]));
                        }
                        db.push_list(ch)
                    } else {
                        c
                    }
                })
                .collect();
            Some((rmatch, merged))
        }
        // A short-circuit connective `(and lhs rhs)` / `(or lhs rhs)` whose rhs runs only conditionally on
        // `lhs`. Threading it as a strict two-operand form would evaluate rhs's perform even when `lhs`
        // short-circuits — an observable-effect change. Instead DESUGAR to the equivalent `if` (`(and lhs
        // rhs)` ≡ `(if lhs rhs false)`, `(or lhs rhs)` ≡ `(if lhs true rhs)`) and re-thread, so rhs is a
        // branch (threaded under the post-`lhs` state, run only on the taken path). `lhs` becomes the `if`
        // condition — evaluated exactly once either way. Only reached when a perform is inside (a pure
        // connective is copied wholesale by the pure-subtree arm below).
        Resolved::And { lhs, rhs, is_and } => {
            let if_head = db.push_name("if");
            let (then_, else_) = if is_and {
                let f = db.push_atom(Leaf::Bool(false));
                (rhs, f)
            } else {
                let t = db.push_atom(Leaf::Bool(true));
                (t, rhs)
            };
            let desugared = db.push_list(vec![if_head, lhs, then_, else_]);
            thread_bounded(db, desugared, states, ctx, inline_depth)
        }
        // A negation `(not operand)` — a STRICT one-operand form. Thread the operand (a perform there
        // reads/threads state, `(not (= (Get.next) 0))`), then rebuild `(not roperand)`.
        Resolved::Not { operand } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let not_head = db.push_name("not");
            Some((db.push_list(vec![not_head, roperand]), cur))
        }
        // A tuple PROJECTION `(. operand index)` — STRICT one-operand. Thread the operand (a perform there
        // reads/threads state, `(. (tuple (Get.next) (Get.next)) 1)`), rebuild the same projection. The
        // index is a literal (copied structurally). `push_list` with the same `.`-head + index re-forms it.
        Resolved::Proj { operand, index } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let dot = db.push_name(".");
            let idx_atom = db.push_atom(Leaf::Int {
                value: IntValue::from_i64(index as i64),
                radix: Radix::Dec,
            });
            Some((db.push_list(vec![dot, roperand, idx_atom]), cur))
        }
        // Member access `(. operand key)` — STRICT one-operand (the key is a label, not a value). Thread
        // the operand; rebuild `(. roperand key)` with the key copied as a bare name atom.
        Resolved::Member { operand, key } => {
            let (roperand, cur) = thread_bounded(db, operand, states, ctx, inline_depth)?;
            let dot = db.push_name(".");
            let key_atom = db.push_name(&key.name);
            Some((db.push_list(vec![dot, roperand, key_atom]), cur))
        }
        // An annotation `(: expr T)` — STRICT one-operand (the type is not runtime code). Thread `expr`,
        // rebuild `(: rexpr T)` with the type expression copied structurally.
        Resolved::Annot { expr, ty_expr } => {
            let (rexpr, cur) = thread_bounded(db, expr, states, ctx, inline_depth)?;
            let colon = db.push_name(":");
            let rty = copy_pure(db, ty_expr);
            Some((db.push_list(vec![colon, rexpr, rty]), cur))
        }
        // A TUPLE / LIST CONSTRUCTOR `("tuple" e0 e1 …)` / `("list" …)` — a STRICT compound constructor:
        // every element is evaluated exactly once, left to right, before the compound is built. So a perform
        // in an element reads/threads state exactly like an arithmetic operand or a call argument (the fold
        // already handles those). Thread each element in order, then rebuild the same constructor with the
        // rewritten elements. This is what threads `(let ((p ("tuple" (Fresh.next) (Fresh.next)))) …)` — the
        // ML tuple/list literal, whose head is the STRING-LITERAL ctor primitive `"tuple"`/`"list"` (a bare
        // `tuple` NAME reduces via `(meta apply)` and threads through the Apply arm; the string-head ctor
        // reaches HERE). The ctor string is re-pushed as a `Leaf::Str` head so the resolver re-recognizes it.
        Resolved::Tuple { elems } | Resolved::List { elems } | Resolved::Set { elems } => {
            let ctor = match resolved_of(db, node) {
                Resolved::List { .. } => CompoundCtor::List,
                Resolved::Set { .. } => CompoundCtor::Set,
                _ => CompoundCtor::Tuple,
            };
            let elems: Vec<StructId> = elems.iter().copied().collect();
            let mut cur = states;
            let mut relems = Vec::with_capacity(elems.len());
            for e in elems {
                let (re, next) = thread_bounded(db, e, cur, ctx, inline_depth)?;
                relems.push(re);
                cur = next;
            }
            let head = db.push_atom(Leaf::Ctor(ctor));
            let mut children = vec![head];
            children.extend(relems);
            Some((db.push_list(children), cur))
        }
        // A RECORD CONSTRUCTOR `("record" (label value)…)` — a STRICT compound constructor whose field
        // VALUES are evaluated in written order before the record is built. Thread each field's value (a
        // perform there reads/threads state), keeping the label, and rebuild the same `("record" (label
        // rvalue)…)` form. The companion of the tuple/list arm above for the ML record literal `{ a = …, b
        // = … }`, which lowers to this string-headed ctor with `(label value)` pair args. (Matched on the
        // RAW form via `compound_form_of` — like the `let` arm — because `Resolved::Record` holds a sorted
        // BTreeMap that loses the written order the source evaluates in. `compound_form_of` reads the native
        // `#record` ctor-leaf head too (superset of the name/string alias), so M3-nativized records are
        // recognized (dc02-class hardening, #5340); the field destructure below already handles a native
        // FieldPair child (the `=`-marker leaf reads as `(= label value)` via the migration bridge).
        _ if db
            .ast
            .compound_form_of(node, crate::ast::CompoundCtor::Record)
            .is_some() =>
        {
            let fields: Vec<StructId> = db
                .ast
                .compound_form_of(node, crate::ast::CompoundCtor::Record)
                .unwrap()
                .to_vec();
            let mut cur = states;
            let mut rfields = Vec::with_capacity(fields.len());
            for field in fields {
                let Struct::List(kv) = db.ast.get(field).clone() else {
                    return None;
                };
                // A field is the canonical `(= label value)` ascription triple (Phase B) — label = child
                // 1, value = child 2, rebuilt WITH the `=` head — or a legacy `(label value)` pair.
                let (eq_head, label_id, value_id) =
                    if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") {
                        (Some(kv[0]), kv[1], kv[2])
                    } else if kv.len() == 2 {
                        (None, kv[0], kv[1])
                    } else {
                        return None;
                    };
                // The label is copied structurally (a field name, not a value to thread); the VALUE is
                // threaded (it may perform). The `=` head (if any) is preserved so the rebuilt field keeps
                // the canonical triple shape.
                let label_copy = copy_pure(db, label_id);
                let (rvalue, next) = thread_bounded(db, value_id, cur, ctx, inline_depth)?;
                cur = next;
                let rebuilt = match eq_head {
                    Some(eq) => {
                        let eq_copy = copy_pure(db, eq);
                        db.push_list(vec![eq_copy, label_copy, rvalue])
                    }
                    None => db.push_list(vec![label_copy, rvalue]),
                };
                rfields.push(rebuilt);
            }
            let head = db.push_atom(Leaf::Ctor(CompoundCtor::Record));
            let mut children = vec![head];
            children.extend(rfields);
            Some((db.push_list(children), cur))
        }
        // A MAP CONSTRUCTOR `("map" (key value)…)` — a STRICT compound whose entries are evaluated in
        // written order, and within each entry the KEY then the VALUE (both may perform). Thread the key,
        // then the value, per entry, and rebuild the same `("map" (rkey rvalue)…)` form. The map companion
        // of the record arm above (a `(key value)` pair like a record field, but the KEY is a value to
        // thread, not a copied label). Matched on the RAW form (`Resolved::Map`'s `entries` slice is fine
        // too, but the raw form keeps the written order uniformly with the other ctor arms). `compound_form_of`
        // reads the native `#map` ctor-leaf head too (superset of the name/string alias) → M3-nativized maps
        // recognized (dc02-class hardening, #5340).
        _ if db
            .ast
            .compound_form_of(node, crate::ast::CompoundCtor::Map)
            .is_some() =>
        {
            let entries: Vec<StructId> = db
                .ast
                .compound_form_of(node, crate::ast::CompoundCtor::Map)
                .unwrap()
                .to_vec();
            let mut cur = states;
            let mut rentries = Vec::with_capacity(entries.len());
            for entry in entries {
                let Struct::List(kv) = db.ast.get(entry).clone() else {
                    return None;
                };
                // An entry is the canonical `(= key value)` triple (M3-nativized `#map`) — key = child 1,
                // value = child 2, rebuilt WITH the `=` head — or a legacy `(key value)` pair. (Mirrors the
                // record-field arm above; UNLIKE a record label, the map KEY is a value to THREAD — it may
                // perform.) A prior `kv.len() != 2 => None` handled ONLY the legacy pair, so a perform in a
                // nativized `(= k v)` entry declined — the map-literal companion of the record dual-form,
                // missed in the M3 `=`-marker migration (baseline-pass regression caught by the 14b gate).
                let (eq_head, key_id, value_id) =
                    if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") {
                        (Some(kv[0]), kv[1], kv[2])
                    } else if kv.len() == 2 {
                        (None, kv[0], kv[1])
                    } else {
                        return None;
                    };
                // KEY then VALUE, both threaded (either may perform), in evaluation order.
                let (rkey, next_k) = thread_bounded(db, key_id, cur, ctx, inline_depth)?;
                let (rvalue, next_v) = thread_bounded(db, value_id, next_k, ctx, inline_depth)?;
                cur = next_v;
                let rebuilt = match eq_head {
                    Some(eq) => {
                        let eq_copy = copy_pure(db, eq);
                        db.push_list(vec![eq_copy, rkey, rvalue])
                    }
                    None => db.push_list(vec![rkey, rvalue]),
                };
                rentries.push(rebuilt);
            }
            let head = db.push_atom(Leaf::Ctor(CompoundCtor::Map));
            let mut children = vec![head];
            children.extend(rentries);
            Some((db.push_list(children), cur))
        }
        // A `(let ((n init)…) body)` — thread state through each initializer in order, then the body.
        // This is what threads range-sum's `(let ((i (Idx.next))) (if (= i 0) …))`: the init `(Idx.next)`
        // performs (reads state, threads next), and `i` binds that resume value. Rebuild the `let` with
        // the rewritten inits + body so `i`'s binding survives (the binder name is copied structurally).
        _ if db.ast.as_form(node, "let").is_some() => {
            let tail: Vec<StructId> = db.ast.as_form(node, "let").unwrap().to_vec();
            if tail.len() != 2 {
                return None;
            }
            let bindings_occ = tail[0];
            let body_occ = tail[1];
            let Struct::List(pairs) = db.ast.get(bindings_occ).clone() else {
                return None;
            };
            // CAPTURED-VALUE INLINE (the closure-capture-reperform fix), gated on the let BODY being a
            // RETURNED LAMBDA — the ONLY shape that suffers capture orphaning. A returned lambda capturing a
            // binding — `(let ((base (Ctr.tick))) (fn (x) (+ x base)))` — has its body `copy_pure`d by the
            // `thread_bounded` no-perform arm, DETACHING `base` (it resolves to Poison-unbound in the copy),
            // so a substitution AFTER the copy has no resolvable `Ref` to match. We instead SUBSTITUTE each
            // pure threaded init into the ORIGINAL body BEFORE threading copies it (below), where `base`
            // still resolves to `Ref { value: kv[1] }` (the init occ, per inc-2b-1) — closing the lambda over
            // the FOLDED value (`(fn (x) (+ x 50))`), so when it escapes + is re-reduced by `lambda_of`/
            // `apply_lambda` (which re-derive from source, bypassing this threaded binding) it carries the
            // value, not the perform. Gated on the closure shape SYNTACTICALLY so a non-closure let body (a
            // recursive multi-value fold, an arithmetic tail, …) is byte-identical — the collect below (and
            // its `strongly_pure` probes, which touch shared caches) only runs for the closure case.
            // A body reached through NESTED `let`s ending in a lambda — `(let ((a (Ctr.tick))) (let ((b
            // …)) (fn …)))`, an outer capture referenced by a closure buried in an inner let — counts too,
            // so peel let-chains (`body_returns_lambda`). Without this the outer `let`'s body (an inner
            // `let`, not a lambda) failed the gate, and the outer capture `a` orphaned → CDZ0101.
            let body_is_closure = body_returns_lambda(db, body_occ);
            let mut cur = states;
            let mut rpairs = Vec::with_capacity(pairs.len());
            let mut capture_subst: HashMap<StructId, StructId> = HashMap::default();
            for pair in pairs {
                let Struct::List(kv) = db.ast.get(pair).clone() else {
                    return None;
                };
                if kv.len() != 2 {
                    return None;
                }
                // The binder NAME is copied structurally (a binder, not a value to thread); the INIT is
                // threaded (it may perform).
                let name_copy = copy_pure(db, kv[0]);
                let init_abort_before = ctx.abort_value.get();
                let (rinit, next) = thread_bounded(db, kv[1], cur, ctx, inline_depth)?;
                cur = next;
                // LET-INIT ABORT COLLAPSE (breaker ax7/ah-x3). If threading THIS init fired an abort — the
                // init `(+ (A.tick) (B.bail 99))` / `(do (ask.ask) (Bail.bail 7))` aborts — the `let` never
                // binds the name and the body + remaining bindings are DEAD (the abort abandons the
                // continuation). `rinit` is already the sound rewrite the abort-lift produced: a `(do
                // <pre-abort-foreign-prefix> abort-value)` (or the bare abort value), carrying any committed
                // foreign advance / issued host call. Return it DIRECTLY as the let's value — do NOT push it
                // as a binding + continue to the body, which would (a) bind the name to the do-tail and run
                // `(+ x 1)` on it (a wrong value, ax7 → 109) and (b) leave the foreign prefix buried in a dead
                // binding the enclosing collapse discards (dropping A.tick's advance / never issuing the host
                // call, ah-x3). The abort cell stays SET so the enclosing fold collapses to this value. This
                // is the let-INIT analog of the strict-operand abort-lift (the body-abort case is handled by
                // the ABORT-VALUE RE-SCOPE below; this is the INIT-abort case it missed — the `abort_before`
                // snapshot there is taken AFTER this loop, so an init abort slipped through).
                if init_abort_before.is_none() && ctx.abort_value.get().is_some() {
                    // PRESERVE the EARLIER bindings' committed effects (breaker ax12). The bindings BEFORE
                    // the aborting one (`rpairs`, already threaded) ran on the strict spine before the abort,
                    // so an earlier binding whose init committed a FOREIGN advance / host call — `(let ((y
                    // (A.tick)) (x (+ 1 (B.bail 99)))) …)`, where `y=(A.tick)` advances the OUTER A-state
                    // before `x`'s abort — must survive. Returning the bare `rinit` (the abort value) drops
                    // them, losing `y`'s advance (109 vs 110). Re-wrap the abort value in a `let` over the
                    // earlier bindings so they run for-effect (and remain in scope, since the abort value may
                    // reference an earlier binder) before the abort value. When there are no earlier bindings
                    // (the aborting init is the FIRST) this is a bare `rinit` — the ax7 case, unchanged. The
                    // enclosing fold's do/bare-abort collapse then discharges the wrapping `let`'s foreign
                    // inits (advancing the outer state) before yielding the value.
                    // Collect the earlier bindings' INITS that committed a FOREIGN effect (a foreign
                    // perform / host call) — the pure ones commit nothing and are dropped. Sequence them as
                    // a for-effect `do` PREFIX before the abort value, so the enclosing fold discharges them
                    // (advancing the outer state / issuing the host call) before yielding the abort value. A
                    // `let`-WRAP would not work: an unused binding whose init performs gets dead-code
                    // dropped, losing the effect (ax12 stayed 109). Reuse each init's ALREADY-THREADED
                    // rewrite (`rpairs[i]` = `(name rinit_i)`, take the init `rinit_i`).
                    let mut foreign_prefix: Vec<StructId> = Vec::new();
                    for rp in &rpairs {
                        if let Struct::List(kv2) = db.ast.get(*rp).clone()
                            && kv2.len() == 2
                            && body_reaches_foreign_perform(db, kv2[1], ctx)
                        {
                            foreign_prefix.push(kv2[1]);
                        }
                    }
                    if foreign_prefix.is_empty() {
                        return Some((rinit, cur));
                    }
                    let do_head = db.push_name("do");
                    let mut ch = vec![do_head];
                    ch.extend(foreign_prefix);
                    ch.push(rinit);
                    return Some((db.push_list(ch), cur));
                }
                // A pure threaded init captured by a returned-lambda body → substitute it into the body
                // (see the block comment above). GATED to the closure shape + an effect-FREE init (an
                // effectful init would duplicate its effect on inline).
                if body_is_closure && strongly_pure(db, rinit, ctx) {
                    capture_subst.insert(kv[1], rinit);
                }
                // NESTED-LET INIT LET-LIFT (the effectful-let capture fix): if the threaded init is itself a
                // `(let ((x e)…) lbody)` — the shape an INLINED effectful helper produces, `let b = inner()`
                // where `inner`'s body is `(let a = S.get() in …)` — LIFT the inner bindings UP into THIS
                // `let`'s binding list (before this pair), and bind this name to the inner `lbody`. WHY: the
                // threaded out-state `cur` may REFERENCE an inner binder (`put(a)` threads its arg `a` as the
                // next state), and that out-state is spliced into the LATER bindings/body of THIS let; left
                // inside `b`'s init-`let`, `a` is out of scope there → a spanless CDZ0101 (the nested-eff-let
                // bug: `(let ((b (let ((a 10)) …))) (+ b a))` — the outer body's re-perform resolved to `a`).
                // Lifting makes `a` a sibling binding of `b`, in scope for the continuation. Sound: the inner
                // `let`'s inits already ran (in order) to produce `b`'s value; hoisting only WIDENS their
                // visibility, it does not reorder or duplicate them (a `let` binding list is sequential, same
                // as nesting). Only fires for a threaded init that IS a `let` (the inline/perform case); a
                // plain init is pushed as-is. Mirrors the `do`-arm let-lift (`inlined_let_of_do_item`).
                if let Some(inner) = db.ast.as_form(rinit, "let").map(|t| t.to_vec())
                    && inner.len() == 2
                    && let Struct::List(inner_pairs) = db.ast.get(inner[0]).clone()
                {
                    for ip in inner_pairs {
                        rpairs.push(ip);
                    }
                    rpairs.push(db.push_list(vec![name_copy, inner[1]]));
                } else {
                    rpairs.push(db.push_list(vec![name_copy, rinit]));
                }
            }
            // Close any captured pure binding into the body BEFORE threading detaches it (see the collect
            // comment above). A no-op when nothing captured (non-closure body → empty subst).
            let body_occ = if capture_subst.is_empty() {
                body_occ
            } else {
                crate::eval::beta_reduce(db, body_occ, &capture_subst)
            };
            // Snapshot the abort cell BEFORE threading the body, so we can tell if THIS body fires an abort.
            let abort_before = ctx.abort_value.get();
            let (rbody, cur) = thread_bounded(db, body_occ, cur, ctx, inline_depth)?;
            let let_head = db.push_name("let");
            let rbindings = db.push_list(rpairs);
            let wrapped = db.push_list(vec![let_head, rbindings, rbody]);
            // ABORT-VALUE RE-SCOPE. If threading the body FIRED AN ABORT (`(let ((v e)) (Bail.bail v))` — the
            // abortive arm `(bail (n) s n)` materializes `v` as the abort value), `reduce_handle` collapses
            // the whole handle to that abort value and DISCARDS this `let` wrapper — so a `v` in the abort
            // value orphans → spurious CDZ0101 (the abortive-perform-referencing-a-body-local-binding bug,
            // the abortive twin of the do-def-in-perform-arg fix). Re-wrap the abort value in THIS let's
            // bindings (and update the cell) so it carries the same scope + init evaluation the discarded
            // wrapper would have. Fired UNCONDITIONALLY whenever this body set the abort cell (`abort_before`
            // was None and it is now Some) — NOT gated on the value's free names: the rewrap preserves the
            // bindings' EVALUATION as well as their name scope, so re-wrapping even a closed abort value (a
            // bare-param abort like `(Bail.bail u)`, whose value binds no `let` name) is still correct — the
            // extra `let` is inert but harmless, and NOT special-casing it keeps this branch simple and
            // uniform. Sound: the bindings' inits ran (pure, in order) before the perform on this spine, so
            // scoping the abort value under them changes no evaluation order; it only restores what the
            // collapsed wrapper carried.
            if abort_before.is_none()
                && let Some(av) = ctx.abort_value.get()
            {
                let rewrapped = db.push_list(vec![let_head, rbindings, av]);
                ctx.abort_value.set(Some(rewrapped));
            }
            Some((wrapped, cur))
        }
        // A NESTED `handle` in the handled body. TWO ways it composes:
        //
        // (1) MERGED context (the two-nested-states case): the inner handle's body reaches a RECURSIVE
        //     callee that performs BOTH the inner AND an outer effect — so neither handler alone can fold
        //     it (the inside-out path below would leave the outer performs inside a specialization keyed on
        //     the inner effect only). Instead MERGE the inner handler's slot into the outer context (the
        //     union of arms, the inner slot APPENDED after the outer slots), push the inner `init` as the
        //     new slot's incoming state, and thread the inner body under the merged context. The recursive
        //     callee then specializes ONCE against the combined context — `f#ctx(args…, s_outer…, s_inner)`
        //     — threading each effect's state as its own trailing param (`DESIGN-effects-rcdzc.md` §4.3).
        //     After threading, the outer slots' states are the merged vector's PREFIX (the inner slot's
        //     final state is discarded — the handle's value is its body's value).
        // (2) INSIDE-OUT (the existing non-merged path): reduce the inner handle in isolation
        //     (`reduce_handle` discharges ITS effect, rewriting its performs to plain code), then thread the
        //     reduced result — which may still perform an outer effect — under the OUTER context. So
        //     `(handle_B … (handle_A … body))` folds `A` away first, leaving `B` performs for `B`'s fold.
        //     Used when the inner body does NOT reach a recursive callee performing an outer effect.
        Resolved::Handle {
            init: inner_init,
            arms: inner_arms,
            body: inner_body,
        } => {
            if let Some(merged) = merged_nested_ctx(db, inner_init, &inner_arms, inner_body, ctx) {
                // CALLER-OBSERVED OUT-STATE under the MERGE (recursive-nested-arm-resume fix). `reduce_handle`
                // runs `mark_caller_observed_outstate` for the OUTER ctx, but the merged body threads HERE
                // (not via reduce_handle), so a post-recursion sibling that observes the merged callee's
                // out-state — `(+ (loop 1) (A.get))`, where `(A.get)` reads the A-advance `loop` made per
                // iteration — is not marked, and the merged spec stays single-return (dropping the advance:
                // 20 vs 21). Mark it under the MERGED ctx so `specialize_recursive` emits multi-value and
                // threads the outer out-state to the observer. Additive (only upgrades a threadable callee).
                mark_caller_observed_outstate(db, inner_body, &merged);
                // Thread the inner body under the merged context, with the inner slot seeded by its init
                // (appended after the outer states). The merged vector = outer states ++ [inner init].
                let mut merged_states = states.clone();
                merged_states.push(inner_init);
                // DRAIN the merged body's pending MULTIVALUE temps (rn post-observer fix, increment 1 tail).
                // A caller-observed merged spec call in `inner_body` — `(+ (loop 1) (A.get))`, where the
                // post-loop `(A.get)` observes `loop`'s A-advance — emits a multi-value spec call let-bound to
                // `{spec}$t{k}`, pushed to `merged.pending`, with `(. t 0)` in its place. The single-handler
                // path drains these at reduce_handle's tail (line ~2528), but the MERGED body threads HERE via
                // `thread_bounded`, which never drained → `(. loop$acc#eff$t0 0)` referenced an unbound `$t0`
                // (CDZ0101). Mark the pending length, thread, then `drain_and_wrap` binds every temp this body
                // produced into wrapping `let`s around `rbody`. No-op when nothing pending (byte-identical to
                // before for a merged body with no multivalue call — the common case).
                let mark = merged.pending.borrow().len();
                let (rbody, out) =
                    thread_bounded(db, inner_body, merged_states, &merged, inline_depth)?;
                let rbody = drain_and_wrap(db, &merged, mark, rbody);
                // Drop the inner slot's final state; return the OUTER slots' states (the prefix).
                let outer_states = out[..states.len()].to_vec();
                Some((rbody, outer_states))
            } else {
                let reduced = reduce_handle(db, inner_init, &inner_arms, inner_body, false)?;
                // Thread the reduced result (which may still perform an outer effect) under the outer ctx.
                thread_bounded(db, reduced, states, ctx, inline_depth)
            }
        }
        // An ordinary application / arithmetic / comparison / connective / `not` over sub-expressions:
        // thread state through the operands in left-to-right order, rebuilding the same head. This
        // covers `(+ (E.op) 1)`, `(List.push s (E.op))`, etc. The head itself is not a perform (that
        // arm above caught it), so it is copied as-is.
        Resolved::Apply { head, args } => {
            let abort_before = ctx.abort_value.get();
            let mut cur = states;
            let (rhead, next0) = thread_or_copy(db, head, cur, ctx, inline_depth)?;
            cur = next0;
            let mut children = vec![rhead];
            // STRICT-OPERAND ABORT-LIFT (the operand face of the abort-outer-advance fix; do-shape landed
            // #2002). When a strict operand ABORTS — `(+ (A.tick) (B.bail 99))` under B — the operands
            // evaluated BEFORE it on the strict spine have already run; a FOREIGN one (an outer handler's
            // effect) committed its state advance and must survive the abort. But rebuilding `(+ (A.tick)
            // 99)` leaves the foreign perform inside a DEAD arithmetic wrapper that reduce_handle's bare-abort
            // collapse discards (the outer `(A.get)` then reads the pre-advance state → 109 vs 110). Lift the
            // pre-abort foreign operands into a for-effect `do` PREFIX around the abort value — `(do (A.tick)
            // 99)` — the SAME shape the do-arm produces, which the landed do-shape fold in reduce_handle then
            // preserves (the outer fold discharges the prefix, advancing the outer state, before the value).
            // Done in `thread` (not reduce_handle) because only here can we preserve UNCONDITIONALLY: sound for
            // BOTH the observed miscompile (→110) AND the correct-because-unobserved deep-nested case ("a
            // strict-operand abort in a DEEP-nested handler stack keeps its 99 when the advances are
            // UNOBSERVED") `(+ (A.a) (+ (B.b) (Bail.bail 99)))` (→ nested `(do (A.a) (do (B.b) 99))` = 99 still, the prefix runs
            // unobserved). `body_reaches_foreign_perform` reads the ORIGINAL operand `a` (parented — no orphan
            // resolve-pin poison, and equivalent to the rewrite: a foreign perform is never folded by this ctx,
            // a discharged one is not foreign). Only lifts when a foreign operand actually PRECEDED the abort
            // (`!kept_foreign.is_empty()`); a plain `(+ 5 (B.bail 99))` / `(+ (B.bail 7) (ask.ask))` keeps the
            // bare-abort collapse (nothing committed before the abort — 7, ask elided).
            let mut kept_foreign: Vec<StructId> = Vec::new();
            let mut abort_tail: Option<StructId> = None;
            for &a in args.iter() {
                let pre_abort = abort_before.is_none() && ctx.abort_value.get().is_none();
                let foreign = pre_abort && body_reaches_foreign_perform(db, a, ctx);
                let (ra, next) = thread_bounded(db, a, cur, ctx, inline_depth)?;
                cur = next;
                children.push(ra);
                if abort_before.is_none() && abort_tail.is_none() && ctx.abort_value.get().is_some()
                {
                    // THIS operand fired the abort (cell None→Some): `ra` is the abort value → the do-tail.
                    abort_tail = Some(ra);
                } else if abort_tail.is_none() && foreign {
                    // A pre-abort foreign operand: keep it for the for-effect prefix.
                    kept_foreign.push(ra);
                }
            }
            // NESTED-OPERAND face (breaker ax4): the aborting operand may ITSELF be an inner strict form
            // whose abort-lift already produced a `(do <foreign-prefix> abort-value)` tail — `(+ 999 (+
            // (A.tick) (B.bail 99)))` threads the inner `+` to `(do (A.tick) 99)`. This level's `kept_foreign`
            // is empty (999 pure), so rebuilding `(+ 999 (do (A.tick) 99))` buries the foreign prefix in a
            // DEAD arithmetic wrapper the bare-abort collapse discards → the advance is lost (109 vs 110,
            // silent cross-backend). The abort abandons the whole `+` anyway, so collapse to the tail
            // directly (dropping the dead pure siblings), preserving its do-prefix. NARROW: only when the
            // tail is ALREADY a `(do …)` (an inner abort-lift's output) — a bare-value tail keeps the
            // existing bare-abort collapse (which is correct + avoids perturbing the #seed-let scoping the
            // broad `reaches_foreign` gate broke — 9 regressions).
            let tail_is_lifted_do = abort_tail.is_some_and(|t| db.ast.as_form(t, "do").is_some());
            if let Some(tail) = abort_tail
                && (!kept_foreign.is_empty() || tail_is_lifted_do)
            {
                let do_head = db.push_name("do");
                let mut ch = vec![do_head];
                ch.extend(kept_foreign);
                ch.push(tail);
                return Some((db.push_list(ch), cur));
            }
            // OPERAND ABORT ABANDONS PURE SIBLINGS (breaker ax9/ah-x2). When an operand ABORTED and there
            // is no foreign prefix to preserve (`kept_foreign` empty) and the tail is a bare abort value
            // (not a lifted `do`), rebuilding `(op <pure siblings> <abort-value>)` — `(+ 999 99)` — SPLICES
            // the abort value into a dead arithmetic form. At the reduce_handle TOP LEVEL a downstream
            // bare-abort collapse reduces that away, but when this `(+ 999 (B.bail 99))` is a `do`-ITEM (or
            // any non-top position) nothing collapses it → the spliced `(+ 999 99)` = 1098 rides forward as
            // the value (ax9 → 1109). The abort abandons the whole operator application, so its value IS the
            // abort value: collapse to `tail` directly, dropping the pure siblings. Sound — the siblings are
            // pure (no `kept_foreign`), so dropping them discards no effect; the abort cell stays set so the
            // enclosing fold treats this as the abort. (A foreign sibling would have populated `kept_foreign`
            // → the do-prefix branch above; a lifted-do tail → that branch too. This is the bare-value,
            // pure-siblings-only residue.)
            if let Some(tail) = abort_tail {
                return Some((tail, cur));
            }
            Some((db.push_list(children), cur))
        }
        // A node that performs nothing — a literal, a bare reference, a param, unit, a type value, a
        // fully-non-effect subtree. It leaves the states unchanged. Copy it structurally so the rewritten
        // body is self-contained (a fresh occurrence re-resolving against the rewritten scope).
        _ if !subtree_performs(db, node, ctx) => {
            let copied = copy_pure(db, node);
            Some((copied, states))
        }
        // Some other form that DOES contain a perform but is not one of the shapes we thread (e.g. an
        // `if`/`match`/`let` with a perform inside — E1c-2/E3 territory). Decline.
        _ => None,
    }
}
