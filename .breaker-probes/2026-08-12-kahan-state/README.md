# 2026-08-12 Kahan compensated summation state (tick 1344, base post-241 trunk)

- `neu1.sexp` — classic Kahan (y=v-comp; t=sm+y; comp=(t-sm)-y) as a (sum,comp)
  Float64 tuple handler state. Feeding [2^53, n, -2^53]: the compensated thread
  RECOVERS n exactly while the in-body naive control ((big+n)-big) absorbs it to
  n-1 (for odd n: 1→0, 5→4 at that magnitude). Every step is an exact binary
  operation — deterministic across backends, no FP-rounding ambiguity; pins that
  the arm's float sub-expressions are evaluated EXACTLY as written (an emitter
  that algebraically simplifies (t-sm)-y to 0 destroys the compensation and k
  collapses to the naive value). PASS ×3 (10.0 / 54.0).
