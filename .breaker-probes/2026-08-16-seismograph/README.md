# ssm ladder — seismograph with drift (2026-08-16, tick 1660)

Attack: the deviation compound `(- mag base)` x5 across the arm (test, event
answer, inline-max test+value, quiet answer, sgn helper arg) + an inline MAX
`(if (> (- mag base) peak) (- mag base) peak)` + the sgn helper (nested-if
-1/0/1, rfr/flk family) — compound + max + helper stacked in one 2-branch arm.

## Envelope
- ssm1 (4 readings): scratch-locals clean decline — the x5 compound + inline
  max + helper stack exceeds brw1's x5-compound-only budget (which passed at
  4). Refines the law: ADDITIVE pressure across compound-kinds (repetition +
  max-test + helper-call) fences lower than any alone.
- ssm2 (3 readings): PASSES x3 all backends. Differential: seeds disagree on
  whether reading #3 (16) is an aftershock EVENT (n=0: dev 6 > 4 after drift
  to 10... rows [771,80,762] read 812, TWO events) or quiet (n=10: dev 4 not
  > 4, rows [751,60,90] read 631, ONE event) — the event COUNTS split.

Hand model: n=10 → 751060090631; n=0 → 771080762812 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 926293d21. ssm1 held for (b).
