# PR #1753 review comment — cdz-kernel replay rename BREAKS cdz-agent-host tests ON TRUNK (v-agent-harness-host + v-agent-harness) — MERGED, URGENT fix-forward

https://github.com/camshaft/cadenza/pull/1753 (MERGED — renamed Session::replay_async → replay).

## `Session::replay_async` renamed to `replay`, but 3 cdz-agent-host test files still call `replay_async` → LIVE TRUNK COMPILE BREAK (Copilot, kernel.rs:897) — correctness [VERIFIED HIGH]
> `Session::replay_async` was renamed to `Session::replay`, but there are still in-repo callers using
> `Session::replay_async` (cdz-agent-host/tests/{agent_runs_e2e,http_agent_e2e,model_agent_e2e}.rs). As-is
> this PR will break compilation for those crates/tests.

VERIFIED against trunk (post-#1753 merge):
- kernel defines ONLY `pub async fn replay(...)` (kernel.rs:897) — no `replay_async`, no alias (the #1753
  diff renamed it and updated in-crate callers/tests).
- but `Session::replay_async(...)` is STILL called in 3 cdz-agent-host TEST files: model_agent_e2e.rs:122,
  agent_runs_e2e.rs:137, http_agent_e2e.rs:105. `replay_async` is defined NOWHERE on trunk → those tests
  fail to compile.

WHY IT LANDED: #1753 is a KERNEL PR, and the cdz-agent-host CI job is NOT path-filtered on kernel changes
(the known [[cdz-agent-host-ci-job-unfiltered]] trap — here in REVERSE: the kernel PR's CI didn't run the
host tests, so the break wasn't caught pre-merge). It will now RED the next cdz-agent-host PR (e.g. #1758
fmt / #1751).

HIGH + time-sensitive: cdz-agent-host's test crate is broken on trunk RIGHT NOW. Fix-forward:
s/Session::replay_async/Session::replay/ in the 3 test files. Owner = v-agent-harness-host (owns
cdz-agent-host, strict seam); v-agent-harness FYI'd (did the rename — the additive-alias-bridge pattern
from [[cdz-agent-host-ci-job-unfiltered-kernel-shared-type-change-needs-alias-bridge]] would have avoided
this: rename + keep a deprecated `replay_async` alias until the host migrates).
