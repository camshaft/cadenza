# PR #1108 review comment — implementation/music/src/set-class.cdz (v-music)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1108
(PR: "cand: v-music — set-class.cdz (oldest-first)").

## `find-transposition` doc says `t..11` but impl wraps past 11 to 0 (Copilot, set-class.cdz:63, also :72) — doc
> `find-transposition`'s doc comment says it searches transpositions `t .. 11`, but the
> implementation wraps `t` via `pc-of(t + 1)` and (depending on `fuel`) can continue past 11 back to
> 0. Updating the doc to describe the actual wraparound search avoids misleading future
> callers/readers.

Doc-accuracy: reword to describe the actual mod-12 wraparound search (up to `fuel` steps) rather
than a bounded `t..11` linear scan.
