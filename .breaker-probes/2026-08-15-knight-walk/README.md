# knt1 — knight walker on an 8x8 board (2026-08-15, tick 1520)

(rank, file, hop-count) 3-tuple: `hop(dr,dc)` attempts the L-move through a
5-branch bounds lattice, answering the landing square's index (r*8+c) or -1
REFUSED with the position held; `cnt` reads completed hops. The seed's
starting rank (n%8: 2 vs 0) makes the SAME move list bounce once on one run
and twice on the other — refusals at different moves, n=0 packs negative
(leading -1).

Envelope note: 5-branch arm × 4 dispatches through it × 3-tuple PASSES —
the branches here recompute only (+ r dr)/(+ c dc) (cheap 2-term sums), one
more datapoint that per-branch recompute SIZE (not just count) is what
multiplies (elv1's 6-branch × 4 with heavier recomputes exploded).

PASS ×3 wasm. **Pool (5th trio seed).**
