# 2026-08-13 ping-pong data thread (tick 1373)

- `pp1.sexp` — a value ping-pongs A→B→A→B: each answer becomes the OTHER effect's
  next argument while both states advance independently (A additive +1, B modular-
  multiplicative %97 +2). vs pal1 (alternating draws, no data flow between effects)
  and ti4 (arm-performs-arm chain): here the CROSS-FEED is in the BODY, each
  dispatch's answer data-dependent on the other handler's prior state. Design note:
  first two drafts had algebraically CANCELLING arms (x2 invariant across seeds —
  weak-pin rule caught it); the mod-mult B arm breaks the symmetry. PASS ×3
  (41201680 / 515610750).
