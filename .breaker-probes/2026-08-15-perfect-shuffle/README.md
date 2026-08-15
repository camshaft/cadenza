# sfl1 — perfect-shuffle position tracker (2026-08-15, tick 1557)

(position, count) state: each `shuffle` doubles the tracked card's position
mod 7 (the out-shuffle orbit on 8 cards; position 7 fixed); `where` packs
position and shuffle count. The doubling map's orbit structure does the
differentiation: positions {1,2,4} form a 3-CYCLE, so seeds 4 and 1 ride the
SAME cycle entered at different points — the shuffle rows are ROTATIONS of
each other (1,2,·,4,1,2 vs 2,4,·,1,2,4) and the where rows pin the phase
(22/25 vs 42/45). Group-theoretic structure as the seed differential.

2-branch arm, 2-tuple — envelope-safe. PASS ×3. **Pool.**
