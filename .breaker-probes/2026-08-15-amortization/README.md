# dbt1 — debt amortization schedule (2026-08-15, tick 1500)

SCALAR principal state (no tuple): `pay` computes truncating interest at the
seed-shaped rate ((n%4)+1 percent, recomputed twice in-arm — the interest
expression appears in BOTH the resume value and the state slot, a scalar
dual-use-by-recompute), remainder reduces principal; `left` reads the balance.

Rates 3% vs 1%: interest slices (3,2,1 vs 1,0,0 — truncation zeroes the low
rate's later slices) and drifting principals (65/36 vs 61/31). The truncating
/ on (p*rate)/100 exercises integer division on composite numerators.

PASS ×3 wasm. **Pool (with lhn1/bkf1/vwp1 — 4 deep now, next trio + 1).**
