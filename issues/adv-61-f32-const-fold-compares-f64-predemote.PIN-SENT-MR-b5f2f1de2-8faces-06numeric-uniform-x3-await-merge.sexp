; adv-61 (breaker, 2026-08-02, MED-HIGH wrong-value, BOTH backends, const-vs-runtime divergence):
; a Float32 equality CONST-FOLDS at f64 (pre-demote) precision, disagreeing with the runtime f32
; comparison on the SAME expression.
;
; canonical pair (identical arithmetic, only param-vs-const differs):
;   runtime: (= (+ (: 0.1 Float32) (* x (: 0.2 Float32))) (: 0.3 Float32)) at x=1.0 -> 1  CORRECT
;   const:   (= (+ (: 0.1 Float32) (: 0.2 Float32)) (: 0.3 Float32))              -> 0  WRONG
; hand-check (binary32): 0.1f32+0.2f32 rounds to 0x3E99999A == 0.3f32 exactly -> equal (1).
; the fold evaluates the sum / comparison at f64: 0.1f64+0.2f64 = 0.30000000447.. != 0.3f64 -> 0.
;
; sharpest face — TWO LITERALS, no arithmetic at all:
;   (= (: 0.30000001192092896 Float32) (: 0.3 Float32)) -> 0, though BOTH demote to 0x3E99999A
;   (and both RENDER as "0.30000001192092896"); the fold compares the un-demoted f64 payloads.
;   Runtime-branch variant ((if c litA litB) shape, corpus 06-numeric:886's own idiom) also 0.
;
; scope: BOTH backends return the same wrong fold answer (shared lower/const-eval tier, not emit);
; opt-sweep O0..O3 identical per face (fold is level-independent); the runtime face is correct on
; wasm AND rust. The corpus's existing Float32 pins compare a literal against Float32.of of the
; SAME literal, so the f64-compare bug is invisible to them (equal either way).
; expected per numeric-model: a (: lit Float32) IS the binary32 value; a Float32 op/compare is at
; f32. A fold must DEMOTE to f32 before comparing (and fold f32 arith by rounding each step to f32).
(case "adv-61 a Float32 equality const-folds at f32 (demoted) precision, agreeing with the runtime answer"
  (input  (do
            (def (main)
              (if (= (+ (: 0.1 Float32) (: 0.2 Float32)) (: 0.3 Float32)) 1 0))
            (export main)))
  (call   main) (output (: 1 Int64)))

; --- EXTRA FACES for the on-land pin set (validated by corpus-bugfix on trunk b2578a318) ---
; sharpest (no arithmetic): two literals demoting to the same f32 bits must compare equal.
(case "adv-61 two Float32 literals that demote to the same bits compare equal (const fold)"
  (input (do (def (main) (if (= (: 0.30000001192092896 Float32) (: 0.3 Float32)) 1 0)) (export main)))
  (call main) (output (: 1 Int64)))
; runtime control (breaker's f1) — PASSES today on wasm+rust; pin it to guard the fix from over-rotating
; the (correct) runtime f32 path.
(case "adv-61 runtime control the same f32 sum compares equal at f32 hardware precision"
  (input (do (def (main (: x Float32)) (if (= (+ (: 0.1 Float32) (* x (: 0.2 Float32))) (: 0.3 Float32)) 1 0)) (export main)))
  (call main (: 1.0 Float32)) (output (: 1 Int64)))

; --- SIBLING-OP FACES (breaker addendum + correction, validated by corpus-bugfix on trunk 4102a18ab) ---
; The f64-pre-demote bug is OP-INCONSISTENT: `<`, `>`, and `-` all fold WRONG; `*` folds correctly
; (genuinely demotes). CRITICAL — the operand order must DISCRIMINATE f64 from f32: pick the order
; where the f64 and f32 answers DIFFER, else the case looks correct by accident (my first `>` test
; used the non-discriminating order `(> 0.3 bigger)` = false at both precisions, missing the bug;
; breaker corrected it — `(> bigger 0.3)` folds 1, WRONG at f32 where both demote to 0x3E99999A).
; So v-core-opt must normalize ALL Float32 fold ops (= < > <= >= + - * /), not just = and +. Pin the
; three CONFIRMED-WRONG discriminating sibling faces on fix land (each flips todo→pass); `*` needs no pin.
(case "adv-61 ordering < of two Float32 literals that demote to the same bits is false (const fold)"
  ; discriminating: f64 says 0.3 < 0.30000001192092896 = TRUE(1); at f32 both = 0x3E99999A so false(0)
  (input (do (def (main) (if (< (: 0.3 Float32) (: 0.30000001192092896 Float32)) 1 0)) (export main)))
  (call main) (output (: 0 Int64)))
(case "adv-61 ordering > of two Float32 literals that demote to the same bits is false (const fold)"
  ; discriminating order (bigger payload LEFT): f64 says bigger > 0.3 = TRUE(1); at f32 both equal so false(0)
  (input (do (def (main) (if (> (: 0.30000001192092896 Float32) (: 0.3 Float32)) 1 0)) (export main)))
  (call main) (output (: 0 Int64)))
(case "adv-61 Float32 subtraction const-folds at f32 (0.4f32 - 0.1f32 = 0.3f32)"
  ; discriminating: f64 0.4-0.1 = 0.30000000000000004 != 0.3 → false(0); at f32 rounds to 0x3E99999A == 0.3f32 → true(1)
  (input (do (def (main) (if (= (- (: 0.4 Float32) (: 0.1 Float32)) (: 0.3 Float32)) 1 0)) (export main)))
  (call main) (output (: 1 Int64)))
(case "adv-61 Float32 division const-folds at f32 (0.3f32 / 3.0f32 = 0.1f32)"
  ; discriminating: f64 0.3/3.0 = 0.10000000397.. != 0.1f32 → false(0); at f32 0.3f32/3.0f32 rounds to 0.1f32 → true(1)
  (input (do (def (main) (if (= (/ (: 0.3 Float32) (: 3.0 Float32)) (: 0.1 Float32)) 1 0)) (export main)))
  (call main) (output (: 1 Int64)))
(case "adv-61 ordering <= of two same-f32-bits Float32 literals is true (const fold)"
  ; discriminating: f64 bigger <= 0.3 = false(0); at f32 both = 0x3E99999A so <= is true(1)
  (input (do (def (main) (if (<= (: 0.30000001192092896 Float32) (: 0.3 Float32)) 1 0)) (export main)))
  (call main) (output (: 1 Int64)))
(case "adv-61 ordering >= of two same-f32-bits Float32 literals is true (const fold)"
  ; discriminating: f64 0.3 >= bigger = false(0); at f32 both = 0x3E99999A so >= is true(1)
  (input (do (def (main) (if (>= (: 0.3 Float32) (: 0.30000001192092896 Float32)) 1 0)) (export main)))
  (call main) (output (: 1 Int64)))
