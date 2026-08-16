# ML round-trip failure: record-typed effect-op payloads (tick 1041, base 67b9cc9e0)

corpus_roundtrip FAILED 6572/6574 when rp2/rp3 (record op-arg cases) were appended - the
corpus-edit-must-run-ML-round-trip trap caught it pre-send; cases HELD, corpus reverted.

Hand bisection via `cdz convert`:
- rt-min1: effect op sig (-> (Record (: k Int64)) Int64) + record-literal ARG -> ML prints
  `step : Record(k : Int64) -> Int64` + `M.step({ k = n })`; ML->sexpr re-parse yields
  ("record" (k n)) - a STRING head, AST mismatch.
- rt-min2: def-param Record ascription + record arg -> same ("record") on re-parse.
- rt-min4: record SEED + record rebuild in arm (rr1's landed shape) -> hand-convert ALSO shows
  ("record"), yet rr1 passes the corpus test - so the test may normalize the literal head and
  the REAL discriminator for rp2/rp3 is likely the (Record ...) ascription inside the effect-op
  signature (or the combination). v-syntax to root-cause; witnesses cover both layers.
