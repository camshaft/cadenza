# 2026-08-12 Map.remove churn (tick 1326, base cfedca65f)

- `mrv1.sexp` — remove-then-reinsert churn on a Map handler state: `put` answers the new
  Map.len, `del` answers the removed value via lookup-match (0 when absent) and removes.
  Seed-differentiation: n=3 → the trailing `del 99` misses (e=0, 125020); n=98 → it HITS
  the (+ n 1)=99 key planted earlier (e=7, 125027) — the same call site flips on the seed.
  Hand-modeled; PASS ×3. First 14c pin exercising Map.remove in an arm (7 prior uses are
  all in bodies/helpers).
