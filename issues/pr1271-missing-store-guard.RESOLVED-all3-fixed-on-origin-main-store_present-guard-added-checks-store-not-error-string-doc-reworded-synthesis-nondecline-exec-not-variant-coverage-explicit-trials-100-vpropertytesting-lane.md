# PR #1271 review comment — cdz/tests/test_manifest_cli.rs (v-property-testing)

Mirrored from https://github.com/camshaft/cadenza/pull/1271 (PR: "cand: v-property-testing — 76ed3cd63").

## Missing runtime-store guard → spurious failure in storeless CI (amazon-q, test_manifest_cli.rs:1528) — CI/correctness
> Missing Runtime Store Guard: This test runs property tests that execute sum-type bodies under the
> runtime, which requires the value-heap store. Without a store guard, this will spuriously fail in
> storeless CI environments (the `test` job that runs `cargo test --workspace` without `cargo xtask
> build`).
> [suggests adding the `if !store_present() { … return; }` skip at the top, matching lines
> 890-895, 1064-1069, 1136-1141, etc.]

This is a real one: the new property test executes `@test` bodies under the runtime (needs the
value-heap store), but the storeless CI `test` job runs `cargo test --workspace` WITHOUT `cargo xtask
build`, so it'll hit `no runtime of content address <hash>` and fail spuriously. Add the
`store_present()` guard at the top matching the sibling runtime-executing tests in this file.
(NB: the guard should check the STORE, not match on a run-error string — that's the correct
storeless-skip pattern.)

## 2. (later Copilot inlines) Doc/trial-count accuracy on the same test
- **:1525** — the doc comment claims the test "witnesses generation across all variants", but the
  property bodies always succeed and don't verify variant coverage. Reword to what it actually
  asserts (generator synthesis + non-decline + successful execution).
- **:1545 (also :1552)** — the comment/expectations say "100 trials", but the invocation relies on
  the CLI default trial count. Pass `--trials 100` explicitly so the test's intent is stable if the
  default changes.
