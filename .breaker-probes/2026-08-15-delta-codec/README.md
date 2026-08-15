# dlt1 — delta codec with shared previous-value slot (2026-08-15, tick 1525)

SCALAR prev slot, branch-free single-expression arms (the simplest arms in
the pool): `enc v` answers v−prev storing the raw v; `dec d` answers prev+d
storing the reconstruction. Interleaving enc/dec CROSS-TALKS through the
shared slot by design — the pin IS the exact cross-talk sequence. Seed rows:
first two differ (14,−5 vs 4,5 — note the sign flip on row 2), then the
seed washes out and the tails converge exactly (12,8,15,17 both) — a
convergence pin (state fully determined by the last write, history erased).

PASS ×3. **Pool (7th trio seed).**
