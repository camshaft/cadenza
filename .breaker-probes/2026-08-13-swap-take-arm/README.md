# 2026-08-13 value-yielding map ops in arms (tick 1396)

- `swt1.sexp` — Map.swap and Map.take (the VALUE-YIELDING two-form ops) power the
  arms: put swaps and answers the PRIOR value (Some p → p, None → -1), del takes
  and answers the removed value; both tuple-project (prior, m2) INSIDE the arm
  and thread m2. Neither op appears anywhere in 14* (their pins are body-side
  05-compound persistence cases); the arm face makes the answer derive from the
  op's REPORTED optional rather than a separate lookup. Sequence: put-fresh(-1),
  put-replace(prior=n), take(8), take-absent(-1), put-new-key(-1).
  PASS ×3 (10510101/14210101).
