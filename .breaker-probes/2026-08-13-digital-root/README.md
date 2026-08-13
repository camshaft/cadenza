# 2026-08-13 digital root (tick 1433)

- `dgr1.sexp` — NESTED fixed-point recursion in the arm: droot repeats dsum
  (itself a recursion) until the value is a single digit — TWO recursion levels
  per dispatch, iteration count data-dependent (999 → 27 → 9 takes two droot
  rounds; 47 → 11 → 2 also two; 5 → one). The accumulator's low digit rides
  in the answer alongside the root. cla1 pins single-level arg-driven depth;
  the RECURSION-INSIDE-RECURSION arm face is new. PASS ×3 (229123/559426).
