# rrb1 — round-robin scheduler with skip mask (2026-08-14, tick 1490)

(cursor, bitmask) state: `turn` scans cyclically (recursive scan def, mod-4
wraparound, budget-guarded at 4 steps) to the next unskipped worker answering
its id, -1 when all four are skipped; `skip` ORs a bit answering the popcount.
9 straight-line dispatches ending in the all-skipped -1 face.

Seed sets the STARTING cursor (n%4: 2 vs 0) so the same skip sequence yields
different service orders (3,0,1,3 vs 1,3,0,3) while the popcount rows (1,2,3,4)
and the drained -1 are seed-invariant anchors. 17-digit packed totals.

PASS ×3 wasm. **Pool.**
