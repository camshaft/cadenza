# tail-foreign-nextstate — bare foreign perform in a TAIL arm's next-state DECLINES (coverage gap)

## pytf1 — TAIL arm, bare foreign perform in the NEXT-STATE hole: DECLINES
```
(handle F 0 ((aux () fs (resume 40 fs)))
  (handle E (% n 3)
    ((tick () s (resume (+ s 1) (F.aux))))    ; TAIL, next-state = bare foreign perform
    (+ (E.tick) (* 10 (E.tick)))))
```
DECLINES uniformly wasm+rust+rust-async: "this handler is not yet reducible by the
tail-resumptive fold (cross-function or non-tail resume arrives in a later increment)".
Model (if it folded): 412/411.

## Verdict: SAFE OVER-DECLINE (coverage gap, NOT a miscompile)
Isolated by controls:
- pytf1-pure (tail next-state = literal 40, no perform): FOLDS 412/411. So tail-next-state
  itself is fine.
- pytf2-ans (same foreign perform in the ANSWER hole instead): FOLDS 461/450 (model
  11*seed+450, verified). So a foreign perform in the tail ANSWER hole folds.
=> The decline is SPECIFIC to a bare foreign perform in the tail NEXT-STATE hole: the
tail-resumptive fold can't yet reduce a next-state expression that itself performs an
effect (the resume's second arg requires evaluating a foreign draw).

## Contrast with the two-hole path
pyfn1 (bare foreign perform in a TWO-HOLE arm's next-state) FOLDS 41412/40411. So this
is the OPPOSITE asymmetry from the nested-handle matrix: for a bare foreign perform in
next-state, the TWO-HOLE path folds but the TAIL path declines. (For a nested closed
handle, tail folds everywhere and two-hole declines.) The tail fold's next-state slot
wants a threadable pure/value expression; a foreign perform there trips the coverage gap.

## Status
FILED to v-effects as a tail-fold coverage gap (safe over-decline). pytf1 held as a
decline/todo-witness (oracle 412/411, flips to pass if the tail fold learns to sequence
a foreign perform into the next-state). pytf2-ans is a promotable PASS-witness.

## RULING (v-effects, tick 1881): COVERAGE GAP — should fold to 412/411, deferred tail-fold increment
v-effects reproduced and ruled pytf1 a COVERAGE GAP (safe over-decline), NOT a miscompile,
NOT intended-permanent. The correct fold is 412/411 (sequence F.aux on the outer F handler,
thread its result 40 as the next-state, resume). The decline message literally says "arrives
in a later increment" — the tail fold just does not yet SEQUENCE a foreign perform into the
next-state slot. Keep pytf1 as a decline/todo-witness with oracle 412/411 so it AUTO-FLIPS
todo->pass when the tail fold gains foreign-perform-in-next-state sequencing. Tracked as a
tail-fold coverage gap (low-urgency), candidate for the same fold-extension work, DISTINCT
from the concierge-deferred two-hole closed-handle correct-fold. The asymmetry is
COMPLEMENTARY: each fold path (tail vs two-hole) has the OTHER's gap for a
draw-feeding-the-resume-arg shape, in different slots.
