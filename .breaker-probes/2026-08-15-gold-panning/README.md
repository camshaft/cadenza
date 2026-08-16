# gld1 — gold-panning yield decay (2026-08-15, tick 1572)

(yield, wear, total) 3-tuple: `pan` answers the current yield then decays it
by a third truncating plus one (branch-free — the stream never hits the
floor, so the guard was droppable); `move` relocates resetting the yield to
the seed base minus a growing wear cost; `total` accumulates. The richer
claim decays through DIFFERENT residues (30,19,12 vs 20,13,8 — truncation
paths diverge) while the wear ladder moves in lockstep offsets.

Frontier note: the GUARDED pan (floor check on the decayed compound, both
via nested-if and via an arithmetic binder + guard) DECLINED on this 3-tuple
— consistent with the guard-structure frontier (the binder didn't save it
because the guard READS the bound decay while branches write yield/total —
cross-field-ish). Branch-free pan passes. Witnesses not banked separately
(family already mapped).

PASS ×3. **Pool — fills rrl1/hnc1/gld1 (10th trio ready).**
