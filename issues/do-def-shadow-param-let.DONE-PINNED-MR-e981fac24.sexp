;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-inference fixes do-def-over-param
;; shadow unbind. Origin: breaker FINDING (issue 000000017571). CONFIRMED trunk 64ee9058c:
;; (def (f (: v Int64)) (do (def v (* v 2)) v)) → CDZ0101 'unbound name v'. A do-def shadowing a
;; PARAM unbinds the name for all later refs (a legal shadow). Even non-self-ref ((def w v)(def v
;; (* w 2)) v) leaves trailing v unbound; returning w compiles (5). def-over-DEF fine (:1267, 15);
;; let-over-param fine (:116) — only def-over-PARAM dies. Resolver kills the shadowed name instead of
;; rebinding. OWNER: v-inference (resolve/scope). ROOT of the nested-handler false-unbound regression
;; (v-effects de4e166de lifts fn-local x → synthesized PARAM → arm-local def-over-param hits this).
;; Oracle main(5)=10, main(0)=0. ON FIX: gate x3 → 10/0; pin into 09-functions.sexp beside the
;; let-over-param (:116) + def-over-def (:1267) shadow pins; baseline x3; MR.

(case "a do-def shadowing a function parameter is a well-defined shadow"
  (input  (do
        (def (f (: v Int64)) (do (def v (* v 2)) v))
        (def (main (: k Int64)) (f k))
        (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

;; LET-SHADOW FACE (breaker #37 third face, SOUNDNESS-worst): a do-def shadowing a LET binding INSIDE
;; A FN is silently DROPPED — (def (f (: k Int64)) (let ((v k)) (do (def v (* v 2)) v))) returns k not
;; 2k; with a trapping init (def v (/ 1 0)) it STILL returns k with NO TRAP (the shadow def is
;; dead-code-eliminated wholesale). Verified trunk 64ee9058c: [main 5] expected 10, ran → 5. Note the
;; FN-WRAPPING is the trigger (a bare-main let-shadow works; inside a fn param→let→def-shadow drops).
;; Same resolver root (shadow def recorded against WRONG scope level → param-shadow kills the name,
;; let-shadow loses the def). def-over-DEF is fine. OWNER: v-inference (resolve/scope), same finding.
(case "a do-def shadowing a LET binding rebinds and its init evaluates"
  (input  (do
        (def (f (: k Int64))
          (let ((v k))
            (do (def v (* v 2)) v)))
        (def (main (: k Int64)) (f k))
        (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
