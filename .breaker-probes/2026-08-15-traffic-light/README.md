# lgt1 — traffic light with a demand sensor (2026-08-15, tick 1578)

(color, remaining) state, 3-phase cycle (green 3, yellow 1, red seed-shaped):
`tick` counts down and cycles on zero; `demand` during a LONG red shortens
the remainder to 1 answering 100+old (the transit-priority rebate) — during
green/yellow/short-red it just reads. The tick arm's cycle logic is a
3-branch color dispatch nested under the countdown check; the demand arm is
a 3-level read-mostly lattice — both single-field guards (envelope-safe per
the dth1 precedent).

Seed red length (3 vs 2): the red-entry row differs (23 vs 22), the demand
rebate differs (103 vs 102), and the post-demand tails CONVERGE exactly
(both greens resume in lockstep) — a two-row divergence bracketed by
identical prefix and suffix.

PASS ×3. **Pool (11th trio seed).**
