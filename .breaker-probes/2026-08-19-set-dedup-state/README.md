# set-dedup-state — Set-state handler, dedup + contains across the seam
## pysd1 — add(k) threads Set.insert (dedup, re-add no-grow) answering size; has(k) reads contains. Model 2110. PASS x3.
CHAMP Set dedup + membership survive resume threading. Promotable. (API: Set.of (list ..), Set.insert/len/contains.)

## 🩸 ROUND-TRIP FINDING: (Set.of (list ...)) as a handler seed does NOT ML-round-trip
pysd1 passes the gate x3 (2110) BUT FAILS the corpus_roundtrip test: the seed
(Set.of (list (% n 3))) does not survive sexpr->ML->sexpr (AST node count 7159 vs 7160,
off-by-one). Isolated: pymr1 (Map.insert seed) + pyls1 (bare (list ...) seed) BOTH round-trip
clean; only the (Set.of (list ...)) nesting breaks. So pysd1 is HELD OUT of the corpus
(round-trip-unsafe); filed to v-metaprogramming/v-syntax as a Set.of-of-list ML-surface
round-trip finding. Replaced in batch-353 by pymx1 (Map remove). The gate alone would have
passed pysd1 — the corpus_roundtrip test is what caught it (per [[corpus-edit-must-run-ml-round-trip-not-just-gate]]).
