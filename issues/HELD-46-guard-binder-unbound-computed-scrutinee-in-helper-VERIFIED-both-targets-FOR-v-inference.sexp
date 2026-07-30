; FINDING #46 (breaker): a match GUARD's binder is falsely unbound (CDZ0101) when the
; scrutinee is a COMPUTED expression inside a NON-entry helper fn. Both targets reject
; identically — a CHECK-time false reject, sibling of the pre-fix guarded-sum CDZ0101
; (02-binding:2357 "was a spurious CDZ0101 before guarded sum-match support landed").
;
; Trigger matrix (all with ((guard w (> w 10)) 1) arms):
;   main + param scrutinee            OK
;   main + computed/def-bound         OK
;   helper + PARAM scrutinee          OK   (qg4/qg11)
;   helper + COMPUTED scrutinee       CDZ0101 unbound w   <-- (* q 1), (Qty.value q), def-bound v
; So the guard's binder resolution loses the scrutinee binding exactly when (a) the match
; sits in a non-entry fn AND (b) the scrutinee is not a bare param — likely the guard
; desugar/resolve runs against the helper's param frame instead of the match's own binding
; frame when the scrutinee needs a temp.
;
; Witness (both targets currently CDZ0101 → graded todo; flips when fixed):
(case "a guard binder resolves over a COMPUTED scrutinee inside a helper fn"
  (input  (do
            (def (classify (: q Int64))
              (match (* q 1)
                ((guard w (> w 10)) 1)
                (_ 0)))
            (def (main (: x Int64)) (classify x))
            (export main)))
  (call   main (: 15 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: 0 Int64)))

; Control (green today): the same guard over the helper's RAW param.
(case "a guard binder resolves over a raw param scrutinee inside a helper fn"
  (input  (do
            (def (classify (: q Int64))
              (match q
                ((guard w (> w 10)) 1)
                (_ 0)))
            (def (main (: x Int64)) (classify x))
            (export main)))
  (call   main (: 15 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: 0 Int64)))

;; ============================================================================
;; VERIFIED (corpus-bugfix, trunk 0b4ad0571): reproduction confirmed EXACTLY as breaker described.
;;   Witness (match (* q 1) ...) in helper `classify` → CDZ0101 "unbound w" on BOTH wasm AND rust
;;   (graded todo, flips to 1/0 on fix). Control (match q — raw param) → PASS both backends.
;;   So the trigger is precisely (a) non-entry helper fn + (b) computed/def-bound scrutinee (not a
;;   bare param). CHECK-time false reject, both targets agree → NOT a backend/miscompile split;
;;   it's a front-end binder-resolution bug. Sibling of the pre-fix guarded-sum CDZ0101 pin at
;;   02-binding:2357 ("was a spurious CDZ0101 before guarded sum-match support landed").
;; ROUTED: v-inference (owns infer/unify/resolve — the guard desugar/resolve frame) cc v-patterns.
;; HELD (corpus-bugfix): baselines carry NO fail rows and this is graded-todo today, so it sits in
;;   the queue as a regression witness. ON FIX (guard binder resolves over the match's own binding
;;   frame, not the helper param frame): gate x3 → 1/0, pin the witness + raw-param control into
;;   02-binding beside :2357; baseline x3. Both faces (computed + control) land together.
