# 2026-08-13 budget-gated accumulator (tick 1381)

- `bud1.sexp` — state (budget, total): spend consumes budget and accumulates while
  armed, answers remaining; EXHAUSTED spend answers the NEGATED running total and
  no-ops; refill re-arms exactly k more spends; total reads the accumulator. The
  exhausted answer LEAKS the second tuple field through the failure path (c row:
  -15 vs -42 — the only seed-sensitive digit until the final read). Short-circuit
  angle was coverage-killed (ae5/ae6 + or-pins at 14c:1729/1788). vs und1 (flag
  self-cleared) and stk2 (sticky Err): the gate here is a COUNTER, re-armable,
  and the failure answer derives from the OTHER field. PASS ×3 (10351024/10081051).
