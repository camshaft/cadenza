# PR #1330 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1330 (PR: "cand: v-fleet-tooling — 90c19553d").
This is the OTHER HALF of the #1244 `changed_files_of` None-handling finding — the in-flight side.

## 1. ⚠ `in_flight_files` silently drops a None (unresolvable) in-flight ref → false file-disjoint (Copilot, fleet.rs:2318, also :7390) — correctness
> `in_flight_files` is built by dropping any in-flight dispatch whose `changed_files_of(..)` returns
> `None`. If an in-flight ref becomes unresolvable (missing object, bad ref, repo not in expected
> cwd, etc.), its changed-file set is silently omitted, which can make a queued MR that *does*
> collide with that in-flight ref look file-disjoint and get falsely classified as
> stale/dispatchable. Consider treating an unresolvable in-flight ref conservatively (e.g., track an
> `in_flight_unknown_files` flag and treat every queued MR as `file_blocked`, or plumb a shared
> sentinel that forces collision when any in-flight file set is unknown).

The #1244 fix made `changed_files_of` return `None` on error and treated a QUEUED MR's None
conservatively — but the IN-FLIGHT side here still drops None entries, so an unresolvable in-flight
ref makes its file set vanish, and a queued MR that actually collides with it reads as disjoint →
falsely dispatchable/stale (exactly the failure mode #1244 fixed, on the other operand). Treat an
unknown in-flight file set conservatively: force collision (every queued MR `file_blocked`) when any
in-flight ref's files can't be resolved.

## 2. `changed_files_of` doc still says "empty on error" but returns `None` now (Copilot, fleet.rs:7302) — doc
> The doc comment for `changed_files_of` still says it returns an empty list on an unresolvable ref /
> git error, but the function now returns `None` in that case. Updating the comment will prevent
> callers from assuming `None` == empty/clean.

Stale doc from the #1244 `Option` change — update it so callers don't read `None` as empty/clean
(which is exactly the confusion that produced point 1).
