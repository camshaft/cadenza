# PR #1244 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1244 (PR: "cand: v-fleet-tooling — c4a2cf01d").

## 1. `changed_files_of` conflates git-error with "no changed files" (Copilot, fleet.rs:7256) — correctness
> `changed_files_of` returns an empty Vec both for "no changed files" and for "git error /
> unresolvable ref". For the new file-level stale-queue logic, an error currently looks like
> "file-disjoint", which can incorrectly mark a queued MR as dispatchable/stale when we actually
> couldn't determine its file set. Consider returning `Option<Vec<String>>`/`Result<…>` (distinguish
> error vs empty) and treating the error case conservatively (e.g., as blocked or skipping the
> stale-queued classification).

The important one: an empty result means both "genuinely touches no files" and "couldn't compute the
file set" — and in the file-level stale-queue logic the latter silently reads as file-disjoint, so a
queued MR whose files we FAILED to resolve gets treated as dispatchable/stale. Distinguish the error
(Option/Result) and treat unknown conservatively (blocked / skip the stale classification) so a git
hiccup can't mis-flag a queued MR.

## 2. Hard-coded date in `files_collide` doc comment (Copilot, fleet.rs:7272) — doc nit
> The `files_collide` doc comment hard-codes a specific date ("(2026-08-02)"). That will go stale
> quickly and makes the comment read like historical context rather than a durable invariant. Prefer
> stating the rule without a date (or point to a stable doc/issue if needed).

Drop the date; state the collision rule as a durable invariant.
