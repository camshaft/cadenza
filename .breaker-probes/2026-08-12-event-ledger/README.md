# 2026-08-12 event-ledger state (tick 1347, base post-242 trunk)

- `ldd1.sexp` — handler state `(List (Tuple Int64 Int64))` as an append-only event
  LEDGER: log pushes (tag,value) tuples, replay runs a FILTERED FOLD in the arm
  (recursive helper over List.at + tuple-match, accumulating only matching tags;
  absent tag → 0). List-of-tuples STATE with tag-filtered arm folds is new — mi1's
  (List (Tuple ..)) is an op RESULT of Map.to-list, not a threaded ledger.
  Seeds differentiate replay-1 (3+5=8 vs 20+22=42). PASS ×3 (123008070/123042070).
