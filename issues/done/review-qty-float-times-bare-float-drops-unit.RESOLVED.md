# Qty Float64 × bare Float64 mis-infers as bare Float64 — the unit is SILENTLY DROPPED

**Severity:** correctness — a type-inference miscompile (silent unit loss). `cdz check` clean,
`cdz type` reports the wrong type. CONFIRMED by `cdz type`.

**Status:** the fix commit `683a305ba` ("reject a numeric-type mismatch when scaling a quantity by
a bare number") DISCLOSED this as a distinct pre-existing issue and said it was "Separately filed
issues/mlrepro-qty-times-bare-float-loses-unit-*.sexp" — **but that file does not exist** anywhere
(not in `.claude/fleet/queue/`, not in the repo `issues/`, not added in the commit). So the
disclosed follow-up was lost. Filing it here so it is tracked as fleet work. Reviewer-confirmed real.

## The bug

`(* (Qty.of 5.0 (Unit.base #"meter")) 2.0)` — a `(Qty Float64 meter)` scaled by a bare `Float64` —
infers as bare **`Float64`**, dropping the `meter` unit entirely, instead of `(Qty Float64 meter)`.

```
$ cdz type f  <(echo '(module m (def (f) (* (Qty.of 5.0 (Unit.base #"meter")) 2.0)) (export f))')
Float64          # ← WRONG: should be (Qty Float64 meter)
```

Contrast: the Int case `(* (Qty Float64) 1)` now correctly REJECTS (CDZ0301, the `683a305ba` fix),
and a same-inner Qty×Qty multiply composes units correctly in `apply_type`. Only the **Qty Float ×
bare Float** shape mis-infers — the bare-Float scaling factor has the same inner type as the
quantity, so it passes the new inner-numeric agreement check, but then a WRONG `apply_type` arm
computes the result type.

## Root cause (per the disclosing commit + confirmed shape)

`apply_type`'s **Float arm preempts the quantity arm**: when both operands' inner types are Float,
the multiplicative-result computation matches the plain Float×Float rule (result = Float64) BEFORE
the quantity arm (which would compose the dimension and keep the unit). So the `Ty::Qty` wrapper is
discarded. The fix is an `apply_type` arm-REORDER (check the quantity/dimensional case before the
bare-Float numeric case), the "later apply_type-reorder slice" the commit named.

## Why it matters

A dropped unit is a silent semantic error: `5.0 meter * 2.0` should be `10.0 meter`, but the
compiler now believes the result is a unitless `10.0`. Downstream a `+` against another quantity, an
`as/in` conversion, or a Qty render will misbehave or mis-typecheck — the unit invariant the whole
quantity feature rests on is violated. It's the Float twin of the "unit dropped entirely" symptom
row in `mlrepro-calc-bare-quantity-relabels-to-base-without-scaling.md`, but a DISTINCT mechanism
(that item is non-base-unit relabel-without-scaling; this is unit-loss on a well-formed Float
multiply) — link them, don't merge.

## Reproducer

```
(module m (def (f) (* (Qty.of 5.0 (Unit.base #"meter")) 2.0)) (export f))
```
`cdz type f` → `Float64` (bug); expected `(Qty Float64 meter)`. A `cdz type` assertion is the
tightest pin; a runtime render/`Unit.in` check is the observable-value companion.

## Verified
Built `cdz` at trunk `a588e5431`, ran `cdz type` on the shape above → `Float64` (unit dropped).
Confirmed the disclosed filing is absent from queue + repo. Distinct from the existing calc-relabel item.

<!-- RESOLVED 2026-07-15 (trunk@1aebedc58, fix 1aebedc58): Qty × bare number keeps its dimension — apply_type reorder gated the BigInt/Rational/Float operand arms on !any_qty so (Qty T u)×bare-T stays (Qty T u). Verified: (* (Qty Float64 meter) 2.0) : (Qty Float64 meter). 2 graded pins + rcdzc test. Took 3 sends (2 silently dropped — pattern flagged to concierge). Residual x*1-identity edge tracked separately by v-quantity. -->
