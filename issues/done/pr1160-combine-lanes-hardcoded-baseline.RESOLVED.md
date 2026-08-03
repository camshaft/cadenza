# PR #1160 review comments — xtask/src/fleet.rs + CI-GATED-LANES-DESIGN.md (v-fleet-tooling)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1160
(PR: "cand: v-fleet-tooling — priority (executor/reservation-fix?)").

## 1. `combine_lanes` hard-codes a second `baseline` Lane definition (Copilot, fleet.rs:6829) — maintainability
> `combine_lanes` re-creates a fresh `Lane { name: "baseline", parallel: false }` when folding
> `{corpus, baseline}`. This hard-codes lane metadata in a second place; if the baseline lane
> definition ever changes in `LANE_RULES` (or gains fields later), this block can silently drift.
> Prefer returning the existing `baseline` lane instance from `lanes` instead of constructing a new
> one.

Real drift risk: the `baseline` Lane is defined in two places. Return the existing instance from
`lanes`/`LANE_RULES` rather than reconstructing it, so a future field addition or metadata change
can't silently desync.

## 2. Design-doc: handle PR-closed-while-polling race in I4 (amazon-q, CI-GATED-LANES-DESIGN.md:142) — design/forward-looking
> The design correctly identifies the closed-but-not-merged PR edge case that could cause infinite
> waiting. Ensure the implementation in I4 also handles race conditions where a PR is closed while
> being polled.

Forward-looking note for the I4 implementation slice: beyond the static closed-but-not-merged case,
handle the race where a PR is closed *during* a poll cycle so the poller doesn't wait forever. Worth
capturing in the design doc's I4 section as an explicit requirement.
