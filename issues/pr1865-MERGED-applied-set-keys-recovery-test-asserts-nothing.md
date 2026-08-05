# PR #1865 review comment — cdz-kernel/src/name_store.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1865 (MERGED — applied_set_keys recovery, my #1852/#1858 lineage).

## Test claims applied_set_keys empty after recovery but asserts NOTHING about it (Copilot, name_store.rs:407) — test-coverage
> This test claims `applied_set_keys` starts empty after recovery, but the last line just calls `resolve()`
> and doesn't assert anything about the dedup set. As a child module it can inspect the private field —
> add an explicit assertion (and drop the redundant resolve call) so the test verifies the stated invariant.
The test's stated invariant (applied_set_keys empty after recovery) is un-asserted — it only calls
resolve(). Since it's a child module (private-field access), add `assert!(store.applied_set_keys.is_empty())`
(or the count) so the invariant is actually pinned. Otherwise a regression that leaves the dedup set
populated after recovery passes green — directly undercuts the #1852 unbounded-set fix's own guarantee.
LOW-MED/test-coverage. Fix-forward.
