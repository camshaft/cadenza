# 2026-08-13 two-level Map state (tick 1380)

- `nmm1.sexp` — handler state `(Map Int64 (Map Int64 Int64))`: put does the full
  lookup-modify-reinsert of the inner CHAMP through the outer one (absent group →
  fresh inner via annotated Map.empty), answers pack BOTH level sizes (10·outer+
  inner); get routes through both levels with a -1 miss sentinel. Nested-map
  helpers exist body-side in 05-compound (getm/bump); no nested-map handler STATE
  in 14*. Seeds differentiate via v=n in the first put (d row: 3 vs 0).
  PASS ×3 (111221031/111221001).
