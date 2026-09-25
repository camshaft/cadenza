//! The SURPLUS-skippable dup analysis (collect_surplus_skippable_dups) — extracted from select.rs
//! to keep it under xtask_support::MAX_SOURCE_BYTES (512 KiB). Pure code move, behavior-neutral; the
//! predicate + rationale are unchanged (see the fn doc). use super::* brings the select module items
//! (Core, core_of, count_param_consumes, is_heap_type, type_of, Emit, ...) into scope, as the sibling
//! select/* submodules do.
use super::*;

/// Populate `out` with the SURPLUS-skippable `dup_sites` occurrences (see [`Emit::surplus_skippable_dups`]):
/// the retain dups that are PROVABLY redundant in a boundary-owned body and may be skipped, the NARROW
/// replacement for the too-broad `body_is_boundary_owned`-alone gate (which stripped load-bearing retains =
/// 159 corpus UAFs). A `dup_sites` occurrence of binder `b` is surplus iff BOTH: (1) `b` is a MatchList
/// SCRUTINEE with a `(.. r)` REST-PATTERN arm (`ListArmCond::LenGe`/`Any`) — the RestFrom family, present
/// whether the rest binder is USED or DEAD; AND (2) `b` has NO consume OTHER than a RestFrom
/// (`count_param_consumes` with `count_restfrom=false` == 0). Rationale: in a BOUNDARY-OWNED body the caller
/// holds a live reference to `b` for the whole body, so a pure-BORROW read needs no retain; the keep-alive
/// `dup` exists ONLY to balance a later CONSUME, and for a rest-pattern match `b`'s only consume (if any) is
/// the `(.. r)` RestFrom, whose `vec-drop` already has its OWN balancer (the emit's RestFrom preservation dup)
/// — so the retain is redundant. Covers BOTH the DEAD rest (05:18721 `f` — 0 consumes) and a sole used
/// RestFrom. Conjunct 1 EXCLUDES non-list-rest borrows (a shared inner map, an RRB list as a map value, a
/// Bytes rope read twice — their keep-alive is load-bearing for value-heap sharing `count_param_consumes` does
/// not model); conjunct 2 EXCLUDES a rest scrutinee ALSO consumed by push/insert/escape/self-call (retain is
/// the SOLE balancer) — together the UAF classes the broad gate hit. Caller gates on `is_boundary_owned`.
/// `dup_sites` occurrences are `LocalRef`/`Param` nodes, so an occurrence's binder is read via `core_of`.
/// REFINE(A) (v-core-opt-committed, restores the scalar-keyed 05/22 owned-fold pins the coarse
/// `consuming>=1` conjunct false-declines): TRUE iff the heap leading-element extraction `n` is
/// DESTRUCTURED ENTIRELY TO SCALARS — every use of `n` is as the scrutinee of a `SumPayload` FIELD
/// projection (`SumPayload{scrutinee: n}`), there is at least one such field, and EVERY field is
/// SCALAR (`is_heap_type` false). A scalar field is COPIED at the match (value semantics), so it does
/// not alias `n`'s cell; once every field is copied out, freeing `n`'s cell after the RestFrom vec-split
/// dangles nothing. If `n` has ANY parent that is NOT such a field projection (a Call/List*/Tuple/… —
/// i.e. `n` used as a WHOLE HEAP VALUE), or any field is itself heap (a lingering heap child borrow),
/// this returns FALSE and the head falls to the coarse `consuming>=1` gate (06: heap child consumed) or
/// declines (choreography: lingering heap borrow). No read-ordering needed: the all-scalar-field copy is
/// unconditionally safe. Measured: 05/22 heads all-scalar->admit; ALL 333 choreography heads not-all-
/// scalar->decline (zero false-admit); 06 heap-field->coarse.
fn head_destructured_only_to_scalars(db: &mut Db, body: StructId, n: StructId) -> bool {
    fn find_parents(
        db: &mut Db,
        id: StructId,
        target: StructId,
        out: &mut Vec<StructId>,
        seen: &mut HashSet<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        let kids = core_child_ids(db, id);
        if kids.contains(&target) {
            out.push(id);
        }
        for c in kids {
            find_parents(db, c, target, out, seen);
        }
    }
    let mut parents = Vec::new();
    let mut seen = HashSet::new();
    find_parents(db, body, n, &mut parents, &mut seen);
    if parents.is_empty() {
        return false;
    }
    let mut has_field = false;
    for p in parents {
        match core_of(db, p) {
            Core::SumPayload { scrutinee, .. } if scrutinee == n => {
                has_field = true;
                if is_heap_type(&crate::infer::type_of(db, p)) {
                    return false; // a heap field lingers as a borrow -> not scalar-only
                }
            }
            // any non-field-projection parent = `n` used as a whole heap value -> not destructure-only
            _ => return false,
        }
    }
    has_field
}

pub(super) fn collect_surplus_skippable_dups(
    db: &mut Db,
    body: StructId,
    dup_sites: &HashSet<StructId>,
    // OWNED-FOLD extension (operator-funded, v-core-opt owns the dead-after GATE): `true` for a
    // CALLEE-OWNED self-recursive fold (body NOT `is_boundary_owned`, the surplus.rs gap the caller
    // now also runs for). It RELAXES conjunct 3 (the heap-leading-element + rest-read exclusion, the
    // #7255 co-element dangle): a self-recursive fold consumes its list via the `RestFrom` `vec-drop`
    // (which frees leading element 0), and the head's preservation `dup` of the scrutinee is SURPLUS iff
    // every heap value extracted from the leading element is DEAD-AFTER — read (a borrow) and dropped
    // BEFORE the split, never kept/threaded/returned. Then skipping the dup lets the `vec-drop` reclaim
    // the head each iteration (ratwalk_noth 18@n=3 → 0). If ANY heap-leading extraction ESCAPES (threaded
    // into the recursive call, returned) the dup is load-bearing → keep it (leak-over-UAF; ratwalk's
    // threaded key stays leaking). Gated per-node by `binding_escapes_dup_aware` (v-core-opt's verified
    // oracle; `tail_borrowed=true`, `dup_sites=None`); guarded-all is the UAF net. Boundary-owned callers
    // pass `false` (conjunct 3 unconditional, unchanged).
    owned_fold: bool,
    out: &mut HashSet<StructId>,
) {
    use crate::core::ListArmCond;
    // (1) Binders that are a MatchList scrutinee with a `(.. r)` REST-PATTERN arm (LenGe/Any) — the RestFrom
    // family, present whether the rest binder is USED or DEAD. This EXCLUDES non-list-rest borrows (a shared
    // inner map, an RRB list as a map value, a Bytes rope read twice) whose keep-alive is load-bearing for
    // value-heap sharing `count_param_consumes` does not model.
    fn gather_rest_scrutinees(
        db: &mut Db,
        id: StructId,
        out: &mut HashSet<StructId>,
        seen: &mut HashSet<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::MatchList { scrutinee, arms } = core_of(db, id)
            && let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, scrutinee)
            && arms
                .iter()
                .any(|a| matches!(a.cond, ListArmCond::LenGe(_) | ListArmCond::Any))
        {
            out.insert(binder);
        }
        for c in core_child_ids(db, id) {
            gather_rest_scrutinees(db, c, out, seen);
        }
    }
    let mut rest_scrutinees: HashSet<StructId> = HashSet::new();
    let mut seen = HashSet::new();
    gather_rest_scrutinees(db, body, &mut rest_scrutinees, &mut seen);
    if rest_scrutinees.is_empty() {
        return;
    }
    // (2) Keep only those with NO consume OTHER than a RestFrom (count_restfrom = false == 0) — excludes a rest
    // scrutinee ALSO consumed by push/insert/escape/self-call (its retain is the SOLE balancer for that consume).
    let mut surplus_binders: HashSet<StructId> = HashSet::new();
    for &b in rest_scrutinees.iter() {
        let mut cseen = HashSet::new();
        let mut nonrest = 0usize;
        count_param_consumes(db, body, b, &mut cseen, &mut nonrest, false);
        if nonrest == 0 {
            surplus_binders.insert(b);
        }
    }
    if surplus_binders.is_empty() {
        return;
    }
    // (3) EMIT-ORDERING SOUNDNESS (bisect #7255 / #7321; co-designed + measured with v-memory-safety):
    // a heap LEADING-element borrow only dangles if the scrutinee is FREED mid-body — which happens IFF
    // the REST is minted. A `RestFrom` read lowers to `vec-split`, and vec-split DROPS the leading (left)
    // elements 0..k-1, freeing their cells; a heap leading-element read (a BORROW aliasing element k's
    // cell) then dangles. A DEAD rest (no `RestFrom` read → no vec-split → measured: no `vec-drop` import,
    // e.g. 05:18721's `r`) never frees the scrutinee, so a heap leading-element cannot dangle. So EXCLUDE a
    // rest-scrutinee `b` from surplus if the body reads BOTH (a) a HEAP-typed leading element (a
    // `SumPayload` rooted at `b`, first path step `Elem(_)`) AND (b) the rest (a `SumPayload` rooted at `b`,
    // first path step `RestFrom`). SOUND-CONSERVATIVE: it also excludes a heap-leading read that happens
    // BEFORE the split (forgoes the opt, never a UAF). Keeps 05:18721 surplus (heap-leading YES, rest-read
    // NO — `r` dead → the leak-fix is preserved); excludes the choreography `a-list-eq` shape (`x` heap +
    // `xr` rest → the co-element dangle #7255 hit). v-mem --guarded-all-verified; v-wasm-opt choreography-verified.
    fn scan_scrutinee_reads(
        db: &mut Db,
        id: StructId,
        b: StructId,
        heap_leading_nodes: &mut Vec<StructId>,
        rest_read: &mut bool,
        seen: &mut HashSet<StructId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        if let Core::SumPayload { scrutinee, path } = core_of(db, id)
            && matches!(core_of(db, scrutinee), Core::Param { binder } | Core::LocalRef { binder } if binder == b)
        {
            match path.first() {
                Some(crate::core::PathStep::Elem(_))
                    if is_heap_type(&crate::infer::type_of(db, id)) =>
                {
                    // Collect the leading-element extraction NODE (not just a bool) so the owned-fold
                    // relaxation can dead-after-check each one.
                    heap_leading_nodes.push(id);
                }
                Some(crate::core::PathStep::RestFrom(_)) => *rest_read = true,
                _ => {}
            }
        }
        for c in core_child_ids(db, id) {
            scan_scrutinee_reads(db, c, b, heap_leading_nodes, rest_read, seen);
        }
    }
    let mut exclude: HashSet<StructId> = HashSet::new();
    for &b in surplus_binders.iter() {
        let (mut heap_leading_nodes, mut rest_read) = (Vec::new(), false);
        let mut s = HashSet::new();
        scan_scrutinee_reads(db, body, b, &mut heap_leading_nodes, &mut rest_read, &mut s);
        if !heap_leading_nodes.is_empty() && rest_read {
            // Conjunct 3: a heap leading-element read ALONGSIDE a rest read normally EXCLUDES `b` (the
            // vec-split frees the leading cells → a kept leading borrow dangles, #7255). OWNED-FOLD RELAX
            // (the COARSE liveness-across-vec-split predicate, v-core-opt-committed, replacing #9537's
            // escapes-only oracle): for a self-recursive fold, admit `b` iff for EVERY heap leading-element
            // extraction `n`, BOTH (a) `n` does NOT escape (`binding_escapes_dup_aware` — not threaded/
            // returned) AND (b) `n`'s payload has a CONSUMING site (`collect_consuming_payload_sites_expr`
            // non-empty = an independent-dup backing that keeps `n`'s live-after children alive across the
            // vec-split). escapes==false is NECESSARY BUT NOT SUFFICIENT: a pure-BORROW head element
            // (consuming_sites==0, e.g. the choreography roundtrip's head Ast) dangles when the vec-split
            // frees the head cell → DECLINE. The consuming conjunct is what distinguishes 06/05/03 (head's
            // threaded child consumed → admit, leak-fixed) from the choreography over-drop UAF (pure-borrow
            // head → decline, safe leak). collect_consuming_payload_sites_expr descends the head element's
            // payload subtree and sees THROUGH the inner-match re-root (06's k found despite Discriminant
            // re-root). Verified: 0 real traps across all 177 choreography @tests under guarded; 06-inc admits
            // clean (value 40). guarded-all is the standing net for the theoretical mixed-child residue.
            let mut all_dead_after = owned_fold;
            if owned_fold {
                for &n in &heap_leading_nodes {
                    let escapes = binding_escapes_dup_aware(
                        db,
                        body,
                        EscapeTarget::Node(n),
                        true,
                        None,
                        false,
                    );
                    let mut cons = HashSet::new();
                    collect_consuming_payload_sites_expr(db, body, n, true, &mut cons);
                    // REFINE(A) (v-core-opt-committed): the coarse `consuming>=1` conjunct is a PROXY for "no
                    // heap child of the head dangles across the split" that FALSE-DECLINES a SCALAR-only head
                    // (05/22 Int64/nullary-sum tries: the head is destructured entirely to SCALAR fields, which
                    // are COPIED at the match — value semantics, no alias to the head cell — so freeing the
                    // head after the split dangles nothing; yet consuming_sites==0 because scalars aren't a
                    // heap-payload consume). ADD the `head_destructured_only_to_scalars` disjunct: admit such a
                    // head too. Sound because a scalar-only destructure copies out; the choreography lingering-
                    // heap-borrow head (a heap child/whole-value borrow-read, NOT all-scalar) still DECLINES.
                    if escapes
                        || (cons.is_empty() && !head_destructured_only_to_scalars(db, body, n))
                    {
                        all_dead_after = false;
                        break;
                    }
                }
            }
            if !all_dead_after {
                exclude.insert(b);
            }
        }
    }
    for b in exclude {
        surplus_binders.remove(&b);
    }
    if surplus_binders.is_empty() {
        return;
    }
    for &id in dup_sites.iter() {
        if let Core::Param { binder } | Core::LocalRef { binder } = core_of(db, id)
            && surplus_binders.contains(&binder)
        {
            out.insert(id);
        }
    }
}
