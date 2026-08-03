# PR #1375 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1375 (PR: "[v-fleet-tooling] da2ba9f11").

## "3 empty-picks causes" comment now stale — code distinguishes 4 (Copilot, fleet.rs:7457) — doc
> The comment says "the 3 empty-picks causes", but the code now distinguishes 4 reasons
> (in_flight_unknown, queued empty, cap reached, and file collisions). This makes the comment
> misleading for future edits; consider rewording it to avoid an incorrect count or update it to
> match the actual branches.

The #1340/#1330 work added the 4th cause (in_flight_unknown) — the comment still says "3". Update the
count (or drop the number and list the branches) so it matches the code.
