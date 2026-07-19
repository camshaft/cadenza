;; BUG (2026-07-19, v-effects; PRE-EXISTING, not from the DES/homing landings): a handler arm whose body
;; contains a LAMBDA CAPTURING THE ARM PARAM (`w`), when the handle DECLINES to fold (an opaque/non-
;; performing body it can't reduce) → CDZ0101 "unbound name `w`". The captured arm param is orphaned: the
;; arm's inner lambda is LIFTED as a runtime closure (needing a funcref slot) independently of the fold, and
;; the lift resolves the captured `w` against the wrong scope (the arm-param binding is not threaded to the
;; lifted lambda). BOGUS-DECLINE (not wrong-value), BOTH backends.
;;
;; ISOLATION (trunk 49d948964):
;;  - arm body = a lambda capturing `w`  `(set (w) s (run (fn (_u) w)))` + OPAQUE body `(b unit)` → w unbound. ✗
;;  - arm body = bare `w` (NO inner lambda) `(set (w) s w)` + opaque body → COMPILES. ✓  (so it's the LAMBDA lift)
;;  - same lambda-capturing arm + a body that PERFORMS the op (fold FIRES, →42) → COMPILES. ✓ (only the DECLINE path leaks)
;;  - `resume` is NOT required: `(fn (_u) w)` (no resume) leaks the same as `(fn (_u)(resume w w))`.
;;  - NOT from my homing helper (stubbing param_apply_extra_handled → still fails) NOR the DES fold (fires
;;    only on a performing body). It's the lambda-lift × declined-handler interaction, pre-existing.
;;
;; FIX DIRECTION (next build): when a handle DECLINES (reduce_handle → None, lower.rs ~2140), its arm bodies
;; must NOT have their lambdas lifted as standalone closures with a broken scope — either the whole declined
;; handle Poisons uniformly (no arm-lambda lift), or the arm-param scope is threaded to the lifted lambda.
;; Locate where the arm-body lambda enters db.lifted / lower_lambda_value for a Poisoned handle.
(do (effect A (op set (-> Int64 Int64)))
 (def (run thunk) (thunk unit))
 (def (with-h (: b (-> Unit Int64))) (handle A 0 ((set (w) s (run (fn (_u) w)))) (b unit)))
 (def (main) (with-h (fn (u) 99))) (export main))
