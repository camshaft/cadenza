# PR #1536 review comments — implementation/seed/crates/cdz/tests/tree_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1536 (PR: "[v-cdz-tooling] eec39b3ec").
This PR fixes the tree_cli `run()` helper to avoid double-waiting a `try_wait()`-reaped child
(the #1526 follow-on). Both Copilot points verified against the diff.

## 1. `read_to_string` on the piped handles panics on non-UTF8 output — regression (Copilot, tree_cli.rs:64) — correctness
> Using `Read::read_to_string` makes the test helper panic on any non-UTF8 byte emitted by the CLI
> (it returns `InvalidData`). Previously the helper used `String::from_utf8_lossy`, which is more
> robust for capturing diagnostics. Consider reading raw bytes and converting lossily to preserve
> prior behavior.

VERIFIED against the diff: the old code was `String::from_utf8_lossy(&out.stdout/stderr)`, the new
code is `.read_to_string(&mut …).expect("read stdout/stderr")`. A single non-UTF8 byte from `cdz
tree` (or a regression) now PANICS the helper on `InvalidData` instead of capturing a lossy
diagnostic. Read into a `Vec<u8>` and `String::from_utf8_lossy` it to preserve the prior robustness.

## 2. `try_wait()` loop drains stdout only after exit — pipe-buffer deadlock risk (Copilot, tree_cli.rs:36) — robustness
> `try_wait()` loop waits for the child to exit before draining the piped stdout/stderr. If `cdz
> tree` (or a regression) produces enough output to fill an OS pipe buffer, the child can block on
> write and never reach exit, causing this test helper to hit the 30s timeout (and lose the intended
> fast fail).

This is a documented tradeoff — the PR's own added doc says "`cdz tree`'s output is a handful of
lines (far under the pipe buffer), so reading it after exit can't deadlock." True for the current
fixtures; the failure mode is only a slow (30s) fail instead of a fast one, and only if a regression
makes `tree` emit >64KB. LOW priority — noting for awareness; point 1 is the substantive one.
