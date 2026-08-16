# Bool-op recursion guards (2026-08-10)

Angle: bs1/bs2/bc3 pin Bool ops in straight-line ladders; nothing pins a Bool
DRAW as the recursion CONDITION itself (each iteration's continue/stop decided
by effect state).

All GREEN x3, python-modeled first:
- bg1: `(if (T.more) (walk ...) acc)` — the guard draw decides continuation;
  walk from seed 1 accumulates 1+2+3=6, post-draw 4 — 64/4
- bg2: guard is `(and (< k 6) (T.odd))` — short-circuit AND with a pure bound
  LEFT and a state-advancing draw RIGHT; odd-seed runs the bound out (6 iters,
  interleaved odd/tick draws), even-seed stops at the first draw — 24691213/3
  (authoring slip caught by the model: I mis-simulated the interleave; the
  model's 24691213 was right, the compiler agreed on all 3 backends.)

Pin candidates: bg1/bg2 (guard-position draws are a distinct consumer position
from ladder/value positions).
