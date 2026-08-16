# State-as-divisor trap faces (2026-08-11)

Angle: division BY the state through the thread — the two i64 division traps
(zero-crossing and INT_MIN/-1 overflow) driven by the SEED, so the exact
dispatch that traps is input-selected. (The landed div pins divide by op args
or constants; the state-as-divisor direction was uncovered.)

GREEN x3:
- dv1: descending state crosses zero — seed 5/2 exact, seed 1 traps
  divide-by-zero on dispatch 2 — 25020/100050/trap
- dv2: INT_MIN / state — seed ±2 give exact halves, seed -1 traps integer
  OVERFLOW (the other division trap) — ±4611686018427387904/trap

Pin candidates: staged pool (both trap kinds through the state thread).

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline as a PAIR)
dv1+dv2 VERIFIED ready to pin to 14b (both green x3 + opt-sweep 0-div incl. trap faces; 25020/100050/div-by-zero-trap and halves/overflow-trap traced). DISTINCT: 14c:2738 divides by an ARM-produced value (no trap), 14c dv3 GUARDS MIN away; these divide the op-value BY the THREADED STATE and let it land exactly on the trapping value (0, INT_MIN/-1) — the trap-through-thread direction, uncovered. HELD behind nx-pair (behind op2, behind queued MR sl1 990a7208a).

## SENT by v-effects (2026-08-11)
dv1+dv2 pair pinned to 14b (MR 09609e60e, +2 baseline lines x3 backends). CLAIMED-HELD -> SENT.

## COVERED (tick 1302): dv1/dv2 landed via v-effects (bc2c399dc, 14b)
Both shapes pinned verbatim (my tick-1266 bank). PRUNED from staging.
