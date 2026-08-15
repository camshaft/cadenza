# ldg1 — leading-digit counter (2026-08-15, tick 1511)

SCALAR hits state: `feed` strips each value to its leading digit through the
recursive divide-down callee `lead` (let-free), a HIT on the seed-wanted digit
((n%4)+1: 3 vs 1) answers digit*10 + the running count while a miss answers
the bare digit; `tally` reads the count. The two seeds hunt DIFFERENT digits
through one shared stream: hit rows migrate (31,·,·,32,33,·,3 vs ·,11,·,·,·,·,1)
— every row shape differs.

Note: `lead` recomputed twice per hit branch (predicate + answer) — cheap
compound, scalar state, 7 dispatches: inside the F24 envelope by the cost
model (width 1 x small recompute).

PASS ×3 wasm. **Pool (4th trio seed).**
