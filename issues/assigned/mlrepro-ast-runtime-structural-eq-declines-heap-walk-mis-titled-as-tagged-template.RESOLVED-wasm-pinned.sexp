
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

;; --- PRECONDITION CHECK RESULT (2026-07-15): the simple relax is UNSAFE -----------------------------
;; enter_reduction (db.rs:2183, the REDUCE_DEPTH_LIMIT/REDUCE_NODE_BUDGET guards) is called at the
;; reduce_to_*/member-value primitive sites (eval.rs:1308,1574,…) but NOT in apply_lambda_uncached
;; (eval.rs:763) or beta_reduce (eval.rs:446). apply_lambda's ONLY recursion protection IS the
;; is_recursive gate (eval.rs:777). beta_reduce copies the body; infer/lower recurse into the copy and
;; re-apply_lambda nested calls with DIFFERENT args (reduce_cache doesn't bound it). So removing the gate
;; → apply_lambda recurses UNGUARDED → native stack overflow + exponential copy blowup. SAFE enablement
;; needs enter_reduction wrapped AROUND apply_lambda's reduction recursion (a real eval-core change,
;; rcdzc-core owner) — NOT just deleting the gate. Reported to corpus-bugfix.

;; TRIAGE 2026-07-16 (corpus-bugfix): evaluator-CORE gap (apply_lambda is_recursive early-decline, no depth guard on the beta path). Owned by v-metaprogramming (tagged-template seam) + rcdzc-core/v-inference (impl). Design-call territory (relax is_recursive on macro path under depth backstop). Parked; deep.

; ===== PM triage (corpus-bugfix, 2026-07-20, trunk 9df1855b2) — RE-DIAGNOSED + RE-ROUTED =====
; STALE TITLE: recursive tagged-template tags DO fold now (v-metaprogramming: 24-tagged-templates 123/146/170
; pass). REAL bug = runtime structural = on an Ast value. VERIFIED: (= (Ast.Int n) (Ast.Int 3)) runtime n ->
; "comparison of a compound value needs a heap walk (not yet built)" all 3 backends. ROOT: Ast admitted by
; neither runtime-= path (ValueEq via Ast.List->Ty::List non-byte-canonical; ValueCmp via Ast.Float->non-
; orderable) — same increment as the PARKED List<Float> =. ROUTED to v-runtime (heap-walk-eq lane); off
; v-metaprogramming (their Ast/quote/eval surface complete). Renamed to reflect the real bug. corpus-bugfix
; to pin an Ast-runtime-= case once v-runtime lands the descriptor-guided value-eq-shaped walk for it.
