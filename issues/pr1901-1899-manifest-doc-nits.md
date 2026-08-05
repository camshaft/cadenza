# PRs #1901 + #1899 review comments — cdz-kernel/src/{kernel,effect}.rs (v-agent-harness) — OPEN

Capability-manifest kernel work (design-host-capability I-slices). Two doc-accuracy nits.

## PR #1901 — comment "all-Absent over ALL never happens" contradicts a tested case (Copilot, kernel.rs:509) — doc/accuracy [VERIFIED]
> The comment says an "all-Absent manifest over ALL never happens", but
> `project_manifest(effect_ct::ALL, |_| false, ...)` is explicitly TESTED to produce all
> `GrantState::Absent` entries (effect.rs:1063-1076). The code here gates on whether `entries` is empty —
> a DIFFERENT property — so the wording is misleading.
An all-Absent manifest over ALL demonstrably CAN happen (it's tested), and the guard is actually on
`entries.is_empty()` (empty vs all-Absent are different). Reword the comment to the real gate condition
(empty entries) and drop the "never happens" claim. LOW/doc.

## PR #1899 — `grant_changes` doc says "linear walk" but it's `.find()` per family = quadratic (Copilot, effect.rs:513) — doc/accuracy [VERIFIED]
> The doc says this is "a linear walk in practice", but the impl does a `.find()` scan over `entries` for
> each family in the union set (repeated linear searches). Precompute a map, or adjust the doc.
VERIFIED: `grant_changes`'s `state_in` closure does `m.entries.iter().find(|e| e.family == fam)` — a linear
find per family, over the union family set → O(families²), not linear. The "same canonical family set"
caveat doesn't make each find cheaper. Either precompute a family→state map (BTreeMap) for a genuinely
linear pass, or reword the doc to "a find-per-family scan (quadratic in the family count; fine for the
small canonical set)". LOW/doc (+ optional perf). Fix-forward.
