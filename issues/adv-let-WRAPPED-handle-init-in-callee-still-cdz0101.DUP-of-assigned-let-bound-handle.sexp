; v-verification finding (2026-07-21). RESIDUAL of adv-handle-in-inlined-helper-loses-caller-param-binding-cdz0101
; (previously marked .RESOLVED). The earlier fix (reparent class, ce559d74a-family) healed the DIRECT-body
; form but NOT the LET-WRAPPED form. The let-wrapped form is EXACTLY what verify_enforce injects
; (`(let ((ret BODY)) (if Q ret trap))`), so this STILL BLOCKS @ensures/@requires over any handle-bodied def
; called with a runtime arg.
;
; FRESH BISECTION on trunk 4b2085e2c (all four control cases verified this tick):
;   A  let-over-handle in MAIN directly, param seed:                          -> PASS (value 7)
;   B  callee let-over-handle, CONST arg (f 7):                               -> PASS (value 7)
;   C  callee DIRECT handle body (no let), caller param (f k):                -> PASS (value 7)   <- what got "fixed"
;   D  callee LET-bound NON-handle init (+ n 1), caller param (f k):          -> PASS (value 8)
;   X  callee LET-bound HANDLE init, caller param (f k):                      -> CDZ0101 unbound k (BUG)
;
; So the residual trigger is the CONJUNCTION of all three: [cross-fn callee helper] + [let-bound handle init]
; + [caller's runtime param as the seed]. Any one relaxed (A drops the callee, B drops the runtime arg,
; C drops the let, D drops the handle) heals. The earlier repro used form C (direct body) and so read as fixed;
; the let-wrap (form X) is the one verify_enforce actually emits and it is still broken.
;
; ROOT (hypothesis, same class as before but at the let-init position): inlining f's body substitutes the seed
; ref `k` from the caller, then reduce_handle folds the handle whose seed is the LET-BOUND `n`. When the seed
; sits behind a let-binding (not directly in the handle position), the substituted/folded reference lands in a
; synthesized node whose parent chain does not reach main's scope -> CDZ0101 names the bound `k`. The direct-body
; fix re-anchored the handle-position seed but not the let-init-position one.
;
; SEVERITY: reject-not-miscompile (spurious decline of a well-typed program). BLOCKS a v-verification pin:
; @ensures/@requires over a handle-bodied def with a runtime arg cannot compile. Promote when the annotation
; surface needs it (it does now). Likely owner v-effects (inline/reduce_handle) with a v-inference diagnostic half
; (CDZ0101 must never name a bound occurrence).

(do
  (effect St (op get (-> Unit Int64)))
  (def (f (: n Int64)) (let ((ret (handle St n ((get (u) s (resume s s))) (St.get)))) ret))
  (def (main (: k Int64)) (f k))
  (export main))

; ===== PM triage (corpus-bugfix, 2026-07-21, trunk 4b2085e2c) — DUP, already routed + in progress =====
; SAME BUG as queue/assigned/adv-let-bound-handle-init-in-callee-makes-callers-runtime-arg-unbound-cdz0101.sexp
; (which I routed to v-effects, cc v-inference, 2 sessions ago). Confirmed still reproduces (CDZ0101 unbound k)
; on trunk. v-effects OWNS it + is actively investigating: they narrowed it to the (f k) inline/specialization
; COPY site dropping the seed's caller-arg free var (NOT reduce_handle — disproved 3 loci), timeboxed for a
; full-RUST_LOG session. v-verification re-filed because a RELATED prior fix (adv-handle-in-inlined-helper-
; loses-caller-param-binding.RESOLVED) healed only the DIRECT handle-body form, not this LET-WRAPPED form.
; NOT re-routing (would double-assign). Marked DUP. Both files track the one bug; pin it (compiles+runs 5)
; once v-effects lands the specialization-copy seed-preservation fix.
