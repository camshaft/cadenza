# Dynamic shift widths from the state (2026-08-12)

Angle: shift AMOUNTS drawn from the state thread (the landed shift pins use
arg/constant amounts). The n=32 seed drives the second draw to 63, where
1<<63 overflows the checked i64 shift and traps UNIFORMLY x3 (semantics
discovery: Cadenza shifts are CHECKED — 1<<63 = would-be sign-bit = trap,
consistent with the no-sentinel/no-silent-wrap philosophy).

GREEN x3 (incl. trap row):
- shd1: draws n and n+31 as widths — 4294967298 / 2147483649 / trap@63

Staged: 14c pool at 10 — cutting batch-236 with the top of the pool next
land. Also verified cross-backend BEFORE banking (a trap row needs the
uniformity check first — a wasm-only trap would be a finding not a pin).
