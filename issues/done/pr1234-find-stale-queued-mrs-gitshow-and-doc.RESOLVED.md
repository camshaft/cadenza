# PR #1234 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1234 (PR: "cand: v-fleet-tooling — 7f8d50e06").

## 1. `find_stale_queued_mrs` runs `git show` per queued MR (Copilot, fleet.rs:2346) — efficiency
> `find_stale_queued_mrs` calls `lane_label_for_ref(&mr.r#ref)` for every queued MR, but that
> function shells out to `git show`. This extra git process can be avoided for MRs that are clearly
> not stale (in flight, landed, or not past the threshold) by computing `lane_blocked` lazily via
> short-circuiting.

Short-circuit: skip the `lane_label_for_ref`/`git show` call for MRs already excluded by the cheaper
predicates (in-flight, landed, under the age threshold) so the audit doesn't spawn a git process per
queued MR.

## 2. `find_stale_queued_mrs` doc omits the new lane-blocked exclusion (Copilot, fleet.rs:2300) — doc
> The doc comment for `find_stale_queued_mrs` still describes stale-queued as only (age > threshold,
> not in flight, not landed), but the implementation now also excludes MRs that are legitimately
> blocked behind an in-flight candidate in the same serialized lane. The comment should be updated so
> the audit criteria match the code.

Add the lane-blocked exclusion to the doc so the audit criteria list matches the implementation.
