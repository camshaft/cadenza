;; SCOPE REFINED (breaker #33): op-PARAM (arm (get (v) s...) vs body v) + STATE (arm s vs body s)
;; binders are ALREADY hygienic (1053/1050 correct — the fold renames those two kinds). ONLY
;; arm-INTERNAL (do (def x ...))/(let ...) locals leak. Fix = extend the existing param/state rename
;; to the arm body's OWN binders (smaller seam than full hygiene). breaker banked a param+state clean
;; perimeter pin (1053/1050) to guard against regressing the working kinds.

;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-effects fixes the handler-fold hygiene
;; capture. Origin: breaker FINDING (issue 000000017415). CONFIRMED trunk 31a5f4f32: [main 1] expected
;; 105, ran → 10 (both backends, shared reduce_handle). SILENT WRONG VALUE. The tail-resumptive handler
;; fold inlines the ARM body at the perform site WITHOUT renaming binders → an arm-local def and a
;; perform-site/handle-body def with the SAME name collide BOTH directions:
;;   F1 arm→body: arm-local (def x 5) leaks into the handle body's x → 10 (=5+5), lexical=105.
;;   F2 body→arm: arm reads the performer's (def x 7) not the handler's enclosing x=100 → 14, exp=107.
;; No-perform control = 100 (correct); rename-the-binder control = 105/107 → it's the FOLD's inline
;; substitution, not static scoping. SAME CLASS as the eval-splice hygiene family (12-meta:101-184);
;; fix template = FRESH NAMES per splice / capture-avoiding substitution. OWNER: v-effects (reduce_handle).
;; rust is NOT an oracle (both wrong). ON FIX: rebuild cdz; gate x3 → 105/107; pin into
;; 14-effects-and-handlers.sexp; baseline x3; roundtrip + silent-omission + --check; MR; notify + breaker.

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

;; NESTED FACE (breaker #33, VERBATIM verified 43 on trunk 31a5f4f32): two nested handles, each arm a
;; do-def-local x, body reads free x through RIGHT-nested (+ x (+ (A.geta) (B.getb))) — stays reducible
;; (right-nested keeps both performs in one strict spine). Leak COMPOUNDS: body's x binds through the
;; inlined arms → 43 (=x 10-leak, geta 11, getb 22) for lexical 1033 (1000+11+22). A nested-composition
;; witness catches deeper arm-body-rename regressions than the single-handle face.
(case "nested handlers with colliding arm-local bindings each keep their own scope (no compounding leak)"
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
