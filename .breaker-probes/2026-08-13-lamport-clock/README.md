# 2026-08-13 Lamport clock (tick 1428)

- `lpc1.sexp` — the causal-clock law through the thread: local events tick +1,
  receives jump to max(local, remote)+1. Seed = the first remote timestamp:
  n=5 jumps the clock (1→6) and everything downstream shifts; n=0 is STALE so
  the max keeps local and the receive degenerates to an ordinary tick — as does
  the always-stale second receive (ts=2 against clock 3). The max-then-tick
  in-arm composition where the BRANCH result feeds the increment. (Id trap:
  lam1 taken by the lambda-forms pin — renamed lpc1 before banking.)
  PASS ×3 (106070809/102030405).
