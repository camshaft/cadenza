# tail-nextstate-handle — dispatching nested handle in a TAIL arm's next-state (folds, unlike two-hole)

## pytn1 — tail-resumptive arm, dispatching nested handle in the NEXT-STATE hole
```
(handle E (% n 3)
  ((tick () s
    (resume (+ s 1)                                     ; TAIL arm: resume is the whole body
            (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2)))))  ; next-state = dispatching handle = 42
  (+ (E.tick) (* 10 (E.tick))))
```
Model: tick1 s=seed ans=seed+1 next=42; tick2 s=42 ans=43; body = (seed+1)+10*43 = seed+431.
→ 432 / 431.

## Verdict: PASS-WITNESS (folds correctly — ASYMMETRY vs pyre3)
- Model 432/431; compiler PASSES on wasm+rust+rust-async.
- Referential control (`pytn1-ctrl.sexp`, inner handle -> literal 42): PASSES 432/431.

KEY ASYMMETRY: a dispatching nested handle in the next-state hole of a TAIL-resumptive
arm FOLDS correctly, whereas the SAME construct in a TWO-HOLE (non-tail) arm's next-state
is exactly pyre3 — which silently miscompiled and now DECLINES (fix 348bd4805). The tail
path threads the next-state via the distinct thread lowering (not the two-hole refold),
and that path reduces the closed dispatching handle to its value correctly. This matches
v-effects' tail-vs-two-hole distinction (their pyre7 pins the tail answer-hole; this pins
the tail next-state hole).

## Promotion
pytn1 promotable as a pass-witness (batch-345+ candidate). Distinct from v-effects' pyre7
(tail ANSWER hole) — this is the tail NEXT-STATE hole.
