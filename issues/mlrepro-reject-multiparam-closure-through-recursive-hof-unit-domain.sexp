;; GAP (2026-07-15, seed rcdzc — HM inference): a MULTI-PARAMETER unannotated closure passed to a
;; SELF-RECURSIVE higher-order function whose function parameter is ALSO unannotated rejects at the
;; def-call SCHEME-UNIFY (a COMPILE-path-only fault — `cdz check` passes, `cdz compile` rejects):
;;   CDZ0203: the argument for `fold-list`'s parameter `f` is a (-> Unit (-> Unit Int64)), but a value
;;            of type (-> _ (-> _ _)) is expected here
;;
;; The `(-> Unit (-> Unit Int64))` is the tell: the closure's UNSOLVED param vars round-trip through the
;; scheme encoding as `Unit` (the elided-unit convention — see eval.rs §"catch-all below encoded it as
;; Unit"), so the def-call arg-check (infer.rs `collect`, the scheme instantiate-and-unify at ~7407)
;; sees `(-> Unit …)` instead of an open arrow and faults against `fold-list`'s generic `f` param.
;;
;; SCOPE — this is the SIBLING of the SINGLE-arg case that was FIXED 2026-07-15
;; (mlrepro-reject-inferred-closure-param-through-recursive-hof): the fix made
;; `lambda_param_ty_from_context` reject a free-`Var` context domain (a fully-generic HOF param's hole),
;; so a bare closure grounds from its OWN body. That fix CLEARS the "no machine representation" DECLINE
;; and the SINGLE-arg recursive-HOF fold now compiles+runs (corpus 09-functions "an unannotated closure
;; is inferred through an unannotated recursive HOF parameter"). But the MULTI-param case additionally
;; hits this SEPARATE `Unit`-domain scheme-round-trip fault at the def-call arg-check — the closure's
;; params DO solve (the closure lifts fine when the HOF param is annotated), the problem is the
;; def-call's typed-arg check reads the closure's type as `(-> Unit …)`.
;;
;; WORKAROUNDS (each compiles + runs): (A) annotate the CLOSURE's params `(fn ((: x Int64) (: a Int64)) …)`;
;; (B) annotate the HOF's `f` param with the arrow type `(: f (-> Int64 (-> Int64 Int64)))`.
;;
;; ROOT to chase: either the arg-check should type an unannotated multi-param lambda argument through its
;; solved param types (not the `Unit`-defaulted scheme encoding), OR the `Unit`-domain round-trip should
;; preserve an open var at an unsolved closure-param position. Compile/check divergence must also close —
;; `check` accepting a program `compile` rejects is itself a coverage bug.
;;
;; IMPACT ON THE PORT: `foldl`/`fold` — the two-arg accumulator fold, the single most common HOF — still
;; needs an arrow-type annotation on its callback when the HOF is generic + recursive.
(do
  (def (fold-list f acc xs)
    (match xs
      ((list) acc)
      ((list h .. t) (fold-list f (f h acc) t))))
  (def (main (: n Int64)) (fold-list (fn (x a) (+ a x)) n (list 5 7 30)))
  (export main))
