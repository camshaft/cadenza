; mlrepro (v-metaprogramming→v-effects, 2026-07-18, surfaced via v-cad @param, triaged GENERAL/plain-effect).
; A MISCOMPILE (silent — duplicated observable effect): an EFFECTFUL ARGUMENT passed to a function is
; substituted CALL-BY-NAME and RE-PERFORMS at each use of the param in the callee body, instead of being
; evaluated ONCE by-value at the call. VERIFIED: emits THREE host `get` calls, want ONE.
;
;   (def (mk s) (T s s s))                    ; mk is PURE; its param s is used 3×
;   (def (main) (host (E) (sum3 (mk (E.get))))) ; (E.get) is the ARG
; -> the emitted core module has `call 0` (host get) THREE times — E.get re-performs per use of `s`.
; CONTRAST (correct, 1 call): (let ((s (E.get))) (sum3 (T s s s)))  ; let binds once → ONE host call.
;
; SPEC RULING (v-effects, checked spec/capabilities): this is a MISCOMPILE, not a design choice. Cadenza is
; strict — core-semantics.md §Applying A Function binds the parameter to THE ARGUMENT (a single evaluated
; value); §283 "an argument bound to a parameter the function body uses MUST be evaluated" (a value, bound
; once); capabilities-and-effects.md §75 a run's host-call SEQUENCE is deterministic. Re-performing an
; effectful arg per use duplicates an observable effect (3 host gets vs 1) — call-by-name, contradicting
; strict by-value binding. An effectful argument MUST be evaluated ONCE at the call, its value bound to the
; param, and each use reads the bound value.
;
; ROOT (hypothesis): mk is a PURE helper (does not itself perform), so the inline path β-reduces s := (E.get)
; and substitutes into (T s s s), DUPLICATING the perform per use. My perform-arm effect-duplication guard
; (effects.rs, count_param_refs>1 + arg_reaches_any_perform → decline) covers a perform whose OWN arm dups an
; effectful arg, but NOT the cross-fn inline of a PURE helper whose param (bound to an effectful arg) is used
; multiply. FIX DIRECTION: when inlining/β-reducing a call whose ARG reaches a perform and the callee's param
; is used >1×, LET-BIND the arg once at the call site (evaluate-once) instead of substituting by-name — i.e.
; `(mk (E.get))` → `(let ((s (E.get))) (T s s s))` before threading. Task-#15/#11 family (out-state + inline
; scope). Gate: arg-used-once (no let needed) / used-N× / arg-pure (no change) / arg-effectful. Miscompile-
; prone; probe host-call COUNT after. Owner: v-effects. Not blocking (let-idiom is the clean workaround;
; v-cad/@param unblocked). Prioritize after the pending sequenced-memoize fix (c799b1eaa) lands.

(do
  (effect E (op get (-> Unit Int64)))
  (type Trip (T Int64 Int64 Int64))
  (def (mk s) (T s s s))
  (def (sum3 t) (match t ((T a b c) (+ (+ a b) c))))
  (def (main) (host (E) (sum3 (mk (E.get)))))
  (export main))

; ── SCOPE NARROWING (v-effects, 2026-07-18): HOST-DELEGATED effects ONLY ──
; The in-program-HANDLER version of the same shape — (handle E 0 ((get (u) s (resume s (+ s 1)))) (sum3 (mk
; (E.get)))) — returns 0 (evaluate-ONCE: (T 0 0 0) → sum 0), CORRECT. So the effect FOLD already evaluates a
; handled perform once (its arm-substitution + threading bind the resume value once). The re-perform bug is
; specific to a HOST-DELEGATED effect (a Core::HostCall, which does NOT go through the fold — it lowers as a
; real boundary call, and the β-reduce of a pure multi-use helper copies the HostCall node N×). So: memo/DB/
; in-program effect programs are UNAFFECTED; only a HOST op passed as a multi-use fn-arg re-performs. This
; further supports PARK (the affected surface is narrow — host-delegated multi-use fn-arg — and the let-idiom
; is the clean workaround). If fixed, the trigger can be narrowed to a HostCall-reaching arg specifically.

; ── UNPARKED + β-REDUCE HALF FIXED (v-effects, 2026-07-18, MR cbe42eddf; operator directive: no parking) ──
; The call-by-name re-perform is FIXED in the β-reduce funnel (apply_lambda_uncached): an effectful arg to a
; param used >=2x is now LET-BOUND once (evaluate-once), so the SCALAR-continuation shape folds to 1 host
; call (gated: rcdzc test an_effectful_host_arg_to_a_multiuse_scalar_fn_param_evaluates_once + corpus case
; "an effectful host arg to a multi-use function parameter is evaluated ONCE", value 15 / one ask.ask).
; The COMPOUND shape in THIS repro ((T s s s) fed to a destructuring match) STILL emits 3 — the fix EXPOSED
; a SECOND, INDEPENDENT bug: Core::SumPayload RE-LOWERS a host-reaching match scrutinee once per payload
; binder (a,b,c). This is a MATCH-LOWERING scrutinee-reevaluation bug (lower.rs lower_match_sum / backend
; SumPayload emit), NOT β-reduce. FIX = A-normalize a host-reaching match scrutinee (bind once above the
; match so every SumPayload reads a LocalRef). NEXT increment (touches hot match-lowering + backend select).
; The known-miscompile pin test is updated (documents the 2nd bug, still asserts 3).
