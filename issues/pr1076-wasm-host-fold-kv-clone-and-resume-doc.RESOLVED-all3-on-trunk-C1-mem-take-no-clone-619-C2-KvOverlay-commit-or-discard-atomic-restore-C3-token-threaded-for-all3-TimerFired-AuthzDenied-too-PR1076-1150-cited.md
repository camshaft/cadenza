# PR #1076 review comments — cdz-kernel/src/wasm_host.rs (v-agent-harness)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1076
(PR: "cand: v-agent-harness — wasm_host + component_reducer_e2e").

## 1. `fold` clones entire KV per event → O(KV size) per fold (Copilot, wasm_host.rs:377) — perf
> `fold` currently calls `self.apply(kv.clone(), ...)`, but `Kv` is backed by a
> `BTreeMap<Vec<u8>, Vec<u8>>` so `clone()` deep-copies the entire session state every event. This
> will make the wasm reducer path O(KV size) per fold and can dominate runtime for non-trivial
> sessions. Consider changing the `apply` API/host state so the fold can run against the session KV
> without cloning (e.g., have `apply` take/return ownership in a way that allows restoring on error,
> or accept a mutable KV handle).

## 2. "KV untouched on error" claim vs upfront clone/overwrite (amazon-q, wasm_host.rs:376) — correctness
> Cloning KV before passing to `apply` but then unconditionally overwriting it on success means
> failed folds could still show partial KV mutations if `apply` modifies the clone before returning
> an error. The comment states "KV untouched" on error but the implementation clones upfront,
> potentially exposing inconsistent state if `apply` errors after KV operations.

(These two are related — both about the clone-then-apply pattern. amazon-q suggests naming the clone
`kv_snapshot` and matching on `apply(kv_snapshot, ...)`. Worth reconciling the error-atomicity claim
with the chosen ownership model while addressing the perf point.)

## 3. Resume-token doc inconsistent with `event_to_guest_inputs` (Copilot, wasm_host.rs:363) — doc
> The correlation doc comment here says that on "result/timer/denial" events the guest `resumes`
> value comes from the event's `token`, but the current `event_to_guest_inputs` implementation only
> threads `token` through for `EffectResult` and returns `resumes=None` for `TimerFired` and
> `AuthzDenied` (called out later in the same function). This makes the module-level documentation
> internally inconsistent and can mislead future work on the resume bridge.
