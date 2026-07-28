; FINDING (v-wasm-opt drift-guard, 2026-07-21): a RESIDUAL invalid-module gap adjacent to 14f94219b
; (v-inference's "ground a bare-ConstFloat MATCH-ARM to the match's Float32 result width"). That fix
; covered a match with a RUNTIME arm (self-call/arith); this case — ALL arms bare ConstFloat literals over
; a RUNTIME SCRUTINEE, under a Float32 annotation — still emits an INVALID MODULE (function[0]).
;
;   (: (match n (0 1.5) (_ 0.25)) Float32)   over runtime n   → INVALID MODULE   ← BUG (2-arm)
;   (: (match n (0 1.5) (1 0.75) (_ 0.25)) Float32)           → INVALID MODULE   ← BUG (3-arm too)
;   (: (match n (0 0.5) (_ (f (- n 1)))) Float32)  runtime ARM → COMPILES (14f94219b's shape, fixed)
;
; LIKELY ROOT (v-wasm-opt read): NOT the emit-side arm grounding — emit_arm_body (select.rs:12487) DOES
; ground a bare-ConstFloat arm to f32 when block_ty==F32. The bug is that block_ty is derived from the
; match's RESULT type (type_of(db, match_id)), and with ALL arms bare float literals the match result
; SOLVES TO Float64 (each literal arm defaults to f64, and the match-result unification takes f64 despite the
; outer (: ... Float32) annotation) → block_ty = F64 → arms emit f64.const into a context the annotation/
; outer compare expects at f32 → invalid module. So this is a WIDTH-SOLVE gap (the match result should take
; the Float32 annotation and push it into the literal arms), same family as 14f94219b + the if/record/expect
; width-descent fixes — v-inference's lane, not an emit-side ground (my emit grounds correctly GIVEN the right
; block_ty). rust computes it (width-solve differs there). O0..O3 stable. Verified on trunk f45c7834a.
;
; If it turns out the annotation IS reaching the match result and the emit block_ty is just misread, hand
; back to v-wasm-opt (emit-side) — but the all-literal-vs-runtime-arm split points at the arm-literal
; width-solve.

(case "REPRO 2-arm all-literal float match under Float32 over a runtime scrutinee computes"
  (input  (do (def (main (: n Int64))
                (< (: (match n (0 1.5) (_ 0.25)) Float32) (: 1.0 Float32)))
              (export main)))
  (call   main (: 0 Int64)) (output (: false Bool))
  (call   main (: 7 Int64)) (output (: true Bool)))
