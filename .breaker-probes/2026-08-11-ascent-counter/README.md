# Ascent counter (adjacent-draw comparison state) (2026-08-11)

Angle: the (prev, hits) pair state where each dispatch compares the NEW
payload against the PREVIOUS one — adjacent-comparison composition. Existing
sw (sliding-window) pins carry windows; the pairwise-comparison face with a
conditionally-bumped counter inside the tuple rebuild was uncovered.

GREEN x3:
- ac1: 4 feeds, ascent verdict per dispatch, high-seed control kills the
  first ascent — 2211/1100
- ac2: fed by a SECOND effect's SQUARED draws through a recursive driver —
  the n=-2 seed dips at the parabola turn (4,1,0,1: ascents at 0->4 and
  0->1 only) — 10/5

Pin candidates: 249 pool.
