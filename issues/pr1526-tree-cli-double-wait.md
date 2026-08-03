# PR #1526 review comment — cdz/tests/tree_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1526 (PR: "cdz-tooling: tree_cli real
wall-clock deadline"). This IS the fix for my #1509 timeout finding — Copilot caught a bug in it.

## Double-wait: `wait_with_output()` after a successful `try_wait()` can fail (Copilot, tree_cli.rs:31) — correctness
> `try_wait()` reaps the child when it returns `Some(status)`. After breaking out of the loop, calling
> `wait_with_output()` will attempt to wait a second time and can fail (e.g., with `No child
> processes`). Capture the `ExitStatus` from `try_wait()` and read the piped stdout/stderr directly
> instead of calling `wait_with_output()` after a successful `try_wait()`.

Real bug in the #1509 timeout guard: once `try_wait()` returns `Some(status)` the child is already
reaped, so the follow-up `wait_with_output()` double-waits (can fail `No child processes` / return
wrong output). Capture the `ExitStatus` from the `try_wait()` that broke the loop, and read the piped
stdout/stderr handles directly rather than calling `wait_with_output()` again.
