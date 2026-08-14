# bkf1 — exponential backoff with seed-shaped cap (2026-08-15, tick 1498)

(delay, total) state: `fail` accrues the current delay into the total then
doubles it clamped at the cap (n+6, recomputed in-arm); `ok` resets the delay
to 1 answering the accumulated wait. The higher cap (16 vs 6) lets n=10 keep
doubling (2,4,8,16) where n=0 saturates two steps earlier (2,4,6,6); the
accumulated totals differ (15/18 vs 13/16) and persist across the reset.

Envelope datapoint: 8 dispatches × 2-branch arm × 2-TUPLE passes — consistent
with the F24 zone being multi-branch × 3+-tuple (scn1's 4-branch × 3-tuple
broke at 6; this 2-branch × 2-tuple is fine at 8).

PASS ×3 wasm. **Pool (with lhn1; +1 fills the trio).**
