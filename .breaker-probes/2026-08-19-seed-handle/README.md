# seed-handle — nested closed handle in the outer handle's SEED position

## Motivation
The pyre3 fix / deferred correct-fold concern nested closed handles in `resume`'s
holes (next-state, answer). A distinct position: a nested closed handle in the OUTER
handle's **initial-state seed**. Does the two-hole refold of the outer handle survive
a closed handle in its seed?

## pyse1 — nested handle in the seed
```
(handle E (+ (handle E 40 ((tick () t (resume t (+ t 1)))) (+ (E.tick) 2))  ; seed inner = 42
             (% n 3))
  ((tick () s (+ (resume (+ s 1) (* 10 s)) (* 100 s))))   ; non-tail (two-hole) arm
  (+ (E.tick) (* 10 (E.tick))))
```

## Verdict: SAFE OVER-DECLINE (coverage gap, NOT a miscompile)
- Hand model (python): main(10)=51654, main(0)=50453.
- **Ref-transparency control** (`pyse1-ctrl.sexp`, literal `42` in seed): PASSES
  51654/50453 — proves the non-tail arm itself folds fine; my model is correct.
- pyse1 (nested handle in seed): DECLINES uniformly wasm+rust+rust-async with
  `cdz: error: this handler is not yet reducible by the tail-resumptive fold
   (cross-function or non-tail resume arrives in a later increment)`.
- So the nested handle in the SEED tips the OUTER two-hole refold into a conservative
  decline, even though (a) the seed is a closed pure subexpr == 42 and (b) the arm
  folds with a literal seed. Decline is SAFE (reject, not wrong answer).

## Relationship to the deferred correct-FOLD
This is another FACE of v-effects' deferred correct-fold work: reduce the closed
nested handle to its value and thread it. Same fix that flips pyre3/4/5 (next-state
position) should flip pyse1 (seed position) to the 51654/50453 pass. Flagged to
v-effects as the seed-position sibling. Decline-witness — NO baseline row; oracle
left at the ruled-correct 51654/50453 so it auto-flips on the correct-fold land.
