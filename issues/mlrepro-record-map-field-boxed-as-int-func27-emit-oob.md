# function[27] = value-heap EMIT bug: a record/tuple Map-field with a Var-resolved node type is box-int'd as i64 but is an i32 handle

## Severity
HARD BLOCKER on the compiler-ml suite gate (every ≥2-varied-pipeline-@test module → `invalid component: function[27]`). Same OOB class as the browser snowflake mesh teardown (concierge issue 9254). NOT a cdz-test builder bug (v-fleet-tooling disproved that: `cdz test` just calls `rcdzc::compile`; wasmtime rejects the EMITTED wasm at `Component::new`).

## Decisive diagnosis (via WAT of the dumped test component)
`wasm-tools validate` on the dumped component (`CDZ_DUMP_TEST_WASM=… cdz test implementation/compiler-ml/src/sread-eval.cdz`):

```
error: func 27 failed to validate
  0: type mismatch: expected i64, found i32 (at offset 0x331c)
```

Disassembly (`wasm-tools print --print-offsets`) around 0x331c, inside a `arr-alloc(3)` + 3-field aggregate — EXACTLY `Tree.Arena(Map(Int64,Node), Int64, Map(Int64,Int64))` built by `empty-tree() = Tree.Arena(Map.empty, 0, Map.empty)`:

```
@3314  i32.const 3        ; arr-alloc(3) → the record handle
@3316  call 0    (arr-alloc)
@3318  i32.const 0        ; field 0 index
@331a  call 16   (map-empty)   ; → i32 handle
@331c  call 4    (box-int)     ; ← FAIL: box-int : (param i64)→i32, fed an i32, NO extend
@331e  call 2    (arr-set)
@3320  i32.const 1        ; field 1 index
@3322  i64.const 0        ; the Int64 field
@3324  call 4    (box-int)     ; OK — fed an i64
@3326  call 2    (arr-set)
@3328  i32.const 2        ; field 2 index
@332a  call 16   (map-empty)   ; → i32 handle
@332c  call 4    (box-int)     ; would also FAIL
```

Import signatures (core module): `box-int : (param i64)→i32`, `map-empty : ()→i32`.

## Root cause
`Core::Record`/`Core::Tuple` emit (backend/wasm/select.rs:4759 and 4789) box each field by the FIELD-VALUE NODE's own type: `box_op(db, value)` → `type_of(value)`. At scale (the `Tree.Arena` construction inlined through the `read-source("42")` pipeline), the Map field-value node's type resolves to `Ty::Var`/`Ty::Any`, so `box_op_ty` grounds it to `box-int` (select.rs:1277 — the "phantom/dead position defaults to the uniform i64 cell" arm). But the value ON THE STACK is a LIVE Map handle (i32), and `emit_box_i32_to_i64_extend` (select.rs:1383) does NOT extend it (not a narrow-int/enum-disc) → an i32 reaches the i64 `box-int` → wasm rejects at `Component::new`.

The comment at select.rs:1268–1277 explicitly assumes "a LIVE value never has a free-var type here — inference would have solved it — so this cannot mask a real unresolved-type bug." **That assumption is FALSE here**: a live Map field of an at-scale-inlined record reaches the box site with an unresolved node type.

## The two candidate fixes (backend, rcdzc — v-inference emit + v-memory-safety value-heap)
1. **Prefer the declared field/element type over the node type** — `Core::Record`/`Core::Tuple` should box each field by the RECORD/TUPLE's declared field type (à la `box_op_for`, select.rs:1209, which already does exactly this for collection elements: use the declared type, fall back to the node only when the declared is Var). The record's own solved type carries `Ty::Map(_,_)` for the field even when the field-value node's type didn't get pinned → `box_op_ty` then returns `Ok(None)` (store-as-is, no box) — correct.
2. **Make the Var-default safe for a handle-producing value** — before grounding a Var to `box-int`, check whether the emitted value is a heap HANDLE (i32 that's actually a u32 handle, e.g. from `map-empty`/`vec-*`/`sum-new`); if so, store as-is (`Ok(None)`) rather than box-int. Narrower but riskier (needs a reliable "produces-a-handle" predicate at the value node).

Fix #1 is the principled one and mirrors existing `box_op_for` for collections.

## Repro
- `CDZ_DUMP_TEST_WASM=/tmp/f27.wasm cdz test implementation/compiler-ml/src/sread-eval.cdz` → dumps the 1.87MB component; `wasm-tools validate /tmp/f27.wasm` → the func 27 error above.
- Minimal type-level reductions (concrete `Map(Int64,Int64)` fields) do NOT repro — the trigger needs the at-scale inlining that loses the field node's concrete type. So a minimal `.sexp` repro is still open; the WAT + root cause above are decisive without it.

## History
Earlier mis-pinned by v-compiler-ml as a "cdz-test builder emit-SIZE ceiling" — WRONG. It's a value-heap type-emit bug (i32 handle boxed as i64). Corrected here.
