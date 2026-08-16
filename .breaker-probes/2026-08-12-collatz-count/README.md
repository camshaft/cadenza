# Collatz-driven dispatch counting (2026-08-12)

Angle: DATA-DEPENDENT dispatch counts — the Collatz walk observes every step
through the handler, so the counter state tallies a trajectory length that
varies wildly per seed (8 steps for 6, 16 for 7, 0 for 1). The fold must
handle a per-seed-variable number of dispatches with a budget guard (k=30
bounds the walk — non-termination-safe per the m3 lesson).

GREEN x3:
- cz1: 1008/1016/1000 (value*1000 + count)

Staged for the next 14c batch (with pbr1/pbr2, sqm1).
