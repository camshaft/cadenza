# PR #2166 review — cdz-kernel (v-agent-harness) — OPEN — doc-precision [VERIFIED, LOW] (batched, 2 sites)

https://github.com/camshaft/cadenza/pull/2166 (reducer-boundary READ marshalling heap_unmarshal + #2159
doc-staging fix-forward). Copilot 2 inline, SAME finding across 2 files → batched.

## the docs say heap_unmarshal projects the handle back into "the kernel's `Vec<EffectRequest>`", but the crate has TWO `EffectRequest` types — `crate::effect::EffectRequest` (kernel's public struct) vs the WIT-generated one re-exported from `crate::wasm_host` (which heap_unmarshal actually returns) → ambiguous, could mislead the fold-path wiring (Copilot, heap_unmarshal.rs:8 & :22, lib.rs:48) — doc-precision [VERIFIED, LOW]
> [heap_unmarshal.rs:8] the host projects the returned handle back into the kernel's `Vec<EffectRequest>`,
> but `EffectRequest` here is the WIT-generated type re-exported from `crate::wasm_host` (not the kernel's
> public `crate::effect::EffectRequest`). Qualifying the type in the docs would avoid confusion …
> [lib.rs:48] there are two `EffectRequest` types in this crate … Qualifying which one is meant … would
> reduce ambiguity.

VERIFIED both types exist:
- `crate::effect::EffectRequest` — the kernel's public struct (`pub struct EffectRequest`, effect.rs:210,
  with `new`/`new_with_family`, the #1563/#1722 zero-alloc shape).
- the WIT-generated `EffectRequest` re-exported via `crate::wasm_host` — and heap_unmarshal.rs actually
  imports + returns THAT one: `use crate::wasm_host::{ComponentError, EffectKind, EffectRequest,
  HeapHandle};` (#2166 diff:66), `-> Result<Vec<EffectRequest>, ComponentError>` (diff:154).
But the module doc (diff:41-42) says "the host projects that handle back into the kernel's
`Vec<EffectRequest>`" — "the kernel's" reads as `crate::effect::EffectRequest`, while the code returns the
WIT one. LOW/doc-precision (no code bug — the code is unambiguous; the DOC's "the kernel's" qualifier is
the misleading part, especially since this bridges toward the fold path where BOTH types are in scope).
Fix per Copilot: qualify the type in the docs — say the WIT-generated `wasm_host::EffectRequest` (the
component-boundary type), and if a later slice converts to `crate::effect::EffectRequest`, note that
conversion explicitly. v-agent-harness owns cdz-kernel/src. PR OPEN → foldable. (This PR also carries my
#2159 doc-staging fix-forward — good; this is the read-direction sibling with its own doc-precision nit.)
