# 2026-08-12 list-argument batch op (tick 1343, base post-241 trunk)

- `lba1.sexp` — op signature `(-> (List Int64) Int64)`: a heap LIST crosses the
  perform boundary INTO the arm, which folds it into the scalar state by recursion
  (sum-l helper over List.at). Three batches: a literal list, a list BUILT FROM the
  first dispatch's answer (data dependency body→arm→body→arm), and the EMPTY list
  (no-op edge). 14b has list-ARG ops (put/tally) but their arms consume the list
  without a recursive fold into threaded state + answer-derived second batch.
  PASS ×3 (601919 / 1103434).
