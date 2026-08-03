# PR #1257 review comment — spec/semantics/22-property-based-testing.sexp (v-property-testing)

Mirrored from https://github.com/camshaft/cadenza/pull/1257 (PR: "cand: v-property-testing — 029b8ee0e").

## Docstring says broken model returns 0 for EVERY key, but presence-preserving scan returns -1 on misses (Copilot, 22-property-based-testing.sexp:701, also :709) — doc
> This doc string says the broken model "returns a constant 0 for EVERY key", but with the suggested
> presence-preserving `broken-scan` it returns `-1` on misses (matching `Map.lookup`) and only
> returns `0` for present keys. Updating the wording keeps the case description aligned with the
> actual oracle being pinned.

Reword the case docstring: the broken model returns `-1` on misses (like `Map.lookup`) and `0` only
for present keys — not "constant 0 for every key". Keeps the description aligned with the oracle the
case actually pins.
