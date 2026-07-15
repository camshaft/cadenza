;; GAP (2026-07-14, seed rcdzc — HM inference): an UNANNOTATED closure passed to a SELF-RECURSIVE
;; higher-order function whose function parameter is ALSO unannotated fails to infer the closure's
;; parameter types — they are left as unsolved type variables and the emit rejects them:
;;   "a closure's parameter type has no machine representation"
;;   CDZ0203: the argument for `fold-list`'s parameter `f` is a (-> Unit (-> Unit Int64)), but a value
;;            of type (-> _ (-> _ _)) is expected here
;; (`cdz check` FAILS — this is a type/inference reject, not a miscompile.) The `(-> Unit …)` in the
;; message is the tell: the closure's param type variables were never constrained, so they print
;; unsolved (`_` / defaulted).
;;
;; This is the CLASSIC left fold (`foldl`) — the single most common higher-order list function — written
;; the idiomatic way (no annotations). NO user sum type is involved; a bare `List Int64` reproduces it.
;;
;; ROOT: at the recursive call `(fold-list f (f h acc) t)`, `f` is re-passed to a `fold-list` whose own
;; `f` parameter type is not yet solved, so the constraint from the closure's USE (`f h acc`, which would
;; fix its params to the element/acc types) never flows back to the closure. A NON-recursive HOF solves
;; the closure from its single application (works). So it is specifically the recursion breaking the
;; back-flow of the closure param constraints.
;;
;; TWO WORKAROUNDS (each compiles + runs, returns 42):
;;   (A) annotate the CLOSURE's params:  (fn ((: x Int64) (: a Int64)) (+ a x))
;;   (B) annotate the HOF's f param with the arrow type:
;;         (def (fold-list (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64))) …)
;; Either one gives inference the anchor it can't derive through the recursion. The idiomatic
;; fully-inferred spelling below is what should work.
;;
;; IMPACT ON THE PORT: `fold` is fundamental to a compiler (fold over AST children, over a symbol list,
;; over constraints). Every such pass must currently carry an arrow-type annotation on its fold's fn
;; parameter — a real ergonomic gap in the language's inference.
(do
  (def (fold-list f acc xs)
    (match xs
      ((list) acc)
      ((list h .. t) (fold-list f (f h acc) t))))
  (def (main) (fold-list (fn (x a) (+ a x)) 0 (list 5 7 30)))
  (export main))
