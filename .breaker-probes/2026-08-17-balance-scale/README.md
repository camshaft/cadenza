# Balance scale, arg-vs-arg (2026-08-17)

- `blc1.sexp` — the corpus's 2-arg op arms almost never compare their two
  ARGUMENTS against each other (sweep found exactly 1 arg-vs-arg comparison
  across 36 two-param arms). weigh's three-way ladder is pure arg-vs-arg
  ((> a b) / (< a b) / equal), answering side+margin and counting only LEFT
  wins; the signed difference folds into a running tilt that goes genuinely
  NEGATIVE (subtraction, not clamp). level sign-splits the tilt without abs
  (no abs prim exists), tagging lean direction in the hundreds digit. The
  seed rides the LEFT pan of three weighings via a body-side let (% n 3)
  consumed by three different call sites. 5/6 rows diverge across seeds,
  both level reads cross the sign boundary. PASS x3 at 19aefaeba.
