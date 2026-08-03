# PR #1712 review comments — xtask/src/fleet.rs (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1712 (MERGED — cherry-pick the full trunk..ref RANGE, the
v-effects #1705 multi-commit-drop fix). Advance-trunk lineage (#1532/#1684/#1690).

## 1. Empty range (ref==trunk) emits "CONFLICTS … needs a rebase" + rejects sender — misleading (Copilot, fleet.rs:8106) — correctness/UX [VERIFIED]
> The empty-range case (no commits in `trunk..ref`) falls into the generic cherry-pick failure path and
> emits "CONFLICTS … needs a rebase" — misleading when the range is empty (usually "nothing to land /
> already on trunk"), and it contradicts the preceding "no-op" comment. Detect an empty range up front
> (`git rev-list --count`) with a dedicated message; keep the conflict path for real conflicts.

VERIFIED against trunk: publish_candidate does `cherry-pick --allow-empty {TRUNK}..{ref}` (fleet.rs:8104),
and on failure emits "`git cherry-pick {range}` … CONFLICTS (or empty range) — this MR needs a rebase; NOT
dispatching. (reject the sender…)" (:8116). So an ALREADY-LANDED ref (empty range) is rejected as a stale-
base conflict — wrong signal (it's "nothing to land", not "rebase needed"). Detect empty range up front
(`git rev-list --count trunk..ref == 0`) and return a dedicated "already on trunk / nothing to land"
outcome (probably an ack-merged/no-op, NOT a stale-base reject). MED — a spurious reject of an
already-landed MR. Fix-forward.

## 2. Step-2 doc still says single-commit `git cherry-pick <ref>` (Copilot, fleet.rs:8100, doc ~7985) — doc [VERIFIED]
> `publish_candidate`'s doc comment still describes step 2 as "`git cherry-pick <ref>` … re-parent the
> SINGLE MR commit" (~7984-7986), but the implementation now cherry-picks the full `trunk..ref` range.

VERIFIED: doc at fleet.rs:7985 reads "`git cherry-pick <ref>` there — re-parent the SINGLE MR commit",
but the code (:8096-8104) cherry-picks the full `trunk..ref` range (the #1705 multi-commit fix). Update
the step-2 doc to the range model so operators don't follow the outdated single-commit recipe. LOW/doc.
