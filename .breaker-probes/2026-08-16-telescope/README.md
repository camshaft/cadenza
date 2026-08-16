# tel1 — telescope tracker with slew-rate limit (2026-08-16, tick 1658)

Attack: a TWO-SIDED clamp as a 3-branch nested-if over a SIGNED difference
(`(- tg az)` > 7 / < -7 / arrive) where the arrive branch snaps az to tg AND
tags — the signed-clamp complements brw1/chr1's one-sided min clamps. The
track arm calls the `dist` helper twice (test + answer) with the answer
re-deriving `(% (dist az t2) 100)`. Helper + signed-clamp + arrival-snap in
one probe.

Differential: starting azimuth 30 vs 10 against target 27: n=10 is WITHIN
lock range at track#1 (103) and arrives in one slew (271); n=0 misses (917),
rate-limits (170 = +7), misses again (913), still mid-slew at the read
(2400 vs 3011 — arrival flag differs). Every row disagrees.

Hand model: n=10 → 1030271010303013011; n=0 → 9170170091302402400
(base-10000).

Pass ×3 wasm + rust + rust-async on trunk bc7437703.
