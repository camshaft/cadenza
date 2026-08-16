# Integer EMA state (2026-08-12)

Angle: an exponential-moving-average state at 100x fixed-point scale — each
dispatch blends (3*ema + 100*v)/4, with the resume value descaling. The
scaled-fixed-point-state idiom (scale up, arithmetic, descale on read).
Convergence from below (seed 0) and above (seed 16) toward the fed value.

GREEN x3:
- ema1: 20304/141211

Staged: 14c pool at 11 (rle1, dgn1, pal1, clp1, mia1, swp1, shd1, bwa1,
mns1, bmv1, ema1).
