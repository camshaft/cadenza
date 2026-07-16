;; GAP (2026-07-14): a POLYMORPHIC TYPE ANNOTATION with an unbound signature type variable is rejected —
;; no form binds a signature's type variables. `(def (len (: l (Lst a))) …)` → CDZ0203 "`Lst` is a type,
;; not a function — a type appears in an annotation, not in call position" + CDZ0101 "unbound name `a`".
;;
;; The UNANNOTATED form WORKS and is the idiomatic spelling: `(def (len l) …)` monomorphizes correctly
;; at both element types (verified: `len` over `Lst Int64` + `Lst String` → 3). A CONCRETE generic
;; annotation `(: l (Lst Int64))` also works. Only a type-VARIABLE annotation `(Lst a)` fails.
;;
;; Two things for the seed: (1) allow a signature to bind its type variables so `(: l (Lst a))` resolves
;; (the "type-variable-in-signature" feature — orthogonal to monomorphization, which already works via
;; inference); (2) even before that, the DIAGNOSTIC is misleading — it reads `(Lst a)` as a call and
;; says "type, not a function" + "unbound name a", rather than "a polymorphic annotation needs `a`
;; bound". Impact on the port: generic passes must currently drop the type annotation on generic params.
(do
  (type Lst (Nil) (Cons a (Lst a)))
  (def (len (: l (Lst a))) (match l ((Lst.Nil) 0) ((Lst.Cons _ t) (+ 1 (len t)))))
  (def (main) (len (Lst.Cons 1 (Lst.Nil))))
  (export main))
