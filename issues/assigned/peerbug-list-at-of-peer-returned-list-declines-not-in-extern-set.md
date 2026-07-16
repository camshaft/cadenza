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
interface shape (a peer returning a collection the consumer indexes). The operator's north star names
"lists … crossing the peer boundary", so this is worth fixing. v-peer-linking owns the peer surface
but this sits on the emit/layout/reclamation seam (shared with v-memory-safety's select.rs dup/drop),
so filing for coordinated triage rather than a rushed patch.
