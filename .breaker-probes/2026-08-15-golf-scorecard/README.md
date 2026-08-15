# gsc1 — golf scorecard with birdie-streak multiplier (2026-08-15, tick 1551)

(total, streak) state, 2-arg hole op: an under-par hole answers
(strokes−par)·(streak+1) — the deepening-streak multiplier — while over-par
answers the plain delta and RESETS the streak. Note the edge: a PAR hole
(delta 0) takes the else branch, so par also resets the streak (hole 1 pins
this: answer 0, streak stays 0). card totals.

Seed shifts hole 3's strokes (4 vs 2... (n%3)+2: 3 vs 2): both are under-par
at streak 2 but the deltas differ (−1·2=−2 vs −2·2=−4); the over-par hole 4
resets both; the final birdie restarts at ×1. Negative packed totals
(−101990103 / −103990105) with the streak-bonus rows as the only divergent
digits.

2-branch arm, 2-tuple, 6 dispatches — envelope-safe. PASS ×3. **Pool.**
