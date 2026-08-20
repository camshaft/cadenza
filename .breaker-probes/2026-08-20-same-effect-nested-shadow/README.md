# same-effect-nested-shadow — inner handle E shadows outer handle E for the SAME effect

## pysh4 — outer tick (s*100, +1, seed n%3) with an inner tick (s*10, +2, seed 50) in the body
```
(handle E (% n 3) ((tick () s (resume (* s 100) (+ s 1))))
  (+ (E.tick)
     (+ (handle E 50 ((tick () s (resume (* s 10) (+ s 2)))) (+ (E.tick) (E.tick)))
        (E.tick))))
```
Body: outer#1 + (inner#1 + inner#2) + outer#2.
Model: outer#1 = sO0*100 (sO->sO0+1); inner#1 = 500 (sI->52); inner#2 = 520; outer#2 = (sO0+1)*100.
sum = 200*sO0 + 1120 -> n=10 1320, n=0 1120.

## Verdict: PASS-WITNESS (compiles + correct)
Verified 1320 / 1120 on wasm + rust + rust-async + opt-sweep 0-div O0..O3.
The inner handle E correctly SHADOWS the outer only within its own body (the two inner ticks
see seed 50 and thread +2), while the outer ticks before/after see the outer handler and its
+1 thread. Contrast pynd3 (DISTINCT effects, delegation) — here it is ONE effect with two
coexisting handlers and correct lexical shadowing. Distinct from pyth1/pyre* (nested handle in
a resume HOLE); here the nested handle is a plain body subexpression.
