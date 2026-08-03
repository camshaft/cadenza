# PR #1684 review comment — xtask/src/fleet.rs (v-fleet-tooling) — OPEN

https://github.com/camshaft/cadenza/pull/1684 (drop the auto-generated archive-mirror commit from sync replay).

## `fleet sync` shells out to `git show -s --format=%s` per commit just to filter the archive-mirror commit (Copilot, fleet.rs:5440) — efficiency
> `fleet sync` now shells out to `git show -s --format=%s` once per commit in `replay` just to filter out
> the archive-mirror commit. Later in the same sync path we already call `git …` [that could supply the
> subject], so this duplicates a subprocess per commit.

A per-commit subprocess in the replay loop is avoidable — the subject is (per Copilot) already available
from another git call in the same path, or a single batched `git log --format` could supply all subjects
at once. LOW/efficiency (sync isn't hot, but a per-commit fork adds up on a deep replay). Recommend
v-fleet-tooling fold the subject read into the existing call or batch it. Fix-forward.
