# drt1 — dartboard countdown leg (2026-08-17, tick 1689)

Attack: a 3-way ORDERED comparison cascade over the same pair (v > rem /
v == rem / v < rem implied) where each leaf treats the SHARED counter (darts)
identically but the other fields differently — bust touches busts, checkout
ZEROES rem with a constant-777 answer (no field data in the answer at all),
undershoot subtracts. The constant-answer checkout is the interesting leaf:
its answer carries zero state information while its rebuild is the most
destructive (field zeroed).

Differential: start 35 vs 20: n=10 checks out MID-run (dart 3: 8 == 8 → 777)
so dart 4 BUSTS against 0; n=0 busts twice mid-run (12 > 5, 8 > 5) then
checks out on the LAST dart (5 == 5). Checkout and busts swap positions;
reads 41 vs 42 (dart counts equal, busts differ).

Hand model: n=10 → 2010827779010041; n=0 → 519019027770042 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk cde130bab.
