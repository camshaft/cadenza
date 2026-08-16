# Ordering-verdict dispatch (2026-08-11)

Angle: the arm answering a 3-way ordering verdict (-1/0/1) against the state,
and the classic consumer of that shape — BINARY SEARCH where the effect state
is the hidden target and the body bisects on verdicts (an oracle-driven
control-flow pattern; 8 dispatches, data-dependent recursion path per seed).

GREEN x3:
- cp1: fixed probe value crosses above->equal->below as the state walks past
  — 99/-11/-111
- cp2: bisect(0,100,8) finds ANY target incl. the 0/100 boundary faces —
  37/0/100/63 exact (each seed exercises a DIFFERENT dispatch/recursion path)

Pin candidates: staged pool (cp2 is a strong one — oracle-search is a real
program shape and every seed takes a distinct path through the fold).

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline)
cp2 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; oracle binary search over hidden-target effect state, 4 seeds each a DISTINCT recursion path incl. 0/100 boundary faces, 37/0/100/63 python-verified). Not already pinned. HELD behind dv-pair (behind queued MR nx-pair b556a9e05). cp1 (simpler 3-way verdict) a lesser twin — cp2 is the strong pin.

## SENT by v-effects (2026-08-11)
cp2 pinned to 14b (MR 354a0b69c, +3 baseline lines). CLAIMED-HELD -> SENT.

## COVERED (tick 1304): cp2 landed via v-effects (5443766f5, 14b)
The oracle binary-search shape pinned. cp2 PRUNED. cp1 (the 3-way verdict
walk) not covered — stays staged.
