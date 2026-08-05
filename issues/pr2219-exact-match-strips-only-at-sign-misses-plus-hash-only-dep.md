# PR #2219 review — cdz-kernel (v-agent-harness) — OPEN — 1 correctness (LOW-MED) + 1 doc (LOW) [VERIFIED] (fold of MY #2208 c2/c3)

https://github.com/camshaft/cadenza/pull/2219 (fold #2208 review nits — c2 bare-name exact runtime-dep
match + c3 b1-e2e CDZ_STORE doc; implements MY #2208 c2/c3). Copilot 2 inline — one deepens the c2 fix.

## the exact-match fix strips only at `@` (`split('@').next()`), but `declared_deps` recognizes a dep by its `+<hash>` suffix even WITHOUT an `@<semver>` → `cadenza:runtime/heap+<hash>` (no `@`) fails the match → wrongly hits "no resolved runtime dep" (Copilot, wasm_host.rs:1525) — correctness [VERIFIED, LOW-MED]
> The runtime-dep identity check only strips the `@...` suffix. Since `declared_deps()` treats any import
> with `+<hash>` as a dep (even if it lacks an `@<semver>` segment), an import name like
> `cadenza:runtime/heap+<hash>` would no longer be recognized as the runtime and would trigger the "no
> resolved cadenza:runtime/heap dep" error. Consider stripping `+<hash>` first (and then `@...`).

VERIFIED against source. The #2219 c2 fix: `if import_name.split('@').next() == Some("cadenza:runtime/
heap")` (diff:15) — strips only at `@`. But `declared_deps` (wasm_host.rs:358) recognizes a dep via
`name.rsplit_once('+')` — it keys on the `+<hash>` suffix and does NOT require an `@<semver>` segment. So a
declared dep `cadenza:runtime/heap+<hash>` (no `@`) is valid, but `split('@').next()` leaves it as
`cadenza:runtime/heap+<hash>` ≠ `"cadenza:runtime/heap"` → the runtime dep is NOT matched → wrongly falls
through to the "no resolved cadenza:runtime/heap dep" error. So my #2208 c2 refinement (exact bare-name
match) is ITSELF incomplete — it fixed the `heap2@…` over-match but introduced an under-match for the
`+<hash>`-without-`@` form. LOW-MED/correctness. Fix per Copilot: strip `+<hash>` FIRST, then `@` — e.g.
`import_name.rsplit_once('+').map_or(import_name, |(i,_)| i).split('@').next()`, OR mirror `declared_deps`'
`rsplit_once('+')` exactly so the SELECTION and the PARSER agree. (Note wasm_host.rs:436 `dep_iface_name =
import_name.split('@').next()` has the SAME `@`-only strip — worth the same fix if it feeds the same
matching.)

## the module CDZ_STORE doc now correctly says `<hash>.wasm`, but the lower `resolve_runtime_deps` doc still says `DiskBlobStore`/`+<hash>` → internal inconsistency (Copilot, reducer_cadenza_b1_e2e.rs:22) — doc [VERIFIED, LOW]
> The updated module-level doc now correctly describes `CDZ_STORE` as `<hash>.wasm` (v-nix
> componentStore), but the `resolve_runtime_deps` doc comment below still says the dep is looked up via a
> `DiskBlobStore` by `+<hash>`. Updating that lower doc comment too would keep the file's documentation
> internally consistent.
VERIFIED: my #2208 c3 fix updated the MODULE doc to `<hash>.wasm` (diff:31), but the diff shows the lower
`resolve_runtime_deps` doc still references `DiskBlobStore` (diff:28 region). So the file now has two
inconsistent store-layout descriptions. LOW/doc (the c3 fix was partial — module doc fixed, the sibling
`resolve_runtime_deps` doc not). Fix: update the `resolve_runtime_deps` doc to match `<hash>.wasm`.

Both foldable pre-merge. c2 is the one that matters (a real under-match introduced by the c2 refinement).
v-agent-harness owns cdz-kernel/src. (Owning the chain: my #2208 c2/c3 → this fold; c2's exact-match needs
to also strip `+<hash>`, c3's doc-fix missed the sibling comment. One-layer-deeper on both.)
