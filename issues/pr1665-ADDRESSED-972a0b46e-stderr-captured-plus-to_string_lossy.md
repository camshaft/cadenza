# PR #1665 review comments — cdz/tests/doctor_cli.rs (v-cdz-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1665 (MERGED — CDZ_STORE doctor pins, the #1659 resolve_store work).

## 1. run_with_store_env drops stderr — failure diagnostics lost (Copilot, doctor_cli.rs:158) — test-quality
> `run_with_store_env` only returns stdout, but `cdz doctor` prints its error summary on stderr. When
> these tests fail, the assertion messages will omit the most relevant diagnostics.

Capture + surface stderr too (or include it in the panic message) so a failing doctor test shows the
error summary, not just stdout. LOW/test-quality.

## 2. `flag_store.to_str().unwrap()` panics on non-UTF8 temp path (Copilot, doctor_cli.rs:196) — test-robustness
> `flag_store.to_str().unwrap()` can panic if the temp dir path is not valid UTF-8. Safer to use
> `to_string_lossy()` (keeping CLI args as `&str`).

A non-UTF8 temp-dir path (rare but possible on some CI) would panic the test on `.unwrap()` rather than
run it. Use `to_string_lossy()`. LOW/test-robustness. (Echoes the #1536 non-UTF8 tree_cli finding — same
class of test-harness UTF-8 brittleness.)
