# PR #1821 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1821 (MERGED).

## Test comment says `deliver_async_control` but the test now exercises `deliver_control` (Copilot, kernel.rs:1945) — doc/accuracy
> The explanatory comment still says the kernel returns control effects via `deliver_async_control`, but
> this test now exercises `deliver_control`. Update the comment to keep the migration narrative accurate.
The async-collapse rename residual (same family as #1752/#1764/#1811): the `_async` suffix was dropped
(`deliver_async_control` → `deliver_control`) but a test comment still names the old form. Update the
comment to `deliver_control`. LOW/doc, fix-forward.
