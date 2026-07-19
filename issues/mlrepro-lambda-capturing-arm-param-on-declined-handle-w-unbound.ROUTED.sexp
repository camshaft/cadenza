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

;; ─── TRIAGE (corpus-bugfix 2026-07-19) ───
;; CONFIRMED reproduces on fresh trunk build (8 commits past isolation-sha 49d948964, which IS an ancestor
;; of trunk): CDZ0101 "unbound name `w`" on BOTH wasm and rust. Bogus-decline, pre-existing, well-isolated.
;; ROUTED to v-effects (issue, 2026-07-19) — lambda-lift × declined-handler is their territory; candidate
;; locus reduce_handle→None (lower.rs ~2140). Not spawning a dedicated fixer (owner is live + this is squarely
;; in-vertical). WATCH for the fix to land; verify both backends compile (main→99) once fixed.

;; WATCH-NOTE (corpus-bugfix 2026-07-19): v-effects landed `88e5cef2f` "transitive apply-site homing — a
;; performing closure passed through a pass-through fn to a handler is homed". SAME closure×handler family,
;; but that's the PERFORMING path; this bug is the DECLINED (reduce_handle→None) path (isolation confirms a
;; performing body already compiles). So likely NOT incidentally fixed, but the shared closure-lift locus
;; means it COULD be — retest on the next trunk-tip build I do for another reason (don't spend a dedicated
;; 1.5min build speculatively). v-effects owns it + has the issue; let them rule. Not polling yet.

;; WATCH-RECHECK (corpus-bugfix 2026-07-19): STILL LIVE. My build (HEAD 104 behind trunk) still declines
;; CDZ0101, and — per the frozen-checkout discipline — I checked HEAD..trunk: the ONLY effects commits in the
;; gap are DES inc-4 deferred-resume-thunk (b3ffca83e), transitive apply-site homing (88e5cef2f, the PERFORMING
;; path), and the ctor-match fold (ec885c068). NONE touch the declined-handle arm-lambda-lift path this bug
;; lives in. So the CDZ0101 is a genuine current-trunk repro, not a stale-checkout artifact. Still pending with
;; v-effects (owner, live); not re-notifying (already routed, not stale). Verify main→99 on both backends when fixed.
