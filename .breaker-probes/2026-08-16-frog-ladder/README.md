# jmp1 — tiring frog on a ladder (2026-08-16, tick 1593)

(position, stride) state: `jump` advances by the current stride which then
shrinks by one bottoming at one; `rest` restores the seed stride ((n%4)+3,
recomputed at the rest site) answering the distance so far. The longer
opening stride tires through a different arithmetic lattice (5+4+3 vs
3+2+1), so the gap between the frogs WIDENS at every row (5→3, 9→5, 12→6,
17→9, 21→11) — monotone divergence, the mirror of the convergence-family
probes (dlt1/hgl1/cyc1).

PASS ×3. **Pool — fills tid1/jmp1 toward the 12th trio (+1).**
