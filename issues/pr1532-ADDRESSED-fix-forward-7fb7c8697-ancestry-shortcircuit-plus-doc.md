# PR #1532 review comments — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1532 (PR: "[v-fleet-tooling] eb00212ed" — the
cutover-critical advance-trunk reap fix).

## 1. Empty-cherry-pick duplicate when merge commit is already an ancestor (Copilot, fleet.rs:8310) — correctness
> The "already present on trunk by patch-id" check uses `git cherry` + `cherry_says_landed`, but if
> the merge commit is actually reachable from trunk (ancestor), `git cherry <trunk> <merge_oid>` can
> output nothing; `cherry_says_landed` treats empty output as not-landed, and the subsequent
> `--allow-empty` cherry-pick can create an empty duplicate commit. Add an explicit ancestry check
> before falling back to patch-id equivalence.

Real edge on the trunk-advance path: an already-ancestor merge commit yields empty `git cherry`
output → treated as not-landed → `--allow-empty` cherry-pick creates a duplicate empty commit on
trunk. Add an explicit `git merge-base --is-ancestor <merge_oid> <trunk>` short-circuit BEFORE the
patch-id fallback.

## 2. Doc still describes update-ref CAS fast-forward, but function now cherry-picks mergeCommit.oid (Copilot, fleet.rs:8268, also :8343) — doc
> The doc comment still describes fast-forwarding trunk to origin/main via update-ref CAS, but the
> function now advances trunk by cherry-picking the PR's mergeCommit.oid. Update the contract doc.

Update the doc to the current cherry-pick-mergeCommit.oid model (2 sites) so the advance-trunk
contract isn't misleading for future debugging.
