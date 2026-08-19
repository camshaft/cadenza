# arg-branch-answer — resume answer that branches on the op argument vs captured state

## pyab1 — resume answer = (if (> v s) (* v 100) (+ v s)), branch flips by seed
```
(handle E (% n 3)
  ((tick (v) s (resume (if (> v s) (* v 100) (+ v s)) (+ s 1))))
  (let ((a (E.tick 1))) (let ((b (E.tick 1))) (+ (* 100 a) b))))
```
Op `tick` carries an Int64 arg `v`; the arm's resume answer branches on `v > s`.
With v=1 the branch FLIPS by seed: only seed 0 has 1 > 0 (scales *100), seeds 1/2 add.
- n=0 (seed 0): tick(1) at s=0 -> 1>0 T -> 100; then s=1 -> 1>1 F -> 2; result 100*100+2 = 10002.
- n=10 (seed 1): s=1 -> F -> 2; s=2 -> F -> 3; result 203.
Model: 203 / 10002.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 203/10002; compiler PASSES on wasm+rust+rust-async.
- Seed-differentiated: the SAME arg (v=1) selects DIFFERENT if-branches depending on the
  captured state, and both branches are exercised across the two calls — a wrong branch
  selection or arg/state confusion would miss 10002.

Confirms: an if-branch over (op-arg vs captured-state) in the resume answer position
compiles correctly, with the arg and the threaded state both correctly in scope in the arm.

## Promotion
pyab1 promotable as a pass-witness (batch-345+ candidate).
