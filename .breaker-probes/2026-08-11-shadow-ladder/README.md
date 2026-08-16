# Same-effect shadow ladders (2026-08-11)

Angle: is2/is3 pin arm-instantiated shadows; a BODY-position depth-3 ladder
where each shadow's SEED draws from the ENCLOSING handler (cross-seeding
through the ladder), and outer-state isolation after a shadow closes, were
uncovered.

GREEN x3:
- sl1: depth-3 ladder, seeds n -> (*10 draw) -> (+5 draw), strides 1/2/3
  stay separate — 3835/805
- sl2: the OUTER stride continues (n, n+1) across a closed inner region —
  the shadow never touched the outer slot — 210034/210001

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline)
sl1 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; 3835/805 traced). DISTINCT from landed same-effect-shadow pins: dn1 (14c:398) uses LITERAL seeds; sl1 CROSS-SEEDS each shadow from the enclosing handler draw (mid seed=(*10 outer-draw), inner seed=(+5 mid-draw)). HELD behind tk1 (behind queued w7 8f39d9b3d). sl2 (outer-isolation) a further candidate.

## SENT by v-effects (2026-08-11)
sl1 pinned to 14b (MR 990a7208a, +3 baseline lines). CLAIMED-HELD -> SENT. sl2 (outer-isolation) a further candidate.

## COVERED (tick 1293): sl1 landed via v-effects (4d5888d1d, 14b)
v-effects pinned the depth-3 cross-seeded ladder verbatim (my sl1 shape from
the tick-1180 note exchange). sl1 DROPPED from staging. sl2 (outer-state
isolation after a closed shadow) NOT covered by their pin — stays staged.
