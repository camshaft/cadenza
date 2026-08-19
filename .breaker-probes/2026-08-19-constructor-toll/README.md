# Constructed pair carries the replay and a capture (2026-08-19)

- `pyc2.sexp` — (match (tuple (resume ...) (+ s 1)) ((tuple r w) (+ (* r
  2) (* 1000 w)))): the doubling COMPOUNDS across frames (inner doubles
  once, outer doubles the doubled) while the weights stack linearly —
  8840 = 2*(2*21 + 2000) + 1000 for s0=1's frame order... (CPS-modeled;
  see model). The digit separation isolates replay-flow errors (low
  range compounds wrong) from capture errors (thousands stack wrong).
  Extends pyc1 (flat combine) with COMPOUNDING through the constructor
  round-trip. PASS x3 at f62a6dc18.
- `pyc3.sexp` — the PURE field precedes the resume: (tuple (+ s 1)
  (resume ...)). Answers match pyc2 (reversed layout) EXACTLY — the
  capture is by-value at construction from PRE-resume state, not
  re-read post-replay. Field order with a pure capture is
  order-insensitive; a re-read would have shown s+2 in the weights.
  Boundary note: a FOREIGN levy in field 1 before the resume in field 2
  declines (pre-resume perform x non-tail resume in one constructor —
  known class, /tmp ladder). PASS x3 at 3020d9000.
