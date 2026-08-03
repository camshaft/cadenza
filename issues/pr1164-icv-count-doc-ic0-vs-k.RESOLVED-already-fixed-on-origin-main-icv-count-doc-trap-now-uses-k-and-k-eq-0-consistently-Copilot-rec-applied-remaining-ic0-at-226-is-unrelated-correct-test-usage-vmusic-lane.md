# PR #1164 review comment — implementation/music/src/interval-vector.cdz (v-music)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1164
(PR: "cand: v-music — interval-vector").

## `icv-count` doc says "ic0" but param is `k`, trap is `k == 0` (Copilot, interval-vector.cdz:87) — doc
> The doc comment for `icv-count` refers to "ic0" even though the parameter is `k` and the trap
> condition is `k == 0`. Clarify this to avoid confusing callers about which value triggers the trap.

Small doc clarity on the (recently reworked) `icv-count` — align the comment's "ic0" reference with
the actual `k`/`k == 0` trap condition.
