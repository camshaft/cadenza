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
pub(super) fn collect_surplus_skippable_dups(
    db: &mut Db,
    body: StructId,
    dup_sites: &HashSet<StructId>,
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
        heap_leading: &mut bool,
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
                    *heap_leading = true;
                }
                Some(crate::core::PathStep::RestFrom(_)) => *rest_read = true,
                _ => {}
            }
        }
        for c in core_child_ids(db, id) {
            scan_scrutinee_reads(db, c, b, heap_leading, rest_read, seen);
        }
    }
    let mut exclude: HashSet<StructId> = HashSet::new();
    for &b in surplus_binders.iter() {
        let (mut heap_leading, mut rest_read) = (false, false);
        let mut s = HashSet::new();
        scan_scrutinee_reads(db, body, b, &mut heap_leading, &mut rest_read, &mut s);
        if heap_leading && rest_read {
            exclude.insert(b);
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
