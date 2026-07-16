;; UPDATE (2026-07-16, v-inference): PART (2) — the misleading DIAGNOSTIC — is RESOLVED. `(: l (Lst a))`
;; no longer reads `(Lst a)` as a call; it now reports a single CDZ0101 "unbound name `a` — a lowercase
;; name in a type position is not a type variable here … Cadenza has no `∀`-binder in an annotation;
;; write a GENERIC parameter by leaving it UNANNOTATED … or annotate a concrete type" (position-aware:
;; "a parameter's annotation" vs "the type position of an annotation"). Pinned as a graded corpus case
;; "a type variable nested in a generic parameter annotation is an unbound name" (spec/semantics/
;; 09-functions.sexp), companion to the bare `(: 5 foo)`/`(: 5 Foo)` pins in 07-type-system.sexp.
;; PART (1) — actually BINDING a signature's type variables so `(: l (Lst a))` RESOLVES (the
;; "type-variable-in-signature" feature) — REMAINS not-yet-built; the idiomatic spelling is the
;; unannotated `(def (len l) …)`, which monomorphizes via inference. Original report below.
;;
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
