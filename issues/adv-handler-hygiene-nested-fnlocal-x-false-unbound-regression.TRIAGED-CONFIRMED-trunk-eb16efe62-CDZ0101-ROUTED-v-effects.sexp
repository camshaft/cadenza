;; FINDING (breaker, regression after de4e166de) — TRIAGED+CONFIRMED trunk eb16efe62: the nested
;; two-effect shape with arm-local (def x) binders AND the colliding x defined FN-LOCALLY
;; ((def (main ...) (do (def x 1000) (handle ...)))) now rejects CDZ0101 'unbound name x' at the
;; handle-body reference. Pre-fix (de4e166de) it MISCOMPILED to 43; post-fix the MODULE-level-x twin
;; correctly computes 1033 (v-effects' eba1a7930 test), but the FN-LOCAL spelling went wrong-value →
;; FALSE-UNBOUND. The arm-rename in the nested composition over-reaches: it renames (or shadow-scopes-
;; out) the BODY's fn-local x. Controls (breaker): nested + fn-local x WITHOUT arm-locals = 1003 OK;
;; single handle + arm-local + fn-local x = 105 OK (landed fix pin). Better than a miscompile but still
;; a false reject of a legal program. OWNER: v-effects (follow-up to de4e166de arm-rename scope).
;; ON FIX: this is the nested-1033 face I hold HELD — flips (declines→1033) when the rename is scoped
;; correctly. Lexical expected 1033.

(case "nested handlers with arm-local + fn-local colliding bindings compute lexically (no false-unbound)"
  (input  (do
        (effect A (op geta (-> Unit Int64)))
        (effect B (op getb (-> Unit Int64)))
        (def (main (: mode Int64))
          (do
            (def x 1000)
            (handle A 1
              ((geta (u) s (do (def x 10) (resume (+ x s) s))))
              (handle B 2
                ((getb (u) s (do (def x 20) (resume (+ x s) s))))
                (+ x (+ (A.geta) (B.getb)))))))
        (export main)))
  (call   main (: 0 Int64)) (output (: 1033 Int64)))
