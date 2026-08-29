//! Local-slot COALESCING — shrink a function's declared-local count by reusing non-interfering
//! slots, so `local.get`/`local.set`/`local.tee` indices get smaller LEB encodings. This mitigates
//! the local-slot BLOAT the effects lowering produces: a tail-resumptive handler distributes its
//! continuation into each branch, so a small source expands into thousands of sequential single-use
//! temps (e.g. the corpus `glb1` case emits 18,738 locals of which only ~7 are ever simultaneously
//! live — wasm-opt coalesces them, and this pass does the same at emit).
//!
//! SOUNDNESS (why this never miscompiles in FORWARD-only control flow, without a CFG): interference
//! is defined as OVERLAPPING `[first-mention, last-mention]` flat-instruction-index spans. With only
//! forward branches (`block`/`if`/`br`-to-`end`), a slot's live range is bounded by its def/use
//! occurrences — every point where it is live lies between its first and last mention, because there
//! is no path from after its last mention back to a use. Hence if two slots are simultaneously live
//! at a point `P`, then `P` lies in both spans and they overlap → we mark them interfering and do NOT
//! share a slot. So overlapping-span is a SUPERSET of real interference: we never coalesce two slots
//! live at once (sound); we may only conservatively keep some coalescable slots apart (a missed
//! opportunity, never a miscompile). Forward branches only SHRINK real liveness (mutually-exclusive
//! arms), so the span stays a sound superset. No explicit control-flow graph is needed.
//!
//! WARNING: LOOPS (back-edges) BREAK this: a `loop`'s `br` jumps BACKWARD, so a declared local read early in
//! a loop body is live ACROSS the back-edge — PAST its textual last mention (a later iteration
//! re-reads it) — which the flat span misses. The only loop the wasm backend emits is the
//! self-tail-call→loop transform, whose loop-carried state lives in PARAMETER slots (never coalesced),
//! but we do not assume a declared local is never loop-carried. So the CALLER (`select.rs`'s
//! `coalesce_func`) conservatively SKIPS coalescing any function that contains a `loop`. A loop-aware
//! span extension (widen a declared slot's span to cover any enclosing loop body) is a later slice
//! that would let loopy functions coalesce too; until then this analysis is applied to loop-free
//! functions only.
//!
//! PARAMETERS are fixed (slots `0..nparams` — the function's signature); only DECLARED locals
//! (`nparams..`) are coalesced, and only within the SAME `ValType` (an i32 and an i64 can never
//! share a slot). A free (non-`pinned`) unmentioned declared local is dead and is dropped entirely.

use crate::backend::wasm::lir::{Lir, ValType};

/// Compute a slot remap coalescing non-interfering declared locals.
///
/// `pinned` names absolute slot indices (declared slots, i.e. `>= nparams`) that must each keep a
/// DISTINCT slot and are NEVER coalesced with each other or with a free slot. This is how a caller
/// protects DEBUG-named locals: a `let`-binding / match-binder that a DWARF DIE points at must not
/// share its slot with another variable, or a debugger would read the wrong value within that
/// variable's scope. A pinned slot is renumbered (into the compacted space) but always to a unique
/// slot, and — unlike a free slot — is kept even if it is never mentioned in `code` (so its DWARF
/// slot reference stays valid). Pass an empty set to coalesce everything.
///
/// Returns `(remap, new_declared)`:
/// - `remap[old_slot] = new_slot` for EVERY slot index (`0..nparams+declared.len()`). Parameters
///   map to themselves. A free declared local that is never mentioned maps to itself (its entry is
///   dead and simply unused — callers rewrite only the local ops that actually appear).
/// - `new_declared` is the reduced declared-local value types, in new-slot order (the slots that
///   follow the parameters).
///
/// The caller applies `remap` to every `Local{Get,Set,Tee}` in the body, replaces the function's
/// `declared` with `new_declared`, and remaps its debug slot references (pinned or not) through
/// `remap`.
pub fn coalesce_locals(
    params: &[ValType],
    declared: &[ValType],
    code: &[Lir],
    pinned: &std::collections::HashSet<u32>,
) -> (Vec<u32>, Vec<ValType>) {
    let nparams = params.len() as u32;
    let total = nparams + declared.len() as u32;

    // First and last flat-index mention per slot (None = never mentioned).
    //
    // WARNING: READ-BEFORE-WRITE: if a slot's FIRST mention is a `local.get` (a read, not a
    // `set`/`tee` write), the slot is live from function ENTRY — it reads wasm's implicit
    // zero-initialization, so its true live range starts at index 0, not at that read. We anchor its
    // first-mention at 0 so an earlier-dying slot can never be coalesced into it and clobber that
    // initial value. (A `tee` writes-then-leaves, so a tee-first is a WRITE-first and safe.)
    let mut first: Vec<Option<usize>> = vec![None; total as usize];
    let mut last: Vec<Option<usize>> = vec![None; total as usize];
    for (i, op) in code.iter().enumerate() {
        let (slot, is_read) = match op {
            Lir::LocalGet(s) => (*s, true),
            Lir::LocalSet(s) | Lir::LocalTee(s) => (*s, false),
            _ => continue,
        };
        let s = slot as usize;
        if s >= first.len() {
            continue; // defensive: an out-of-range slot ref is left untouched
        }
        if first[s].is_none() {
            first[s] = Some(if is_read { 0 } else { i });
        }
        last[s] = Some(i);
    }

    // Identity remap; parameters stay put.
    let mut remap: Vec<u32> = (0..total).collect();

    // Linear-scan interval coloring over DECLARED slots, per ValType. Process declared slots in
    // ascending first-mention order; a "color" (a new slot) is reusable for an interval when its
    // current occupant's last mention is strictly before this interval's first mention.
    struct Color {
        end: usize,
        new_slot: u32,
    }
    // Group colors by ValType. `ValType` is `Copy + Eq` (not `Hash`), and there are only a handful of
    // variants, so a small linear-scanned assoc list is simpler than a map.
    let mut colors_by_ty: Vec<(ValType, Vec<Color>)> = Vec::new();
    let mut new_declared: Vec<ValType> = Vec::new();

    let mut order: Vec<u32> = (nparams..total).collect();
    order.sort_by_key(|&s| first[s as usize].unwrap_or(usize::MAX));

    for old in order {
        let ty = declared[(old - nparams) as usize];
        let is_pinned = pinned.contains(&old);
        let (st, en) = match (first[old as usize], last[old as usize]) {
            (Some(a), Some(b)) => (a, b),
            // Unmentioned declared local. A FREE one is dead — drop it (no new slot; `remap[old]`
            // stays identity but is never used). A PINNED one is kept anyway (a DWARF DIE references
            // it), given its own fresh slot so that reference stays valid.
            _ => {
                if is_pinned {
                    let ns = nparams + new_declared.len() as u32;
                    new_declared.push(ty);
                    remap[old as usize] = ns;
                }
                continue;
            }
        };
        // A pinned slot always takes a fresh, NON-reusable slot (never shared with another variable —
        // keeps its DWARF location correct). A free slot reuses an expired free color of its ValType,
        // else takes a fresh one.
        if is_pinned {
            let ns = nparams + new_declared.len() as u32;
            new_declared.push(ty);
            remap[old as usize] = ns;
            continue;
        }
        let colors = match colors_by_ty.iter_mut().position(|(t, _)| *t == ty) {
            Some(i) => &mut colors_by_ty[i].1,
            None => {
                colors_by_ty.push((ty, Vec::new()));
                &mut colors_by_ty.last_mut().unwrap().1
            }
        };
        let mut chosen = None;
        for c in colors.iter_mut() {
            if c.end < st {
                c.end = en; // reuse this expired color for the new interval
                chosen = Some(c.new_slot);
                break;
            }
        }
        let ns = match chosen {
            Some(ns) => ns,
            None => {
                let ns = nparams + new_declared.len() as u32;
                new_declared.push(ty);
                colors.push(Color {
                    end: en,
                    new_slot: ns,
                });
                ns
            }
        };
        remap[old as usize] = ns;
    }

    (remap, new_declared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wasm::lir::{Lir, ValType};
    use std::collections::HashSet;

    /// The common "coalesce everything" case — no pinned (debug-named) slots.
    fn no_pins() -> HashSet<u32> {
        HashSet::new()
    }

    #[test]
    fn disjoint_sequential_slots_coalesce_to_one() {
        // slot0 mentioned at [0,1], slot1 at [2,3] — disjoint → share one new slot.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64]); // collapsed to 1 local
        assert_eq!(remap, vec![0, 0]); // both old slots → new slot 0
    }

    #[test]
    fn overlapping_slots_stay_distinct() {
        // slot0 [0,2], slot1 [1,3] — overlap → must NOT share.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(0),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64, ValType::I64]);
        assert_eq!(remap, vec![0, 1]);
    }

    #[test]
    fn different_types_never_coalesce() {
        // disjoint spans but distinct ValType → separate slots.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I32], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64, ValType::I32]);
        assert_eq!(remap, vec![0, 1]);
    }

    #[test]
    fn params_are_fixed_declared_coalesce_after_them() {
        // 1 param (slot 0), 2 disjoint declared i64 (slots 1,2) → declared collapse to 1 (new slot 1).
        let code = vec![
            Lir::LocalGet(0), // param use
            Lir::LocalSet(1),
            Lir::LocalGet(1),
            Lir::LocalSet(2),
            Lir::LocalGet(2),
        ];
        let (remap, decl) = coalesce_locals(
            &[ValType::I64],
            &[ValType::I64, ValType::I64],
            &code,
            &no_pins(),
        );
        assert_eq!(decl, vec![ValType::I64]); // 2 declared → 1
        assert_eq!(remap, vec![0, 1, 1]); // param fixed at 0; both declared → new slot 1
    }

    #[test]
    fn tee_counts_as_a_mention() {
        // LocalTee is a def+use — it must extend the slot's span so an overlap is seen.
        // slot0 tee'd at 1 (live [0,1]); slot1 [2,3]. Disjoint → coalesce.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalTee(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64]);
        assert_eq!(remap, vec![0, 0]);
    }

    #[test]
    fn many_disjoint_temps_collapse_to_one_the_blowup_shape() {
        // The glb1 shape: N sequential single-use i64 temps, each [2k, 2k+1], all disjoint → 1 slot.
        let n = 50u32;
        let mut code = Vec::new();
        for k in 0..n {
            code.push(Lir::LocalSet(k));
            code.push(Lir::LocalGet(k));
        }
        let declared = vec![ValType::I64; n as usize];
        let (remap, decl) = coalesce_locals(&[], &declared, &code, &no_pins());
        assert_eq!(decl.len(), 1); // 50 locals → 1
        assert!(remap.iter().all(|&r| r == 0));
    }

    #[test]
    fn unmentioned_declared_local_is_dropped() {
        // slot1 (i64) is declared but never referenced → dropped; slot0 kept.
        let code = vec![Lir::LocalSet(0), Lir::LocalGet(0)];
        let (_remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64]); // the dead local is gone
    }

    #[test]
    fn pinned_slots_never_coalesce_even_when_disjoint() {
        // slot0 [0,1], slot1 [2,3] — disjoint, so they WOULD coalesce (see the first test) — but both
        // are pinned (debug-named), so each keeps a DISTINCT slot. slot2 [4,5] is free and disjoint from
        // both, but it must NOT reuse a pinned slot's number → it takes its own fresh slot too.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
            Lir::LocalSet(2),
            Lir::LocalGet(2),
        ];
        let pinned: HashSet<u32> = [0u32, 1].into_iter().collect();
        let (remap, decl) = coalesce_locals(
            &[],
            &[ValType::I64, ValType::I64, ValType::I64],
            &code,
            &pinned,
        );
        assert_eq!(decl, vec![ValType::I64, ValType::I64, ValType::I64]); // all 3 distinct
        assert_eq!(remap, vec![0, 1, 2]); // no sharing: identity here
    }

    #[test]
    fn free_slots_still_coalesce_around_a_pinned_one() {
        // slot0 pinned [0,1]; slot1 free [2,3]; slot2 free [4,5]. The two FREE slots are disjoint → they
        // coalesce with EACH OTHER (to one slot), while the pinned slot keeps its own. 3 declared → 2.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
            Lir::LocalSet(2),
            Lir::LocalGet(2),
        ];
        let pinned: HashSet<u32> = [0u32].into_iter().collect();
        let (remap, decl) = coalesce_locals(
            &[],
            &[ValType::I64, ValType::I64, ValType::I64],
            &code,
            &pinned,
        );
        assert_eq!(decl.len(), 2); // pinned slot0 + one shared free slot
        assert_eq!(remap[0], 0); // pinned → its own slot
        assert_eq!(remap[1], remap[2]); // the two free slots share
        assert_ne!(remap[1], remap[0]); // but not the pinned one
    }

    #[test]
    fn read_before_write_slot_is_live_from_entry_and_not_clobbered() {
        // slot0: set@0 (dead after, textual span [0,0]). slot1: its FIRST mention is a GET@1 (reads the
        // zero-init) then set@2, get@3. A naive [first,last] span for slot1 would be [1,3], disjoint from
        // slot0's [0,0] → they'd wrongly coalesce, and `set slot0`'s value would clobber the 0 slot1
        // reads at index 1. The read-before-write anchor makes slot1 live from entry ([0,3]) → overlap →
        // NOT coalesced.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(1), // read-before-write: slot1 reads its zero-init here
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &no_pins());
        assert_eq!(decl, vec![ValType::I64, ValType::I64]); // stay distinct
        assert_ne!(remap[0], remap[1]);
    }

    #[test]
    fn unmentioned_pinned_local_is_kept() {
        // slot1 (i64) is declared, never referenced in code, but PINNED (a DWARF DIE points at it) →
        // it must be KEPT (given a slot), unlike the free unmentioned case which is dropped.
        let code = vec![Lir::LocalSet(0), Lir::LocalGet(0)];
        let pinned: HashSet<u32> = [1u32].into_iter().collect();
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code, &pinned);
        assert_eq!(decl, vec![ValType::I64, ValType::I64]); // both kept (the pinned dead one survives)
        assert_eq!(remap[1], 1); // pinned slot gets a valid distinct slot
    }
}
