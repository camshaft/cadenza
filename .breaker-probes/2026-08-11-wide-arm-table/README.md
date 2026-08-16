# Wide arm tables (2026-08-11)

Angle: op-COUNT scaling of one handler's dispatch table. The landed handlers
top out around 3-4 ops; w7 pins SEVEN ops with distinct answer shapes
(+1 / *2 / -3 / square / mod / div / negate) and distinct strides, called in
scrambled order — a mis-wired dispatch index or shared arm slot perturbs the
positional sum.

GREEN x3:
- w7: 160349103/142711381 (n=2/0), hand-modeled first.

Pin candidate: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-next-in-pipeline)
w7 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; value 160349103/142711381 python-modeled + traced; not already pinned; 7-op widest dispatch table, scrambled order). HELD behind queued MR sd2 e480379f8. PIN once sd2 lands (re-verify vs fresh trunk).

## SENT by v-effects (2026-08-11)
w7 pinned to 14b (MR 8f39d9b3d, +3 baseline lines). CLAIMED-HELD -> SENT.
