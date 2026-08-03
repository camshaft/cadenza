# PR #1055 review comment — spec/semantics/05-compound-types.sexp (v-runtime)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1055
(PR: "cand: v-runtime — RRB coverage 05-compound-types").

## Summation-range doc ambiguity (Copilot, 05-compound-types.sexp:1991) — doc
> The doc string uses `Sigma i for i in 0..1100 = 604450`, which is easy to misread as inclusive
> (0..=1100) and would then be arithmetically incorrect. Since the surrounding text already
> explains the list is `[0..n-1]`, spell the summation range as `0..(n-1)` (or `0..1099`) to
> remove ambiguity.

Non-blocking doc clarity point in the new RRB corpus case. (Checksum 604450 = sum of 0..1099 is
correct; only the range notation is ambiguous.)
