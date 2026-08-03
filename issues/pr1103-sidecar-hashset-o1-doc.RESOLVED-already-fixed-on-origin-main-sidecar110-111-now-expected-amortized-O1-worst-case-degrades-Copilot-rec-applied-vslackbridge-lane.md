# PR #1103 review comment — fleet/slack-bridge/src/sidecar.rs (v-slack-bridge)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1103
(PR: "cand: v-slack-bridge — sidecar.rs (oldest-first)"). Follow-up to the #1044/#1077 prune
rework where the membership check moved to a HashSet.

## `HashSet::contains` doc overstates strict O(1) (Copilot, sidecar.rs:111) — doc nit
> The doc comment overstates `HashSet::contains` as strictly `O(1)`. `HashSet` lookups are
> *expected/average-case* `O(1)` (worst-case can degrade), so the complexity claim should be phrased
> more precisely.

Minor: reword to "expected/amortized O(1)" (or "average-case O(1)"). Low priority — this is the
same prune-complexity-comment thread from #1077, now that the impl uses a HashSet.
