# do-def blocks in handler positions (2026-08-11)

Angle: corpus-bugfix just pinned the ABORTIVE do-def-in-perform-arg row
(d756c9153); the remaining handler positions for a do-def block — SEED,
RESUME VALUE, NEXT-STATE — complete the position matrix.

GREEN x3:
- dd1: do-def computes the SEED — 3837/807
- dd2: do-def computes the RESUME VALUE (arm-local scope, rebuilt per
  dispatch) — 907/301
- dd3: do-def computes the NEXT-STATE — 803/200

Pin candidates: staged pool (position-matrix completion set).
