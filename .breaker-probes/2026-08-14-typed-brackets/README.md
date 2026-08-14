# 2026-08-14 typed-bracket matching (tick 1457)

- `brk1.sexp` — the TYPED extension of prn1's depth counter: the state carries
  a STACK of expected closer codes (LIFO), a close must match the TOP (len-1
  read), a wrong-type close sticky-fails. Seed orders the closes: n=0 closes
  square-then-paren (proper nesting, drains to 0); n=1 closes paren FIRST
  against a square top → sticky -9. Composes cst1's stack discipline with
  prn1's sticky validity — the wrong-ORDER (not just wrong-count) face.
  PASS ×3 (11121110/11120101).
