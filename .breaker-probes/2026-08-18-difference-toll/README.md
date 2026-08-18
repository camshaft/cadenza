# Non-commutative difference toll (2026-08-18)

- `pyv2.sexp` — the toll is (v - s): argument MINUS captured state, so
  operand order matters (swapping negates). The raising seed SHRINKS the
  answer through both differences (868 for s0=1 vs 1057 for s0=0 — the
  inverted ordering is itself the signature). All prior capture tolls
  were commutative products/sums; pyv2 pins operand ORDER within the
  toll expression across the suspend. PASS x3 at b7972ffd6.
