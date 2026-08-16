# Three-phase lifecycle state machines (2026-08-11)

Angle: the state as a SUM-OF-RECORDS lifecycle (Idle | Running(count,peak) |
Done(total)) — variant TRANSITIONS with payload accumulation inside Running,
a sentinel-driven terminal transition, and the absorbing terminal phase.
(The landed variant-transition pins are Option/Result two-phase; a 3-phase
machine with per-phase payload arithmetic was uncovered.)

GREEN x3:
- ph1: Idle -> Running (accumulate count, track peak) -> Done on negative
  input; the query decodes whichever phase holds — 10017/10025
- ph2: Done is ABSORBING (two post-terminal steps thread it unchanged,
  n-independent by construction); the mid-Running query pins the interior —
  50510010/50510010

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-12, HELD-in-pipeline)
ph1 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; 10017/10025 traced). DISTINCT: a user 3-variant SUM (Idle | Running(count,peak) | Done(total)) threaded as effect state w/ variant transitions + per-phase payload arithmetic + absorbing terminal — landed variant-transition pins are Option/Result TWO-phase. HELD behind lx2 (behind queued MR f20-inline fa388e51d). ph2 (absorbing-Done) a further candidate.

## SENT by v-effects (2026-08-12)
ph1 pinned to 14b (MR 9f9c9593b, +3 baseline lines). CLAIMED-HELD -> SENT. ph2 (absorbing-Done) a further candidate.

## ph1 shape COVERED (tick 1315): v-effects pinned the 3-phase lifecycle
(2ae9dcd32, 14b). ph1 PRUNED. Checking their pin for the ABSORBING face...
Their pin covers BOTH ph1 AND the absorbing face (Done absorbs, in-doc).
ph1 AND ph2 both PRUNED — the phase-machine bank fully landed via v-effects.
