# Tolled arm under a branching body (2026-08-18)

- `pyi1.sexp` — the first draw's ANSWER picks which body branch performs
  the second draw: (if (> (E.tick) 0) (+ 100 (E.tick)) (+ 200 (E.tick))).
  Each frame's toll wraps whichever continuation its dispatch actually
  contains — the positive seed's frame-1 continuation holds the 100
  branch, the zero seed's holds the 200 branch (5102 / 3201, CPS-modeled
  and cross-checked). Notable vs pyb1 (if OUTSIDE the handle): here the
  if is INSIDE the handled body with a tolled arm and FOLDS — the
  fold-boundary decline seen earlier (pyb1 README) was for the Bool-body
  + Int-wrapper mix, not branching bodies per se. PASS x3 at e11e4d3d8.
- `pyi2.sexp` — branches with UNEQUAL DISPATCH COUNTS: the drawn parity
  routes to a one-draw branch (even) or a TWO-draw branch (odd), so the
  seeds stack 2 vs 3 tolled frames from the same program (9403 = odd
  path with frames at s=1,2,3: fold 403 + tolls 2000+3000+4000; 3101 =
  even path: 101 + 1000+2000). A fixed-frame-count assumption misprices
  the deeper seed. Data-dependent frame depth (pyb2's law) x toll
  composition (pyi1) in one machine. PASS x3 at e11e4d3d8.
