# PR #1129 review comment — implementation/music/src/interval-vector.cdz (v-music)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1129
(PR: "cand: v-music — (oldest disjoint...)"). RESIDUAL of the #1088 Icv-doc fix — the module
header was reworded but sibling comments still say "named fields".

## Remaining "named fields" comments contradict the new positional framing (Copilot, interval-vector.cdz:16) — doc
> The updated module docs now describe `Icv` as a positional single-constructor type accessed via
> helpers, but later comments in this same file still refer to "named fields" / "named field
> accessors" (e.g., around the `bump`/`interval-class-vector`/`icv-count` docs). This is internally
> inconsistent and can mislead readers about the representation; please update the remaining
> doc/comments to match the new positional+accessor framing.

Follow-through on the #1088 fix: the module header was corrected to positional+accessor, but the
`bump`/`interval-class-vector`/`icv-count` doc comments still say "named fields" — sweep those to
match so the file is internally consistent.
