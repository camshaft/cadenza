# PR #1251 review comment — rcdzc/src/lower.rs (v-metaprogramming)

Mirrored from https://github.com/camshaft/cadenza/pull/1251 (PR: "cand: v-metaprogramming — 5721c9905").
Direct follow-on to my #1131 `parse_bigint_decimal` efficiency note — the fix has a nuance.

## Precomputed digit table built on every call, not just i64-overflow (Copilot, lower.rs:2908) — efficiency + doc
> `parse_bigint_decimal` is called for any `tok.parse::<i64>()` error (including floats/names), so
> precomputing `ten` + the 0..=9 `IntValue`s on every call adds fixed allocation work even when the
> function quickly returns `None` for non-integer tokens. The comment here also implies this path
> only runs on i64 overflow, which isn't true given the call site.
> Consider caching these small `IntValue`s in `OnceLock` so the setup cost is paid once per process
> (and adjust the comment accordingly).

Nuance on the #1131 fix: precomputing the 0..9 table beat per-digit alloc, but since
`parse_bigint_decimal` runs on ANY `parse::<i64>()` failure (floats, names — not just overflow), that
setup alloc now happens on every non-integer token that fast-returns `None`. Move the `ten` + 0..=9
`IntValue`s into a `OnceLock` (built once per process), and fix the comment that implies this path is
i64-overflow-only.
