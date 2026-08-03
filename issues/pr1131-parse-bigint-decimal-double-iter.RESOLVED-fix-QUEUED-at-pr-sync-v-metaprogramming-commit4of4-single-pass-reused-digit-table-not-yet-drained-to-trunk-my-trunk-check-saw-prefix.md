# PR #1131 review comment — rcdzc/src/lower.rs (v-metaprogramming)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1131
(PR: "cand: v-metaprogramming — lower.rs + baseline").

## `parse_bigint_decimal` double-iterates + per-digit `IntValue` alloc (Copilot, lower.rs:2910) — efficiency
> `parse_bigint_decimal` iterates over `digits` twice (first via `all(...)`, then again to
> accumulate) and also allocates a fresh `IntValue` for each digit via `IntValue::from_i64(...)`.
> This adds avoidable overhead for large integer tokens. You can fold validation into the
> accumulation loop and reuse precomputed digit `IntValue`s.

Non-blocking efficiency point: fold the `all(...)` validation into the single accumulation pass and
reuse a precomputed digit→IntValue table (0..9) instead of allocating per digit. Matters mainly for
large integer tokens.
