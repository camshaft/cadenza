; FINDING (breaker, 2026-07-21): a user sum whose name collides with the built-in `Ast` type
; renders its constructors DIFFERENTLY per backend at the value boundary:
;
;   (type Ast (Lit Int64) (Node (List Ast)))  (Node (list (Lit 5) (Lit 6)))
;     wasm:            (Node (list (Lit 5) (Lit 6)))               — UNQUALIFIED
;     rust/rust-async: ((. Ast Node) (list ((. Ast Lit) 5) ...))   — QUALIFIED
;
; A non-colliding name (type Tree ...) renders UNQUALIFIED on all three backends, so the
; qualification is triggered by the name collision with the metaprogramming `Ast` — but only
; the rust render applies it; wasm renders bare ctor names. Whichever form is canonical, the
; two backends must agree (the corpus cannot pin this case today: any (output ...) expectation
; fails one backend). Existing corpus pins DO use the qualified form in INPUT position
; ((. Ast Int) ...) so the qualified spelling is a real, parseable form.
;
; Both repro cases below are the SAME program with the two observed expectations — each fails
; exactly one backend today; after the fix exactly one should pass everywhere (and the other
; should be deleted).

(case "REPRO-A user sum named Ast renders UNQUALIFIED (wasm's current form)"
  (input  (do
            (type Ast (Lit Int64) (Node (List Ast)))
            (def (main (: n Int64))
              (Node (list (Lit n) (Lit (+ n 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: (Node (list (Lit 5) (Lit 6))) Ast)))

(case "REPRO-B user sum named Ast renders QUALIFIED (rust's current form)"
  (input  (do
            (type Ast (Lit Int64) (Node (List Ast)))
            (def (main (: n Int64))
              (Node (list (Lit n) (Lit (+ n 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: ((. Ast Node) (list ((. Ast Lit) 5) ((. Ast Lit) 6))) Ast)))
