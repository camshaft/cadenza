# PR #1719 review comment — xtask/src/fleet.rs (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1719 (MERGED — the empty-range detection, the fix for my #1712
finding). Good fix; Copilot flags a robustness gap in the just-added rev-list check.

## `git rev-list --count` error conflated with empty range via unwrap_or(0) → masks real failures (Copilot, fleet.rs:8126) — correctness [VERIFIED]
> `git rev-list --count` errors (bad ref, missing object, not-a-git-repo, spawn failure) are treated as
> `range_count == 0` via `unwrap_or(0)`, which prints "already on trunk" and returns early — masking real
> failures and incorrectly skipping dispatch/rejection. Handle failure explicitly (only treat SUCCESSFUL
> 0 as the empty-range no-op); and word the message as reachability (graph-based, not patch-id).

VERIFIED in the #1719 diff: the added detection does `git rev-list --count {range}` … `.unwrap_or(0)`
(diff line 37), then `if range_count == 0 { print "NOTHING to land … already on trunk by patch-id"; return
}`. So a rev-list ERROR (bad ref / spawn failure) collapses to 0 → the MR is silently treated as a no-op
and NOT dispatched/rejected — masking the real error. Fix: distinguish spawn/exit failure from a
successful "0" (only the latter is the empty-range no-op); on error, fall through to the real
dispatch/reject path (or surface the error). Also: the message says "by patch-id" but `rev-list` is
graph-REACHABILITY, not patch-id — reword. MED (a masked git failure on the integrator path). Fix-forward.
