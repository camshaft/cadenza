# wtr1 — water-jug puzzle (5-cap and 3-cap) (2026-08-15, tick 1530)

(a, b) jug contents: `fill` tops a jug, `pour` transfers min(src content,
dst headroom) via per-direction branches, `emptyj` drains; every answer
packs both jugs as a*10+b. The SEED picks which jug the routine works from
(all six op arguments are seed-conditional ifs in the PERFORM arguments —
the heaviest use of the argument-side seed placement so far): n=10 works
from the 3-jug and reaches the classic measures (30,33,51,1,10 — including
the 1-liter measure); n=0 works from the 5-jug and CYCLES (50,23,53,53,50,23
— the pour-into-full no-op appears as a repeated row).

PASS ×3. **Pool — fills phs5/cel1/wtr1 (sixth trio ready).**
