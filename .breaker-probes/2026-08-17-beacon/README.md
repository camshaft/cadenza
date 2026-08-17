# Beacon, let chain inside resume's argument (2026-08-17)

- `bcn1.sexp` — the corpus has ZERO cases with a let expression in resume's
  ARGUMENT position (sweep: `(resume (let ` count 0; 49 if-in-arg cases but
  no binder). bcn1 nests a two-deep let chain (turn, then beam = turn%8)
  scoped ONLY within the answer expression, while the next-state tuple
  recomputes the SAME turn and the flash test inline OUTSIDE the binders'
  scope — the emitter must keep the answer-side binders from leaking into
  (or being shared with) the state-side recomputation. The answer's flash
  digit uses the POST-increment count (if beam>=4) matching the state-side
  fold, so a de-dup that shares the wrong side shows up as an off-by-one in
  the low digit. 6/6 rows diverge across n%3 seeds. PASS x3 at 19aefaeba.
