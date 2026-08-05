# PR #1988 review — cdz-agent-host/src/host.rs (v-agent-harness-host) — MERGED — test-precision ×2 [VERIFIED]

https://github.com/camshaft/cadenza/pull/1988 (HostedSession::with_sink — durable-log attach seam).
Copilot 2 inline, both VERIFIED on the `with_sink_persists_a_sessions_events_durably` test.

## test uses a FIXED temp path + swallows fs errors → parallel-run collisions + hidden failures (Copilot, host.rs:660) — test-flakiness [VERIFIED]
> This test uses a fixed temp directory/file name and ignores filesystem errors, which can make it flaky
> when tests run in parallel (collisions between concurrent runs) and can hide failures to create/remove
> the file. Use a per-run unique temp path and avoid silently ignoring directory creation failures.

VERIFIED (host.rs:657): `let dir = std::env::temp_dir().join("cdz-with-sink-test");` — a FIXED shared path,
and `let _ = std::fs::create_dir_all(&dir); … let _ = std::fs::remove_file(&path);` swallow the results.
Two concurrent runs of this test (cargo runs tests in parallel by default; and the new nix test-check adds
another runner) share `session-durable.log` → one run's `remove_file` / recover races the other's writes.
A leftover file from a crashed prior run also poisons the next. LOW-MED/flakiness. Fix: unique per-run temp
dir (include pid + a nonce, or use a `tempfile`-style unique dir), and at least `.expect()` the
`create_dir_all` so a real fs failure surfaces instead of a confusing downstream `LogStore::open` error.

## the test discards `deliver`'s result → a KernelError turn can pass on log length (Copilot, host.rs:668) — test-precision [VERIFIED]
> The test drives a turn but discards the result of `deliver`. If the session fails its turn
> (KernelError), this test can still pass based on log length, masking real regressions. Assert the
> `deliver` outcome is `Some(Ok(()))`.

VERIFIED (host.rs:668): `host.deliver(&id, inbound_go(), None).await;` — the `Option<Result<(),
KernelError>>` is discarded. The subsequent asserts check `in_mem > 1` and `recovered.events.len() == in_mem
- 1` — both about the LOG, not the turn's success. A regression that made `deliver` return `Some(Err(..))`
(kernel error) could still leave a log with events (the Inbound is appended before the fold) and pass. Same
class as my #1963 FoldFailed finding: assert the OUTCOME, not just a downstream count. LOW-MED. Fix:
`assert!(matches!(host.deliver(&id, inbound_go(), None).await, Some(Ok(()))), "the turn folds cleanly");`
before the log asserts. v-agent-harness-host owns cdz-agent-host/src.
