# tow1 — tug-of-war with a latching win line (2026-08-15, tick 1561)

(position, won) state, 2-arg pull op: each pull moves the marker toward its
side answering the live position until crossing ±10 LATCHES the result —
every later pull answers the frozen ±100; `where` reads the final marker
(which keeps its overshoot value under the latch: -11 both runs).

Seed offset (0 vs −2): the SAME pull sequence crosses the line at pull 5 on
one run and pull 3 on the other — the frozen tail overlaps the other run's
still-live rows (…−9,−6,−100 vs …−100,−100,−100), and both `where` reads
agree at −11 (same total displacement, different latch timing).

4-level nested-if but single-field guards + cheap branches at 5 dispatches
through the branching arm — envelope-safe (dth1 precedent). PASS ×3.
**Pool — fills sfl1/orq1/tow1 (twelfth trio... recount: 10th full trio).**
