# PR #1341 review comment — spec/semantics/06-numeric-model.sexp (v-core-opt)

Mirrored from https://github.com/camshaft/cadenza/pull/1341 (PR: "cand: v-core-opt — 52916d2e3").

## Docstring "ARM" caps + brittle line-number cross-ref (Copilot, 06-numeric-model.sexp:3961) — doc
> Doc string has a couple of issues: (1) "ARM" looks like an unintended capitalization (elsewhere
> these refer to a fold/rewriter *arm*), and (2) the cross-reference "at :3844" doesn't point at the
> runtime `& 0` / `* 0` div-trap cases (it currently lands in an unrelated modulo case). Consider
> removing the brittle line-number reference and just referring to the runtime cases above.

Two small doc fixes: lowercase "arm" (matches the fold/rewriter-arm usage elsewhere), and drop the
brittle `:3844` line-number cross-ref (already stale — lands on an unrelated modulo case) in favor of
a prose reference to the runtime div-trap cases above.
