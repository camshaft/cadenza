# PR #1098 review comment — guide/src/playground/examples.ts (v-guide-infra)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1098
(PR: "cand: v-guide-infra — check-examples + playground").

## `expected` only asserted on the sexpr pass, not the ML toggle (Copilot, examples.ts:25) — doc/correctness
> The `expected` value is not checked in the example's arbitrary authored `surface`; `checkProgram`
> only runs + compares `expected` on the `sexpr` pass (`if (surface === "sexpr")`), and the ML
> toggle pass does not assert it. As written, this doc comment could mislead someone into thinking
> `expected` will be asserted for ML-authored playground examples too.

Either reword the doc to state `expected` is only asserted on the sexpr pass, or extend
`checkProgram` to also assert `expected` on the ML toggle pass (the stronger fix, if ML-authored
examples should be pinned too).
