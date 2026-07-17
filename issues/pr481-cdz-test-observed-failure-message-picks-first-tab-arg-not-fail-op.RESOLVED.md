# PR #481 (merged, batch 110) — cdz test `observed_failure_message` returns the FIRST tab-arg host call, not the `.fail` op

Mirrored from Copilot inline on merged PR #481 (comment id 3596215018). Confirmed on trunk.
Owner: **v-cdz-tooling** (`cdz` CLI test harness).

## Finding (cdz/src/main.rs:2246)
> `observed_failure_message` currently returns the message from the *first* observed host call that
> has a tab-delimited string argument. But `cdz_run::run_capturing` appends `\t...` for *any* host
> call with string args (e.g. `log.emit("...")`, not just `*.fail`). This can cause `cdz test` to
> report a non-failure string (or an earlier log line) as the assertion message. Filter to entries
> whose op label ends with `.fail` (covers `test.fail` / `report.fail`) before extracting the message.

Trunk: `fn observed_failure_message(observed: &[String])` does
`.iter().find_map(|entry| entry.split_once('\t').map(|(_op, msg)| msg.to_string()))` — it drops the
op label (`_op`) and takes the message of the first tab-carrying entry, whatever op it came from.

## Impact
A test that does `log.emit("progress...")` before a failing assertion would report the log string as
the failure message instead of the actual `Test.fail(...)` text — wrong/misleading `cdz test` output.

## Suggested fix
Filter to entries whose op label ends with `.fail` (covers `test.fail` / `report.fail`) BEFORE
extracting the message — i.e. `split_once('\t')` then check `op.ends_with(".fail")`. Add a test with a
`log.emit` preceding a `Test.fail` to pin the correct message is chosen.

## Related light perf nit (same file, comment 3596215046, cdz/main.rs:2202)
`run_one_trial_with_pool` does `runtime: runtime.map(<[u8]>::to_vec)` — clones the full runtime
component bytes on EVERY property-test trial though the runtime is identical across trials. Consider a
shared buffer (`Arc<[u8]>` in `RunOpts`) or a borrowing entrypoint so `cdz test` reuses one allocation.
Optional; correctness item above is the priority.

PR: https://github.com/camshaft/cadenza/pull/481

---
RESOLVED (corpus-bugfix 2026-07-17, trunk@1c255812b): already fixed. `observed_failure_message`
(implementation/seed/crates/cdz/src/main.rs:3078) now filters entries whose op label
`.ends_with(".fail")` (case-insensitive) AND iterates `.rev()` — exactly the recommended fix. No
longer picks the first tab-carrying entry regardless of op. Stale finding.
