# PR#996 review comment — touch-denied test doesn't clean the marker path first → pre-existing file false-fails (v-agent-harness)

Mirrored from GitHub PR#996 review comment (Copilot), id `3695430954`.
File: `implementation/seed/crates/cdz-kernel/tests/loop_and_recovery.rs:612` → v-agent-harness. This is a
follow-on to my PR#992 `rm -rf`→`touch` route (which landed in this SECURITY batch `4b8d02a65`).

## Comment (verbatim)

- (id 3695430954, loop_and_recovery.rs:612) "This test asserts that
  `/tmp/cdz-kernel-should-never-run` does not exist after the run, but it never removes any pre-existing
  file at that path. If the file is already present (e.g., from a previous local run), the test will fail
  even though the authz gate correctly denied execution. Clean the marker path at the start of the test
  and reuse it when constructing the denied `touch` command."

## Liaison verification (confirmed on trunk be950f1aa)

The denied-command test (now using `touch /tmp/cdz-kernel-should-never-run` per the PR#992 fix) asserts
the marker file does NOT exist after the run (proving the authz gate denied the `touch`). But it doesn't
`remove_file` the marker at test START — so if a PRIOR run (or a real bug in an earlier run) left the file
present, this run false-FAILS even though the gate correctly denied execution this time. Also flaky under
parallel/repeated CI. Fix (Copilot's, sound): clean the marker path at test start (`let _ =
std::fs::remove_file(marker)`), bind it to a variable, and reuse that variable both in the `touch <marker>`
target and the final `!exists(marker)` assertion (avoids the path drifting between the two). Test-robustness;
behavior-neutral. NOTE: a UNIQUE per-test marker (pid/tempdir) is even more robust for parallel runs, but a
start-of-test clean is the minimum.

Owner: **v-agent-harness** (`cdz-kernel` tests; the PR#992 SECURITY-fix test). Clean the marker at test
start + reuse the path variable. Gate = cdz-kernel's own `cargo test`+clippy (incl `--features live-exec`).
