# PR review comment — mirrored from GitHub PR #437 (Copilot inline)

- **PR:** #437 "fleet: sixtieth batch (units miscompile FIX, …)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/rcdzc/src/infer.rs:545` (`type_of` `Resolved::Annot` quantity same-dimension case)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592467385
- **Link:** https://github.com/camshaft/cadenza/pull/437#discussion_r3592467385

## Comment (verbatim)
> In `type_of` for `Resolved::Annot`, the quantity same-dimension special-case returns `expr_ty` early without unifying/applying the annotation's inner numeric type. That preserves the unit (good), but it also prevents the annotation from grounding a deferred integer width/sign inside the quantity (e.g. `(: (Qty.of 5 kilometer) (Qty UInt8 meter))` would keep a deferred/Int64-ish inner type instead of constraining it), which is inconsistent with how annotations generally constrain/ground types.
> Consider unifying the *inner* numeric types and returning a `Ty::Qty` that keeps the expression's unit but applies the substitution to the expression's inner type.

## Liaison triage — CONFIRMED against trunk (completeness gap in the just-landed units-miscompile fix)
Confirmed in infer.rs: the same-dimension quantity annotation arm is
`if let (Ty::Qty{unit:au,..}, Ty::Qty{unit:eu,..}) = (&annot_ty, &expr_ty) && au.same_dimension(eu) {
return expr_ty; }`. It correctly keeps the expression's UNIT (the fix for the "same-dimension
annotation must not rebrand the scale" miscompile), but returns `expr_ty` WHOLESALE — so it never
unifies/applies the annotation's INNER numeric type. An annotation like `(: (Qty.of 5 kilometer)
(Qty UInt8 meter))` therefore won't ground a deferred inner width/sign (stays Int64-ish) — inconsistent
with how annotations elsewhere constrain types. FIX (as reviewer): unify the inner numeric types and
return a `Ty::Qty` that keeps the EXPRESSION's unit but applies the substitution to the inner type.
Quantity + inference territory (v-quantity owns Qty typing; v-inference relevant). Fix on `trunk`.
Quote + link in queue file.
