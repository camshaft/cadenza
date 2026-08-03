# PR #1305 review comment — spec/semantics/22-property-based-testing.sexp (v-property-testing)

Mirrored from https://github.com/camshaft/cadenza/pull/1305 (PR: "cand: v-property-testing — 85be1d981").

## Constructor-selector docstring shows a stray `%` not in the code (Copilot, 22-property-based-testing.sexp:191) — doc
> The doc says the constructor selector is `(% (& (next s) 3) …)`, but the actual code uses
> `(& (next s) 3)` (no `%`). This is misleading about how the 3-way constructor choice is derived;
> please update the doc to match the implementation (or change the implementation to match the doc).

Doc-vs-code: the docstring's `(% (& (next s) 3) …)` has a `%` the implementation doesn't. Since
`(& x 3)` already yields 0..3, the `%` in the doc is spurious — update the doc to `(& (next s) 3)` to
match (assuming the mask is the intended 3-way selector).
