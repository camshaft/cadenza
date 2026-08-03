# PR #1487 review comments — cdz-kernel/src/{kernel,reducer,executor}.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1487 (PR: "[v-agent-harness] 8961aea80").
Fallout from the AsyncReducer/AsyncExecutor -> Reducer/Executor consolidation (follows #1351/#1360).

## Residual "async" test names + "an Reducer" grammar after the rename (Copilot) — naming/grammar
- **kernel.rs:559 (also :720)** — grammar: "an [`Reducer`]" → "a [`Reducer`]" (2 sites).
- **reducer.rs:178** — test still named `dyn_async_reducer`, but the trait is now `Reducer`; rename so
  grepping for async-specific behavior isn't misled.
- **executor.rs:212** — test still named `dyn_async_executor`, but the trait is now `Executor`;
  rename to match the API.

All consistency cleanup from the async→sync trait consolidation: fix the "an Reducer" article at
the 2 kernel sites, and rename the two `dyn_async_*` tests to drop the stale "async" now that the
traits are `Reducer`/`Executor`.
