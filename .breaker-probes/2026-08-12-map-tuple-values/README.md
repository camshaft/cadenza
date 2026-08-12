# 2026-08-12 Map with tuple values (tick 1335, base post-239 trunk)

- `mts1.sexp` — handler state `(Map Int64 (Tuple Int64 Int64))`: per-key (count,sum)
  stats, updated by tuple-rebuild inside the arm (lookup-match → tuple-match →
  fresh pair → insert), answer packs the NEW pair (100*count+sum). The tuple-VALUED
  map complement to tug1 (tuple OF collections) and mml1 (list-valued map).
  The seed needs `(: Map.empty (Map Int64 (Tuple Int64 Int64)))` and the second obs
  uses v=n so seeds differentiate (109207104 / 109211104). PASS ×3.
