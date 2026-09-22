//! The K1/#4139 grandchild-consume walk (`arm_consumes_binder_grandchild`) — extracted from select.rs to
//! keep it under xtask_support::MAX_SOURCE_BYTES (512 KiB). Pure code move, behavior-neutral; the predicate
//! and rationale are unchanged (see the fn doc). `use super::*` brings the select module items into scope,
//! and `payload_or_proj_chain_roots_at_binder` is reclaim's `pub(super)` helper.
use super::reclaim::payload_or_proj_chain_roots_at_binder;
use super::*;

/// Whether `id` CONSUMES a heap GRANDCHILD of the loop-param `binder` — a projection `(. e field)`
/// (`Proj`/`SumExpect`, or a non-`RestFrom` `SumPayload`, heap-typed) in a CONSUME position whose OPERAND is
/// a PROPER projection-chain of `binder` (an element `e` extracted from the loop-param list, `e != binder`,
/// so consuming only `e`'s field leaves `e` itself OWNED BY the list). This is the K1/#4139 loop-skip
/// over-free precondition: FBIP-reusing `binder` (a `RestFrom`) frees the old spine — and with it the still-
/// owned element `e` and its live grandchild — → use-after-free, so the preservation dup must NOT be skipped.
///
/// The DISTINGUISHER vs [`arm_borrows_heap_subvalue`] (which was depth-blind and over-retained clean
/// accumulators, v-mem #5090 report): a DIRECT element consumed — `(List.concat acc h)` where `h =
/// SumPayload{scrutinee: binder, path:[Elem]}`, operand IS `binder` — is MOVED OUT (ownership transfers), so
/// freeing the spine is safe and no dup is needed (FLATTEN, 05-compound). Only a consumed GRANDCHILD (operand
/// a PROPER chain of `binder`, not `binder` itself) fires. Confirmed empirically: FLATTEN's rhs is a direct
/// `SumPayload{scrutinee: binder}` (excluded → reclaims); ksd1's is `Proj{operand: SumPayload{scrutinee:
/// binder}}` (a grandchild → fires → K1 UAF fence holds).
///
/// UAF-SAFE-BY-CONSTRUCTION (biased toward FIRING = keep the dup): only a match SCRUTINEE / borrowing-
/// projection OPERAND — genuine reads — relax to `borrowed` (skipped, since a borrowed grandchild does not
/// over-free); EVERY other position recurses CONSUMING, so a genuine grandchild-consume is never MISSED (a
/// miss = the UAF). An over-fire (a grandchild in a key/probe borrow this simplified walk does not relax, vs
/// the full [`arm_borrows_heap_subvalue`]) only KEEPS a dup → a leak, never a double-free.
pub(super) fn arm_consumes_binder_grandchild(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    // ORDERING-ADMIT excuse (v-core-opt-ruled #4139 relaxation, ksd 0262): `Some((scope, dup_sites))` excuses
    // a grandchild consume DEAD-AFTER under `dup_sites=Some` from forcing the preservation dup (see the
    // `is_grandchild_consume` block). `None` = the original K1 fence (every grandchild consume fires).
    excuse: Option<(StructId, &HashSet<StructId>)>,
) -> bool {
    let mut seen = HashSet::new();
    arm_consumes_binder_grandchild_seen(db, id, binder, false, excuse, &mut seen)
}

pub(super) fn arm_consumes_binder_grandchild_seen(
    db: &mut Db,
    id: StructId,
    binder: StructId,
    borrowed: bool,
    excuse: Option<(StructId, &HashSet<StructId>)>,
    seen: &mut HashSet<(StructId, bool)>,
) -> bool {
    if !seen.insert((id, borrowed)) {
        return false;
    }
    // A CONSUMED heap grandchild of `binder`: a projection whose OPERAND is a PROPER projection-chain of
    // `binder` (operand roots at `binder` but is NOT `binder` itself — the intermediate element stays owned).
    if !borrowed {
        // GRANDCHILD = the projection's OPERAND itself roots at `binder` through a projection (an element of
        // `binder`), NOT a DIRECT `Param`/`LocalRef` to `binder`. A direct-element projection
        // `SumPayload{scrutinee: <ref to binder>}` (FLATTEN's `h`) has its operand a bare binder reference —
        // that element is MOVED OUT when consumed, so it is NOT a grandchild. `is_direct_binder_ref` peels the
        // binder-ID-vs-reference-node distinction (the `Param{binder}` node id differs from the binder id).
        let is_direct_binder_ref = |db: &mut Db, n: StructId| matches!(core_of(db, n), Core::Param { binder: b } | Core::LocalRef { binder: b } if b == binder);
        let is_grandchild_consume = match core_of(db, id) {
            Core::Proj { operand, .. }
            | Core::SumExpect {
                scrutinee: operand, ..
            } => {
                is_heap_type(&type_of(db, id))
                    && !is_direct_binder_ref(db, operand)
                    && payload_or_proj_chain_roots_at_binder(db, operand, binder)
            }
            Core::SumPayload {
                scrutinee,
                ref path,
            } => {
                !matches!(path.last(), Some(crate::core::PathStep::RestFrom(_)))
                    && is_heap_type(&type_of(db, id))
                    && !is_direct_binder_ref(db, scrutinee)
                    && payload_or_proj_chain_roots_at_binder(db, scrutinee, binder)
            }
            _ => false,
        };
        if is_grandchild_consume {
            // ORDERING-ADMIT (v-core-opt-ruled #4139 relaxation, ksd 0262): with an `excuse` (ONLY the
            // self-tail-loop is_restfrom_consume back-edge, where PART-2 sequences this head-consume BEFORE
            // the RestFrom vec-drop), a grandchild consume DEAD-AFTER under `dup_sites=Some` (dup-backed like
            // `concat acc e.val`, or borrow-only) is NOT counted — its refs are dup-balanced before the spine
            // frees, so skipping the preservation dup cannot dangle it. An escaping un-dup'd grandchild (move /
            // non-dup-backed thread) is NOT excused → fires → dup kept (leak-over-UAF). `None` = K1 unchanged.
            let excused = matches!(excuse, Some((scope, dup_sites))
                if !binding_escapes_dup_aware(db, scope, EscapeTarget::Node(id), true, Some(dup_sites)));
            if !excused {
                return true;
            }
        }
    }
    // Position walk (mirrors [`arm_borrows_heap_subvalue_seen`]'s genuine-borrow relaxations; every other
    // position stays CONSUMING so a grandchild-consume is never missed).
    match core_of(db, id) {
        Core::Match { scrutinee, .. }
        | Core::MatchSum { scrutinee, .. }
        | Core::MatchList { scrutinee, .. } => {
            arm_consumes_binder_grandchild_seen(db, scrutinee, binder, true, excuse, seen)
                || core_child_ids(db, id).into_iter().any(|c| {
                    c != scrutinee
                        && arm_consumes_binder_grandchild_seen(db, c, binder, false, excuse, seen)
                })
        }
        Core::Proj { operand, .. }
        | Core::SumExpect {
            scrutinee: operand, ..
        }
        | Core::ListLen { operand }
        | Core::BytesLen { operand }
        | Core::StrScalarLen { operand } => {
            arm_consumes_binder_grandchild_seen(db, operand, binder, true, excuse, seen)
        }
        Core::SumPayload { scrutinee, .. } => {
            arm_consumes_binder_grandchild_seen(db, scrutinee, binder, true, excuse, seen)
        }
        // The SAME borrow relaxations as [`arm_borrows_heap_subvalue_seen`] — REQUIRED so a grandchild read
        // ONLY as a borrowed key/probe/compare/scalar-extract is not mistaken for a consume (omitting them
        // over-fired Map.to-list's `Bytes.at k 0` borrowed key → a spurious dup/leak, 19-sets:1878). `Bytes.at`
        // scalar-extracts (bytes borrowed); `Bytes.compact` passes the borrow status through; the key-ops
        // borrow their key/probe/compare operands (and consume the collection).
        Core::BytesAt { bytes, index, .. } => {
            arm_consumes_binder_grandchild_seen(db, bytes, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, index, binder, false, excuse, seen)
        }
        Core::BytesCompact { operand } => {
            arm_consumes_binder_grandchild_seen(db, operand, binder, borrowed, excuse, seen)
        }
        Core::MapLookup { map, key, .. } => {
            arm_consumes_binder_grandchild_seen(db, key, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, map, binder, false, excuse, seen)
        }
        Core::SetContains { set, elem, .. } => {
            arm_consumes_binder_grandchild_seen(db, elem, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, set, binder, false, excuse, seen)
        }
        Core::MapRemove { map, key, .. } => {
            arm_consumes_binder_grandchild_seen(db, key, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, map, binder, false, excuse, seen)
        }
        Core::SetRemove { set, elem, .. } => {
            arm_consumes_binder_grandchild_seen(db, elem, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, set, binder, false, excuse, seen)
        }
        Core::ValueEq { lhs, rhs }
        | Core::ValueEqShaped { lhs, rhs, .. }
        | Core::ValueCmp { lhs, rhs, .. } => {
            arm_consumes_binder_grandchild_seen(db, lhs, binder, true, excuse, seen)
                || arm_consumes_binder_grandchild_seen(db, rhs, binder, true, excuse, seen)
        }
        _ => core_child_ids(db, id)
            .into_iter()
            .any(|c| arm_consumes_binder_grandchild_seen(db, c, binder, false, excuse, seen)),
    }
}
