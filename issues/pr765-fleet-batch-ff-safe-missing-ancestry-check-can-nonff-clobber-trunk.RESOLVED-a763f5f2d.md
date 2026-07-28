# PR#765 review comment — batch_ff_is_safe misses an ancestry check; a stale staging ref can move trunk non-FF (discards commits)

Mirrored from GitHub PR review comment (Copilot), id `3627274026`.
PR: https://github.com/camshaft/cadenza/pull/765 (merged; fix still belongs on trunk)
Location: `xtask/src/fleet.rs:6070` (`batch_commit_inner`), guard fn `batch_ff_is_safe` (fleet.rs:5906).

## Comment (verbatim)

> `batch_commit_inner` treats `current_trunk == staged_base` as sufficient to claim a fast-forward,
> but it never verifies that `staged_tip` is actually a descendant of `staged_base`. If
> `refs/fleet/batch-staging` is stale/corrupted (or accidentally pointed elsewhere), `git update-ref`
> will still move `trunk` to a non-FF commit, discarding commits reachable only from the old trunk
> tip.
>
> Add an explicit ancestry check (e.g., `git merge-base --is-ancestor <base> <tip>`) and refuse if it
> fails, before reporting OK or executing the CAS update.

## Liaison verification (CONFIRMED on trunk — real safety hardening)

`batch_ff_is_safe` (fleet.rs:5906-5911) is:
```rust
fn batch_ff_is_safe(current_trunk: &str, staged_base: &str, staged_tip: &str) -> bool {
    !current_trunk.is_empty() && !staged_base.is_empty() && !staged_tip.is_empty()
        && current_trunk == staged_base
}
```
It checks ONLY that trunk still equals the staged base (drift guard) — it does NOT verify `staged_tip`
descends from `staged_base`. The subsequent `update-ref <trunk_ref> <staged_tip> <staged_base>` CAS
(fleet.rs ~6082) only guards against a concurrent trunk MOVE (trunk still == staged_base at write) — it
does NOT enforce fast-forward-ness of the new value. So if `refs/fleet/batch-staging` is stale,
corrupted, or mispointed such that `staged_tip` is NOT a descendant of `staged_base` (== current trunk),
`batch-commit --execute` moves `trunk` to a non-FF commit, **discarding every commit reachable only
from the old trunk tip**.

This directly threatens the fleet's hard invariant #2 ("`trunk` only ever moves FORWARD — never a
backward reset"; the single-writer/no-clobber guarantee) — pr-sync IS the single writer this must
protect. The `reference-transaction` clobber-logger would RECORD such a backward move, but this guard
is what should PREVENT it.

Fix (per Copilot): before the FF assert / CAS, add
`git merge-base --is-ancestor <staged_base> <staged_tip>` and refuse (return Err, trunk NOT moved) if it
fails. Cheap, and closes the non-FF-clobber hole. Owner: v-fleet-tooling (SINGLE owner of `fleet.rs`).
Routed as a note flagged SAFETY (protects the single-writer/forward-only trunk invariant).
