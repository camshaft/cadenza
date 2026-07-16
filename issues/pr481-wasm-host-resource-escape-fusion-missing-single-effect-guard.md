# PR #481 (merged, batch 110) — wasm host+resource-escape fusion path takes iface from host_imports[0] with no single-effect guard

Mirrored from Copilot inline on merged PR #481 (comment id 3596214934). Confirmed on trunk.
Owner: **v-effects** (host-composition / resource-escape envelope fusion).

## Finding (wasm/mod.rs:1984)
> In the host+resource-escape fusion path, `iface` is taken from `host_imports[0]` but (unlike the
> non-resource host envelope path earlier in this module) there is no check that all `host_imports`
> share the same effect. `assemble_host_runtime_resource` imports exactly one host interface
> instance, so multiple distinct host effects here would be mis-serialized (ops from different
> effects would be treated as one interface). Add an explicit single-effect guard and decline with
> the same message shape used elsewhere.

Trunk `implementation/seed/crates/rcdzc/src/backend/wasm/mod.rs:1984`:
`let iface = host_imports[0].effect.clone();` — takes the effect from element 0 unconditionally. The
non-resource host-envelope path earlier in the same module does verify a shared effect before
committing to one interface instance; this resource-escape branch does not.

## Impact
A program that reaches the resource-escape fusion path with host ops from >1 distinct effect would
emit a single wasm interface conflating them — a silent mis-serialization rather than a clean decline.
This is exactly the host-composition-invariant surface v-effects owns (see the effect-decline pins).

## Suggested fix
Add a single-effect guard over `host_imports` in the resource-escape branch (all share
`.effect`), declining with the same message shape the non-resource envelope path uses when the
invariant is violated. Add a decline test for the multi-effect resource-escape case.

PR: https://github.com/camshaft/cadenza/pull/481
