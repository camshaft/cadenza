# cbk1 — circuit-breaker state machine (2026-08-14, tick 1466)

3-field state (mode: 0 closed / 1 open / 2 half-open, fail-count, cooldown).
`req` walks a 4-level nested-if arm: open answers -1 and counts the cooldown
down to half-open; the half-open probe restores closed on success or re-trips
on failure; closed trips open on the SECOND failure. `mode` reads the machine.

Seed-differentiated end-to-end: n=10 → first req succeeds (4), two failures
trip it, cooldown eats the 5, half-open probe n-1=9 RESTORES closed, final 7
succeeds, mode=0 → 3999998990700. n=0 → first req n-6=-6 is already a failure,
trips one req earlier, half-open probe hits n-1=-1 and RE-TRIPS, mode=1 →
-101000099 (negative packed total — the -1 sentinel leads the digits).

PASS ×3 wasm. 7 dispatches, arms are pure nested-if branch selection (zero
chained lets) — cliff-safe. **Pool (batch-273).**
