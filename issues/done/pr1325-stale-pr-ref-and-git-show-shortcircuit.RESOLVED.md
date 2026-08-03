# PR #1325 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1325 (PR: "cand: v-fleet-tooling — 894ec703c").

## 1. Comment hard-codes "PR #1234 review" (Copilot, fleet.rs:2339) — doc
> This comment hard-codes a specific PR number ("PR #1234 review"), which is likely to become stale
> and isn't necessary to justify the ordering here. Consider rephrasing to a timeless rationale.

Drop the PR-number reference; state the ordering rationale timelessly.

## 2. `mr_is_stale_queued` can skip `git show` after the landed check (Copilot, fleet.rs:2358) — efficiency
> `file_blocked` requires `changed_files_of()` which shells out to `git show`. If `landed` is true,
> `mr_is_stale_queued` can never return true, so we can skip the `git show` call entirely by
> early-continuing after the landed check.

Same git-churn family as #1234/#1244/#1260: short-circuit — if `landed`, return before computing
`file_blocked` so a landed MR doesn't spawn a `git show`.
