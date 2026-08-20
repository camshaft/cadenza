# result-answer — a Result-typed resume answer with a parity-selected payload

## pyres1 — Ok/Err answer crossing the boundary per dispatch
```
(step () s (resume (if (= (% s 2) 0) (Ok (* s 10)) (Err (+ s 100))) (+ s 1)))
```
Op result type is `(Result Int64 Int64)`. Even state -> Ok(s*10), odd state -> Err(s+100);
the body negates Err. Two dispatches, state threads 0,1 (n=0) or 1,2 (n=10).

## Verdict: PASS-WITNESS (compiles + correct, promoted batch-358)
Model: n=10 (s0=1): d1 s=1 odd -> Err(101) -> -101, *1000; d2 s=2 even -> Ok(20) -> 20;
sum = -100980. n=0 (s0=0): d1 even -> Ok(0) -> 0; d2 s=1 odd -> Err(101) -> -101; sum = -101.
Verified value -100980 / -101 on wasm + rust + rust-async (fresh worktree-local cdz).
Distinct axis from pych2 (Option Char) and pyfl1 (Float64): a two-arm sum type whose
PAYLOAD varies per dispatch and whose ARM (Ok vs Err) flips across the state thread.

## NOTE on the abandoned pyrc1 (resume-returns-a-closure)
An adjacent probe (op resumes a first-class closure capturing handler state) is NOT
expressible on the surface: an op result type `(-> (-> Int64 Int64))` auto-curries to a
1-arg op, and tuple-wrapping the closure hits `(: ...)` annotation parsing. Not a
soundness gap — surface curried-normal-form. Abandoned as ill-formed.
