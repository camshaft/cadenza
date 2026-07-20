; MINIMAL REPRO (v-effects, 2026-07-20): a boxed 2-param closure applied at PARTIAL arity where the
; surviving intermediate closure is LET-BOUND and then applied → INVALID WASM (function fails to compile).
; SURFACED while reviewing v-memory-safety's SITE-A closure-cell design (probing the surviving-partial escape).
;
; BISECTION (identical except adjacency of the two applies):
;   DIRECT   ((f 3) 4)               → WORKS → 7   (run spine-flattens to ONE CallClosure{args:[3,4]})
;   LET-BOUND (let ((g (f 3))) (g 4)) → INVALID WASM  ← THIS FILE (the let breaks the spine-flatten;
;                                        the intermediate 1-param closure (f 3) is emitted as a surviving
;                                        partial the flattened-call path doesn't produce, so the later
;                                        (g 4) call_indirect references a functype the value doesn't implement
;                                        — the SAME class as a_boxed_nested_unary_curried_closure… before its
;                                        flatten fix, but here the flatten CAN'T fire because the applies are
;                                        non-adjacent across a let.)
; compile SUCCEEDS (writes .wasm) then `cdz run` → "invalid component: failed to compile: wasm[0]::function[5]".
; This is a MISCOMPILE (invalid module emitted), NOT a clean decline — per operator "no parking known
; miscompiles". Severity: MODERATE — narrow shape (a let-bound surviving partial re-applied); the common
; full-arity curried apply works. Owner: v-effects closure lift/lowering (partial-arity lift must produce a
; genuinely-applicable surviving 1-param closure when the spine can't flatten), possibly w/ v-inference on the
; residual arrow type. FIX DIRECTION: either (a) emit a real chained lift for a non-flattenable partial (the
; surviving (f 3) is a proper (env,i64)->i64 closure the later (g 4) call_indirect can dispatch), or (b)
; DECLINE cleanly ("a let-bound surviving partial application is not yet emittable") instead of an invalid
; module. (b) is the safe stopgap; (a) is the real capability.
(module m
  (type Box (C (-> Int64 (-> Int64 Int64))))
  (def (mk) (Box.C (fn ((: a Int64)) (fn ((: b Int64)) (+ a b)))))
  (def (main) (let ((p (mk))) (match p ((Box.C f) (let ((g (f 3))) (g 4))))))
  (export main))
; EXPECTED (either): 7 (a valid module that computes 3+4), OR a clean decline. NOT an invalid module.

; SHARPENED DIAGNOSIS (v-effects, 2026-07-20 investigation):
; - `wasm-tools validate` → "func 5 failed to validate: type mismatch: expected i64, found i32 (at offset 0x200)".
;   A SCALAR-WIDTH mismatch (i32 closure-handle vs i64), the signature of a partial-closure value's rep
;   disagreeing with its use site.
; - The applies do NOT lower to `Core::CallClosure` (a VEFF_CC trace on the CallClosure emit arm fired ZERO
;   times for this repro). So the invalid emit is NOT in the CallClosure decline path (which already declines
;   cleanly on a functype miss) — it's a DIFFERENT lowering path for the let-bound partial `(f 3)` (likely the
;   partial-application lift / residual-arrow lowering treats the surviving 1-param closure as a scalar i32
;   handle but the `(g 4)` use expects the i64 arrow-result shape, or vice versa).
; So the clean-decline stopgap (option b) must detect this shape at the PARTIAL-LOWERING site, not at
; CallClosure — trace where `(f 3)` (a 1-arg apply of a 2-param closure bound to a let) lowers. NEEDS a focused
; fresh trace of the partial-application lowering (lower.rs) — this is where the i32/i64 rep is chosen wrong.
