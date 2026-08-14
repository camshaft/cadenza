# odf1 — token-bucket rate limiter with overdraft penalties (2026-08-14, tick 1461)

Three-op handler over (tokens, penalties): `spend` succeeds only when the bucket
covers the request (else penalty++ and a 0 answer), `refill` saturates at cap 10,
`pens` reads the accumulated penalty count. Seven straight-line dispatches mix
success/overdraft/saturation faces; seed-differentiated (n=10 → 4001008001002,
n=0 → 500031003 — different penalty counts AND different saturation paths).

PASS ×3 wasm. **Pool candidate (batch-273).**

## Cliff contrast datapoint
7 straight-line dispatches + branching if/match arms compiles FINE — the arms
have ZERO chained lets (each branch is a single resume expression). Paired with
tsq4 (declines at 4 dispatches with a branch-local dual-use let), this isolates
the ≥2-chained-lets factor: dispatch count alone doesn't trip the cliff.
