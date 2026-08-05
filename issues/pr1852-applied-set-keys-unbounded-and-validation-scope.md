# PR #1852 review comments — cdz-kernel/src/{name_store,kernel}.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1852 (§4c store/* idempotent set — the fix for my #1844
crash-recovery finding). The fix is right, but it introduced an unbounded set.

## 1. `applied_set_keys` grows UNBOUNDED — never pruned → memory leak / DoS vector (Copilot, name_store.rs:100) — correctness/resource [VERIFIED]
> `applied_set_keys` grows monotonically for every successful `store/set` and is never pruned →
> unbounded memory growth in long-lived sessions (and a DoS vector if store/set is broadly exposed).
> Dedup is only needed for re-driving OPEN dispatched-but-unsettled store effects, so scope retention to
> that window (track at the Session layer, remove once the EffectResult is durably recorded, or bound/
> expire).
VERIFIED on the cand branch: `applied_set_keys: HashSet<Hash>` (name_store.rs:103) is only ever
`.insert(idempotency_key)` (:203) — NO remove/prune/clear anywhere. So it accumulates one entry per
successful store/set forever → unbounded memory in a long-lived session. The dedup only needs to cover the
crash-recovery re-drive window (dispatched-but-unsettled), so a key can be dropped once its EffectResult is
durably recorded. Ironic follow-on: this is the set added to fix the #1844 idempotency gap — the fix is
correct, it just needs bounded retention. MED (resource/DoS). Fix-forward: prune on EffectResult (or bound
+ expire). RECOMMEND v-agent-harness scope key retention to the open-dispatch window.

## 2. Payload-name/target validation runs for ANY store family; mismatch branch is store/set-specific (Copilot, kernel.rs:931) — correctness/validation
> Payload-name/target validation runs for any store family with an inline payload, and the mismatched-name
> branch returns a "store/set"-specific error [even for a non-set family].
Gate the name==target validation to `store/set` specifically (a non-set family with an inline payload
shouldn't hit a store/set-worded error). LOW-MED. Fix-forward. (This is the #1844 c2 validation area —
tighten it per-family.)
