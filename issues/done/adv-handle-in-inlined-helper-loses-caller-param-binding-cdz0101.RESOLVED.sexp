; v-effects finding (2026-07-17). DISTINCT from the Any-leak (Locus 1, FIXED by reparent in reduce_handle's
; thread path, commit ce559d74a). This is the HELPER face (breaker's Locus 2): a handle inside a NON-RECURSIVE
; helper `f(n)`, called with the CALLER'S PARAM as the seed `(f k)`, mis-emits CDZ0101 "unbound name `k`"
; (k IS bound — main's param). rc=1, blocks compile. NOT just a diagnostic-quality issue — the compile fails.
;
; BISECTION (post-Locus-1-fix, verified on this branch):
;   - (def (f (: n Int64)) (handle St n ((get (u) s (resume s s))) (St.get))) (def (main (: k Int64)) (f k))
;       -> CDZ0101 unbound k   (BUG)
;   - same f called with a LITERAL: (def (main) (f 7))                          -> compiles (7)  OK
;   - a NON-handle helper returning its param: (def (f (: n Int64)) n) (f k)    -> compiles       OK (control)
;   - advancing arm OR compound body in f's handle + param arg                 -> STILL CDZ0101 unbound k
; So the trigger is: a HANDLE anywhere in a non-recursive helper's body, inlined at a call site whose ARGUMENT
; is the caller's param. The inline (beta_reduce n:=k) + reduce_handle interaction loses the caller-param
; binding of the seed reference. (Independent of the arm shape — unlike Locus 1, advancing/compound do NOT heal.)
;
; ROOT (hypothesis, verify): the cross-function inline reduces f's body (containing the handle) then folds the
; handle; the folded seed reference `k` (substituted from the caller) lands in a synthesized node whose parent
; chain does not reach main's scope. Same re-parenting CLASS as Locus 1 but at the INLINE site, not the
; reduce_handle thread return. Likely fix: when inlining a helper whose body reduces a handle, re-anchor the
; folded result under the call site (or ensure beta_reduce's substituted seed keeps a resolvable pin). Also:
; the CDZ0101 emitter must never name a bound occurrence with no span (breaker's Locus 2 diagnostic point —
; that half is v-inference's; the unbound-k FATAL is the effects/inline half here).
;
; SEVERITY: reject-not-miscompile (spurious decline of a well-typed program). Not urgent (no consumer forces a
; handle-in-helper-with-param-seed today; the direct form is fixed). Promote when a real program hits it.
; Tracked in v-effects memory index-effects-capabilities.

(do
  (effect St (op get (-> Unit Int64)))
  (def (f (: n Int64)) (handle St n ((get (u) s (resume s s))) (St.get)))
  (def (main (: k Int64)) (f k))
  (export main))
