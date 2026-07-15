;; WITNESS (2026-07-15, v-inference) — the MINIMAL, CANONICAL trigger of the recursive-generic-producer
;; element-tie gap (sibling of mlrepro-decline-recursive-generic-producer-drops-element-tie.cdz): the
;; single most important HOF, `map` over a `List`, has its RESULT element DISCONNECTED from its callback's
;; output type. `mapl : (a -> b) -> List a -> List b` is inferred
;;   (-> (-> _ _) (-> (List Int64) (List _)))
;; — the result `(List _)` element is a FREE var unrelated to the callback's result `b`. (`cdz type mapl`.)
;;
;; SHARPER THAN the `Iter` sibling in TWO ways:
;;   1. BUILT-IN `List` (no user sum), just `map`.
;;   2. Declines at a SINGLE instantiation — NOT only ≥2. The moment the mapped result is consumed by an
;;      ELEMENT-TYPED consumer, the free result element can't be pinned:
;;        (suml (mapl (fn (x) (+ x 1)) xs))   -> DECLINES  (suml reads `h` as Int64; mapl's result elem is _)
;;      whereas an ELEMENT-AGNOSTIC consumer hides it:
;;        (List.len (mapl (fn (x) (+ x 1)) xs)) -> COMPILES + RUNS  (List.len ignores the element type)
;;   So the true minimal trigger is "a recursive-generic producer whose result element is CONSUMED at a
;;   concrete type", not "instantiated at two types" — the two-instantiation form is just one way to force
;;   the consumption. Filing so the fix's gate covers the single-instantiation face too.
;;
;; SAME ROOT + FIX as the `Iter` sibling (see that file's PART A/B/C analysis): `mapl`'s param `xs` shapes
;; to `List ?a` only with the `pattern_implied_ty` list arm (PART A); the callback result `b` is tied to
;; neither `?a` nor the result element because `compute_def_scheme` types the return through `apply_type`'s
;; `freshen_free` (PART C), severing the tie. The result element stays a free var the monomorphizer cannot
;; bind. Fix locus: `compute_def_scheme`/`solve_recursive_params` shared-subst fixpoint (deferred, large).
;;
;; This declines at EMIT (`cdz compile`/`test`); `cdz check` PASSES. `suml`'s `h` is Int64, mapl's callback
;; is `Int64 -> Int64`, so the intended result is `List Int64` and `suml` sums it: over `[n,n,n]` mapped by
;; `+1`, the sum is `3*(n+1)`. At n=10 → 33. (When the tie fix lands, promote this to the graded corpus.)
(do
  (def (mapl f xs)
    (match xs ((list) (list)) ((list h .. t) (List.push (mapl f t) (f h)))))
  (def (suml xs)
    (match xs ((list) 0) ((list h .. t) (+ h (suml t)))))
  (def (main (: n Int64)) (suml (mapl (fn (x) (+ x 1)) (list n n n))))
  (export main))
