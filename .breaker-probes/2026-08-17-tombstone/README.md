# Tombstone, resume value discarded (2026-08-17)

- `tmb1.sexp` — RESUME FOR EFFECT ONLY: (do (resume s (+ s 3)) (+ (* s 100)
  7)). The pyr family's complement: pyr consumes resume's value, tmb1
  DISCARDS it (sweep: `(do (resume` count 0 in corpus). The body's whole
  positional fold and the inner frame's tombstone are both thrown away —
  only the FIRST dispatch's tombstone survives, so the answer collapses to
  s0*100+7 and the seed is visible solely through the first frame's
  captured state. A dead-value elision bug that skips the resume (it has
  effects: the whole rest-of-body runs through it) or one that returns the
  wrong frame's tombstone shows immediately. PASS x3 at 5dc705ee2.
