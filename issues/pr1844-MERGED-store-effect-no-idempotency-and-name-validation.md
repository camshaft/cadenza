# PR #1844 review comments — cdz-kernel/src/kernel.rs (v-agent-harness) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1844 (MERGED — store/* effect dispatch → NameStore).

## 1. store/* mutations have NO idempotency/dedup → crash-recovery re-applies (duplicate entry) (Copilot, kernel.rs:682) — correctness/durability [VERIFIED]
> `store/*` effects mutate an external NameStore but have no idempotency mechanism like executor-side keys.
> A crash after `apply_store_effect` mutates but before a durable EffectResult → recovery re-drives the
> open dispatch and re-applies (store/set appends a duplicate, changing NameStore::history). Plumb
> idempotency_key into apply_effect + track applied keys, or log the mutation durably before/with it.
VERIFIED: the store dispatch does S1-latch (persist_error → not applied), then `apply_store_effect(&req)`,
then `record_result` (kernel.rs:678-690). But `apply_store_effect(&mut self, req: &EffectRequest)`
(:912) takes ONLY &req — NO idempotency_key, no dedup — even though an `idempotency_key` IS computed at
:663 (and passed to the durable Dispatched record, NOT to the store mutation). So a crash between the
mutation and the durable EffectResult → re-drive re-applies `store/set` → duplicate entry /
NameStore::history divergence. The S1 latch only guards persist-FAILURE, not re-drive-after-crash — same
crash-recovery class as #1668. MED-HIGH (durable-store correctness). Fix: plumb idempotency_key into
apply_store_effect/NameStore + dedup by applied-key (or make the mutation itself part of a durable log
before recording). RECOMMEND v-agent-harness treat as a recovery-correctness follow-up.

## 2. `apply_store_effect` ignores the decoded name + no family-specific payload rules (Copilot, kernel.rs:935) — correctness/validation [VERIFIED]
> `apply_store_effect` decodes a name-set payload but ignores the decoded `name` and doesn't enforce
> family-specific payload rules — silently accepts an inconsistent store/set (payload name != target), and
> relies on NameStore::apply_effect to reject malformed store/resolve (less precise errors).
VERIFIED: the decode is `Ok((_name, h)) => Some(h)` — the embedded `_name` is DROPPED, so a `store/set`
whose payload name ≠ `req.target` is silently accepted. Add: (1) store/set must have an inline name-set
payload whose embedded name matches req.target; (2) store/resolve must have NO payload. MED/validation.
Fix-forward.
