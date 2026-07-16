; BREAKER FINDING — WASM-backend: a literal-PAYLOAD refinement arm on a SINGLE-VARIANT sum, combined with a
; binding arm, emits an INVALID wasm component (decline-don't-miscompile violation, worse than a clean decline).
;
; SYMPTOM: `cdz-run: invalid component: failed to compile: wasm[0]::function[2]`. wasm-tools validate:
;   func 2 failed: type mismatch: expected i32, found i64 (at offset 0x11b)
; The emitted body (wasm-tools print) for `(match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))`:
;   local.get 0        ;; payload i64
;   call 0             ;; a heap helper (get-int)
;   i64.eqz            ;; <- the literal-0 equality check; i64.eqz vs the i32 the call produced -> mismatch
;   if (result i64) (i64.const 100) else (local.get 0)
; The single-variant sum's payload-literal-equality check emits a type-inconsistent discriminant/unwrap +
; i64.eqz sequence.
;
; MINIMIZED (the bug is SINGLE-VARIANT + literal-payload-refinement + a binding arm — all four probed):
;   single-variant (Wrap Int64), binding-only  (match … ((W.Wrap x) x))                 -> WORKS (m1)
;   single-variant + literal-refine + binding   (match … ((W.Wrap 0) 100) ((W.Wrap x) x)) -> INVALID (m2, this bug)
;   MULTI-variant  + literal-refine + binding    (match … ((W.A 0) 100) ((W.A x) x) ((W.B y) y)) -> WORKS (m3)
;   bare Int64 literal-refine + binding          (match n (0 100) (x x))                  -> WORKS (m4)
; So single-variant sums, literal refinements, and binding arms each work; only the SINGLE-VARIANT +
; literal-payload-refinement COMBINATION emits the invalid component. (On the rust backend the single-variant
; path DECLINES — a known rust gap — so this is a wasm-emit bug, not a differential wrong-value.)
;
; SUGGESTED FIX (v-patterns / wasm-match-emit owner): the single-variant-sum literal-payload equality check
; emits `i64.eqz` (or an i64 compare) against a value the payload-unwrap produced at a different width — align
; the compared operand's width with the payload type (an Int64 payload → i64 compare; the `call 0` unwrap
; result width must match). The multi-variant path (m3) does this correctly; the single-variant path skips or
; mis-widths the discriminant so the literal-compare lands on the wrong type. VERIFY emit locus via WAT.
;
; The cases below assert the CORRECT results (n=5 → 5 via the binding arm; n=0 → 100 via the literal arm).
; They FAIL on wasm today (invalid component); the control m1/m3/m4 shapes pass. Flip to pass when fixed.

(case "adv single-variant-refine: a literal-payload arm + binding arm on a single-variant sum (miss the literal)"
  (doc "`(match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))` with n=5: the payload 5 misses the `(W.Wrap 0)`
        literal arm and binds via `(W.Wrap x)` → 5. On wasm this emits an INVALID component (i64.eqz vs i32
        type mismatch in the single-variant payload-literal check); should return 5.")
  (input (do (type W (Wrap Int64)) (def (main (: n Int64)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))

(case "adv single-variant-refine: the literal arm is selected when the payload matches"
  (doc "The literal-hit companion: n=0 matches `(W.Wrap 0)` → 100. Same invalid-component emit today; should
        return 100. Together with the miss case, pins that BOTH arms of the single-variant literal refinement
        must compile — the whole match is invalid, not just one arm.")
  (input (do (type W (Wrap Int64)) (def (main (: n Int64)) (match (W.Wrap n) ((W.Wrap 0) 100) ((W.Wrap x) x))) (export main)))
  (call main (: 0 Int64))
  (output (: 100 Int64)))

(case "adv single-variant-refine: a MULTI-variant sum with the same literal refinement works (the control)"
  (doc "The control that PASSES: the SAME literal-payload refinement on a MULTI-variant sum `(A Int64) (B
        Int64)` compiles and runs — `(match (W.A n) ((W.A 0) 100) ((W.A x) x) ((W.B y) y))` with n=5 → 5.
        Pins that the bug is the SINGLE-variant case specifically; the multi-variant emit does the payload-
        literal compare at the right width.")
  (input (do (type W (A Int64) (B Int64)) (def (main (: n Int64)) (match (W.A n) ((W.A 0) 100) ((W.A x) x) ((W.B y) y))) (export main)))
  (call main (: 5 Int64))
  (output (: 5 Int64)))
