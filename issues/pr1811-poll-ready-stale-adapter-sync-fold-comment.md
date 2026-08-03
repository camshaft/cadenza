# PR #1811 review comment — cdz-kernel/src/reducer.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1811 (trait-rename beat T3 — DROP the _async aliases; the
async-collapse migration completing).

## `poll_ready` test-helper comment references the removed "adapter"/"sync fold" (Copilot, reducer.rs:189) — doc/accuracy [VERIFIED]
> The test helper comment is stale: it refers to an "adapter" and a "sync fold", but the sync/async
> dual-trait adapter has been removed and `Reducer::fold` is now the only method. This can mislead future
> readers about why `poll_ready` is safe here.
VERIFIED on the cand branch: the `poll_ready` helper comment (reducer.rs:186-188) says "the adapter's
`fold` wraps a sync fold, so it never returns Pending". After beat T3 dropped the dual-trait adapter,
there's no "adapter" or "sync fold" — `Reducer::fold` is the sole (async) method. So the comment's stated
RATIONALE for why polling once is safe is now inaccurate. Reword to the current reason (e.g. "the test
Reducer's `fold` completes synchronously — no real await — so the future is immediately ready"). LOW/doc —
async-collapse residual (same family as #1752/#1764). Fix-forward.
