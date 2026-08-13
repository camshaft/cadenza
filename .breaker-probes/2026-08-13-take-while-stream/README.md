# 2026-08-13 take-while pull stream (tick 1401)

- `twl1.sexp` — pull-based stream with IN-BAND termination: the body's recursive
  driver keeps drawing (budget k=8) until the arm's value is divisible by 4,
  accumulating survivors ×100 and counting ALL pulls including the terminator.
  Generator arm: v = s²%23, s+1 stepping — quadratic residues make the stop
  position seed-dependent (n=3 stops at pull 2 on 16; n=7 at pull 4 on 12).
  vs bis1 (oracle narrows an interval): here the TERMINATION SIGNAL rides
  in-band in the data stream, and the terminator is counted but not accumulated.
  PASS ×3 (902/31803).
