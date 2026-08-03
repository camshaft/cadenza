# PR #1183 review comment — guide/src/notebook/OutputView.tsx (v-notebook)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1183
(PR: "cand: v-notebook — 880935b68").

## `formula.unit && …` renders an empty text node for `""`, not eliminating the gap (Copilot, OutputView.tsx:30) — correctness (React)
> `formula.unit && …` evaluates to the empty string when `formula.unit === ""`. React renders
> strings (including ""), so this can still create an empty text node inside the flex container and
> may not fully eliminate the extra flex item / gap you're trying to avoid for `Unit.one`. Prefer an
> explicit check that returns `null` when the unit is empty.

Real React gotcha: `cond && jsx` with a string `cond` renders the empty string (unlike a falsy
boolean/null), so `formula.unit === ""` still emits an empty text node and the flex gap for `Unit.one`
persists — exactly what this PR aims to remove. Use an explicit `formula.unit !== "" ? (…) : null`
(or `formula.unit ? … : null` guarding the empty string) so nothing renders for the dimensionless case.
