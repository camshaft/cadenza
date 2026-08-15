# pkg1 — first-fit bin packing, two bins (2026-08-15, tick 1545)

(b0, b1) state: `place` walks the bins in order answering index*100 +
remaining room, or -1 when nothing fits (the 8 overflows BOTH bins on both
seeds — a shared reject row); `loads` packs both levels. The seed's first
item (n%4+3: 5 vs 3) cascades every later placement: rows
5,104,0,-1,100 vs 7,1,105,-1,101 — placements land in different bins.

Frontier note: the 3-BIN version (4-branch × 3-tuple) DECLINED ×3 — exactly
the wlk/bnf family shape (4-branch lattice × 3-tuple); the 2-bin shrink
(3-branch × 2-tuple) passes. Consistent with the mapped frontier; no new
routing (the family is already with the fold owner).

PASS ×3. **Pool (with hmg1; +1 fills the 8th trio).**
