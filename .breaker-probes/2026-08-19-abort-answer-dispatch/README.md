# abort-answer-dispatch — an ABORTING arm whose answer dispatches a foreign effect

## pyad1 — abort arm's answer draws a distinct effect F (abort-side sibling of pyce1)
```
(handle F 0 ((aux () fs (resume 100 fs)))
  (handle E (% n 3)
    ((tick () s (resume (+ s 1) (+ s 10)))     ; resuming: threads next-state s+10
     (stop () s (+ (* s 1000) (F.aux))))        ; ABORT (no resume); answer draws F
    (let ((a (E.tick))) (+ a (E.stop)))))
```
E.tick resumes (a = seed+1), threading handler state to seed+10; then E.stop ABORTS,
its answer = 1000*state + F.aux = 1000*(seed+10) + 100. Abort discards the continuation.
Model: 11100 / 10100.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 11100/10100; compiler PASSES 11100/10100 on wasm+rust+rust-async.
- Referential control (`pyad1-ctrl.sexp`, F.aux -> literal 100): PASSES 11100/10100 —
  the foreign draw in the abort answer behaves identically to its constant answer.

Confirms: a dispatching foreign effect in an ABORTING (non-resuming) arm's answer folds
correctly, and the aborted handler state is the threaded next-state from the prior
resuming dispatch (seed+10, not the seed). Complements pyce1 (foreign draw in a RESUMING
arm's answer).

## Promotion
pyad1 promotable as a pass-witness (batch-343 candidate alongside pyce1, pyx2).
