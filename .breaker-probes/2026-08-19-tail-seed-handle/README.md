# tail-seed-handle — dispatching nested handle in a TAIL arm's outer SEED (folds, unlike two-hole pyse1)

## pyts1 — tail-resumptive arm, dispatching nested handle in the outer handle SEED
```
(handle E (+ (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2))   ; seed inner = 42
             (% n 3))
  ((tick () s (resume (+ s 1) (+ s 10))))    ; TAIL arm
  (+ (E.tick) (* 10 (E.tick))))
```
seed = 42 + n%3. Model: (seed+1) + 10*(seed+11) → 584 / 573.

## Verdict: PASS-WITNESS (folds — completes the tail-vs-two-hole matrix)
- Model 584/573; compiler PASSES on wasm+rust+rust-async.
- Referential control (`pyts1-ctrl.sexp`, seed inner -> literal 42): PASSES 584/573.

## The completed matrix (dispatching closed nested handle by position × arm shape)
|            | answer hole | next-state hole | seed        |
|------------|-------------|-----------------|-------------|
| two-hole   | FOLDS pyre6 | DECLINES pyre3  | DECLINES pyse1 |
| tail       | FOLDS pyre7 | FOLDS pytn1     | FOLDS pyts1    |
Tail arms fold a dispatching nested handle in ALL THREE value positions (via the thread
path); the two-hole refold declines next-state + seed (+ toll pyth1). The durable
closed-handle correct-fold is therefore TWO-HOLE-refold-specific.

## Promotion
pyts1 promotable as a pass-witness (batch-345+ candidate).
