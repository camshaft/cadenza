# 2026-08-13 two-phase commit (tick 1434)

- `tpc1.sexp` — 2PC coordinated in the BODY across two handlers, each an
  (balance, hold) escrow: prepare debits into the hold when covered; fin(ok)
  either burns the hold (commit) or restores it (abort). The DECISION (ok =
  both-prepared) is computed in the body from both prepare verdicts and then
  DISTRIBUTED to both handlers as the fin argument. Seed = B's balance: n=8
  both prepare → commit (a=6, b=2 stay debited); n=3 B refuses → BOTH abort
  (balances restored 10/3 — including A which HAD prepared). The atomicity
  law: A's rollback despite its own success is the pin. Composes esc1's
  single-side escrow into the distributed protocol. PASS ×3 (1110602/1001003).
