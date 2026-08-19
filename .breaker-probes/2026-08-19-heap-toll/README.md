# Heap list built and measured inside the toll (2026-08-19)

- `pyl1.sexp` — the post-resume toll CONSTRUCTS a heap list (size gated
  on captured state) and charges 100*List.len: heap allocation happens
  DURING the unwind (610 / 400, CPS-modeled). Notable vs the scalar-only
  boundary (pys2 README): a String flowing THROUGH the post-resume
  expression declined, but a list built AND fully consumed WITHIN the
  toll (Int64 in, Int64 out) FOLDS — the boundary is about heap values
  crossing the resume seam, not heap ops inside the toll. Sharpest
  statement yet of the scalar-seam law. PASS x3 at 3020d9000.
- `pyl2.sexp` — a GROWING LIST as the state thread: seed = head, every
  dispatch answers 10*head + len and pushes the old length (1211 / 201).
  Heap state through TAIL resumes is fine (the scalar seam applies to
  post-resume flow, not the state thread itself — consistent with tt4/
  push2-class corpus cases). Design notes: List.range not in prelude;
  Option arms are capitalized (Some/None) — lowercase (some) is a
  CDZ0101. PASS x3 at 3020d9000.
- `pyl3.sexp` — HEAP state through DIVERGENT replays: discarded replay
  pushes 9, survivor pushes 5 onto the SHARED starting list; the next
  dispatch reads slot 1 and finds 5, never 9 (1510 / 500 — the +5 in
  the hundreds-range answer is the survivor's stamp). The heap analogue
  of dbr6's survivor-thread law — critical because persistent-structure
  sharing could leak the discarded push if the runtime aliased instead
  of versioning. Draft-1's answers didn't read slot 1 (weak observable)
  — strengthened before pinning. PASS x3 at 3020d9000.
- `pyl4.sexp` — the heap TOMBSTONE reads the arm's ORIGINAL pre-replay
  binding: both replays push (9 then 5) yet the surviving tombstone
  reads len=1 and the seeded head (101 / 1) — neither push contaminates
  the captured list. The heap twin of pyk3's capture-not-live law and
  the tombstone complement of pyl3's survivor-visibility. PASS x3 at
  3d3ef1d49.
