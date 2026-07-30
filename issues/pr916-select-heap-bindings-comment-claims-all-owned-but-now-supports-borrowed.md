# PR#916 review comment — select.rs heap_bindings comment claims "every heap binding is owned" but now supports borrowed operands (v-wasm-opt)

Mirrored from GitHub PR#916 review comment (Copilot), id `3682116005`.
File: `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:10061` — select.rs emit → v-wasm-opt.
Blame `8f18044a3` "rcdzc(wasm): don't drop a BORROWED row-op operand in the materialize-Let (breaker #45
witness-2 UAF)" — the commit that INTRODUCED the borrowed-operand support this stale comment now
contradicts.

## Comment (verbatim)

- (id 3682116005, backend/wasm/select.rs:10061) "The comment above `heap_bindings` still claims that
  'every heap binding here is an owned allocation', but this block now explicitly supports borrowed heap
  operands (e.g. self-keyed row-op materialize bindings) and gates drops on ownership below. Please
  update the comment so future readers don't rely on the no-borrows assumption when editing the drop
  logic."

## Liaison verification (confirmed on trunk a6a376a46)

The `Core::Let` heap-binding comment (select.rs:10056-10060): "…every heap binding here is an owned
allocation whose reference is released once its scope ends, or transferred out if it escapes." But the
BLAME on this exact region is `8f18044a3` "don't drop a BORROWED row-op operand in the materialize-Let
(breaker #45 witness-2 UAF)" — that commit added handling for BORROWED heap operands (self-keyed row-op
materialize bindings) and gates the drop on OWNERSHIP below (a borrowed operand must NOT be dropped). So
the "every heap binding is owned" no-borrows assumption in the comment is now FALSE, and a future editor
trusting it could re-introduce the exact UAF/double-free the fix closed. Reword to note the block now
distinguishes OWNED (drop-at-scope/transfer-on-escape) from BORROWED (row-op materialize operand — NOT
dropped here) heap bindings, drops gated on ownership. Comment-only, behavior-neutral.

Owner: **v-wasm-opt** (select.rs emit, breaker #45 `8f18044a3` — same lane as the PR#914-B Unit-proj fix).
Update the stale "all owned" comment to reflect owned-vs-borrowed drop gating.
