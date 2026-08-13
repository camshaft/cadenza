# 2026-08-13 sieve segment (tick 1443)

- `sie1.sexp` — flags for 2..13 in a 12-slot list: sieve(p) clears every
  multiple ≥ 2p via STRIDED List.update writes in one recursive walk (stride p,
  multiple updates per dispatch on one persistent list); count answers the
  survivor sum; probes read the composite 9 (cleared by 3) and the seeded slot
  (7 prime = 1, 4 composite = 0). Strided-multi-update-per-arm extends odo1's
  carry (adjacent cells) with arbitrary-stride jumps. PASS ×3 (70601/70600).
