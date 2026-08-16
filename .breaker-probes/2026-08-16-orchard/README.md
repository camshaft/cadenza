# orc1 — orchard Map ripening with threshold pick (2026-08-16, tick 1640)

Attack: MAP-state with the insert-or-bump pattern where the tend arm computes
`(Map.insert m t (+ r 2))` TWICE (answer's Map.len reads the INSERTED map;
rebuild threads it) — a heap-collection compound duplicated across answer and
rebuild slots. The pick refuse branch resumes the ORIGINAL st (untouched map)
vs the taken branch threading (Map.remove m t) — heap-op divergence per branch.
Report Option-matches a lookup for the ripeness low digit.

Differential: three tends ripen tree 1 to six; the seed shifts the pick
threshold (5 vs 7) so n=0 PICKS (map empties, 506/100) while n=10 REFUSES
(tree stays, 906/016 — report reads len 1 + ripeness 6).

Model notes: 7-row packing overflowed Int64 (assert caught); single-tree
5-row redesign; first threshold spread (n%3 = +0/+1) made both seeds accept —
widened to *2.

Pass ×3 wasm + rust + rust-async on trunk 4c75635d9.
