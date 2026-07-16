; BREAKER FINDING — reflection soundness gap (silent wrong Type.eq, both backends): Type.of of a function
; stored in a SUM/variant payload leaves its DOMAIN undetermined (Any), so two functions differing only in
; DOMAIN reflect the SAME type and Type.eq returns a wrong `true`. The e0b3c0827 fix ("Type.of grounds a
; function inside a compound") recursed TUPLE/LIST/RECORD elements but MISSED the sum-variant payload.
;
; CONFIRMED (both backends — Type.of folds at compile time, so backend-independent):
;   (Type.eq (Type.of (Some f)) (Type.of (Some g)))  f:Int64->Int64, g:Bool->Int64  -> 1 WRONG (should be 0)
; Isolated by container kind (all "different-domain" fns in the container):
;   TUPLE  (tuple f 0) vs (tuple g 0)  -> 0 correct (e0b3c0827 recurses tuples)
;   LIST   (list f)    vs (list g)     -> 0 correct (recurses lists)
;   SUM    (Some f)    vs (Some g)     -> 1 WRONG   (sum payload NOT recursed for the domain)
; Narrowed to the DOMAIN specifically — the CODOMAIN through a sum payload IS solved:
;   (Some f)[Int64->Int64] vs (Some p)[Int64->Bool]  -> 0 correct (codomain differs, caught)
;   (Some f) vs (Some h) SAME arrow                  -> 1 correct (equal)
; So the sum-payload reflection grounds the codomain (from the body result) but leaves the DOMAIN Any (the
; unannotated param), exactly the bottom-up `type_of` gap e0b3c0827 fixed for tuple/list/record.
;
; SUGGESTED FIX (v-inference / the Type.of reflection site, eval.rs Prim::TypeOf): the compound-recursion
; that grounds a fn element's body-solved domain must ALSO descend a SUM VARIANT PAYLOAD, not only
; Tuple/List/Record elements. The tuple/list/record path is the template; add the variant-payload arm.
;
; The cases below assert the CORRECT results. s1/cod-are-fine; the DOMAIN case fails today (wrong Type.eq
; `true`), flips to pass when the sum payload is recursed. Both backends fail the domain case identically.

(case "adv typeof-sum: two Some-wrapped functions differing in DOMAIN reflect distinct types"
  (doc "`(Type.eq (Type.of (Some f)) (Type.of (Some g)))` with f : Int64->Int64 and g : Bool->Int64 — the
        wrapped functions have DIFFERENT domains, so their Option(-> …) types differ → Type.eq should be
        false (0). But the sum-payload reflection leaves the fn DOMAIN as Any, so both reflect Option(-> Any
        Int64) and Type.eq wrongly returns true (1). The sum-payload analogue of the tuple/list cases the
        e0b3c0827 fix covers. WRONG on both backends today.")
  (input (do
           (def (f x) (+ x 1))
           (def (g b) (if b 0 1))
           (def (main) (if (Type.eq (Type.of (Some f)) (Type.of (Some g))) 1 0))
           (export main)))
  (output (: 0 Int64)))

(case "adv typeof-sum: the TUPLE version correctly distinguishes the domains (control — the fix covers it)"
  (doc "The control that PASSES: the same different-domain functions in a TUPLE — (tuple f 0) vs (tuple g 0)
        — reflect distinct types → 0. Pins that the gap is the SUM payload specifically; tuple/list/record
        element reflection (e0b3c0827) grounds the domain correctly.")
  (input (do
           (def (f x) (+ x 1))
           (def (g b) (if b 0 1))
           (def (main) (if (Type.eq (Type.of (tuple f 0)) (Type.of (tuple g 0))) 1 0))
           (export main)))
  (output (: 0 Int64)))

(case "adv typeof-sum: two Some-wrapped functions differing in CODOMAIN are distinct (the codomain IS solved)"
  (doc "The complement narrowing the gap to the DOMAIN: (Some f)[Int64->Int64] vs (Some p)[Int64->Bool] —
        differing in the CODOMAIN — correctly reflect distinct types → 0. So the sum-payload reflection DOES
        solve the codomain (from the body result); only the DOMAIN (the unannotated param) is left Any. Pins
        that the fix needs only to ground the domain through the variant payload, the codomain already works.")
  (input (do
           (def (f x) (+ x 1))
           (def (p x) (> x 0))
           (def (main) (if (Type.eq (Type.of (Some f)) (Type.of (Some p))) 1 0))
           (export main)))
  (output (: 0 Int64)))
