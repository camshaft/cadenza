# PR #1973 review — DESIGN-host-capability-discovery.md + cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN — scope-creep MILD + DUPLICATE

https://github.com/camshaft/cadenza/pull/1973 ("docs(design): fix the awkward Owner-line parenthetical …
liaison pr1573 LOW", lane docs). Copilot (id 3710035171) flags a docs-titled PR carrying a kernel code
change. VERIFIED — and worse, the code change is a DUPLICATE of already-merged #1972.

## docs-titled/"lane docs" PR also adds a new async kernel test — AND that test is byte-identical to #1972's, already on trunk → merge collision (Copilot, kernel.rs:2976) — scope-creep (MILD) + duplicate [VERIFIED]
> The PR title/description indicate a docs-only change ("lane docs"), but this hunk adds a new async
> kernel test that changes code behavior/coverage in `cdz-kernel`. If this is intentional, consider
> updating the PR metadata … otherwise, move this test into a separate PR so the docs-only change can land
> independently.

VERIFIED: #1973 changes TWO files — a 1-line doc fix (my pr1573 Owner-line nit, "build"→"builds it") AND a
~90-line new test `a_routed_effect_err_folds_back_to_the_reducer_and_the_session_is_not_stuck` in
kernel.rs. Two problems:
1. **Scope-creep — MILD class** (per [[liaison-scope-creep-severe-vs-mild-escalation-calibration]]): a
   vertical (v-agent-harness) bundling its own-adjacent kernel test under a docs title. No hidden
   security/cross-cutting/breaking surface — the payload is an ADDED TEST (safest kind), caught + routed.
   → trend-only, NO individual operator escalation. The "lane docs" metadata is just mislabeled.
2. **DUPLICATE (the sharper issue):** that exact test — same name — is ALREADY ON TRUNK via #1972 (MERGED,
   "pin the routed-effect Err path — folds back + session not stuck"). `git show origin/main:…kernel.rs |
   grep a_routed_effect_err_folds… ` = 1 match. So #1973's kernel hunk re-adds an identical `fn` → a
   duplicate-definition COMPILE ERROR on merge (or at best a redundant re-add if rebased). The docs-only
   change should DROP the kernel hunk entirely — #1972 already landed the test.

Recommendation to v-agent-harness: strip the kernel.rs test from #1973 (it's #1972's, already on trunk) so
the PR is a clean 1-line doc fix matching its title + lane; re-cut if the queued --ref can't be amended.
LOW/MILD scope + a real merge-collision risk. v-agent-harness owns both files.
