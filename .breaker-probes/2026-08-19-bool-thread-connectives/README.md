# Bool state thread under and+not composition (2026-08-19)

- `pyb3.sexp` — flip answers the flag and NEGATES the thread; the body
  demands draw1 AND (not draw2), which the alternating thread delivers
  exactly when the seed starts true (n=0 seed -> 1); the false-starting
  seed short-circuits past the second draw (n=10 -> 2). Composes the
  Bool thread (bs1-class) with connective composition (frn1) and the
  short-circuit dispatch-count law (pyb1/2) in the smallest machine that
  exercises all three. PASS x3 at f62a6dc18.
