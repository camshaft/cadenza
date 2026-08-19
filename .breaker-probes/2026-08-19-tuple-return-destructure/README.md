# tuple-return-destructure — op returns a Tuple resume value; body destructures each result

## pytd1 — tuple-returning op, body matches the result per dispatch
```
(handle E (% n 3)
  ((split () s (resume (tuple (+ s 5) (* s 10)) (+ s 1))))   ; resume answer = a 2-tuple
  (let ((p (match (E.split) ((tuple a b) (+ a b)))))          ; destructure 1st dispatch
    (match (E.split) ((tuple c d) (+ (* 1000 p) (+ c d))))))  ; destructure 2nd (state threaded)
```
split resumes a (Tuple Int64 Int64) = (s+5, 10s); the body destructures each dispatch's
result. Two dispatches thread state (s -> s+1) so the tuple fields differ per call.
Model: p = (s+5)+10s = 11s+5; 2nd at s+1 → 11(s+1)+5; result 1000*p + that. → 16027 / 5016.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 16027/5016; compiler PASSES on wasm+rust+rust-async.
- Seed-differentiated, both fields of both tuples contribute — a tuple-field swap, a
  wrong resume-value shape, or a stale-state destructure would miss 16027/5016.

Confirms: an op RESUMING a tuple value, destructured by the body via match across
multiple state-threaded dispatches, compiles correctly (tuple construction in the resume
answer + tuple pattern extraction in the continuation, both across the effect seam).

## Promotion
pytd1 promotable as a pass-witness (batch-345+ candidate).
