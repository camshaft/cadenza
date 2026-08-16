# Prepend-order rope states (2026-08-11)

Angle: every landed rope-state pin APPENDS (concat s x); the PREPEND direction
(concat x s) builds the rope leftward — a rope rep that fast-paths append
could mis-handle prepend seams — and the alternating both-ends face.

GREEN x3:
- pv1: prepend per recursive dispatch builds digit(1)..digit(n) left-to-right;
  len + FIRST byte pin the order — 397/197
- pv2: the arm alternates prepend/append by payload PARITY (both ends grow);
  first AND last bytes pin the two ends — 407008/307008

Pin candidates: 252 pool.
