# chn1 — butter churn (2026-08-17, tick 1718)

Attack: a FLOOR-MIN over a QUARTER ratio expanded to 3 leaves (empty sentinel
/ floored-scoop / quarter-scoop) where the quarter compound `(/ cream 4)`
appears x4 in the live leaf (answer, subtract in answer-mod, both rebuild
fields) and the FLOORED leaf uses literal 1 in the same four roles (glb1's
family at quarter ratio + a spill clamp on the OTHER op — reset-to-constant
with the excess split into answer + accumulation, the ftn1 shape).

Envelope: 4 dispatches declined (3-leaf churn + 2-leaf pour); 3 passes.

Differential: cream 9 vs 3: n=10 churns quarters (27 — scoop 2), spills its
pour (811 — excess 1), churns 2 again (28); n=0 churns the FLOOR (12 —
scoop 1 from cream 3), takes the pour whole (64), floors again (15). Reads
481 vs 250.

Hand model: n=10 → 278110280481; n=0 → 120640150250 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 8deb431dd.
