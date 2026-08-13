# 2026-08-13 escrow protocol (tick 1426)

- `esc1.sexp` — (balance, escrow) conservation pair: hold moves funds ONLY when
  covered (v <= bal guard; the bounce touches NEITHER slot), rollback returns
  the whole escrow, commit burns it. The n=5 seed makes the SECOND hold bounce
  (3 > remaining 1) so the rollback returns only the first hold. Conservation
  invariant (bal+esc constant except commit) checkable across every row.
  vs bud1 (budget counter): two-slot transfer semantics with a guard that
  protects BOTH slots on the failure path. PASS ×3 (66391070100/16010540100).
