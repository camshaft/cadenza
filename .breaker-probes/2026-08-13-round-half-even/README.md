# 2026-08-13 round-half-to-even accumulator (tick 1410)

- `rhe1.sexp` — banker's rounding at the integer level: v/2 rounded half-to-even
  (exact halves bump only when the truncated quotient is ODD — rhe(3)=2, rhe(5)=2,
  rhe(7)=4), accumulated through the thread. Composes trunc-div + double parity
  test (of x and of q) per dispatch. Odd seeds hit the bump path on the first
  dispatch (3→2), even seeds skip it (6→3). PASS ×3 (2040809/3050910).
