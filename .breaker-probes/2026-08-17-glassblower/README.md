# glb1 — glassblower's bench (2026-08-17, tick 1711)

Attack: a FLOOR-MIN expanded into 4 leaves where the floored value (the
thinning = max(1, gather/3)) is a DIVISION on one side and a CONSTANT 1 on
the other, each side re-testing the crack line with its own thinning
(`(- wall (/ gather 3))` vs `(- wall 1)` — the mow1 crossed-clamp family with
the clamp on a DERIVED value rather than the argument). The crack leaves
freeze BOTH fields; the live leaves halve gather (a second division) while
subtracting the first — two divisions of one field in one rebuild.

Differential: gather 9 vs 3: n=10 thins by 3 (36), reheats, thins 2 then 1
(24, 13 — wall to 3, read 320); n=0 thins by 1s (18, 54, 17, 16 — wall 6,
read 610). No cracks on either (the crack leaves stay dark — this probe pins
the LIVE paths; the crack line is covered by rnk1/frg1's threshold family).

Hand model: n=10 → 360840240130320; n=0 → 180540170160610 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 141665bdd.
