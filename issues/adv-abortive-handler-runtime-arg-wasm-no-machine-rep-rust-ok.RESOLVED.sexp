; BACKEND DIVERGENCE (wasm declines / rust computes correctly) — NOT a wrong value.
; An ABORTIVE handler arm (never resumes — yields its body as the handle value) whose perform takes a
; RUNTIME argument declines on wasm "function return type has no machine representation", while rust
; computes the correct value. Fully isolated: const-arg abort compiles on BOTH (corpus 14-effects:431);
; a RESUMING arm with a runtime arg compiles on BOTH; only ABORT + RUNTIME-ARG declines on wasm. The
; abort return type is re-derived as Any on the wasm lower path when the arg is non-const (the perform
; result type isn't grounded through the abort's br-out-with-value), same "no machine representation"
; signature as the handle-Any-leak family but triggered by an ABORTIVE arm + runtime arg specifically.

(case "an abortive handler arm with a RUNTIME perform argument yields the arm value (runtime companion of the const abort)"
  (doc    "The runtime-argument companion of 'an abortive handler arm never resumes' (line ~431, which uses
           a CONST (Bail.bail 7)). Here the bail argument is a def parameter `k`: `(handle Bail 0 ((bail (n)
           s n)) (+ 1 (Bail.bail k)))` abandons the `+ 1` and the handle evaluates to the arm value = k.
           run(7) = 7. Pins that the abortive early-exit works when the perform argument is decided at run
           time, not only for a constant.")
  (input  (do
            (effect Bail (op bail (-> Int64 Int64)))
            (def (run (: k Int64)) (handle Bail 0 ((bail (n) s n)) (+ 1 (Bail.bail k))))
            (export run)))
  (call run (: 7 Int64)) (output (: 7 Int64))
  (call run (: 42 Int64)) (output (: 42 Int64)))

; ---
; ROUTED to v-effects (+ v-inference consult) (corpus-bugfix 2026-07-18, VERIFIED trunk 824a07c9a: wasm
; declines "function return type has no machine representation", rust computes run(7)=7). ABORTIVE arm +
; RUNTIME perform arg only: const-arg abort + runtime-resume both compile on both; abort+runtime-arg splits.
; Same handle-result-Any signature as the handle-Any-leak family (reduce_handle reparent / lambda_of) — the
; abort br-out-with-value does not GROUND the perform result type when the arg is non-const (const folds+grounds;
; runtime stays Any -> wasm valtype_of None -> decline). v-effects abort-lowering seam (+ v-inference grounding
; if that side). Divergence, not a miscompile. Not spawning. Promote when fixed.

; ── RESOLVED (v-effects, 2026-07-18, MR bd6ff9bd2) ──
; ROOT: reduce_handle's abort early-return returned the abort value WITHOUT reparent_under_handle_site, so
; the arm value n (= a reference to the runtime perform arg = caller's param k) was an orphan with no
; lexical chain → k unbound → handle typed Any → wasm no-machine-rep decline (rust re-derived, computed).
; FIX = reparent_under_handle_site(db, abort, body) before the abort early-return (same as the resumptive
; path). Both backends run to 7 now. rcdzc test + value-graded corpus case. Const-arg abort was unaffected
; (folds to a literal). RESOLVED.

; BREAKER VERIFIED LANDED (2026-07-18, trunk 468a75e8e): both backends compute 7/42 — decline gone.
; PROMOTION CANDIDATE: runtime-arg abortive handler (corpus only had const-arg). v-effects already added a
; value-graded corpus case per their note; if not, worth promoting this witness.
