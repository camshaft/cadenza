# Parameterized handle helper (2026-08-11)

Angle: a def-wrapped handle whose SEED is a def parameter, called multiple
times — each call instantiates a fresh region whose recursive draws start
from its own seed. Interacts with the spec-dedup (the region def is one spec
serving two seed values) and the seed-strictness family (param seed, always
consumed here).

GREEN x3:
- hc2: region(5,n) + region(70,n) — same def, two seeds, recursive draws
  per region — 213018/0
- hc3: CHAINED — region(region(2,n),n), the drained result seeds the next
  region through the same def — 30/2

Pin candidates: 242 pool.
