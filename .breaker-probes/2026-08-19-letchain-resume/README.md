# letchain-resume — arm binds a let-chain; resume answer + next-state reference earlier bindings

## pylc1 — nested let chain feeding both resume holes
```
(tick () s
  (let ((a (* s 2)))
    (let ((b (+ a 3)))          ; b references a
      (resume (+ a b) (+ b 1)))))  ; answer=(a+b), next-state=(b+1), both use let-bindings
```
a=2s, b=2s+3; answer=4s+3, next=2s+4. body (+ E.tick (* 100 E.tick)). Model 2707/1903.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 2707/1903; compiler PASSES on wasm+rust+rust-async.
- Seed-differentiated; both let-bindings (a, and b-derived-from-a) feed the resume answer
  and the threaded next-state — a binder-scope error across the resume seam (stale a/b,
  or dropping the let chain in the two-hole refold) would miss 2707/1903.

Confirms: a nested let chain of intermediates in the arm, with earlier bindings consumed
by BOTH the resume answer and the next-state, compiles correctly — the arm's local
bindings are correctly in scope at both resume holes across the effect seam.

## Promotion
pylc1 promotable as a pass-witness (batch-346+ candidate).
