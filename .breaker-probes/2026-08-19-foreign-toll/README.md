# foreign-toll — a BARE foreign perform in the post-resume toll position (boundary pin vs pyth1)

## pyft1 — bare foreign draw in the toll (NOT a nested handle)
```
(handle F 0 ((aux () fs (resume 100 fs)))
  (handle E (% n 3)
    ((tick () s (+ (resume (+ s 1) (* 10 s)) (F.aux))))   ; toll = bare foreign perform = 100
    (+ (E.tick) (* 10 (E.tick)))))
```
Model: 312 / 211 (each frame's toll adds a fresh F.aux = 100).

## Verdict: PASS-WITNESS (correctly compiled)
- Model 312/211; compiler PASSES 312/211 on wasm+rust+rust-async.
- Referential control (`pyft1-ctrl.sexp`, F.aux -> literal 100): PASSES 312/211.

## Boundary pin vs pyth1
This is the critical scope check for the pyth1 toll fix (7bc8916f9): a bare foreign
PERFORM in the post-resume toll FOLDS correctly, whereas a nested HANDLE in the same
position DECLINES (pyth1). Confirms the fix is correctly scoped to nested handles whose
body dispatches — it does NOT over-decline a bare foreign perform in the toll. The
distinction is handle-installation-in-the-toll, not effect-activity-in-the-toll.

## Promotion
pyft1 promotable as a pass-witness (batch-343+ candidate).
