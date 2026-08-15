# cfr1 — continued-fraction expander (2026-08-15, tick 1513)

(p, q) state: each `step` peels the integer part a = p/q answering it, then
inverts the remainder — (p,q) ← (q, p−a·q), the Euclid step reframed as CF
coefficients; a drained fraction (q=0) answers -1 forever. The quotient p/q
is recomputed in both slots (truncating dual-use-by-recompute).

Seeds share the denominator 37: 110/37 = [2;1,36] terminates after 3 steps
(tail -1,-1) while 100/37 = [2;1,2,2,...] is still peeling at step 5 —
DIFFERENT termination structure, not just different values. Complements
gcd1 (divisor-chain sums) and gc1 (single Euclid steps) with the
coefficient-stream face.

Single-op, 2-branch, 2-tuple, 5 dispatches — envelope-safe. PASS ×3. **Pool
(fills ldg1/cfr1 toward the 4th trio).**
