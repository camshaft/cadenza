# cel1 — rule-90 cellular automaton in one byte (2026-08-15, tick 1529)

SCALAR world (8 bits of an Int64): `step` computes XOR of the left- and
right-shifted worlds masked to 8 bits — the classic rule-90 Sierpinski
generator — with the whole compound recomputed in both slots (branch-free);
`density` popcounts between steps via the let-free recursive bits callee.

Seeds 14 vs 4 diverge into different orbits (27,·,59,107,·,227 vs
10,·,17,42,·,65) with different densities (4/5 vs 2/3). Rule 90 on a
power-of-two seed (4) generates the Sierpinski doubling pattern (10=8+2,
17, 42...) — a self-checking structure.

Branch-free scalar, 6 dispatches — envelope-safe. PASS ×3. **Pool (with
phs5; +1 fills the 8th trio... pool now runs 5 trios + phs5/cel1).**
