# Arm next-state trap ordering (2026-08-11)

Angle: nothing pins a TRAP inside the arm's NEXT-STATE expression (s6d pins
wrapper-init traps in the BODY; the 6 landed trap pins are @requires/decode/
one bp-family div).

GREEN x3:
- nt1: next-state = (/ 100 (- s 4)) — seeds 6/2 thread both signs through,
  seed 4 traps div-by-zero ON THE DISPATCH WHOSE STATE IS CONSUMED NEXT —
  110 / trap / -30
- nt2: the SAME trapping next-state on the LAST dispatch (never consumed) —
  returns normally on all 3 backends (4/6). The unconsumed next-state is
  elided by the demand-driven fold.

RULED (operator via concierge, tick 1202): STRICT. resume evaluates next-state
before continuing, so nt2's silent elision is a UNIFORM 3-BACKEND MISCOMPILE —
FINDING #17. Filed: queue adv-unconsumed-next-state-trap-elided.sexp (strict
expectations: seed 6 -> 6, seed 4 -> trap) + issue to corpus-bugfix. nt1 is the
green consumed-state face, promotable NOW. nt2 flips to a corpus pin when the
fold fix lands (watch).

## Strictness sweep (tick 1203) — the full face matrix
- resume VALUE trap, result DISCARDED by continuation: ALREADY STRICT (traps
  x3, /tmp/rv1) — no gap.
- SEED trap, state never consumed (body never performs): ELIDED x3 (/tmp/sd1)
  — FILED as the seed face of #17 (adv-unconsumed-seed-trap-elided.sexp).
  With one consuming dispatch the seed trap fires (/tmp/sd2 control).
- next-state trap, unconsumed (LAST dispatch): FILED as #17 proper.
The gap is exactly the unconsumed STATE THREAD (seed + next-state); value-
position traps are already strict. One fold increment should fix both.

## Third face (tick 1204): unread op ARGUMENT
- aborting arm ignoring its param (/tmp/ab1): elided x3 (999/999)
- resumptive arm resuming a constant (/tmp/ab2): elided x3 (7/7)
- arm that READS the param (/tmp/ab3): traps correctly (950/trap)
FILED adv-unread-op-argument-trap-elided.sexp. Family complete: next-state +
seed + op-argument — all three are "value not demanded => user trap dropped".

## Fourth face (tick 1205): unread op-arg DROPS A FOREIGN PERFORM
- (A.fire (B.tick)) where A's arm ignores its param: the B dispatch is dropped
  with the argument — later B.tick reads STALE state (37 vs strict 47) x3.
- NO TRAP involved — silent effect-count divergence in ordinary code. This
  upgrades the family from trap-order to general effect-soundness.
FILED adv-unread-op-arg-drops-foreign-perform.sexp + concierge backlog.
Family: next-state / seed / trapping-arg / PERFORMING-arg (4 faces, 1 home).

## Boundary controls (tick 1206): body-side discards are STRICT
- unused LET-bound perform (/tmp/dl1): strict x3 (40/10)
- wildcard-MATCH-discarded perform (/tmp/dm1): strict x3 (40/10)
The drop is EXACTLY the op-arg->unread-arm-param path + the two state-thread
positions (seed/next-state). Body-side demand analysis is correct. Noted to
corpus-bugfix to narrow the fix surface.

## Face-4 fix edge-test (tick 1238)
- f4a: TWO foreign performs in one unread arg — BOTH advances thread (507/207
  strict) — fix is compositional on the resumptive path.
- f4b: the ignoring arm ABORTS — the foreign advance is STILL dropped
  (55003 vs strict 55004) x3. FILED as the face-4 abort RESIDUAL
  (adv-abort-arm-unread-arg-drops-foreign-perform.sexp).

## Face-4 residual 2 (tick 1239): HEAP-state foreign advance
- f4c: unread arg performing the SAME (inner-handled) effect: 9, correct.
- f4d: the FOREIGN handler's state is HEAP (List push per dispatch) — the
  push in the unread arg is DROPPED (7 vs strict 17) x3. FILED
  (adv-unread-arg-heap-state-foreign-perform-dropped.sexp). The fix threads
  the scalar slot but not the heap multi-value state route.

## RETRACTION (tick 1242): residual-2 (heap-state) was FALSE — my stale binary
corpus-bugfix + v-effects verified f4d is strict-correct (17/17) on trunk with
a fresh binary; my report ran a pre-face-4 cdz. Queue reproducer deleted,
retraction sent. f4d is now a PASS witness (pin candidate). The ABORT residual
WAS real — fixed as 2bfb2bbdf (pending land; verify then).
RULE: rebuild the pipeline binary IN the same command chain as any post-fix
probe. (Second stale-binary instance in two ticks — the first I caught, this
one I filed on.)
