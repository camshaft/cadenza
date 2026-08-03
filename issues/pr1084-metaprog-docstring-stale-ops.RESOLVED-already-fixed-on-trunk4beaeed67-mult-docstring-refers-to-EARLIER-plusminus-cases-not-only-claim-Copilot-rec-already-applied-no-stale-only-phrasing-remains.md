# PR #1084 review comment — spec/semantics/12-metaprogramming.sexp (v-metaprogramming)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1084
(PR: "cand: v-metaprogramming — 12-metaprogramming corpus (baseline group)").

## Docstring claims +/- are the only witnessed arithmetic ops, but same PR adds */ / /% (Copilot, 12-metaprogramming.sexp:185, also :190) — doc
> This docstring claims `+`/`-` are "the only arithmetic operators the eval-fold path witnessed",
> but this same PR adds `*`/`/`/`%` eval-fold cases immediately after. The statement is now
> inaccurate; consider rephrasing it to refer to *earlier* cases to avoid going stale.

Simple doc-accuracy fix: reword to refer to the earlier cases (or drop the "only" claim) so the
docstring doesn't contradict the `*`/`/`/`%` cases added right after it.
