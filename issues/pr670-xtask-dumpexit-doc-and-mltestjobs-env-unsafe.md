# pr670 — xtask: "dump-exit on FIRST failure" doc misleads (joins first) + ml_test_jobs test mutates global env unsafely (3 Copilot)

Mirrored from GitHub PR #670 review comments (Copilot). All VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/670 (re-land parallel-sweep) — v-fleet-tooling (xtask).

## #1,#2 — id 3613607611 (main.rs:4280) + 3613607633 (main.rs:4328) — "dump-exit on FIRST failure" doc [one finding]
> The comment says the main thread "dump-exits on the FIRST failure", but the code joins the scoped worker
> threads before replaying results. Failures are only acted on after all workers complete — different from
> "dump-exit" semantics; can mislead about failure latency/resource usage.

VERIFIED: the parallel ML-test sweep runs workers in `std::thread::scope` (closes/joins at main.rs:4324),
THEN an ordered replay loop (4326+) writes per-file results in file order and dump-exits on the first FAILED
slot. So both comments' "dump-exits on the FIRST failure" is true of the REPLAY but happens only AFTER all
workers finish — not first-failure-latency. Reword to "during the ordered replay after workers complete"
(the localization/log parity claim is correct; just the timing wording misleads). Doc-only.

## #3 — id 3613607652 (main.rs:5829) — ml_test_jobs test mutates process-global env unsafely [SUBSTANTIVE]
> This test mutates process-global env via `std::env::set_var`/`remove_var` (unsafe: concurrent env access
> is UB). Rust's harness runs tests in parallel by default, so the "SAFETY: single-threaded test" assumption
> isn't guaranteed. Refactor `ml_test_jobs` to take an injected override (so the test doesn't touch env), or
> guard all test env reads/writes with a shared lock + run serially.

VERIFIED: `ml_test_jobs_clamps_default_and_override` (main.rs:5825) does `unsafe { std::env::remove_var(
"CDZ_ML_JOBS") }` etc. with comment "SAFETY: single-threaded test." But cargo runs #[test]s in PARALLEL by
default, and OTHER tests in the file also touch env (`CDZ_ML_PER_FILE_TIMEOUT_SECS` at 5831) — so concurrent
env mutation (documented-UB in modern Rust, hence the `unsafe`) is NOT actually prevented. Real test-soundness
gap. Best fix (Copilot's first option): make `ml_test_jobs` take an injected override value so the test needn't
touch the environment at all; else a shared env-lock + serial run. → v-fleet-tooling.

## Owner
All `xtask/src/main.rs` = v-fleet-tooling. #1/#2 doc reword (one edit), #3 test-soundness refactor.
