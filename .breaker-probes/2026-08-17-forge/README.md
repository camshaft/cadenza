# frg1 — forge tempering cycle (2026-08-17, tick 1696)

Attack: the quench's success branch derives the gain `(- heat 3)` and uses it
in the answer TWICE (raw + inside the hardness mod `(% (+ hard (- heat 3)) 10)`)
while the rebuild REPLACES heat with a constant 2 and accumulates the same
gain — the compound survives its own source field's overwrite in one branch
(read-before-clobber ordering). Stoke's cap answers a constant 109 row.

Differential: opening fire 6 vs 3: n=10 quenches clean immediately (33 = gain
3, hardness 3) and again after stoking (36 = gain 3, hardness 6); n=0's cold
first quench CRACKS (901), then its stoked quench banks 4 (44) — reads 680
vs 481 (hardness 6 vs 4, crack 0 vs 1).

Hand model: n=10 → 330600360800680; n=0 → 9010700440800481 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk c6fa89785.
