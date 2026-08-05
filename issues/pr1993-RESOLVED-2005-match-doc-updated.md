# PR #1993 review — rcdzc/src/effects.rs (v-effects) — MERGED — doc-vs-impl [VERIFIED]

https://github.com/camshaft/cadenza/pull/1993 (FIX recursive-branch-perform selector — the Match-arm face).
Copilot (id 3711017613) flags the Match-arm doc still describes the OLD (pre-merge) behavior.

## the `Resolved::Match` doc comment says out-state is always post-scrutinee, but #1993 LANDED the per-arm out-state merge below it (Copilot, effects.rs:4838) — doc-vs-impl [VERIFIED]
> The `Resolved::Match` doc comment immediately above still describes the old threading behavior (captured
> by `thread_branch_local_abort` and match out-state always being the post-scrutinee state). With the new
> per-arm out-state merge, this comment is now inaccurate and should be updated to match the implementation.

VERIFIED on trunk — and this UPDATES my #1983 verify note. When I reviewed #1983, v-effects confirmed the
Match per-arm merge was QUEUED (e61e5b3d1), not landed, and merged #1983 was If-arm-only. That merge has
now LANDED (in #1993): effects.rs:4838+ collects `arm_outs` and merges them ("the `Match` analogue of the
`if` per-branch out-state merge … without the merge the match returns the post-SCRUTINEE state and the
advance is dropped (the recursive-branch-perform … match-arm face)"). BUT the doc comment immediately above
(effects.rs:4823-4832) still reads: "The out-state is the post-scrutinee state (the single-return shape
does not observe a per-arm out-state)." That now directly CONTRADICTS the merge code below it. LOW/doc-vs
-impl — the fix is correct; the stale comment describes the behavior the fix REPLACED. Fix: update the
Match-arm doc to describe the per-arm out-state merge (mirror the `if` arm's doc), removing the
"post-scrutinee state / does not observe a per-arm out-state" sentence. v-effects owns effects.rs.
