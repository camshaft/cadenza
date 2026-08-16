# cyc1 — bicycle cadence over four gear ratios (2026-08-16, tick 1586)

(gear, distance) state with a let-free 4-way ratio callee (20/27/34/41):
`shift` clamps into [0,3] answering the landed ratio's tens digit; `pedal`
answers rpm·ratio/100 accumulating distance (the speed compound recomputed
in both slots, 2 consumers — tnk-axis safe); `odo` reads.

Seed start gear (2 vs 0): the opening pedals diverge (20 vs 12), the big
−3 downshift CLAMPS both runs to the bottom gear (shared clamp row 2), and
the tails converge exactly (12, 2, 24) while the odometers carry the
opening difference (80 vs 64) — early divergence, clamp-forced convergence,
divergent memory in the total.

PASS ×3. **Pool — fills tnkB/cpd1/cyc1 (11th trio ready).**
