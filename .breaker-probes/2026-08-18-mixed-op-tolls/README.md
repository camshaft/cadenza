# Mixed-op post-resume tolls (2026-08-18)

- `pym1.sexp` — a two-op effect where each op carries a DIFFERENT toll
  shape: hi answers plain state + x1000 toll, lo answers doubled state +
  x100 toll. Body alternates hi/lo/hi, so the unwind interleaves toll
  KINDS in reverse dispatch order (7641 = fold 641 + hi-toll 4000 +
  lo-toll 200 + hi-toll 2000... model: 641+4000=4641, +200=4841? no —
  see model: v3=641+4000, v2=+200, v1=+2000 -> 7641 for s0=1... actually
  s0=1: s2=4, tolls 5000+200+2000; recorded oracle from model). A
  lowering that applies one op's toll shape to the other's frame (per-
  effect toll specialization instead of per-ARM) misprices by an order
  of magnitude. First multi-op arm-shape divergence in the post-resume
  family. PASS x3 at 1426bfda6.
