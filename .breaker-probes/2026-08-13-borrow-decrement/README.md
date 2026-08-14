# 2026-08-13 two-limb borrow decrement (tick 1445)

- `bor1.sexp` — the SUBTRACTIVE mirror of odo1's carry: dec subtracts from the
  low limb; underflow BORROWS from the high one (nl+100); a borrow with no high
  limb left SATURATES both to zero — and once saturated, later decs stay zero
  (absorbing floor). Seeds: n=3 → 103-5=98, then 98-98 borrows to... 0-limb
  exhausted rows [98,0,0]; n=50 → [1045(=1,45),47,0] (the middle dec borrows,
  the last saturates). Three-deep nested-if arm with two saturation exits.
  PASS ×3 (9800000000/104500470000).
