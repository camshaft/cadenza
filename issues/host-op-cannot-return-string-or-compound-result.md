# rcdzc feature-gap: a HOST op cannot RETURN a String or compound result

**Filed by:** v-agent-harness, 2026-07-16 (at the concierge's direction — the gap is real and broader
than Bedrock). **Kind:** honest feature-limitation (a deliberate `(declines)`, NOT a soundness bug).
**Owner seam:** v-peer-linking / cross-component-interop (the host-result ABI is theirs; this is "Route
A" in `implementation/design/DESIGN-agent-harness.md`).

## The gap

A genuine HOST-delegated effect op (`(host (E) …)` → `Core::HostCall`, NOT a peer-bound effect) can
today take a `String`/scalar ARGUMENT and a scalar RESULT, but **cannot return a `String` or any
compound value.** The compiler emits an honest decline instead of a component whose WIT import it can't
form.

- `backend/wasm/host.rs::abi_val_type` (line 59) maps ONLY the aliased scalars (Bool/Char/f32/f64/
  s8…u64). `Ty::String` and every compound (Tuple/Record/Sum/List/Map/Set/Bytes/BigInt/Rational) fall
  through to `_ => None`.
- `backend/wasm/host.rs::first_unrepresentable_host_op` (~line 636) therefore flags a non-scalar
  non-Unit host result as unrepresentable: *"a `String`/`list<u8>` result needs the memory +
  list-lifting envelope the closure-`Bytes` path has but the plain host envelope does not (a later
  increment)."* → `(declines)`.

## Why it is NOT already covered

- A **PEER-bound** effect (`db.effect_bindings`) crosses a compound as its opaque `u32` runtime handle
  via `extern_abi_val_type` — so a Cadenza *peer* op returning a `String` already works
  (`is_extern_heap_type` covers String/compound). The gap is host-path-only.
- A **string ARGUMENT** to a host op already works (`HostParam::Str`, the `(ptr,len)` lift +
  `set_needs_memory`). So the machinery to read a string out of linear memory across the host boundary
  partly exists on the argument side; the RESULT side (lifting a guest-produced string/list back out to
  the host) is what's missing.

## Why it matters (beyond Bedrock)

- **Bedrock-direct (the immediate forcing case):** a model call is fundamentally `(String prompt ->
  String completion)`. A host-op Bedrock binding declines today. (The agent-harness vertical works
  around it for bring-up via Route B — model Bedrock as a Cadenza *peer* with a SigV4 shim, which uses
  the handle transport — but the durable, fully-in-Cadenza form needs this host-result widening.)
- **Any host capability that returns text/bytes:** read-file, HTTP fetch, env lookup, a tool that
  returns a JSON string, `getenv`, `now() -> String`, etc. This is a general host-boundary limitation,
  not a Bedrock quirk.

## Repro (once a String-result host op is expressible, this should compile+run)

```
(effect Model (op complete (-> String String)))
(def (main (: prompt String)) (host (Model) (Model.complete prompt)))
(export main)
```
Today: declines with `first_unrepresentable_host_op` naming `complete`'s `String` result.
Contrast the PEER path, which already works:
```
(effect Model (op complete (-> String String)))
(bind Model "cadenza:model/api")     ; peer-bound → crosses by handle, no decline
(def (main (: prompt String)) (host (Model) (Model.complete prompt)))
(export main)
```

## Fix sketch (for the peer-linking owner)

Give the plain host RESULT path the same memory + list-lifting envelope the closure-`Bytes` escape and
the peer handle transport already use: on a `String`/`list<u8>` host result, thread the shared-memory
module + a Memory canon-option so the guest-produced `(ptr,len)`/list is lifted out to the host at the
boundary (the inverse of the existing `HostParam::Str` argument lower). Widen `abi_val_type` (or add a
host-result-specific predicate) to admit `String`/`Bytes` (and, later, richer compounds), and drop the
corresponding `first_unrepresentable_host_op` decline. Gate: a `.sexp` case with a String-result host op
that runs to a value (via a recorded `(host-responses …)` string), plus an rcdzc unit test.

## Cross-refs

- Design: `implementation/design/DESIGN-agent-harness.md` §2 (the gap analysis), §7 Inc-1′ (Route A).
- Spike memory: this is the "STILL OPEN" constraint the CodeAct spike flagged
  (`[[cadenza-agent-harness-codeact-spike]]`).
- Spec: `spec/contracts/host-interface-binding.md#a-host-import-is-a-wit-typed-function-the-manifest-enumerates`
  (the reject-rather-than-emit-a-mismatched-import contract the decline honors).
