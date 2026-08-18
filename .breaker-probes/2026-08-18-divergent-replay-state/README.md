# Divergent replay states (2026-08-18)

- `dbr6.sexp` — the two replays thread DIFFERENT states: replay 1 (+ s 1)
  additive (discarded), replay 2 (* s 2) doubling (survives). The next
  dispatch answers from whichever state its replay actually threaded —
  main(10): surviving path is a=11 via replay 2 threading s=2, so b's
  dispatch sees 2 and answers 12 -> 131. Reusing replay 1's state thread
  for replay 2 would give b=13 -> different answer. Completes the dbr
  family's state-thread independence pin (dbr1 value, dbr2 depth, dbr5
  count, dbr6 state). PASS x3 at 29f934387.
