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
