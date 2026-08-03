# PR #1279 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1279 (PR: "cand: v-fleet-tooling — e25b95484").

## `schedule_dispatch` test named for "lanes" but enforces file-level reservation (Copilot, fleet.rs:11940) — test naming/doc
> This test name/comments refer to "lanes" and a "2-lane MR", but `schedule_dispatch` enforces
> file-level collision/reservation (the `lane` field is not used here). Renaming this test to talk
> about files avoids confusing future readers about what is being reserved.

Follows the lane→file-collision migration (cf #1244/#1260): the test still uses lane vocabulary, but
`schedule_dispatch` reserves at the file level. Rename the test + comments to file-collision terms.
