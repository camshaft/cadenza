# spr1 — sprinkler scheduler on a depleting tank (2026-08-15, tick 1575)

(tank, shortfall) state with a let-free indexed-duration callee (5/8/3):
`water z` runs the zone for min(duration, tank) — the starved branch drains
the tank to 0 and accrues the shortfall; `refill` restores the seed capacity
answering the shortfall so far (a deferred-error readout).

Small tank (10): starves twice in pass one (got 5/5/0, short 6), once in
pass two (8/2, short 12); large tank (20) NEVER starves — the shortfall
readouts are 0/0 vs 6/12 and the starved rows show partial service (5→5→0
truncation cascade). The dur callee is called in both branches of the
comparison (guard + both resume slots — a call-recompute face, in-envelope).

PASS ×3. **Pool (with wrd1; +1 fills the 11th trio).**
