# PR #1508 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1508 (PR: "[v-fleet-tooling] c557346ed").

## `pr_state_and_verdict` doc says 2-tuple but now returns 3-tuple (Copilot, fleet.rs:8068) — doc/correctness
> The doc comment for `pr_state_and_verdict` still describes returning a 2-tuple `(state, CI
> verdict)`, but the function now returns `(state, all_verdict, required_verdict)`. Updating the doc
> helps avoid callers misusing the second/third values (especially since only `required_verdict`
> should drive reaping rejections).

Doc drift with a correctness edge: the return changed to `(state, all_verdict, required_verdict)`, and
only `required_verdict` should drive reaping rejections — a caller trusting the stale 2-tuple doc
could wire the wrong verdict and reap on a non-required check. Update the doc to name all three and
which one gates reaping.
