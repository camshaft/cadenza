# foreign-nextstate — bare foreign perform in a two-hole arm's next-state (folds; decline is handle-specific)

## pyfn1 — bare foreign draw in the TWO-HOLE next-state hole (not a nested handle)
```
(handle F 0 ((aux () fs (resume 40 fs)))
  (handle E (% n 3)
    ((tick () s (+ (resume (+ s 1) (F.aux)) (* 1000 s))))   ; next-state = bare foreign perform = 40
    (+ (E.tick) (* 10 (E.tick)))))
```
Model: 41412 / 40411.

## Verdict: PASS-WITNESS (folds)
- Model 41412/40411; compiler PASSES on wasm+rust+rust-async.
- Referential control (`pyfn1-ctrl.sexp`, F.aux -> literal 40): PASSES 41412/40411.

Confirms the two-hole NEXT-STATE decline (pyre3) is HANDLE-specific: a bare foreign
perform in the same next-state position FOLDS. Mirrors pyft1 (bare foreign in the
two-hole TOLL folds). So the two-hole value-position decline set is precisely
"a nested closed handle whose body dispatches", not any effect activity in the hole.

## Promotion
pyfn1 promotable as a pass-witness (batch-345+ candidate).
