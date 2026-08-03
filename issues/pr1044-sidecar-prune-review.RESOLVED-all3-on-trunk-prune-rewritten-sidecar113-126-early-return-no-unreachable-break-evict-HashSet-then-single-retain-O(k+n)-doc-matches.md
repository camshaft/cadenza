# PR #1044 review comments — fleet/slack-bridge/src/sidecar.rs (v-slack-bridge)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1044
(PR: "cand: v-slack-bridge — sidecar.rs"). Three actionable points on the new `prune` logic.

## 1. Unreachable `break` in `prune` (amazon-q-developer[bot], sidecar.rs:111) — 🛑 logic
> The loop condition checks `by_thread.len() > MAX_THREADS` but then breaks if no keys exist,
> which can never happen when the length is greater than zero. The `break` statement is
> unreachable. If `by_thread` has any entries (which it must to satisfy the while condition),
> `keys().next()` will always return `Some`. This creates unnecessary complexity and suggests a
> misunderstanding of the invariant.

Suggested simplification:
```rust
            // First key = oldest thread_ts.
            if let Some(oldest) = self.by_thread.keys().next().cloned() {
                self.by_thread.remove(&oldest);
                self.by_key.retain(|_, ts| *ts != oldest);
            }
```

## 2. O(k·n) prune (Copilot, sidecar.rs:115) — perf
> `prune` currently calls `by_key.retain(...)` inside a `while` loop, which makes pruning
> potentially O(k*n) (scan the whole `by_key` map once per evicted thread). If the on-disk map
> ever grows large, the first prune can become unnecessarily expensive. You can evict all excess
> threads first, then do a single `retain` pass over `by_key`.

## 3. Doc-comment vs code mismatch (Copilot, sidecar.rs:36) — doc
> The MAX_THREADS doc comment says the outbound loop both `load`s and `save`s the whole map every
> couple seconds, but the current outbound tick path loads every tick and only saves after a
> successful mirror post. Tweaking this wording will avoid misleading future maintainers about
> where the repeated serialization cost actually comes from.

---
Dismissed as nit (not filed): typo in test comment "monotically" → "monotonically" (sidecar.rs:318, Copilot).
