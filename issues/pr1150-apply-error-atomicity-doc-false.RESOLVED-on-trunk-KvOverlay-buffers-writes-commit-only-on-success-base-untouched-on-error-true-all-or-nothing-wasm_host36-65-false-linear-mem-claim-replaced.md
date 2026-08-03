# PR #1150 review comments — cdz-kernel/src/wasm_host.rs + component_reducer_e2e.rs (v-agent-harness) — OPERATOR TOP PRIORITY

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1150
(PR: "cand: v-agent-harness — cdz-kernel (OPERATOR TOP PRIORITY)"). Follow-up to the #1076
error-atomicity flag — Copilot now says the atomicity claim is affirmatively FALSE, not just
unreconciled.

## 1. `apply` docs promise error-atomicity the KV-import model doesn't provide (Copilot, wasm_host.rs:318, also :405, :430) — correctness/doc
> The `apply` docs claim a trapped fold's partial KV writes "never reach" the returned `Kv` because
> the guest ran against a wasm-linear-memory copy. That's not accurate: the guest mutates the host
> KV via the imported `kv.put`/`kv.delete` functions (see `ReducerHost`'s `kv::Host` impl earlier in
> this file), so a trap/fuel-exhaustion can still leave partially-applied KV mutations in the host
> state. The docs should not promise error-atomicity unless the host KV import is made
> transactional/rollbackable.

## 2. e2e test comment overclaims general atomicity (Copilot, component_reducer_e2e.rs:185, also :186) — test/doc
> This test's doc comment currently claims `fold` avoids cloning and restores the *pre-fold* KV to
> guarantee KV error-atomicity. With the current host `kv` import semantics (directly mutating the
> host KV), this test only verifies that a failed fold doesn't clear the map for the specific
> guest+fuel behavior, not a general atomicity guarantee. Consider rewording the comment to only
> claim what's actually asserted (that the KV isn't left empty/cleared on failure), or add a fixture
> that performs a KV write before trapping to truly pin atomicity.

This is the important one this batch — it's on the OPERATOR TOP PRIORITY PR and is the direct
continuation of the #1076 error-atomicity concern (which was "reconcile the claim"). Copilot now
asserts the claim is FALSE: because the guest mutates host KV via `kv.put`/`kv.delete` imports, a
trap/fuel-exhaustion CAN leave partial mutations, so the docs must not promise error-atomicity unless
the KV import becomes transactional/rollbackable. Either (a) weaken the docs + test comment to what's
actually guaranteed, or (b) make the host KV import transactional if true atomicity is intended — and
add a write-then-trap fixture to actually pin it.
