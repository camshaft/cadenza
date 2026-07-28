; FINDING (breaker, 2026-07-28): the tail-resumptive handler fold inlines the ARM BODY at the
; PERFORM SITE without hygiene — names cross the arm/body scope boundary in BOTH directions,
; changing program VALUES silently. BOTH backends identical (shared reduce_handle fold).
;
;   F1 arm->body: (def x 100) ... handle arm (do (def x 5) (resume (+ x s) s)) body (+ x (E.get))
;      = 10 (the BODY's x reads the ARM's 5: 5+5) — lexically 105 (arm x local; body x = 100).
;      Control: renaming the arm's binder to y gives 105 ✔. let-form identical (10).
;   F2 body->arm: arm (resume x s) with NO arm-local x, body (do (def x 7) (+ x (E.get)))
;      = 14 (the ARM's x reads the PERFORMER's 7: 7+7) — lexically the arm's x is the HANDLER's
;      enclosing scope x=100 → expected 107.
;   Control: body WITHOUT a perform reads its own x correctly (100) — the leak is
;      perform-triggered (the fold's inline substitution), not a static scope bug.
;
; This is the EFFECTS twin of the eval-splice hygiene capture family (12-meta :101-184, the
; breaker-found miscompile): same class — an inline/splice step substituting un-renamed binders.
; Silent wrong VALUES (not a reject): any handler arm that names a variable also used near a
; perform site computes with the wrong one.
;
; GRADED REPRO (both faces; FAILS today 10/14, expected 105/107):
(case "handler-arm bindings and perform-site bindings stay in their own scopes across the fold"
  (input  (do
        (effect E (op get (-> Unit Int64)))
        (def (main (: mode Int64))
          (do
            (def x 100)
            (if (= mode 1)
                (handle E 0
                  ((get (u) s (do (def x 5) (resume (+ x s) s))))
                  (+ x (E.get)))
                (handle E 0
                  ((get (u) s (resume x s)))
                  (do (def x 7) (+ x (E.get)))))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 105 Int64))
  (call   main (: 2 Int64)) (output (: 107 Int64)))
