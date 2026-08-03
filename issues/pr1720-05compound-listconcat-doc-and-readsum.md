# PR #1720 review comments — spec/semantics/05-compound-types.sexp (v-runtime) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1720 (MERGED; author v-runtime).

## 1. Comment claims no existing List.concat case is big enough — verify (Copilot, 05-compound-types.sexp:2171) — doc/accuracy
> The surrounding comment claims there are no existing `List.concat` corpus cases big enough and that
> existing cases are only single-element/scratch. [Verify the claim vs the actual corpus.]

If the comment asserts an absence ("no existing case big enough"), confirm it's true — otherwise soften.
LOW/doc.

## 2. Docstring refers to `readsum` but no such helper — computed inline via `at` (Copilot, :2175) — doc/accuracy
> The docstring refers to `readsum`, but the case doesn't define a `readsum` helper (it computes the probe
> sum inline via `at`). Misleading narrative.

Reword the docstring to match the inline `at`-based probe (drop the `readsum` name, or introduce the
helper). LOW/doc. Fold both into the next 05-compound edit per the no-standalone-polish steer.
