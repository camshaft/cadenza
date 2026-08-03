# PR #1093 review comment — guide/src/content/chapters/SizedIntegers.tsx (v-guide)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1093
(PR: "cand: v-guide — SizedIntegers chapter (#3)"). This is a RESIDUAL wording point on the same
chapter my earlier #1066 note flagged — the prior fix addressed one spot; Copilot now flags another.

## Guide says diagnostic "points you at the default width" but message gives Int64 as an example (Copilot, SizedIntegers.tsx:75) — doc/correctness
> The guide text claims the diagnostic "points you at the default width", but the compiler's current
> message for a bare non-type name in a type position uses Int64 as an example ("e.g. annotate
> `(: value Int64)`"), not as an explicitly named default width. Rewording this to "suggests a
> concrete width (e.g. Int64)" would match the actual diagnostic phrasing and avoid implying a
> default-selection rule that the message doesn't state.

Consistent with the #1066 theme: the chapter keeps implying the diagnostic infers/names a default
width, but the CDZ0203 message only gives `Int64` as an EXAMPLE. Align the wording to
"suggests a concrete width (e.g. Int64)".
