# answer-hole-handle — nested closed handle in the RESUME-ANSWER hole (pyre3 sibling)

## Motivation
The pyre3 fix (348bd4805) declines a nested closed handle in the **next-state** hole
(2nd arg to `resume`) of the two-hole refold — the position that silently miscompiled.
The untested sibling is a nested closed handle in the **resume-answer** hole (1st arg
to `resume`). Distinct refold position; the guard may or may not have covered it.

## pyre6 — nested closed handle in the answer hole
```
(handle E (% n 3)
  ((tick () s
    (+ (resume (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2))  ; answer = 42
               (* 10 s))                                                        ; next-state = plain arith
       (* 1000 s))))
  (+ (E.tick) (* 10 (E.tick))))
```
Deep handler, two outer ticks. Inner handle is closed+pure, reduces to 40+2 = 42.

## Verdict: PASS-WITNESS (correctly compiled — NOT a miscompile)
- Hand model (independent python): main(10)=11462, main(0)=462.
- **Referential-transparency control** (`/tmp/pyre6-ctrl.sexp`, banked shape below):
  inner handle replaced by literal `42` → PASSES 11462/462 on wasm+rust+rust-async.
- pyre6 (nested handle producing 42) → PASSES 11462/462 on wasm+rust+rust-async.
- The nested-handle answer equals the literal-value answer ⇒ referential transparency
  holds ⇒ genuine PASS, not a routing-independent false-negative.

This PINS the boundary: the two-hole refold handles a nested closed handle correctly
in the FIRST hole (resume answer) even though the SECOND hole (next-state) needed the
pyre3 decline fix. Answer-hole and next-state-hole are asymmetric in the refold.

## pyre6-distinct — DISTINCT-effect nested handle in the answer hole (tick 1860)
Same shape but inner handle is over a fresh effect `F` (op `ping`), reducing to 42.
Removes the same-effect-shadowing variable. PASSES 11462/462 on wasm+rust+rust-async.
With pyre6's referential-transparency control this confirms the answer-hole refold is
correct for BOTH same-effect and distinct-effect nested closed handles.

## pyre7-bothholes — nested handle in BOTH holes (tick 1861): DECLINE-WITNESS
Nested closed handle in the answer hole (=42) AND the next-state hole (=53). The
next-state hole triggers the pyre3 guard, so the whole `resume` DECLINES cleanly
(todo, "declined (compiler can't compile it yet)") uniformly on wasm+rust+rust-async
— even though the answer hole alone (pyre6) compiles fine. Confirms the guard is
driven by the next-state position, not defeated by a well-formed answer hole.
Oracle 99999 is a sentinel (never reached while declined); flips to a real pass only
if/when the correct-FOLD follow-on lands. Decline-witness — NO baseline row.

## Control shape (referential-transparency pin)
Same program with `(resume (: 42 Int64) (* 10 s))` — literal in the answer hole.

## Promotion
pyre6 is promotable as a pass-witness. Held in the pool while batch-340 is queued
(no layering on an unmerged MR). Candidate for batch-341+.
