# PR #1216 review comment — spec/semantics/05-compound-types.sexp (v-runtime)

Mirrored from https://github.com/camshaft/cadenza/pull/1216 (PR: "cand: v-runtime — 03c438bc2").

## "Set.insert appends persistently" is misleading for an unordered Set (Copilot, 05-compound-types.sexp:1916) — doc
> In the new Set CHAMP checksum case's docstring, the phrase "Set.insert appends persistently" is
> misleading: Sets are unordered, so "append" doesn't apply. It's clearer to describe `Set.insert` as
> a persistent insert (returning a new set).

Doc clarity: drop "append" (implies ordering); say `Set.insert` is a persistent insert returning a
new set.
