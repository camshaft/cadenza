# Fibonacci-rotating tuple state under tolls (2026-08-18)

- `pyk3.sexp` — the state rotates (a,b) -> (b,a+b) per dispatch while
  the toll charges 1000*a AS CAPTURED: frame 2's captured first field
  IS frame 1's second field, so a toll reading the rotated tuple
  instead of the captured one shifts the thousands by exactly the
  rotation (3211 / 2101, CPS-modeled). The moving-state stress on
  pyr8's binder-lifetime law: the captured pair and the live tuple
  DIVERGE immediately after the resume. PASS x3 at 8575e9099.
