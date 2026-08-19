# Constructed pair carries the replay and a capture (2026-08-19)

- `pyc2.sexp` — (match (tuple (resume ...) (+ s 1)) ((tuple r w) (+ (* r
  2) (* 1000 w)))): the doubling COMPOUNDS across frames (inner doubles
  once, outer doubles the doubled) while the weights stack linearly —
  8840 = 2*(2*21 + 2000) + 1000 for s0=1's frame order... (CPS-modeled;
  see model). The digit separation isolates replay-flow errors (low
  range compounds wrong) from capture errors (thousands stack wrong).
  Extends pyc1 (flat combine) with COMPOUNDING through the constructor
  round-trip. PASS x3 at f62a6dc18.
