# inner-delegates-outer — inner handler's arm performs the OUTER effect (cross-effect delegation)

## pynd3 — In.ii delegates to Out.oo, both deep/tail, state threaded independently
```
(handle Out (% n 3) ((oo () s (resume (* s 10) (+ s 1))))
  (handle In 100 ((ii () t (resume (+ t (Out.oo)) (+ t 1))))
    (+ (In.ii) (In.ii))))
```
Each In.ii performs a fresh Out.oo that must route PAST the inner In handler to the outer
Out handler, threading Out's state independently of In's.
Model: In.ii#1 t=100 + Out.oo#1 (s0*10); In.ii#2 t=101 + Out.oo#2 ((s0+1)*10).
sum = 201 + 20*s0 + 10 -> n=10 (s0=1) 231, n=0 (s0=0) 211.

## Verdict: PASS-WITNESS (compiles + correct)
Verified 231 / 211 on wasm + rust + rust-async; opt-sweep 0 divergence O0..O3.
Routing check: the two Out.oo calls see DISTINCT threaded states (s0, s0+1) — if the fold
shadowed or double-counted, the sum would differ. Confirms cross-effect delegation from an
inner deep handler's arm to an outer deep handler threads BOTH states correctly.
Distinct from the same-effect nested-handle bank (pyth1/pyre*) which shadow ONE effect.
