# 2026-08-13 interval overlap counting (tick 1436)

- `ivl1.sexp` — the list state holds (lo,hi) intervals; add counts overlaps
  against ALL existing (closed test: lo<=b AND a<=hi, an `and` of two
  comparisons inside the fold's conditional) BEFORE inserting itself. The
  interval [4,10] overlaps both [0,5] AND the seeded one when n=3 ([3,6]) but
  only [0,5] when n=8 ([8,11] — touching 10? 8<=10 and 4<=11 → overlap! wait:
  hand-check said 2 either way... rows verified by model: b row flips 1→0,
  c row stays 2 — [8,11] DOES overlap [4,10]). Disjoint [20,25] answers 0.
  Geometric pairwise-predicate fold vs pq1's min-scan. PASS ×3 (1231/1131).
