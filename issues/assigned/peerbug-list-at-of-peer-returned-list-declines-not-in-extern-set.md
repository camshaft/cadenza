# Bug: `List.at` of a peer-returned List declines "peer-bound effect op is not in the extern-import set"

**Filed by:** v-peer-linking (2026-07-16)
**Area:** rcdzc — cross-component / peer-linking emit (layout `extern_index` vs `collect_host_imports`)
**Severity:** a VALID program declines (safe — a compile error, NO bad wasm emitted); not a miscompile.

## Symptom
A consumer that binds a peer op returning a `(List Int64)` and reads an element with `List.at`
declines at emit:

```
cdz: error: a peer-bound effect op is not in the extern-import set
```

## Minimal repro
PROVIDER (`cdz compile prov.sexp --component-name cadenza:l/api -o prov.wasm`):
```
(do (def (mklist (: x Int64)) (list (+ x 1) (+ x 2))) (export mklist))
```
CONSUMER (declines):
```
(do (effect L (op mklist (-> Int64 (List Int64)))) (bind L "cadenza:l/api")
    (def (main (: x Int64)) (host (L) (List.at (L.mklist x) 0))) (export main))
```

## The tell — `List.len` WORKS, `List.at` does NOT
The SAME peer-returned list read with `List.len` compiles + runs correctly (main(7)→3):
```
(def (main (: x Int64)) (host (L) (List.len (L.dup x))))   ; dup returns (list x x x) → 3  ✓
```
Only `List.at (peer-list) idx` declines. A `List.at` on a LOCAL list works fine. Let-binding the
peer list first (`(let ((xs (L.mklist x))) (List.at xs 0))`) still declines. Both `List.len` and
`List.at` consumers import `cadenza:runtime/heap`, so it is NOT an envelope (extern-only vs
extern+runtime) difference.

## ROOT CAUSE (traced 2026-07-16 by v-peer-linking — NOT what the title suggests; NOT `List.at`-specific, NOT the reclamation dup)
The trigger is the ENTRYPOINT RESULT TYPE, not `List.at`. `List.at` returns `Option Int64`; when that
Option IS the whole entrypoint body, `main`'s result is a compound that ESCAPES via
`emit_runtime_resource` (mod.rs:1842, dispatched at mod.rs:441 for a runtime value-form result). That
emit path collects the RUNTIME ops (`collect_module_used_ops`) but **NEVER threads the peer
extern-import set** — it builds `layout.with_import_base(k+2)` and never calls `.with_extern_order(...)`,
so `extern_order` stays `[]`. Then select.rs:8132 `extern_index` returns `None` → the decline.

Proof (instrumented emit): `List.len` (scalar Int64 result) → normal `emit` → `extern_order=
[("cadenza:l/api","mklist")]` → works. `List.at` returning the raw Option → `emit_runtime_resource` →
`extern_order=[]` → declines. CONTROL that confirms the diagnosis: `(match (List.at …) ((Some v) v)
(None 0))` — element read but entrypoint returns a SCALAR — takes the normal `emit` path and WORKS
(compiles + runs, main(7)=8). So the real gap: **a peer-bound `Core::HostCall` in a body whose EXPORT
RESULT escapes as a runtime resource** (`emit_runtime_resource`, and likely `emit_recursive_sum_resource`
+ the closure-resource paths) has NO extern-import threading and NO extern×resource envelope.

## The real fix (a resource-escape × peer-envelope FUSION — bigger than a one-tick patch)
`emit_runtime_resource` (+ `emit_recursive_sum_resource`) must, like the main `emit`:
(1) `collect_host_imports` over the reachable bodies, (2) split peer-bound ops into `extern_imports`,
(3) `layout.with_extern_order(...)` + shift `import_base` past the extern ops, (4) assemble a component
composing the RESOURCE envelope WITH the peer extern envelope — a NEW fusion (`assemble_runtime_resource`
and `assemble_extern_runtime` are separate today; neither imports a peer interface AND publishes a
resource). (4) is the substantial part. Until then the shape is a clean DECLINE.

## Repro shapes
- WORKS (common case, PINNED as rcdzc test `an_element_of_a_peer_returned_list_is_read_and_used`):
  peer List result + `List.at` + match-to-scalar → entrypoint returns Int64 → normal emit → main(7)=8.
- WORKS (PINNED `a_peer_op_returning_a_list_crosses_and_its_length_is_read`): peer List + `List.len` → 3.
- DECLINES (the gap): entrypoint whose RESULT IS the raw compound/Option (`List.at` as the whole body,
  or any peer op whose result the entrypoint returns as a List/Map/Set/Option) → `emit_runtime_resource`
  → "not in the extern-import set". Fix = the resource×extern envelope fusion above; then pin this.

## Not-a-miscompile note
This is a clean DECLINE (compile error, no artifact), so it is safe; it just blocks a valid rich-
interface shape (a peer returning a collection the consumer RE-EXPORTS as its own entrypoint result).
The COMMON shape (call a rich peer op, USE its result, return derived data) already WORKS + is pinned
(PL15/18/19); only re-exporting the raw peer compound as the entrypoint's escaping result declines.
PL21 (merged) turned the opaque internal message into an ACTIONABLE decline naming op/iface/workaround.

## STATUS (2026-07-16, v-peer-linking): actionable decline SHIPPED (PL21); the FUSION is the enhancement.
Not rushing the fusion — it is a large frozen-ABI-adjacent byte-emit change (a partial landing =
INVALID components). The impl plan below is written so a dedicated session / unblocked fix agent can
execute it without re-deriving the byte structure.

## IMPLEMENTATION PLAN for the resource×peer-extern envelope fusion (studied the byte layout 2026-07-16)
The gap is ONLY in `emit_runtime_resource` (mod.rs:~1846) + its sibling `emit_recursive_sum_resource`;
the NORMAL `emit` already does all of this. Mirror it into the resource path:

1. **Collect + split imports** (copy from `emit` mod.rs:568-605): `collect_host_imports` over
   `layout.order`; move peer-bound ops (`db.effect_bindings`) into `extern_imports: Vec<ExternImport>`;
   the rest stay host imports (a resource escape with a plain HOST op is a separate, also-unsupported
   case — scope THIS fix to extern-only + runtime, decline host+resource as today).
2. **Thread `extern_order`** (copy mod.rs:648-656): `layout.with_extern_order(extern_order)` so
   select.rs:~8189 finds the op. Shift `import_base` to `k(runtime) + 2(resource intrinsics) +
   e(extern ops)` — the extern ops are core funcs, so decide their position vs the runtime ops (put
   extern FIRST like the non-resource extern path `core_module_with_extern_runtime`, or after the
   runtime k — pick ONE and keep the alias/lower indices consistent).
3. **New core-module builder** `runtime_resource_core_module_with_extern` (serialize.rs): like
   `runtime_resource_core_module` but the core module ALSO imports the peer ops from module `"peer"`
   (mirror `core_module_with_extern_runtime` serialize.rs:577 — it already fuses extern+runtime for the
   non-resource case; the resource core adds the `make`/`t-encode`/`cabi_realloc` + memory on top).
4. **New envelope** `assemble_runtime_resource_extern` (envelope.rs): compose `assemble_runtime_resource`
   (envelope.rs:1854 — imports runtime instance, aliases/lowers k ops, wires dtor, publishes resource)
   WITH the peer-interface import instances from `assemble_extern_runtime` (envelope.rs:1099 — per-iface
   component-type instance, alias each op out, canon-lower, the `"peer"` core instance). The component
   ends up importing BOTH `cadenza:runtime/heap` AND each peer interface, and exporting the resource.
   🪤 the two assemblers each number component-type instances from 0 — merging needs a single instance
   counter (runtime instance, then g peer instances, then the resource type). Byte-validate with
   `wasm-tools validate` per component (a fresh Validator each).
5. **Route** in `emit_runtime_resource`: if `!extern_imports.is_empty()`, take the new fusion path; else
   the existing `assemble_runtime_resource`. **Runner:** `run_with_peers` already composes a consumer
   that imports both a runtime and peer interfaces (X5a), so no runner change — the consumer is just
   ALSO a resource-exporter now.

**Test to add once landed** (the DECLINES shape above): `(op mklist (-> Int64 (List Int64)))` bound to
`cadenza:l/api`, `(def (main x) (host (L) (L.mklist x)))` returns the raw List → composed via
`run_with_peers` with a source provider → `List.len`/`List.at` of the returned handle in a SECOND run,
or assert the consumer VALIDATES + imports both. Est. size: ~150-250 lines across serialize + envelope
+ mod, plus 1-2 byte oracles. HIGH byte-bug risk → validate every intermediate component.

v-peer-linking owns the peer surface; the reclamation dup/drop seam (v-memory-safety) is NOT involved
(confirmed 2026-07-16 — this is the extern-import/envelope path, distinct from List.at reclaim).
