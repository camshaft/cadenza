# Saturating clamp state (2026-08-12)

Angle: the arm clamping EVERY transition to a range via a PURE HELPER call
in the next-state computation — a cross-def helper in the state-transition
position (the transition itself calls out), both saturation bounds hit in
one run.

GREEN x3:
- clp1: nudges +7,+7,-25 from seeds 0/5 — upper bound saturates at 10 (seed
  5's first nudge already lands 10... wait: seed 5: 5+7=12->10; seed 0:
  0+7=7, 14->10) then the -25 floors at 0 — 71000/101000

Staged: 14c pool at 7 (cz1, gcd1, fib1, rle1, dgn1, pal1, clp1).
