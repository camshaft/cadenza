# pr687 — infer-db.cdz comment says a bare literal is "width-polymorphic" but it also adapts across SIGN

Mirrored from GitHub PR #687 review comment (Copilot), id 3614651766.
PR: https://github.com/camshaft/cadenza/pull/687 (compiler-ml narrow-width + metaprog doc)
Location: `implementation/compiler-ml/src/infer-db.cdz:247`

## Reviewer comment (verbatim)
> Comment says a bare integer literal is only width-polymorphic, but this adaptation logic also allows the
> literal to unify across signedness (via `fits-width(v, ts, tw)` using the target sign). Update wording to
> avoid misleading future changes/readers.

## VERIFIED (git show trunk)
The `arith-result-type` mixed-width comment (infer-db.cdz:248-253) repeatedly says a bare integer literal is
"width-POLYMORPHIC" and adapts to a "narrow sibling" / grounds "to the sibling's WIDTH." But the mechanism
`arith-lit-adapt`→`lit-fits-narrow` (infer-db.cdz:262-275) grounds the defaulted-Int64 literal to the
sibling's FULL type `(ts, tw)` — sign AND width. So a signed-default `10` adapting to a `UInt8` sibling
takes `(unsigned, 8)` when the value fits: the literal unifies across SIGNEDNESS too, not just width.
Copilot is right — "width-polymorphic" undersells it; the bare literal is width-AND-sign-polymorphic
(bounded by fit, via lit-fits-narrow's `(ts,tw)` target). Reword the comment to say the literal grounds to
the sibling's width AND sign. Doc-only, no behavior change (the code already does the sign adaptation).

## Owner
`implementation/compiler-ml/src/infer-db.cdz` = v-compiler-ml (PORT source). Inference-semantics wording, so
v-inference could advise — but it's a doc-shape fix on the port. PM to place (v-compiler-ml).
