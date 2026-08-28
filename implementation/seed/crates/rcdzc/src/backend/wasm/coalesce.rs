//! Local-slot COALESCING — shrink a function's declared-local count by reusing non-interfering
//! slots, so `local.get`/`local.set`/`local.tee` indices get smaller LEB encodings. This mitigates
//! the local-slot BLOAT the effects lowering produces: a tail-resumptive handler distributes its
//! continuation into each branch, so a small source expands into thousands of sequential single-use
//! temps (e.g. the corpus `glb1` case emits 18,738 locals of which only ~7 are ever simultaneously
//! live — wasm-opt coalesces them, and this pass does the same at emit).
//!
//! SOUNDNESS (why this never miscompiles, across ALL wasm control flow, without a CFG):
//! interference is defined as OVERLAPPING `[first-mention, last-mention]` flat-instruction-index
//! spans. A slot's true live range is bounded by its def/use occurrences — every point where the
//! slot is live lies between its first and last mention (loop-carried uses included: the use is
//! itself a mention, so it extends the span). Hence if two slots are ever simultaneously live at a
//! point `P`, then `P` lies in both spans and the spans overlap → we mark them interfering and do
//! NOT share a slot. So overlapping-span is a SUPERSET of real interference: we never coalesce two
//! slots that could be live at once (sound); we may only conservatively keep some coalescable slots
//! apart (a missed opportunity, never a miscompile). No explicit control-flow graph is needed.
//!
//! PARAMETERS are fixed (slots `0..nparams` — the function's signature); only DECLARED locals
//! (`nparams..`) are coalesced, and only within the SAME `ValType` (an i32 and an i64 can never
//! share a slot). An unmentioned declared local is dead and is dropped entirely.

use crate::backend::wasm::lir::{Lir, ValType};

/// Compute a slot remap coalescing non-interfering declared locals.
///
/// Returns `(remap, new_declared)`:
/// - `remap[old_slot] = new_slot` for EVERY slot index (`0..nparams+declared.len()`). Parameters
///   map to themselves. A declared local that is never mentioned maps to itself (its entry is dead
///   and simply unused — callers rewrite only the local ops that actually appear).
/// - `new_declared` is the reduced declared-local value types, in new-slot order (the slots that
///   follow the parameters).
///
/// The caller applies `remap` to every `Local{Get,Set,Tee}` in the body and replaces the function's
/// `declared` with `new_declared` (and remaps any debug slot references).
pub fn coalesce_locals(
    params: &[ValType],
    declared: &[ValType],
    code: &[Lir],
) -> (Vec<u32>, Vec<ValType>) {
    let nparams = params.len() as u32;
    let total = nparams + declared.len() as u32;

    // First and last flat-index mention per slot (None = never mentioned).
    let mut first: Vec<Option<usize>> = vec![None; total as usize];
    let mut last: Vec<Option<usize>> = vec![None; total as usize];
    for (i, op) in code.iter().enumerate() {
        let slot = match op {
            Lir::LocalGet(s) | Lir::LocalSet(s) | Lir::LocalTee(s) => *s,
            _ => continue,
        };
        let s = slot as usize;
        if s >= first.len() {
            continue; // defensive: an out-of-range slot ref is left untouched
        }
        if first[s].is_none() {
            first[s] = Some(i);
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
        let (st, en) = match (first[old as usize], last[old as usize]) {
            (Some(a), Some(b)) => (a, b),
            // Unmentioned declared local: dead — drop it (no new slot). `remap[old]` stays identity
            // but is never used (nothing in `code` references it).
            _ => continue,
        };
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

    #[test]
    fn disjoint_sequential_slots_coalesce_to_one() {
        // slot0 mentioned at [0,1], slot1 at [2,3] — disjoint → share one new slot.
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code);
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
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code);
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
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I32], &code);
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
        let (remap, decl) = coalesce_locals(&[ValType::I64], &[ValType::I64, ValType::I64], &code);
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
        let (remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code);
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
        let (remap, decl) = coalesce_locals(&[], &declared, &code);
        assert_eq!(decl.len(), 1); // 50 locals → 1
        assert!(remap.iter().all(|&r| r == 0));
    }

    #[test]
    fn unmentioned_declared_local_is_dropped() {
        // slot1 (i64) is declared but never referenced → dropped; slot0 kept.
        let code = vec![Lir::LocalSet(0), Lir::LocalGet(0)];
        let (_remap, decl) = coalesce_locals(&[], &[ValType::I64, ValType::I64], &code);
        assert_eq!(decl, vec![ValType::I64]); // the dead local is gone
    }
}
