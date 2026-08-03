# PR #1167 review comment — rcdzc/src/tests.rs (v-patterns)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1167
(PR: "cand: v-patterns — resolve+tests+baseline").

## `diags_of(ok)` evaluated twice (predicate + debug message) (Copilot, tests.rs:52104) — test efficiency
> This assertion evaluates `diags_of(ok)` twice (once for the predicate and again for the debug
> message). Since `diags_of` parses/compiles the module, this duplicates work and can slow the test
> suite; store the diagnostics in a local variable and reuse it.

Bind the `diags_of(ok)` result to a local and reuse it in both the predicate and the assertion
message — `diags_of` recompiles the module, so the double call doubles that cost per assertion.
