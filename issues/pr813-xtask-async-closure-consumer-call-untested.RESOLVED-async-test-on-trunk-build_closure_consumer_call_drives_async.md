# PR#813 review comment — xtask async closure-consumer-call path has no unit test (only sync covered)

Mirrored from GitHub PR review comment (Copilot), id `3634949454`.
PR: https://github.com/camshaft/cadenza/pull/813 (batch-staging; fix belongs on trunk)
Location: `xtask/src/main.rs:2353` (`build_closure_consumer_call` async branch)

## Comment (verbatim)

> The async-mode branch now synthesizes a fully-driven `{ let __gN = block_on(...); block_on(...) }`
> block for closure-parameter consumers, plus special-cases it in `run_program_rust` to avoid
> double-wrapping. There are existing unit tests for `build_closure_consumer_call` in sync mode, but
> none exercising this async path; adding a focused test would help prevent regressions in the
> env-threading / borrow-sequencing logic.

## Liaison verification (CONFIRMED on trunk)

- `build_closure_consumer_call` (xtask/src/main.rs:2173) gained an async-mode branch synthesizing the
  `{ let __gN = block_on(...); block_on(...) }` driven block, with a `run_program_rust`
  (xtask/src/main.rs:1443) special-case to avoid double-wrapping. Landed `8d43e2b03` ("rcdzc rust-async:
  drive closure-param consumers with a factory producer").
- There IS a sync unit test — `build_closure_consumer_call_synthesizes_the_producer_closure`
  (xtask/src/main.rs:6204) — but NO test exercising the async branch (the block_on double-drive +
  run_program_rust no-double-wrap interaction). Test-coverage gap on non-trivial env-threading /
  borrow-sequencing logic.

Fix: add a focused unit test for the async path (assert the synthesized block shape + that
run_program_rust doesn't double-wrap it). Test-only. Owner: v-rust-backend (owns the rcdzc rust
backend + its gate run-harness in xtask/src/main.rs; commit `8d43e2b03` is theirs). Routed as a note.
Minor (test hardening, no runtime defect).
