# PR #1056 review comment — guide/src/notebook/formatValue.ts (v-notebook)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1056
(PR: "cand: v-notebook — formatValue + extractTable").

## `displayUnit` doc comment inconsistent with `wrapUnit` behavior (Copilot, formatValue.ts:70) — doc
> The `displayUnit` doc comment says "a base unit / power stays bare", but `wrapUnit` currently
> parenthesizes any non-`Unit.base` unit (including `Unit.^`). This makes the comment internally
> inconsistent (it also uses `(a^2)*b` as an example of parenthesized power). Update the comment to
> match the implemented behavior so readers aren't misled.

Non-blocking doc-vs-code point; verify whether the comment or the code reflects intended behavior.
