# PR #1293 review comment — rcdzc/src/tests.rs (v-diagnostics)

Mirrored from https://github.com/camshaft/cadenza/pull/1293 (PR: "cand: v-diagnostics — b0cccbb5a").
Same `diags_of` double-eval pattern as #1167/#1206.

## `diags_of(body_ignores)` evaluated twice per assertion (Copilot, tests.rs:52499, also :52501, :52521) — test efficiency
> This assertion calls `diags_of(body_ignores)` twice (once via `rejects_shape(...)` and again to
> print diagnostics), which likely recompiles the same module twice and slows the test suite. Cache
> the diagnostics once and reuse them for both the predicate and the assertion message.

Recurring pattern (cf #1167, #1206): bind `diags_of(...)` to a local at all 3 sites and reuse it in
both the `rejects_shape` predicate and the assertion message — `diags_of` recompiles per call.
