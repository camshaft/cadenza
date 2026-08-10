# PR #1991 + #1995 review — cdz-agent-host (v-agent-harness-host) — MERGED — test-flakiness [VERIFIED] (batched, same family)

Two more instances of the fixed-temp-dir test-flakiness class (my #1988), on the durable-log-wiring +
with_sink-hardening PRs. Batched — same owner, same fix pattern.

## #1991 (factory.rs:413 & :433, id 3710952908): two NEW file-log-sink tests use FIXED temp dirs [VERIFIED]
> This test uses a fixed directory name under `std::env::temp_dir()`. Since Rust tests run in parallel by
> default, concurrent runs (or a leftover dir from a crashed run) can collide … Use a unique temp
> directory name per test invocation.
VERIFIED (factory.rs): `file_log_sink_builder_opens_a_per_session_log` uses
`std::env::temp_dir().join("cdz-filelog-test")` (:413) and `file_log_sink_builder_sanitizes_a_slashed_
session_id` uses `..."cdz-filelog-sanitize-test"` (:433) — both FIXED paths. Same parallel-collision /
crashed-leftover flakiness the #1988 with_sink test had (and which you already fixed there with pid+seq).
These two sibling tests in the durable-log-wiring PR didn't get the same treatment. Fix: reuse the same
unique-per-run pattern (pid + process-local atomic counter) you applied to with_sink.

## #1995 (host.rs:668, id 3711033436): the with_sink fix's `create_dir_all` can reuse a stale dir on PID reuse [VERIFIED, refinement of my #1988]
> `create_dir_all` does not guarantee this directory is unique: if a previous crashed run left the same
> `cdz-with-sink-<pid>-<seq>` behind (PID reuse is possible), the directory already exists and the test
> will reuse an old log file, reintroducing flakiness. Prefer `create_dir` and retry on `AlreadyExists` so
> the test *provably* gets a fresh directory.
VERIFIED — the #1988 fix (host.rs:664) builds `cdz-with-sink-{pid}-{seq}` (seq = a process-local
`AtomicU64` from 0) then `create_dir_all(&dir).expect(...)`. `create_dir_all` SUCCEEDS if the dir already
exists, so the narrow window — a prior run CRASHED (didn't clean up), the OS later REUSED that pid, and the
new run's `seq` hit the same value (starts at 0 each process) — would silently reuse the stale dir + its
old log file. Real but narrow (needs pid reuse AND seq collision). Fix per Copilot: `create_dir` (not
`_all`) and retry with an incremented seq on `AlreadyExists`, so a fresh dir is PROVEN. LOW — a hardening on
the #1988-derived fix. (Composes with #1991 — apply the same proven-fresh helper to all three tests.)
v-agent-harness-host owns cdz-agent-host/src.
