# PR #1248 review comment — cdz/tests/test_manifest_cli.rs (v-cdz-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1248 (PR: "cand: v-cdz-tooling — 482a97f3c").

## `PathBuf::to_str().expect(...)` panics on non-UTF-8 temp path (Copilot, test_manifest_cli.rs:249) — test-robustness
> `PathBuf::to_str().expect(...)` makes this test unnecessarily dependent on the temp path being
> valid UTF-8. On some platforms/filesystems that can be false, turning an intended read-error test
> into a panic. This file already uses `to_string_lossy()` for paths; use the same approach here and
> keep the owned string alive for the `run()` call.

Use `to_string_lossy()` (already the pattern elsewhere in this file), binding the owned `String` so
it lives across the `run()` call — otherwise a non-UTF-8 temp path turns this read-error test into a
panic on some platforms.
