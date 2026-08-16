# Map.remove generations (2026-08-11) — 05-target (hold-safe)

Angle: three structure-sharing REMOVE generations (a, remove(a,2),
remove(remove(a,2),3)) — each ancestor still holds what its descendant
removed. The landed Map.remove pins are single-generation (drain-all and
alternating-build); the chained-generations face with ancestor re-reads was
uncovered. The Map twin of ug1's update generations.

GREEN x3:
- mr1: lens 3/2/1 + ancestor re-reads exact — 3230305/3230300

05 batch pool: lc1 + as1 + dc1 + ug1 + nl2 + mr1 (6 — full batch).
