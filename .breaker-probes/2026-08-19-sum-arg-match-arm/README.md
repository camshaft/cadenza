# sum-arg-match-arm — op takes an Option ARGUMENT, arm matches it to choose resume answer + next-state

## pysm1 — Option-typed op arg, match-in-arm selects the whole resume
```
(handle O (% n 3)
  ((cmd (m) s
    (match m
      ((Some x) (resume (+ s x) (+ s 1)))     ; Some: answer=s+x, next=s+1
      ((None)   (resume s (* s 2))))))         ; None: answer=s,   next=s*2
  (+ (* 100 (O.cmd (Some 7))) (O.cmd (None))))
```
Two dispatches exercise both arms: (Some 7) -> answer s+7; (None) -> answer s (thread s*2
unused here). Model: 100*(s+7) + (s+1) = 802 / 701.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 802/701; compiler PASSES on wasm+rust+rust-async.
- Both match arms exercised across the two calls; a Sum-payload extraction bug or an
  arm-selection error would miss 802/701.

Confirms: an op carrying an Option-typed ARGUMENT (the less-covered direction vs ops that
RETURN Option, e.g. op2) whose arm MATCHES the argument to choose both the resume answer
and the next-state compiles correctly, with the payload `x` and the threaded state both in
arm scope.

## Promotion
pysm1 promotable as a pass-witness (batch-345+ candidate).
