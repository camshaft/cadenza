# PR #1509 review comment — cdz/tests/tree_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1509 (PR: "[v-cdz-tooling] 57bb722df").

## "Guarded by the harness" test can hang CI on a `cdz tree` regression (Copilot, tree_cli.rs:126, also :146) — CI/robustness
> This test claims it's "Guarded by the harness" against infinite recursion, but `run()` uses
> `Command::output()` with no timeout. If `cdz tree` regresses and loops, this test will hang the
> entire test suite/CI job. Add a local timeout/kill guard around the `cdz tree` invocation so the
> test fails fast instead of hanging indefinitely.

The "guarded" claim is false — `Command::output()` blocks with no timeout, so a `cdz tree` infinite
loop hangs the whole CI job (not just this test). Add a timeout/kill guard around the invocation so a
regression fails fast. Two sites (:126, :146).
