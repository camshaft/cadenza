# PR #1731 review comment — xtask/src/fleet.rs (v-fleet-tooling) — ADDRESSED (delta d3f9893cc, held behind #1731; #1712→#1731 chain CLOSED)

https://github.com/camshaft/cadenza/pull/1731 (surface a rev-list ERROR distinctly — the #1725 error-face
fix landing). 4th link in the #1712→#1719→#1725→#1731 chain. The three-face match is correct; Copilot flags
a redundant re-run.

## `None` arm re-runs `git rev-list` just to capture stderr (Copilot, fleet.rs:8125) — efficiency [VERIFIED]
> The `None` arm reruns `git rev-list --count` just to capture stderr, because the first command's
> `Output` is discarded when building `range_count`. Capture the `Output` once and derive both
> `range_count` and `stderr` from it.

VERIFIED in the diff: `range_count: Option<usize>` is built from a first `git rev-list --count` whose
`Output` is discarded (only the parsed count is kept), then the `None` arm (:42-46) runs `git rev-list
--count {range}` a SECOND time solely to read `.stderr`. Capture the `Output` once (`let out = …
.output()`; derive `range_count` from `out.stdout` and reuse `out.stderr` in the None arm) — avoids a
redundant subprocess on the error path (and avoids a possible stderr mismatch if repo state changes
between the two calls). LOW/efficiency — the error path is rare, but it's a clean single-capture. Completes
the #1712 chain's polish. Fix-forward.
