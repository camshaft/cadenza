# answer-transforming-inner — answer-hole nested handle with a state-transforming inner handler

## pyat1 — dispatching nested handle in the answer hole, inner handler TRANSFORMS state
```
(handle E (% n 3)
  ((tick () s
    (+ (resume (handle E 5 ((tick () t (resume (* t 2) (+ t 1))))   ; inner DOUBLES state
                 (+ (E.tick) (E.tick)))                              ; two inner dispatches -> 10+12 = 22
               (* 10 s))
       (* 1000 s))))
  (+ (E.tick) (* 10 (E.tick))))
```
Distinct from pyre6 (which used a +1 inner handler folding to 42): here the inner handler
doubles its state across two dispatches, folding to 22. Model: 11242 / 242.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 11242/242; compiler PASSES on wasm+rust+rust-async.
- Referential control (`pyat1-ctrl.sexp`, inner handle -> literal 22): PASSES 11242/242.

Confirms: a dispatching nested handle with a NON-trivial (state-transforming, multi-dispatch)
inner handler folds correctly in the ANSWER hole, on the narrowed-guard trunk (eacbabbf7).
Strengthens pyre6 — the answer-hole fold reduces the inner handle to its value regardless
of the inner handler's internal complexity.

## Promotion
pyat1 promotable as a pass-witness (batch-344+ candidate).

## Side note (pyep1, NOT banked as a fold probe)
An inner handle catching effect G whose BODY performs the OUTER effect E in the toll is
rejected up front with CDZ0401 ("effect operation reached with neither an enclosing
handler nor host delegation") — a well-formedness/scoping reject, not a fold-lowering
issue. Not a finding; the escaping-E-from-inner-G-body shape simply isn't well-typed here.
