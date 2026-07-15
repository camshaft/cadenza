
;; --- THE EXACT LEVER (investigated 2026-07-15) ---------------------------------------------------
;; `eval::apply_lambda` (eval.rs:777) HARD-DECLINES any recursive body BEFORE reducing:
;;     if is_recursive(db, body) { return Err("a recursive function needs runtime specialization") }
;; `is_recursive` (eval.rs:1041) is a call-graph cycle detector. Separately, the evaluator ALREADY has a
;; `REDUCE_DEPTH_LIMIT` depth guard (eval.rs:~1305/1569) that bounds NON-statically-recursive reduction
;; (denies entry past the limit → yields None, a clean decline, no stack overflow).
;; PROPOSAL for the design call: on the MACRO-EXPANSION path (a tagged-template tag application, and
;; arguably `(eval …)`), RELAX the early `is_recursive` decline and instead let the body reduce UNDER the
;; existing depth-limit backstop. Then a TERMINATING recursive tag fn (bounded input — a real DSL parser
;; over a fixed template string) folds to a constant Ast; a non-terminating one hits the depth cap and
;; declines cleanly (never a hang, never a wrong value). Risk: this is evaluator-CORE; must not regress
;; the ordinary fold path (the is_recursive gate exists to stop exponential body-copy blowup on branching
;; recursion — so relaxing it must stay scoped to the macro path + keep the depth+memo guards). Likely
;; rcdzc-core / v-inference territory to implement; v-metaprogramming owns the tagged-template seam.
