# The abort arm itself performs on an outer handler (2026-08-18)

- `abm2.sexp` — the bail arm builds its aborting answer from the
  accumulated inner state PLUS an outer audit drawn WHILE aborting:
  (bail () s (+ (* s 10) (T.audit))). The audit advances the outer
  thread exactly once (body's later audit reads +5: 3106 = 100*(30+t0)
  + t0+5). Completes the abort matrix: abl1 = levy BEFORE the abort in
  a sequence; abm1 = abort reads its own accumulated state; abm2 = the
  abort's ANSWER EXPRESSION performs cross-handler. PASS x3 at 67ef1f754.
