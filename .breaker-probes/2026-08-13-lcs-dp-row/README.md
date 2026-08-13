# 2026-08-13 LCS DP-row state (tick 1409)

- `lcs1.sexp` — the state is a dynamic-programming ROW (LCS against fixed pattern
  [1,2,3]): each fed character rebuilds the whole row via a recursive fold that
  reads the OLD row (diagonal old[j-1] on match, vertical old[j] on miss) while
  writing the NEW one cell-by-cell (horizontal new[j-1] via the growing
  accumulator). The one-row-back read/write discipline through the state thread —
  the effect-threaded sibling of the body-side subset-sum DP in 05-compound.
  Seeds: feed [1,2,3] climbs to 3; [1,9,3] stalls at 1 then 2. PASS ×3 (123/112).
