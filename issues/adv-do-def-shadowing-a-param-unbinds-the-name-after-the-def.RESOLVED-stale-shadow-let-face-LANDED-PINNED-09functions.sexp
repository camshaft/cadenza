
; ---------------------------------------------------------------------------------------------
; ADDENDUM (breaker, same day): a THIRD face — do-def shadowing a LET binding is SILENTLY
; DROPPED wholesale (worse than the param face's false-unbound):
;   (let ((v k)) (do (def v (* v 2)) v))  -> k  (5 at k=5; shadow semantics say 10)
;   (let ((v k)) (do (def v (/ 1 0)) v))  -> k, NO TRAP — the def's RHS never evaluates
;   (both backends agree; "k unused" fires on the trap variant = the def is dead-code-eliminated)
; Matrix now: def-over-DEF works (15) · def-over-PARAM = false-unbound CDZ0101 ·
;             def-over-LET = SILENT NO-OP incl. side-effect/trap elision.
; The silent-drop face is the soundness-worst: a strict-spine trap disappears.
(case "a do-def shadowing a LET binding rebinds and its init evaluates"
  (input  (do
        (def (f (: k Int64))
          (let ((v k))
            (do (def v (* v 2)) v)))
        (def (main (: k Int64)) (f k))
        (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
