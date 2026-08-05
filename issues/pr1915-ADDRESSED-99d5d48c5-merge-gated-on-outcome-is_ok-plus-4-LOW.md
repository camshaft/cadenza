# PR #1915 review comments — cdz-agent-host/src/host.rs + tests (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1915 (shared-store host — merge-back).

## 1. Store merged back into canonical even when `deliver` returns Err(KernelError) → invalid state leaks across sessions (Copilot, host.rs:365) — correctness [VERIFIED]
> The host merges a session's store back into the canonical store even when `deliver` returns
> `Err(KernelError)`. A KernelError indicates session/log corruption or invalid state — folding writes into
> the canonical store can leak partial/invalid state across sessions. Only merge back on a successful turn.
VERIFIED on the cand branch: `let outcome = s.deliver(...).await;` then UNCONDITIONALLY `if let Some(canonical)
= &mut self.canonical { if let Some(session_store)=... { canonical.merge_appends_from(session_store) } }`,
regardless of `outcome`, before `Some(outcome)`. So a turn that returned Err(KernelError) (corruption /
invalid state) STILL merges the session's name-store appends into the canonical shared store → partial/
invalid writes leak to the next-spawned session. MED/correctness. Fix: gate the merge-back on
`outcome.is_ok()` (only fold a successful turn's writes). Recommend v-agent-harness-host confirm the
KernelError-during-partial-write semantics.

## 2. `spawn` doc says a session keeps its own store, but the impl always attaches a shared/canonical store (Copilot, host.rs:330) — doc/behavior
Reconcile: either spawn honors an own-store request, or the doc should say the host always attaches the
canonical-backed store. LOW-MED (verify intended spawn-store semantics).

## 3. Share-less regression asserts `resolved` absent from KV but not the full claim (Copilot, shared_store_host_e2e.rs:190) — test-coverage
The share-less host regression only checks `resolved` is absent from KV; tighten to verify the actual
"no cross-session leak" claim (e.g. the canonical store didn't gain the entry). LOW/test-coverage.
