# PR #1139 review comment — guide/src/content/chapters/Effects.tsx (v-guide)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1139
(PR: "cand: v-guide — Effects chapter").

## Prose says both `tick`s read advanced state, but the first reads initial state (Copilot, Effects.tsx:201) — doc/correctness
> The explanation says both `tick`s are "reading the advanced state", but in the example the first
> `tick` reads the initial state (`s = 0`) and only the second reads the advanced state (`s = 1`).
> This is a factual mismatch with the code example and may confuse readers about when state updates
> take effect.

Factual mismatch between prose and the code example — reword so the first `tick` reads `s = 0` and
only the second reads `s = 1` (the whole point of when state updates take effect).
