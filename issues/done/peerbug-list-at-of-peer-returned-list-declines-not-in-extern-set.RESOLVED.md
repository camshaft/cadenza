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

## PRECISE INDEX-SHIFT DERIVATION (2026-07-16, v-peer-linking — turnkey for a dedicated build)
The fused assembler `assemble_extern_runtime_resource` = `assemble_runtime_resource` with `p` peer ops
prepended (single peer interface, `g=1`, first cut). Let `p = peer_fns.len()`, `k = imports.len()` (runtime
ops incl. `drop`). Every runtime/resource index in `assemble_runtime_resource` shifts by `p` (core funcs)
and the component-type/instance numbering gains the peer's slots. Concretely, relative to the current
`assemble_runtime_resource`:

CORE FUNCS: peer ops lowered → `0..p`; runtime ops lowered → `p..p+k` (were `0..k`); dtor `t-dtor` alias
→ `p+k` (was `k`); `resource.new` → `p+k+1`, `resource.rep` → `p+k+2` (were `k+1`,`k+2`); program
`make` → `p+k+3`, `t-encode` → `p+k+4`, `cabi_realloc` → `p+k+5` (were `k+3..k+5`).
COMPONENT TYPES: peer instance-type → type 0; runtime import instance-type → type 1 (was 0); resource
type `t` → type 2 (was 1); minted make tuple types → `3..3+shift`; `own<t>` → `3+shift` (was `2+shift`);
make-ft → `4+shift`; borrow-ft/list/encode-ft → `5..7+shift` (each +1 vs today, +`shift`).
COMPONENT INSTANCES: imported peer instance → comp instance 0; imported runtime instance → comp instance
1 (was 0); the inner re-export instantiation → comp instance 2 (was 1) → the final `export_instance_item`
references instance 2.
CORE INSTANCES: peer core-instance (lowered peer ops under their op names) → core instance 0; dtor's
`heap-dtor` source instance → 1; dtor instance → 2; `heap` export instance (runtime ops + resource
intrinsics) → 3; program instance (module instantiated with `peer`=inst0 AND `heap`=inst3) → 4. (All +1..+2
vs today; the program instantiation gains a second module-arg `PEER_MODULE`=core instance 0 alongside
`HEAP_MODULE`=core instance 3.)
CANON-LOWER: `p+k` lowers (peer ops `0..p` then runtime `p..p+k`), mirroring `assemble_extern_runtime`'s
`op_alias_sec`/`lower_sec` (which already lays peer-then-runtime in this exact order — COPY that block).

CORE MODULE: needs a new `serialize::runtime_resource_core_module` variant that imports BOTH `"peer"`
(p ops) and `"heap"` (k ops + 2 intrinsics) — today it imports only `"heap"`. The program core's
`CallExternImport(i)` resolves peer op `i` to imported func `i` (peer block first), and `CallImport`
resolves runtime op to `p+i`; so `import_base = p + k + 2`. `emit_runtime_resource` must
`collect_host_imports`→split peer ops into `extern_imports`→`layout.with_extern_order(...)`
+`with_import_base(p+k+2)` (mirror the main `emit` at mod.rs:581-660), and pass `peer_fns` (built like the
main path's `extern_op_comp_functype`) to the new assembler.
VALIDATION: build the smallest shape first — 1 peer op returning a `(Tuple Int64 Int64)`, main returns it
raw → `emit_runtime_resource`. `wasm-tools validate` the consumer, then `run_with_peers` (expect the
projected value). Add `emit_recursive_sum_resource` (Option/sum result) as a SECOND increment. Est. still
~200 lines + a byte oracle; ~2-3 ticks; NOT green-partial-able (a wrong index = invalid component).

## RESOLUTION SEAM CONFIRMED (2026-07-16, v-peer-linking — the last unknown, now closed)
Traced how a peer op's call resolves inside the resource core module, the one piece that could have made
the fusion intractable. It does NOT: `Lir::CallExternImport(index)` (serialize.rs:237) emits `call
<index>` DIRECTLY, where `index` is the op's position in the extern set (`layout.extern_index`), and the
serializer lays peer imports FIRST (`0..e`) by convention (X5 extern-first, mirrored by
`core_module_impl` serialize.rs:610-656 which ALREADY threads extern_fns+host_fns+runtime in one fixed
order). So the body-emit needs NO change — IF the resource core module lays peer imports at `0..e` and
`emit_runtime_resource` calls `layout.with_extern_order(...)` so `extern_index` returns the right `0..e`.

CONSEQUENCE — the fusion is purely MECHANICAL index arithmetic, no new body logic:
- `runtime_resource_core_module_form_ex` (serialize.rs:1135): prepend `e` extern functypes/imports; runtime
  ops shift `0..k` → `e..e+k`; `resource-new`/`resource-rep` → `e+k`/`e+k+1`; `defined_type_base` → `e+k+2`;
  rebuild `import_index` so a runtime `CallImport` resolves to `e+i`; `import_base = e+k+2`.
- The envelope `assemble_extern_runtime_resource`: COPY `assemble_extern_runtime`'s peer sections
  (instance-type 0, import → comp inst 0, alias → comp funcs `0..e`, canon-lower → core funcs `0..e`, peer
  core-instance) BEFORE `assemble_runtime_resource`'s runtime+resource sections, shifting each `k+N` core
  func by `e`, each comp type by 1 (+`shift`), each comp instance by 1, and threading `PEER_MODULE`=peer
  core-instance into the program instantiation alongside `HEAP_MODULE`.
- `emit_runtime_resource` (mod.rs:1864): `collect_host_imports`→split peer ops into `extern_imports`
  (mirror mod.rs:581-600)→`layout.with_extern_order(...)`→pass `peer_fns` (built via `extern_op_comp_functype`)
  to the new assembler; same for `emit_recursive_sum_resource` (Option/sum result, increment 2).

DE-RISKED: the CORE-MODULE layer already supports extern+runtime (`core_module_impl`); the resource core
is the only builder lacking it, and the gap is index math, not new control flow. Est. holds at ~200 lines
+ one byte oracle across 2-3 ticks; STILL not green-partial-able (a wrong shift = invalid component, no
intermediate lands). Build smallest-first: 1 peer op → `(Tuple Int64 Int64)` result, `wasm-tools validate`
+ `run_with_peers` each step.

## ✅ RESOLVED (2026-07-16, v-peer-linking) — the resource×peer-extern envelope fusion LANDED
The fusion described above was executed exactly as planned across the PL28–PL46 increments (this queue
file's earlier STATUS predated those landings and was stale). It is now shipped on trunk:
- **Envelopes** (envelope.rs): `assemble_extern_runtime_resource` (envelope.rs:2305, the plain
  Tuple/Record/List/Map/Option compound escape, `g`-generalized to MULTIPLE peer interfaces in
  `95a82ee37`) + `assemble_extern_runtime_resource_with_scalar_methods` (envelope.rs:3095, the
  String/Bytes methods-envelope escape, `5159df77f` — the full `(-> String String)` model-call shape).
- **Emit routing** (mod.rs): ALL FOUR resource-escape paths now `collect_host_imports` → split
  peer-bound ops into `extern_imports` → `layout.with_extern_order(...)` + shifted `import_base` →
  dispatch to the fused assembler when `!extern_imports.is_empty()`: `emit_runtime_resource`
  (mod.rs:1915), `emit_runtime_sum_resource` (mod.rs:7013), `emit_recursive_sum_resource`
  (mod.rs:7280), and the bytes/string resource path (mod.rs:6869).
- **Core module** (serialize.rs): `runtime_resource_core_module_form_ex2` threads `extern_fns` +
  `leading_is_host` so the resource core imports BOTH `"peer"` and `"heap"`, layout byte-identical.
- **Pinned tests** (all PASS on trunk `6c1e89f65`, verified this tick): the exact DECLINES shape —
  a peer op whose compound result IS the escaping entrypoint result — is now green:
  `a_peer_compound_result_escapes_the_entrypoint_via_the_fused_envelope` (tests.rs:71755),
  `a_peer_option_result_escapes_...` (72190), `a_peer_list_result_escapes_...` (72260),
  `a_peer_bytes_result_escapes_...` (72776), plus the multi-peer `a_two_peer_interface_compound_result_
  escapes_via_the_fused_envelope` (72331) and `two_peer_interfaces_with_a_string_result_escape_via_the_
  methods_envelope` (72417). Landed: batch 91 (#461, `d623a5bdd`) for the compound/list/option escape;
  `5159df77f` for the String/Bytes methods envelope; `95a82ee37` for the multi-interface `g`-loop.
No further work: the peer-returned compound re-exported as the entrypoint result crosses zero-cost and
validates. This queue item is CLOSED.
