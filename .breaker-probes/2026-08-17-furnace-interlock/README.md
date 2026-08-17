# Furnace interlock, Bool ops under and/or/not (2026-08-17)

- `frn1.sexp` — BOOL-RETURNING ops composed under and / or / not (sweep:
  ae5/ae6 short-circuit INT-comparison draws, but zero cases compose
  Bool-RETURNING op results directly under the connectives, and `(not
  (E.op))` count was zero). hot answers (>= t 2) advancing one degree; cold
  answers evenness advancing two; the and skips its right draw when hot is
  false, the or skips when cold is true, the not inverts a lone draw. Each
  op advances the state by a DIFFERENT stride and counts calls, so every
  skip is visible in the interleaved tally rows (first model draft read
  tally once at the end — branch outcomes collapsed to 1/4 divergent rows;
  interleaving tally after each connective exposed the path split, 3/6).
  PASS x3 at dc649b874.
