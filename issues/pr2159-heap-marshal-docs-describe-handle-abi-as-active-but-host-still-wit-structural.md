# PR #2159 review — cdz-kernel (v-agent-harness) — OPEN — doc-accuracy/staging [VERIFIED, LOW] (batched, 2 sites)

https://github.com/camshaft/cadenza/pull/2159 (reducer-boundary INPUT marshalling heap_marshal — build the
apply(u32,u32,u32) args on the value-heap). Copilot 2 inline, SAME finding across 2 files → batched.
Config/doc-ahead-of-wiring staging class (cf #1981/#2076/#2105).

## the new heap_marshal + crate docs describe the handle-ABI marshalling ("host marshals inputs INTO value-heap handles before the call") as the ACTIVE boundary, but the host still calls the WIT-structural `fold.apply` (call_apply_async with ContentType/Option<&[u8]>) → reads as an always-on requirement that isn't wired yet (Copilot, lib.rs:42 & heap_marshal.rs:7) — doc-accuracy/staging [VERIFIED, LOW]
> [lib.rs:42] the host "marshals … INTO value-heap handles … before the call", but the current reducer
> invocation still calls the WIT-structural `fold.apply` … misleading unless/until the host is switched
> to the handle-based apply path.
> [heap_marshal.rs:7] docs assert rcdzc lowers `fold.apply` to `apply(u32,u32,u32)->u32` and "the HOST
> must marshal" … but the host calls the WIT-structural `fold.apply` (component bindgen) with ContentType
> and Option<&[u8]> … reword as a handle-ABI *mode*/helper rather than current always-on behavior.

VERIFIED: the new docs (heap_marshal.rs:13-14 "So the HOST must marshal the kernel's (content_type,
payload, resumes) fold inputs INTO value-heap handles before the call"; lib.rs:293 same) describe the
handle-ABI as the active call boundary. But wasm_host.rs is still on the WIT-STRUCTURAL bindgen path:
it uses wasmtime `bindgen!` exporting `fold.apply`, `ContentType` (wasm_host.rs:29), and
`call_apply_async` (wasm_host.rs:44-54) — NOT the `apply(u32,u32,u32)` handle-lowered path. So heap_marshal
is a helper for a FUTURE mode (the option-C handle-ABI lowering) that isn't the live boundary yet — the
docs state it as an unconditional current requirement. LOW/doc-accuracy (same staging class as prior
config-ahead-of-wiring findings — the code is fine, the module is real + correct as a helper; only the
"the host must marshal … before the call" framing implies it's the active path). Fix per Copilot: reword
the intro to describe this as the handle-ABI MODE / marshalling helper (what it does + when it becomes the
boundary), not the always-on current behavior — e.g. "when the reducer is invoked via the option-C
handle-lowered `apply(u32,u32,u32)`, the host marshals inputs into value-heap handles using this module;
the current path is the WIT-structural `fold.apply` bindgen." v-agent-harness owns cdz-kernel/src. PR OPEN
→ foldable pre-merge.
