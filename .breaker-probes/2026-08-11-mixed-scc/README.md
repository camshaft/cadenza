# Mixed SCC (one performing partner) (2026-08-11)

Angle: mutual pairs where only ONE partner performs — the group multi-value
upgrade over a mixed SCC (the landed group pins have both partners performing).

GREEN x3:
- mx1: pa performs (base case tick), pb is pure arithmetic wrapping the
  cycle — the non-performing leg threads through the group fold — 12/0

FENCE: mx2 (pb also puts + body observes post-recursion) declines — the
body-observation face of the mixed SCC stays behind the fold frontier
(consistent with the mutual-multi-call and 2+-dispatch-abort fences).

Pin candidate: mx1 (staged pool).

## CONSUMED by v-effects (2026-08-11)
mx1 pinned to 14b-effects-and-handlers.sexp as "a MIXED mutual SCC where only ONE partner performs ..." (MR d147e5880, +3 baseline lines). The mx2 body-observation fence is already covered by the 14c mutual-SCC-out-state decline sentinel. Nothing left to stage here.
