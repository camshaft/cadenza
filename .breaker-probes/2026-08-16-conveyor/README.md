# cnv ladder — conveyor with reject gate and one-shot rework (2026-08-16, tick 1612)

Attack: a 4-branch arm (ship / rework-ship / rework-scrap / scrap) where the
biased quality `(+ q (% n 3))` recurs 6x across conditions and answers, and the
rework branches further derive `(+ (+ q (% n 3)) 3)` from it (compound-on-
compound). The rework-ship vs rework-scrap split nests an if INSIDE the middle
band — branch depth 3.

## Envelope datapoint
- cnv1 (5 dispatches x 4-branch): instruction-budget clean decline. With irg
  (2 compounds, 2 branches: fence 3-4) and tns (1 compound, 3 branches: fence
  5-6), confirms branches x compounds jointly load the fence: 4-branch +
  derived-compound fences below 5.
- cnv2 (3 dispatches): PASSES x3 all backends. Differential: each seed takes a
  DIFFERENT branch at every feed position ([ship, rework-ship, scrap] vs
  [rework-ship, scrap, scrap]) — full branch-coverage swap between seeds.

cnv2 hand model: n=10 rows [71,702,30] tally 312 → 71702030312;
n=0 rows [902,30,20] tally 311 → 902030020311.

Pass x3 wasm + rust + rust-async on trunk 68122fd42. cnv1 held for (b).
