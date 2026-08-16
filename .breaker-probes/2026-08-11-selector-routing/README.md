# Selector routing — effect results drive op CHOICE (2026-08-11)

Angle: a draw's value selects WHICH op the body performs next (control flow
from effect results). bc3 pins flag-driven if-ladders over values; the
op-SELECTION face (different dispatch targets per route) was uncovered.

GREEN x3:
- se1: parity draw selects left/right op twice, straight-line — 101202/201102
- se2: the selection happens per RECURSIVE hop with positional weights —
  parity of the advancing state alternates the route — 125521/216421
  (first draft used a commutative sum: BOTH seeds gave 610, a weak pin that
  couldn't distinguish route orders — rewrote with *10 positional weighting.)

Pin candidates: 251 pool.
