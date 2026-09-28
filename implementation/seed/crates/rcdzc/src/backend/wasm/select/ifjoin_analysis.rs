//! If-join / divergent-arm BORROW-DUP analysis for the select emit pass (peeled from `select/emit.rs` for the
//! size-lint cap). The join-liveness-aware drop planner and the divergent-arm dup-equalize predicates the
//! `emit` pass consults when a heap handle threads through control-flow joins: `plan_ifjoin_nested` (the
//! nested-If per-arm D-arm drop planner), `ifjoin_arm_dead` (an arm neither escapes nor consumes the binder),
//! `divergent_match_borrow_dupable` / `divergent_if_borrow_dupable` (the node#6 match-/if-join borrow-dup
//! equalize), `scrut_reemit_safe` (is re-emitting a scrutinee a fresh independently-owned rebuild — the
//! `Core::SumPayload` owned-producer reclaim precondition), and `owned_proj_child_dupd` (the Proj-child
//! drop-iff-dup'd mirror). Re-exported via `select`'s `pub(crate) use ifjoin_analysis::*`, so every call site
//! (emit.rs, select.rs, tests.rs) resolves unchanged. Pure mechanical split — no logic change.

use super::*;

/// NESTED IF-JOIN per-arm drop planner (v-memory-safety, join-liveness aware). Walk the If-subtree at
/// `node` and, for the `slot`'s handle (alias set = the binding ∪ the bare ref its value materializes — a
/// runtime bin-match scrutinee's `Let{(inner, Param(p))}`, where the return arms reference `Param(p)`
/// directly while `inner` is used only for borrows), plan a D-arm drop at EACH DIVERGENT nested If.
///
/// An arm is DEAD for the binding only if it neither ESCAPES it (dup-aware) NOR CONSUMES it: escape via the
/// result is caught by `binding_escapes_dup_aware`, and a live-carrying CONSUME (a call arg — INCLUDING a
/// recursive-call arg that threads the binding onward — or a ctor field) by `count_param_consumes` (which,
/// unlike the dup-aware escape, is NOT fooled by a dup_site: a dup'd recursive-call arg still counts). This
/// count guard is the join-liveness fix for the #8976 UAF: `binding_escapes_dup_aware(Some(dup_sites))`
/// returns false for a binding consumed at a dup_site ("the slot survives, drop it"), which is sound only at
/// the WHOLE-BODY scope where `dup_sites` was balanced — at a nested If where the binding threads into a
/// RECURSIVE continuation that reads it, the count guard keeps it LIVE (v-json-codec's parse-* OOB). A
/// `(bytes r)` dup-before-slice is NOT a consume of the binding (it mints a fresh slice, `count==0`), so
/// drain-digits still reclaims its dead scrutinee. `count_restfrom=false`: a RestFrom mints a new value, not
/// a live carry of the binding.
/// Is re-emitting `id` a FRESH independently-owned rebuild — the soundness precondition for the
/// `Core::SumPayload` owned-producer reclaim (#9532), which re-emits the scrutinee once per projection and
/// drops it after the leaf read? A DIRECT single-node allocating producer (a call/ctor/closure) is: each
/// re-emit rebuilds a fresh handle, so the per-projection drop balances. A CONTROL-FLOW-JOIN node
/// (`If`/`Match`/`MatchList`/`MatchSum`) is NOT: re-emitting duplicates its whole arm structure and — in an
/// effect-handler arm feeding a `resume` (14c tt5) — re-references the resume continuation's one-shot fn
/// index → `u32::MAX` → invalid component (#9533 fenced these). REFINEMENT (#9533 follow-up, v-memory-safety):
/// PEEL a `Core::Let` and test the value it actually produces, rather than excluding every `Let`. A
/// `let`-wrapped DIRECT producer — the 06-numeric fraction-add pin `r = (fadd …)`, a straight-line Call — is
/// a fresh rebuild, so it stays eligible (restored to live-objects 0); a `let`-of-`if` recurses to the `If`
/// head and is (correctly) still excluded. #9533's blanket `Let` exclusion over-fenced the fraction-add pin
/// to a safe known-leak; peeling it is strictly more precise (never re-includes a join). A join that leaks is
/// left un-dropped — leak-over-UAF; a miscompile is never acceptable.
pub(crate) fn scrut_reemit_safe(db: &mut Db, id: StructId) -> bool {
    match core_of(db, id) {
        Core::Let { body, .. } => scrut_reemit_safe(db, body),
        Core::If { .. } | Core::Match { .. } | Core::MatchList { .. } | Core::MatchSum { .. } => {
            false
        }
        _ => true,
    }
}

pub(crate) fn ifjoin_arm_dead(
    db: &mut Db,
    arm: StructId,
    aliases: &HashSet<StructId>,
    dup: &HashSet<StructId>,
    net_borrow: bool,
) -> bool {
    for &a in aliases {
        if binding_escapes_dup_aware(
            db,
            arm,
            EscapeTarget::Binder(a),
            false,
            Some(dup),
            false,
            false,
        ) {
            return false;
        }
        let mut seen = HashSet::new();
        let mut n = 0usize;
        count_param_consumes(db, arm, a, &mut seen, &mut n, false);
        // DEAD arm: neither escapes (checked above) NOR consumes (`n == 0`).
        //
        // NET-BORROW arm (PER-PATH AXIS B, v-core-opt-blessed): `n > 0` but the binder's OWN ref is
        // nonetheless surplus/dead-after this arm, so dropping it once reclaims it. This is sound BECAUSE
        // `binding_escapes_dup_aware(…, Some(dup))` above already returned false: a consume that is NOT
        // dup-backed (a genuine last-use MOVE) reads as an ESCAPE under the dup-aware oracle → it would have
        // returned `true` and we'd have bailed. So `escape == false && n > 0` ⟺ EVERY consume on this arm is
        // DUP-BACKED (net-borrow) — the emitted dups service the consumes and leave the incoming ref undropped
        // (the effect-handler state-accumulator leak: rope2 14b:127 + net-borrow recursive-fold siblings). The
        // base-MOVE arm (an un-dup'd consume) is an escape → declined above → gets NO drop (dropping it would
        // double-free). Admitted ONLY under `net_borrow`, which the caller sets iff GATE-1 holds
        // (`!def_nonlooped_callee_reclaims_threaded_param` — an arm that already reclaims via the conditional
        // threaded-param drop must not get a second drop; closes the go two-sibling double-free). GATE-2 (lf1's
        // handle-continuation capture-escape) is caught by the dup-aware oracle when the capture-construction
        // consumes the binder, and empirically backstopped by the mandatory lf1 census negative control.
        if n != 0 && !net_borrow {
            return false;
        }
    }
    true
}

/// MATCH-JOIN OWNERSHIP-EQUALIZE detector (v-memory-safety co-design, node#6; the `Core::Match` analogue of
/// the FIX-A `Core::If` ownership-equalize). When a heap `Core::Match` is a BORROW operand of a length-op
/// (`Core::BytesLen`), its arm-blind ownership join (`heap_operand_ownership` → `join_arm_ownership`) reads
/// `Borrowed` the moment ONE arm is a bare alias (a `LocalRef`/`Param`), which SUPPRESSES the borrow-op's
/// post-borrow owned-operand drop → an OWNED-FRESH sibling arm's allocation LEAKS (node#6 = the mode-2
/// `String.concat`). This detects the DIVERGENT-OWNERSHIP case that is safe to equalize. Admit iff EVERY arm
/// is one of: (i) OWNED-FRESH (`heap_operand_ownership==Owned`); (ii) a bare-ALIAS `LocalRef`/`Param{B}` of an
/// OWNED+LIVE-AFTER binder `B`, dup-safe iff `keep_scope_drop_despite_body_escape(fn_body, B, dup_sites)` (my
/// c2236 predicate: TRUE = arm-result alias re-borrowed post-body = B has its own surviving reclaim; FALSE = a
/// real transfer, e.g. #8976 / the src-not-live-after DFBAR → NOT admitted); or (iii) a WHITELISTED STATIC
/// literal (`returns_immortal_singleton` or a `ConstStr`/`ConstBytes` — a POSITIVE whitelist, never a fallback:
/// a borrowed VIEW arm must NOT read as static, else the forced drop double-frees its source). AND there is
/// ≥1 owned-fresh arm AND ≥1 dup-safe-alias arm (genuinely divergent; a uniform match is unaffected).
/// Returns the bare-alias arm-BODY ids to `dup` (so the joined temp becomes uniformly OWNED and the
/// borrow-op's forced post-borrow drop is sound on every path — net-zero on `B`). ANY unclassifiable arm →
/// `None` (conservative leak, never a guess). The dup is emitted at [`emit_arm_body`] (keyed by body id);
/// the caller sets `reclaim=true`. LOAD-BEARING: force-reclaim ⟺ every non-owned-fresh arm dup'd/static.
pub(crate) fn divergent_match_borrow_dupable(
    db: &mut Db,
    operand: StructId,
    slots: &HashMap<StructId, u32>,
    fn_body: Option<StructId>,
    dup_sites: &HashSet<StructId>,
) -> Option<Vec<StructId>> {
    // Collect the arm BODY ids for either a scalar/sum `Core::Match` or a runtime-list `Core::MatchList`
    // (node#6 operand-node-kind extension #2, v-core-opt SCENARIO-B co-design 085417). Both are
    // divergent-arm heap producers whose arm-blind ownership join reads `Borrowed` when ≥1 arm is a bare
    // alias, so the length-op's post-borrow reclaim is suppressed and the owned-fresh arm leaks. The
    // classification + emit path is IDENTICAL: a `MatchList` alias arm body routes through `emit_arm_body`
    // (dispatch.rs 526/608/631, `arm_slots` a superset of `slots` — v-core-opt RED-review), which consults
    // `matchjoin_dup_arms`. The branchless-list `select` fold (dispatch.rs) EXCLUDES heap-result arms, so a
    // divergent-heap `MatchList` is always the block form → the dup fires on exactly one path (Match-equivalent
    // safety). SELECT-safe, DFBAR-safe (keep_scope self-excludes a transfer alias), same as the Match family.
    let arm_bodies: Vec<StructId> = match core_of(db, operand) {
        Core::Match { arms, .. } => arms.iter().map(|a| a.body).collect(),
        Core::MatchList { arms, .. } => arms.iter().map(|a| a.body).collect(),
        _ => return None,
    };
    let fn_body = fn_body?;
    let mut alias_ids: Vec<StructId> = Vec::new();
    let mut has_owned_fresh = false;
    for body in arm_bodies {
        if matches!(heap_operand_ownership(db, body), Ok(HandleOwnership::Owned)) {
            has_owned_fresh = true;
            continue;
        }
        if let Core::LocalRef { binder } | Core::Param { binder } = core_of(db, body) {
            // Dup-safe ⟺ B is OWNED+LIVE-AFTER (has its own surviving reclaim so the dup+forced-drop is
            // net-zero on B) AND B has a materialized slot to `LocalGet` at the arm. Otherwise (a transfer,
            // or no slot) → conservative decline (leak-over-UAF).
            if slots.contains_key(&binder)
                && keep_scope_drop_despite_body_escape(db, fn_body, binder, dup_sites)
            {
                alias_ids.push(body);
                continue;
            }
            return None;
        }
        // POSITIVE whitelist for a drop-safe static arm (immortal singleton / Const producer). NOT a
        // catch-all: a borrowed-view arm (SumPayload/Proj/StrAt/SumExpect) is NOT static → decline.
        if returns_immortal_singleton(db, body)
            || matches!(core_of(db, body), Core::ConstStr(_) | Core::ConstBytes(_))
        {
            continue;
        }
        return None;
    }
    if has_owned_fresh && !alias_ids.is_empty() {
        Some(alias_ids)
    } else {
        None
    }
}

/// `Core::If` analogue of [`divergent_match_borrow_dupable`] (node#6 operand-node-kind extension, v-core-opt
/// carry-forward #2, keep_scope co-design). A length-op over a divergent-ownership `Core::If` borrow-operand
/// — one arm OWNED-FRESH, the other a dup-safe bare-ALIAS — leaks the owned-fresh arm (the arm-blind join
/// reads `Borrowed`). Returns the FIX-A `ifjoin_arm_dups` plan `[(binder_slot, dup_is_then)]` to `dup` the
/// alias arm so the `if` result is uniformly OWNED, REUSING FIX-A's blessed dup emit at the `Core::If` handler
/// (NO new emit site); the caller inserts it into `out.ifjoin_arm_dups` (debug_assert no-overwrite — an
/// inline-If length-op operand and a FIX-A let-value If are structurally disjoint `Core::If` nodes) and sets
/// `reclaim = true`. SELECT-SAFE: a heap-result `if` is always the if/else BLOCK form (`select` excludes heap
/// results), so the dup fires on EXACTLY ONE arm (net-zero on the binder), Match-equivalent. Dup-safety is
/// `keep_scope_drop_despite_body_escape` (its `arms_inherit_borrow` variant already covers `Core::If`
/// arm-results, reclaim.rs 1019/1132), so a DFBAR transfer (alias binder NOT live-after) fails keep_scope →
/// no dup — the same self-exclusion that held across the Match family. INLINE only: a let-bound
/// `(def r (if …)) (List.len r)` has operand `LocalRef(r)`, NOT a `Core::If` — a SEPARATE path, not fixed
/// here. ANY unclassifiable arm → `None` (conservative leak, never a guess).
pub(crate) fn divergent_if_borrow_dupable(
    db: &mut Db,
    operand: StructId,
    slots: &HashMap<StructId, u32>,
    fn_body: Option<StructId>,
    dup_sites: &HashSet<StructId>,
) -> Option<Vec<(u32, bool)>> {
    let Core::If { then_, else_, .. } = core_of(db, operand) else {
        return None;
    };
    let fn_body = fn_body?;
    let mut plan: Vec<(u32, bool)> = Vec::new();
    let mut has_owned_fresh = false;
    for (body, is_then) in [(then_, true), (else_, false)] {
        if matches!(heap_operand_ownership(db, body), Ok(HandleOwnership::Owned)) {
            has_owned_fresh = true;
            continue;
        }
        if let Core::LocalRef { binder } | Core::Param { binder } = core_of(db, body) {
            // Dup-safe ⟺ B is OWNED+LIVE-AFTER (its own surviving reclaim makes dup+forced-drop net-zero on
            // B) AND B has a materialized slot to `LocalGet` at the arm. A transfer / no slot → decline.
            if let Some(&bslot) = slots.get(&binder)
                && keep_scope_drop_despite_body_escape(db, fn_body, binder, dup_sites)
            {
                plan.push((bslot, is_then));
                continue;
            }
            return None;
        }
        // POSITIVE static whitelist (mirror the Match detector): immortal singleton / Const producer only.
        if returns_immortal_singleton(db, body)
            || matches!(core_of(db, body), Core::ConstStr(_) | Core::ConstBytes(_))
        {
            continue;
        }
        return None;
    }
    if has_owned_fresh && !plan.is_empty() {
        Some(plan)
    } else {
        None
    }
}

/// NESTED IF-JOIN per-arm drop planner (v-memory-safety, join-liveness aware). Walk the If-subtree at
/// `node` and, for the `slot`'s handle (alias set = the binding ∪ the bare ref its value materializes — a
/// runtime bin-match scrutinee's `Let{(inner, Param(p))}`, whose return arms reference `Param(p)` directly
/// while `inner` is used only for borrows), plan a D-arm drop at EACH nested If where the binding DIVERGES:
/// one arm is DEAD ([`ifjoin_arm_dead`]: neither escapes nor is live-consumed) and the other is LIVE. The
/// caller keeps the ORIGINAL dup-aware single-level check for the ROOT (let-body) If and runs this nested
/// walk ONLY when the binding escapes the whole body (post-body scope-drop suppressed → a nested drop is the
/// SOLE reclaim, never a double).
///
/// SOUNDNESS (no path double-drops): at a divergent If, plan the drop on the DEAD arm and recurse ONLY the
/// LIVE arm (dead ⇒ no deeper divergence). Both-live → recurse both. Both-dead → stop (leak-safe).
pub(crate) fn plan_ifjoin_nested(
    db: &mut Db,
    node: StructId,
    aliases: &HashSet<StructId>,
    slot: u32,
    dup: &HashSet<StructId>,
    net_borrow: bool,
    plan: &mut HashMap<StructId, Vec<(u32, bool)>>,
) {
    let Core::If { then_, else_, .. } = core_of(db, node) else {
        return;
    };
    let then_dead = ifjoin_arm_dead(db, then_, aliases, dup, net_borrow);
    let else_dead = ifjoin_arm_dead(db, else_, aliases, dup, net_borrow);
    match (then_dead, else_dead) {
        (false, false) => {
            plan_ifjoin_nested(db, then_, aliases, slot, dup, net_borrow, plan);
            plan_ifjoin_nested(db, else_, aliases, slot, dup, net_borrow, plan);
        }
        (true, false) => {
            plan.entry(node)
                .or_default()
                .push((slot, /* d_is_then = */ true));
            plan_ifjoin_nested(db, else_, aliases, slot, dup, net_borrow, plan);
        }
        (false, true) => {
            plan.entry(node)
                .or_default()
                .push((slot, /* d_is_then = */ false));
            plan_ifjoin_nested(db, then_, aliases, slot, dup, net_borrow, plan);
        }
        (true, true) => {}
    }
}

/// Whether `id` is a NESTED-COMPOUND `Core::Proj` whose emit DUP'd the extracted child into a standalone
/// OWNED handle — i.e. the SAME gate the `Core::Proj` emit (this file) uses to `dup`-child + `drop`-record:
/// operand is OWNED (a fresh producer, e.g. `(mk i)`) + NOT slot-materialized + the projected element is a
/// NESTED-COMPOUND heap child (`get_op` `None`, not a scalar copy, not `Unit`). When true, the extracted
/// child is a fresh standalone-owned handle (rc1) that a BORROWING scalar-read consumer
/// (`Map.len`/`List.len`/`Bytes.len`/`Set.len`) must `drop` after its borrow — else it leaks (the Map.len-
/// over-a-projected-fresh-record disjoint-slot leak, corpus-05 #4547). DROP-IFF-DUP'D: this MUST mirror the
/// `Core::Proj` emit's dup gate EXACTLY — a mismatch is a double-free (drop with no dup) or a leak (dup with
/// no drop). Scalar elements (`get_op` `Some`) copy out and are NEVER dup'd here (so this returns false).
///
/// The `sumexpect_shell_reclaim` disjunct mirrors the parallel clause #9071 added to the `Core::Proj` emit
/// gate: an inner Proj whose operand is a SumExpect VIEW in the shell-reclaim set ALSO dup'd its extracted
/// child (the SumExpect emit dup'd the view; #9071's Proj gate then took the nested-compound-child dup
/// branch), so an OUTER borrowing read of it must likewise drop that child. Without this clause a DOUBLE
/// projection off the view (`(. (. (Option.expect (Map.lookup m k)) inner) x)`, V8) leaks the inner child —
/// the leak side of the mirror. Exact match: the inner Proj dup'd iff `!slots.contains(operand) && (Owned ||
/// shell-set) && get_op None && !Unit`, which is precisely this predicate, so restoring the disjunct keeps
/// dup==drop (never a double-free — a false-positive drop would be an unmatched reclaim).
///
/// The recursive `owned_proj_child_dupd(operand)` disjunct mirrors the SAME term already in the emit gate: a
/// NESTED projection `(. (. producer a) b)` has its INNER Proj `(. producer a)` dup its child (it was
/// Owned/shell), so the OUTER Proj `(. <inner> b)` ALSO takes the nested-child dup branch — a further read of
/// the outer must then drop ITS child. Without recursing here, a TRIPLE projection off a view/owned producer
/// (`(. (. (. (Option.expect (Map.lookup m k)) a) b) c)`) leaks the middle child: the outer Proj dup'd it
/// (its emit gate recursed through `owned_proj_child_dupd(inner)`) but this predicate — read by the final
/// read's reclaim gate — reported "not dup'd". Recursing makes the predicate EXACTLY the emit gate
/// (terminating on the finite operand chain); drop-iff-dup'd holds.
pub(crate) fn owned_proj_child_dupd(
    db: &mut Db,
    id: StructId,
    slots: &HashMap<StructId, u32>,
    sumexpect_shell_reclaim: &HashSet<StructId>,
) -> bool {
    if let Core::Proj { operand, .. } = core_of(db, id) {
        !slots.contains_key(&operand)
            && (matches!(
                heap_operand_ownership(db, operand),
                Ok(HandleOwnership::Owned)
            ) || sumexpect_shell_reclaim.contains(&operand)
                || owned_proj_child_dupd(db, operand, slots, sumexpect_shell_reclaim))
            && matches!(get_op(db, id), Ok(None))
            && !matches!(type_of(db, id).strip_nominal(), Ty::Unit)
    } else {
        false
    }
}
