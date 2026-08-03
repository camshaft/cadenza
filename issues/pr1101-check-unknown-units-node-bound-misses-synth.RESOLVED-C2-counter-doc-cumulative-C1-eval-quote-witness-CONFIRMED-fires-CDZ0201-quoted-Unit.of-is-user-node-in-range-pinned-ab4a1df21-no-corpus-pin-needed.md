# PR #1101 review comments — rcdzc/src/infer.rs + db.rs (v-compiler-perf)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1101
(PR: "cand: v-compiler-perf — db+infer+tests (oldest, 114min-flagged)").

## 1. ⚠ Bounding scan to `user_node_count` drops CDZ0201 for `Unit.of` in synthesized nodes (Copilot, infer.rs:7721) — CORRECTNESS
> Bounding the scan to `user_node_count` means `check_unknown_units` will no longer see
> `(Unit.of ...)` occurrences that live in synthesized nodes grafted under user nodes (e.g. code
> reconstructed by `eval_ast::desugar_eval` uses `push_list`/`push_atom` to append fresh list/name
> nodes, and only overwrites the `(eval ...)` root id). If `Unit.of` is nested (e.g.
> `(eval (quote (Qty.of 5 (Unit.of #"zorks"))))`), the outer user node won't match `Prim::UnitOf`,
> and the nested `Unit.of` node id is > `user_node_count`, so CDZ0201 would no longer be produced
> (falling back to later generic failures).

This is the important one: a perf optimization (bounding the scan) may have narrowed a diagnostic's
reach. Verify whether `Unit.of` inside `eval`/quote-synthesized nodes still produces CDZ0201; if the
bound genuinely skips them, either the bound needs to cover grafted-under-user synth nodes or this is
a knowingly-accepted gap that should be documented + test-pinned.

## 2. Counter doc says "≤ user_node_count" but it's cumulative-since-reset (Copilot, db.rs:213) — doc
> The doc comment says the counter "stays ≤ `user_node_count`" even though it is defined as a
> cumulative total "since the last reset". If `diagnostics()`/`compile()` is run multiple times on
> the same thread without resetting the Cell, the counter will legitimately exceed `user_node_count`.

Doc-vs-behavior: either the invariant claim is wrong (it's cumulative) or a reset is missing on the
repeated-compile path. Given the known trap that per-compile metric counters must be per-Db /
thread_local and get contaminated across a parallel test harness, worth confirming the reset
discipline here.
