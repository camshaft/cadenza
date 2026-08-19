# cross-effect-answer — foreign dispatch feeding the resume-answer

## pyce1 — E's tick arm answers with a value that dispatches a DISTINCT effect F
```
(handle F 0 ((aux () fs (resume 100 fs)))          ; outer F, tail-resumes 100
  (handle E (% n 3)
    ((tick () s (resume (+ s (F.aux)) (+ s 1))))    ; E answer = state + foreign F draw
    (+ (E.tick) (* 10 (E.tick)))))
```
Each `(F.aux)` is a fresh tail dispatch to the outer F handler (answers 100). E.tick
answer = s + 100, next-state = s + 1. Model: 1121 / 1110.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 1121/1110; compiler PASSES 1121/1110 on wasm+rust+rust-async.
- Referential control (`pyce1-ctrl.sexp`, `(F.aux)` replaced by literal 100): PASSES
  1121/1110 — the foreign draw behaves identically to its constant answer, confirming
  correctness (referential transparency across the effect boundary in the answer hole).

A dispatching FOREIGN effect in the resume-ANSWER position folds correctly — consistent
with the refined class statement (dispatching handle in the answer hole folds); here it
is a bare foreign perform rather than a nested handle, and it likewise folds.

## Promotion
pyce1 promotable as a pass-witness (batch-343 candidate).
