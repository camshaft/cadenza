;; NARROWED (v-effects, ACTIVE): needs >=2 PERFORMS (a single (Sim.step) compiles; the 3-perform body
;; + seed (tuple 0 k) triggers it). It's BOTH a MIS-FOLD (the fold reaches a final return wrapped =
;; (let ((#seed (tuple 0 k))) (+ (. #seed 0) 0)) — drops the performs to a single seed-slot read) AND a
;; scope-loss (reparent sees the handle as parent yet k still rejects) in the conditional-abort fold path.

;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-effects fixes abortive-arm scope re-home.
;; Origin: breaker FINDING #39 (issue 000000017766). CONFIRMED trunk 6225686a8: an ABORTIVE handler
;; arm (a path returning without resume) unbinds the enclosing fn's PARAM for the whole handle —
;; (handle Sim (tuple 0 k) ((step (u) st (if (>= (. st 0) (. st 1)) -999 (resume ...)))) ...) → CDZ0101
;; 'unbound name k' at the SEED (tuple 0 k) (3:26). The all-resume twin computes (3). PERIMETER
;; (breaker): a CONST-limit conditional-abort WORKS (-999) — the resume-vs-abort CHOICE is fine; the
;; false-unbound needs the arm/seed to reference the ENCLOSING FN PARAM. So abortive-arm lowering
;; re-homes the handle and drops the enclosing param from scope. #37/#38-family, distinct trigger.
;; Blocks deadline/timeout sims. F2 secondary (recursive body): CDZ0201 leaks the mangled internal
;; name 'loop#eff2 has no body' — diagnostic leak, should be honest not-yet. OWNER: v-effects
;; (abortive lowering; may share scope-recovery with v-inference 053ee453f/6566bff81). Oracle -999/3.
;; ON FIX: gate x3 → -999/3; pin into 14-effects beside the handler pins; baseline x3; MR.

(case "an abortive arm reads the enclosing fn's param through the handler seed"
  (input  (do
        (effect Sim (op step (-> Unit Int64)))
        (def (main (: k Int64))
          (handle Sim (tuple 0 k)
            ((step (u) st (if (>= (. st 0) (. st 1)) -999 (resume (. st 0) (tuple (+ (. st 0) 1) (. st 1))))))
            (+ (Sim.step) (+ (Sim.step) (Sim.step)))))
        (export main)))
  (call   main (: 2 Int64)) (output (: -999 Int64))
  (call   main (: 10 Int64)) (output (: 3 Int64)))
