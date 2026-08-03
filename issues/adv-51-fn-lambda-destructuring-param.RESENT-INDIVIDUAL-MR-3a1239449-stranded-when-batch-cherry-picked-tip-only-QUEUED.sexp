; adv-51 (breaker tick 1059) — OVER-REJECT: a tuple-destructuring parameter on a `fn` (LAMBDA)
; rejects CDZ0101 'unbound name' while the SAME pattern on a `def` parameter works — but
; core-semantics.md:137 grants the pattern to BOTH: "A binding position — a `let` binder, a
; function or `fn` parameter — MUST accept an irrefutable pattern in place of a bare name".
;
; Observed (trunk 19ee669e5, all 3 targets consistent):
;   (def (dist (tuple x y)) ...)                       WORKS (probe green ×3, pinned this tick)
;   (let (((tuple p (tuple q r)) v)) ...)              WORKS (nested let face green ×3)
;   (fn ((tuple x y)) (+ (* x 10) y))                  CDZ0101 unbound x/y  ← THIS filing
;
; The failure signature (CDZ0101 = the pattern's names fall through to scoping) matches the adv-49
; class: the lambda-param binder position doesn't run the destructuring desugar, so the pattern's
; inner names never bind. Spec basis is unambiguous — :137 names "fn parameter" explicitly and the
; tuple pattern here is irrefutable (both elements are names). Expected: 34 at a=3.
;
; Severity: moderate — the workaround is a wrapper def or a let-destructure inside the body, but
; the shape is natural for HOF call sites ((map (fn ((tuple k v)) ...) pairs) once iterators land).

(case "a tuple-destructuring fn (lambda) parameter binds its parts"
  (doc    "core-semantics.md:137: a binding position — a `let` binder, a function or `fn` parameter —
           MUST accept an irrefutable pattern. The def-param and let faces work; the fn (lambda) face
           rejects CDZ0101 with the pattern's names unbound. Graded against the SPEC (output 34), so
           this case is red today and flips green when the lambda-param desugar lands.")
  (input  (do
            (def (main (: a Int64))
              (let ((f (fn ((tuple x y)) (+ (* x 10) y))))
                (f (tuple a 4))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
