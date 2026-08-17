# Drawbridge tidal channel (2026-08-17)

- `drb1.sexp` — the low-tide Bool is LET-BOUND once and consumed by TWO
  SEPARATE ifs in the same arm: one an expression-position if selecting the
  answer's hundreds digit (typed-literal branches), the other selecting which
  next-state TUPLE the resume threads (divergent field updates: +1/+cross vs
  +3/no-cross). Complements gnd1 (let-bound Int compound reused across
  branches) with a let-bound Bool fanned into value-select and state-select
  consumers. log reads without advancing. Seeds n%3 shift the starting tide
  so the slip-under pattern differs across runs. PASS x3
  (wasm/rust/rust-async) at 84c93b0cc.
