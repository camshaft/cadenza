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

    // Assign new slots in this order: (0) DEBUG-NAMED + mentioned slots FIRST, so a `let`-binding /
    // match-binder that a DWARF DIE points at keeps a STABLE, LOW slot (right above the params) — its
    // debug location stays predictable and doesn't get bumped above a transient scratch temp; then
    // (1) free scratch (coalesced among themselves, numbered above the pinned block); then (2) any
    // debug-named-but-unmentioned slot last (rare). Within each tier, ascending first-mention (the
    // linear-scan coloring needs free slots in first-mention order). Pinned assignment is independent
    // of the free coalescing, so this changes only slot NUMBERING, never the coalesced COUNT.
    let mut order: Vec<u32> = (nparams..total).collect();
    order.sort_by_key(|&s| {
        let mentioned = first[s as usize].is_some();
        let tier = match (pinned.contains(&s), mentioned) {
            (true, true) => 0u8, // debug-named + used → lowest, stable slots
            (false, _) => 1,     // free scratch
            (true, false) => 2,  // debug-named but unused → last
        };
        (tier, first[s as usize].unwrap_or(usize::MAX))
    });

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

/// A control-construct kind for the backward-liveness frame stack.
#[derive(Clone, Copy, PartialEq)]
enum CtrlKind {
    Loop,
    Block,
    If,
}

/// One OPEN control construct during the backward-liveness reverse walk.
struct Frame {
    /// The opener's instruction index (`Loop`/`Block`/`If`).
    open: usize,
    kind: CtrlKind,
    /// The live set immediately AFTER this construct's `End` — the branch target for a `Br` to a
    /// `block`/`if` label (jump-to-end), and the reset point at `else` (both arms rejoin here).
    join_live: std::collections::HashSet<u32>,
    /// The `else`-branch's live-IN, stashed at the `Else` marker and unioned into `live` at the `If`
    /// opener (so `live-in(if) = live-in(then) ∪ live-in(else)`).
    else_in: Option<std::collections::HashSet<u32>>,
}

/// The live set at the LABEL a `br <depth>` targets: the `(depth+1)`-th enclosing open construct
/// (innermost = the last frame). A `loop` label is its TOP (its live-IN, from `loop_live_in`); a
/// `block`/`if` label is its END (`join_live`). A depth past the function's blocks is a return — no
/// locals live.
fn branch_target_live(
    frames: &[Frame],
    depth: u32,
    loop_live_in: &std::collections::HashMap<usize, std::collections::HashSet<u32>>,
) -> std::collections::HashSet<u32> {
    let Some(k) = frames.len().checked_sub(1 + depth as usize) else {
        return std::collections::HashSet::new();
    };
    match frames.get(k) {
        Some(f) if f.kind == CtrlKind::Loop => {
            loop_live_in.get(&f.open).cloned().unwrap_or_default()
        }
        Some(f) => f.join_live.clone(),
        None => std::collections::HashSet::new(),
    }
}

/// Per-instruction LIVE-OUT: `live_out[i]` = the local slots live immediately AFTER `code[i]`.
///
/// Backward liveness over wasm's STRUCTURED control flow, iterated to a fixpoint (loop back-edges
/// converge, monotone growth). A slot is live at a point if a forward path reads it (`local.get`)
/// before the next def (`local.set`/`local.tee` — a `tee` DEFS its slot from the stack, it does NOT
/// read the slot). This gives a PRECISE interference relation: a re-defined slot that is dead in a
/// "hole" is correctly NOT live there, so it does not interfere with a slot live only in that hole —
/// the imprecision a flat `[first-mention, last-mention]` span has.
///
/// SOUNDNESS: liveness here must OVER-approximate. Claiming a slot LIVE when it is dead only adds
/// spurious interference (fewer coalesces — safe); claiming it DEAD when live would let two live slots
/// share one slot = a MISCOMPILE. So no case ever spuriously CLEARS `live` (e.g. `unreachable` is a
/// no-op that keeps `live`); an unconditional `br` sets `live` to the target label's set because the
/// fall-through after it genuinely never executes.
fn compute_live_out(code: &[Lir]) -> Vec<std::collections::HashSet<u32>> {
    use std::collections::{HashMap, HashSet};
    let n = code.len();
    // Pair each control construct: `End` index → (opener index, kind), via a forward depth stack.
    let mut end_to_open: HashMap<usize, (usize, CtrlKind)> = HashMap::new();
    {
        let mut stack: Vec<(usize, CtrlKind)> = Vec::new();
        for (i, op) in code.iter().enumerate() {
            match op {
                Lir::Loop(_) => stack.push((i, CtrlKind::Loop)),
                Lir::Block(_) => stack.push((i, CtrlKind::Block)),
                Lir::If(_) => stack.push((i, CtrlKind::If)),
                Lir::End => {
                    if let Some(o) = stack.pop() {
                        end_to_open.insert(i, o);
                    }
                }
                _ => {}
            }
        }
    }
    let mut live_out: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    let mut loop_live_in: HashMap<usize, HashSet<u32>> = HashMap::new();
    // Fixpoint: repeat the reverse pass until `loop_live_in` stops growing (bounded by `n + 2` passes;
    // the `break` on `!changed` exits as soon as it converges — usually 2 passes).
    for _ in 0..n + 2 {
        let mut changed = false;
        let mut frames: Vec<Frame> = Vec::new();
        let mut live: HashSet<u32> = HashSet::new();
        for i in (0..n).rev() {
            live_out[i] = live.clone();
            match &code[i] {
                Lir::End => {
                    let (open, kind) = end_to_open.get(&i).copied().unwrap_or((i, CtrlKind::Block));
                    frames.push(Frame {
                        open,
                        kind,
                        join_live: live.clone(),
                        else_in: None,
                    });
                }
                Lir::Loop(_) => {
                    // `live` now = the loop's live-IN (its top) — the `br`-to-top target. Record it;
                    // a change means the fixpoint has not converged.
                    let start = frames.pop().map(|f| f.open).unwrap_or(i);
                    match loop_live_in.get(&start) {
                        Some(p) if p == &live => {}
                        _ => {
                            loop_live_in.insert(start, live.clone());
                            changed = true;
                        }
                    }
                }
                Lir::Block(_) => {
                    frames.pop(); // a block always executes → live-in(block) = live-in(body)
                }
                Lir::If(_) => {
                    if let Some(f) = frames.pop() {
                        match f.else_in {
                            // with an `else`, both arms are covered → union the else-branch live-in
                            Some(else_in) => live.extend(else_in),
                            // no `else` → the cond-false path skips to the end → its live is `join_live`
                            None => live.extend(f.join_live.iter().copied()),
                        }
                    }
                }
                Lir::Else => {
                    // Reverse: we have processed the else-body (`live` = its live-in). Stash it, then
                    // reset `live` to the join (live-after-End) for the then-body.
                    if let Some(f) = frames.last_mut() {
                        f.else_in = Some(live.clone());
                        live = f.join_live.clone();
                    }
                }
                Lir::Br(d) => {
                    // Unconditional: the fall-through never runs → live-in = target label only.
                    live = branch_target_live(&frames, *d, &loop_live_in);
                }
                Lir::BrIf(d) => {
                    // Conditional: live-in = fall-through ∪ target label.
                    live.extend(branch_target_live(&frames, *d, &loop_live_in));
                }
                Lir::BrTable(ts, def) => {
                    // Always branches → live-in = ∪ of every target's + the default's label live.
                    let mut u = branch_target_live(&frames, *def, &loop_live_in);
                    for d in ts {
                        u.extend(branch_target_live(&frames, *d, &loop_live_in));
                    }
                    live = u;
                }
                Lir::LocalGet(s) => {
                    live.insert(*s);
                }
                Lir::LocalSet(s) | Lir::LocalTee(s) => {
                    live.remove(s);
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    live_out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wasm::lir::{BlockType, Lir, ValType};
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

    fn set_of(xs: &[u32]) -> HashSet<u32> {
        xs.iter().copied().collect()
    }

    #[test]
    fn live_out_straight_line() {
        let code = vec![
            Lir::LocalSet(0),
            Lir::LocalGet(0),
            Lir::LocalSet(1),
            Lir::LocalGet(1),
        ];
        let lo = compute_live_out(&code);
        assert_eq!(lo[0], set_of(&[0])); // 0 live (used at 1)
        assert_eq!(lo[1], set_of(&[])); // 0 dead after last use; 1 not yet live
        assert_eq!(lo[2], set_of(&[1])); // 1 live (used at 3)
        assert_eq!(lo[3], set_of(&[])); // end
    }

    #[test]
    fn live_out_hole_reused_slot_not_colive_with_hole_slot() {
        // slot0 def@0 use@1 (dead), def@4 use@5 — a HOLE [1,4] where slot0 is DEAD; slot1 def@2 use@3
        // lives ONLY in that hole. Precise liveness: at NEITHER of slot0's defs is slot1 live, and at
        // slot1's def slot0 is not live → never simultaneously live (a flat [0,5] span would wrongly
        // overlap [2,3]). This imprecision is exactly what the precise pass fixes.
        let code = vec![
            Lir::LocalSet(0), // 0
            Lir::LocalGet(0), // 1
            Lir::LocalSet(1), // 2
            Lir::LocalGet(1), // 3
            Lir::LocalSet(0), // 4
            Lir::LocalGet(0), // 5
        ];
        let lo = compute_live_out(&code);
        assert!(
            !lo[0].contains(&1),
            "slot1 not live after slot0's first def"
        );
        assert!(
            !lo[4].contains(&1),
            "slot1 not live after slot0's second def"
        );
        assert!(
            !lo[2].contains(&0),
            "slot0 not live after slot1's def (slot0's hole)"
        );
    }

    #[test]
    fn live_out_loop_carried_slot_live_across_back_edge() {
        // loop { get0 ; set0 ; br 0 } — slot0 READ at the top before its def ⇒ loop-carried, live
        // across the back-edge. After the fixpoint it is live after its def (re-read next iteration).
        let code = vec![
            Lir::Loop(BlockType::Empty), // 0
            Lir::LocalGet(0),            // 1
            Lir::LocalSet(0),            // 2
            Lir::Br(0),                  // 3
            Lir::End,                    // 4
        ];
        let lo = compute_live_out(&code);
        assert!(
            lo[2].contains(&0),
            "loop-carried slot0 live after its def (read next iteration via the back-edge)"
        );
    }

    #[test]
    fn live_out_within_iteration_loop_slot_dead_after_last_use() {
        // loop { set0 ; get0 ; br 0 } — slot0 def-before-use each iteration ⇒ within-iteration, NOT
        // live across the back-edge.
        let code = vec![
            Lir::Loop(BlockType::Empty), // 0
            Lir::LocalSet(0),            // 1
            Lir::LocalGet(0),            // 2
            Lir::Br(0),                  // 3
            Lir::End,                    // 4
        ];
        let lo = compute_live_out(&code);
        assert!(
            !lo[2].contains(&0),
            "within-iteration slot0 dead after its last use"
        );
    }
}
