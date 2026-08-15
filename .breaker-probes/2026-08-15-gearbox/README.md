# gbx1 — three-speed gearbox odometer (2026-08-15, tick 1538)

(gear, odometer) 2-tuple with a let-free speed lookup callee (1→2, 2→5,
3→9): `shift` clamps into [1,3] via a 3-branch lattice answering the landed
gear; `drive` accrues t*speed(gear), the compound recomputed in both slots.

Seed starting gear (n%3)+1 (2 vs 1) compounds through EVERY drive — the
odometers pull apart (10,37,46,54 vs 4,19,28,36) while the shift answers
CONVERGE (3,3 mid-run then both clamp to 1 at the -2 downshift: identical
rows 2/4/6). Divergent-and-convergent rows interleaved in one stream.

Note the callee (speed) is called from inside the drive compound in both
slots — a call-in-recompute face that stays green (2-tuple). PASS ×3.
**Pool (with bnfE; +1 fills the 8th trio).**
