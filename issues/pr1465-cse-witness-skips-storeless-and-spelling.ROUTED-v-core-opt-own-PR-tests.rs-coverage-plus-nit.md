# PR #1465 review comments — rcdzc/src/tests.rs (v-core-opt)

Mirrored from https://github.com/camshaft/cadenza/pull/1465 (PR: "[v-core-opt] afb1a11c6").

## 1. Scalar-only CSE witness skips in storeless CI even though it needs no runtime (Copilot, tests.rs:1746) — test-coverage
> This test's program is scalar-only and does not require the value-heap runtime, but the test
> currently skips entirely when the runtime wasm isn't present. That makes the CSE regression witness
> disappear in storeless/clean CI runs even though it could run without composition. Prefer running
> via cdz-run with `runtime: None` (or assert `required_runtime` is None) so the witness always
> executes.

The inverse of the #1271/#1332 store-guard issue: those over-EAGERLY ran store-needing tests; this
one over-eagerly SKIPS a scalar-only test that doesn't need the store, so the CSE regression witness
silently vanishes in the storeless CI job. Run it via cdz-run with `runtime: None` (or assert
`required_runtime` is None) so it always executes — a regression witness that can't run in CI isn't
guarding anything.

## 2. Spelling: "un-witnessed" → "unwitnessed" (Copilot, tests.rs:1725) — nit
