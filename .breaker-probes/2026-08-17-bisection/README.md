# Bisection window, match in handle INIT (2026-08-17)

- `wnw1.sexp` — MATCH expression in the handle's INIT position (sweep: 0 in
  corpus; 40 op-calls, 6 ifs, 1 let, 0 matches). The init destructures a
  seed tuple and an if ladder inside the match ARM picks among three
  (lo,hi,probes) windows. narrow halves toward the guess with a let-bound
  midpoint; a CLOSED window (lo>=hi) answers a frozen 3xx readout WITHOUT
  probing (guard arm added after the first model draft inverted the tight
  window to hi-lo = -7 — Int packing bounds assert caught it pre-gate).
  span reads the gap. The tight seed closes two probes early so its last
  narrows freeze while the wide window still hunts. 5/6 divergent.
  Original id bis1 COLLIDED with the corpus binary-search oracle
  (free-id grep caught it); renamed wnw1, model re-run post-rename.
  PASS x3 at e46e64712.
