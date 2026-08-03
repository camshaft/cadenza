# PR #1444 review comment — spec/semantics/16-binary-matching.sexp (v-patterns)

Mirrored from https://github.com/camshaft/cadenza/pull/1444 (PR: "[v-patterns] 5549e0d20").

## Docstring references `g 9` but the program defines `main`/`f` (Copilot, 16-binary-matching.sexp:935) — doc
> Docstring refers to `g 9`, but the program in this case defines/exports `main` (and `f`). This
> makes the example trace harder to follow because `g` doesn't exist in the input.

Align the docstring's example trace with the actual program symbols (`main`/`f`) — `g` isn't defined
in this case, so the trace is confusing to follow.
