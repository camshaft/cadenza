# ddp1 — first-time dedup stream with seed-driven key collision (2026-08-14, tick 1464)

Map-of-seen-counts handler state: first observation of a key echoes the value,
a repeat answers the NEGATED repeat count (-2, -3, ...). The kicker: the seed
computes one key as n+3, colliding with the literal 13 exactly when n=10 — so
the two runs disagree on WHICH draws are repeats, not just on values:
- n=10: keys (13,13,7,13,13,7) → 13, -2, 7, -3, -4, -2, kinds=2 → 12980696959802
- n=0:  keys (3,13,7,3,13,7)  → 3, 13, 7, -2, -2, -2, kinds=3 → 3130697979803

Exercises Map.lookup Some/None branch × Map.insert in both branches × Map.len
readout, with negative answers riding the *100 digit-packing.

PASS ×3 wasm. 7 dispatches, arms are single-resume per match branch (zero
chained lets) — cliff-safe shape. **Pool (batch-273).**
