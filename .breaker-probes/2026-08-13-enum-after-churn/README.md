# 2026-08-13 enumeration after remove-churn (tick 1371)

- `mec1.sexp` — five keyed inserts through the state thread (n=1 seed makes the
  fifth insert COLLIDE with literal key 3, an overwrite face), two removes, then
  walk folds the sorted Map.to-list key sequence + count in one answer. Pins
  CHAMP enumeration order/count after interleaved insert-remove churn (existing
  churn pins: mrv1 remove-reinsert answers lookups, mi1 delta counts — neither
  walks the SORTED enumeration post-remove). Seeds: n=4 → keys 3,4,6 (304063);
  n=1 → keys 1,3 w/ overwrite (1032). PASS ×3.
