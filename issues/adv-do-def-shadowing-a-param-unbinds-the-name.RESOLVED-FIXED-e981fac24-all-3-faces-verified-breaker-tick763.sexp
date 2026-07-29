; FINDING (breaker, 2026-07-28): a do-def that SHADOWS A FUNCTION PARAMETER makes the name
; UNBOUND for every later reference in the do — false CDZ0101 on a legal shadow. Both backends
; (shared resolve).
;
;   ok    (do (def x 5) (def x (+ x 10)) x) = 15          def-over-DEF: documented backward-only
;                                                          sequential scope (02-binding :1267)
;   FAIL  (def (f (: v Int64)) (do (def v (* v 2)) v))    def-over-PARAM, RHS reads param -> CDZ0101
;   FAIL  (def (f (: v Int64)) (do (def w v) (def v (* w 2)) v))  even with NO self-ref RHS, the
;         TRAILING v is "unbound" (col of the final reference) — the def-over-param BREAKS the name
;   ok    same but return w -> 5 (the def itself compiles; only post-def references to the
;         shadowed NAME die)
;
; Spec angle: 02-binding pins "the shadow is well-defined" for LET-over-param (:116 = true) and
; def-over-def (:1267 = 15); a def-over-PARAM should shadow identically — instead the name is
; neither the param NOR the new binding afterward. This is also the root of the tick-735 nested-
; handler false-unbound: the hygiene fix's arm-local (def x ...) shadows x that arrived AS A
; SYNTHESIZED PARAM in the folded/lifted body (fn-local x threads as a param; module-level x
; doesn't — exactly the observed module-vs-fn-local split).
;
; GRADED REPRO (expected = shadow semantics; FAILS CDZ0101 today):
(case "a do-def shadowing a function parameter is a well-defined shadow"
  (input  (do
        (def (f (: v Int64)) (do (def v (* v 2)) v))
        (def (main (: k Int64)) (f k))
        (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
