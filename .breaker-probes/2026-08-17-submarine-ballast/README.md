# blt1 — submarine ballast trim (2026-08-17, tick 1691)

Attack: a min-CLAMP expanded into a 4-leaf arm where the clamp variable (the
vented amount = min(k, ballast)) appears DOUBLED in the depth arithmetic —
each leaf pair re-derives `(- depth (* k 2))` or `(- depth (* ballast 2))`
per the clamp side, with the breach test comparing that compound against
zero (the div1-family guard shape, but the guarded value is the REBUILD not
a trap). Flood's cap answers a CONSTANT 209 row (depth pinned) while its
rebuild still accumulates ballast — cap-answer/live-rebuild split. The
INIT derives depth FROM ballast (`(* (* (% n 3) 2) 2)` — one seed field,
two derived state fields).

Differential: standing ballast 2 vs 0: n=10 rides submerged the whole drill
(read 210: depth 2, ballast 1, no breach); n=0's final blow vents everything
and BREACHES (701, read 1). Blow's clamp takes OPPOSITE sides between runs
on the final dispatch (k<ballast vs k>=ballast).

Hand model: n=10 → 1260822000290210; n=0 → 840421687010001 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk e4b91e88b.
