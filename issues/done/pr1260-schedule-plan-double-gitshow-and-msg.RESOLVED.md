# PR #1260 review comments — xtask/src/fleet.rs `schedule_plan` (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1260 (PR: "cand: v-fleet-tooling — 70765f821").
Same efficiency theme as #1234/#1244 (git-subprocess churn in the scheduler).

## 1. `schedule_plan` runs `git show` twice per queued MR (Copilot, fleet.rs:7353) — efficiency
> In `schedule_plan`, each queued MR currently runs `git show` twice: once via
> `lane_label_for_ref(&mr.r#ref)` and again via `changed_files_of(&mr.r#ref)`. Since
> `changed_files_of` is already needed for file-collision scheduling, compute it once and derive the
> lane label from that same list to avoid redundant subprocess calls (can matter when the queue is
> large).

Compute `changed_files_of` once per MR and derive the lane label from that file list, instead of a
second `git show` via `lane_label_for_ref` — halves the subprocess count per queued MR.

## 2. "dispatch nothing" message stale — says lane-serialized but it's now file-collision (Copilot, fleet.rs:7355) — doc/UX
> The `schedule_plan` "dispatch nothing" message still mentions "queued lanes serialized-and-busy",
> but dispatch selection is now blocked by file collisions rather than lane serialization. Updating
> the message will avoid misleading operators when picks are empty due to file-level blocking.

The empty-picks operator message still describes the old lane-serialization model; reword to reflect
file-collision blocking so an operator isn't misled about why nothing dispatched.
