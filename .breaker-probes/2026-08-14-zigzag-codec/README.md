# zgz1 — zigzag codec accumulator (2026-08-14, tick 1473)

`enc` zigzag-maps signed to unsigned (2v non-negative, -2v-1 negative) and
folds the code into a running state sum; `dec` un-zigzags the accumulated sum,
its PARITY deciding the sign of the answer. Uses % and / on the state plus a
scalar dual-use let (z feeds both slots) in a mixed-op region — scalar-let
mixed-op control staying green.

Seeds land the decodes on OPPOSITE signs: n=10 → enc(7),enc(4),dec=+11,
enc(-11),dec=-22 → 1408112078 (negative last digit-pair rides the packing);
n=0 → enc(-3),enc(4),dec=-7(odd sum),enc(-1),dec=+7 → 507930107 — the decode
sign flips BETWEEN the seeds at both readout points.

PASS ×3 wasm. **Pool (batch-274 → full).**
