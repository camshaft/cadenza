# BUG: single-variant sum erased by the optimizer loses its NOMINAL type through the cadenza hop — dual-path render DIVERGENCE (and it bypasses #4913's multi-argument-variant decline guard)

Found tick 446 (2026-08-28) via the dual-path VALUE oracle probing #4913 (cadenza runtime SumNew emit). ROUTED to v-cadenza-backend.

## Defect
The optimizer represents a SINGLE-variant sum transparently (newtype erasure): the payload value carries the nominal `Ty::Sum` type but the Core node is the payload's node (Arith/Tuple), NOT a `Core::SumNew`. The cadenza backend emits that node with no nominal ascription, so the hop output re-types STRUCTURALLY and the rendered result diverges from the direct path:

- `(module m (type (Box a) (Mk a)) (def (main (: n Int64)) (Mk (* n 3))) (export main))`, arg 7
  → direct `(: 21 Box)` vs hop `21` (**diverge**; hop `.ast` contains no `Mk`/`Box` at all).
- `(module m (type (Pair a b) (Both a b)) (def (main (: n Int64)) (Both n (+ n 1))) (export main))`, arg 5
  → direct `(: (tuple 5 6) Pair)` vs hop `(: (tuple 5 6) (Tuple Int64 Int64))` (**diverge**).

The `Pair` case also shows #4913's "a multi-argument variant declines" guard is BYPASSED for a single-variant sum: the multi-payload variant reaches the backend as a `Core::Tuple`, so instead of the intended decline it EMITS a wrong (nominal-type-losing) surface. A ≥2-variant multi-payload sum declines correctly (verified: `(type T2 (A Int64 Int64) (B Int64))` → clean "multi-argument variant" decline), so the bypass is exactly the erasure class.

## Fix direction
When the node's solved type is a nominal `Ty::Sum` but the Core value is the erased payload, re-emit with the nominal ascription `(: <payload-value-as-ctor-application-or-value> <SumType>)` — or decline the erased-single-variant-sum case until it can be re-emitted faithfully. (Note the type decl must also be present — see the sibling user-sum-decl BUG; fixing that alone won't fix this one, since the hop `.ast` has already lost the sum entirely.)

## Impact
Silent type-identity loss through the hop: the round-tripped program renders a different value form (`21` vs `(: 21 Box)`), so corpus-cadenza value baselines mismatch and — worse — a hop consumer sees a structurally-typed value where the source had a nominal one. Repros: ~/breaker-scratch/2026-08-28-cadenza-sum/{gen,multi,multi2}.sexp. breaker did NOT touch compiler code.
