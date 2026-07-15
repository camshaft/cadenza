# DESIGN — a value crossing the boundary is a RESOURCE with methods (rcdzc)

## Goal (operator, 2026-07-13)

> "I would prefer that everything is consistent. Having exceptions for stuff makes the language
> surprising. Can we pass the string as a resource — a method on it called `to-string` the host calls
> to get it across — and expose more APIs (len, iterate over chars)? Same for bytes."

Every non-scalar value (String, Bytes, List, tuple, record, sum, map) crosses the component boundary
as **one uniform shape**: a component-model **resource** carrying methods. No value type gets a
special-cased native WIT primitive. The endgame removes the native WIT `string` / `list<u8>` boundary
types entirely (the two remaining exceptions today) so there is exactly ONE way a value leaves the
guest.

## What is already proven (landed, running under wasmtime 37)

Two hard unknowns are settled by tests, not reasoning:

1. **`borrow<t>` is sound, and methods use the rep DIRECTLY.** `encode` migrated own→borrow
   (`spec@63a13d8b`). wasmtime's canonical ABI `lift_borrow` (`resources.rs::resource_lift_borrow`)
   hands the guest the **rep itself** as the method's i32 param — NOT a resource-table index. So a
   borrow method does `local.get 0` and treats it as the heap rep (no `resource.rep`, which traps on a
   borrow with "unknown handle index <rep>"). A borrow does not own the value, so the method does NOT
   drop — reclamation is the resource DTOR's job on host-drop. (`RepSource::{Own,Borrow}` in
   `serialize.rs` threads this through the four walk bodies.)

2. **A value resource carries MULTIPLE repeatable borrow methods; one handle survives a sequence.**
   Oracle `a_value_resource_carries_a_repeatable_len_method_beside_encode` (`spec@605976b1`): a
   `(tuple 7 9)` resource with `len : borrow<t> -> u32` (= `arr-len(rep)`) beside `encode`, driven
   `make → len → len → encode → drop`. `len` returns 2 both times; `encode` still decodes correctly
   AFTER the repeated `len` calls; the handle drops cleanly (dtor reclaims, `live-objects == 0`).
   Cross-call borrow-lend scoping is sound.

The reclamation model is dtor-only: `make()` builds the value on the heap + `resource.new(handle)`;
every method borrows (no consume, no drop); host-drop → `t-dtor(rep)` → `heap.drop(rep)` (cascades to
children; the value heap is acyclic).

## The method surface (operator-selected)

Per value type, all methods are `borrow<t>` (repeatable):

| method                         | String | Bytes | List | tuple/record/sum | core body                                  |
|--------------------------------|:------:|:-----:|:----:|:----------------:|--------------------------------------------|
| `encode() -> list<u8>`         |   ✓    |   ✓   |  ✓   |        ✓         | the canonical value-form walker (kept)     |
| `len() -> u32`                 |   ✓    |   ✓   |  ✓   |        —         | `bytes-len` / `bytes-len` / `vec-len`      |
| `to-string() -> string`        |   ✓    |   —   |  —   |        —         | `str-get(rep)` (String is a UTF-8 leaf)    |
| `to-bytes() -> list<u8>`       |   —    |   ✓   |  —   |        —         | compact the rope + copy the raw bytes out  |
| `char-at(i) -> option<char>` / |   ✓    |       |      |                  | (later increment — per-element access)     |
| `byte-at(i)/at(i)`             |        |   ✓   |  ✓   |                  |                                            |

`encode` is the DEFAULT the host renders `(: value type)` from (what `cdz-run` decodes today); the
new methods are ADDITIONAL affordances a host that wants raw content / a length / an element uses
directly. `encode` stays on every value type so the pretty-render path is uniform.

## The envelope: adding a method is a bounded, byte-exact edit

`assemble_runtime_resource` (envelope.rs) is the hand-emitted component, byte-gated against a
`ComponentBuilder` oracle (`combined_envelope_matches_component_builder_oracle`). Its index spaces
today (k = imports.len()): core funcs — lowered ops `0..k`, `t-dtor` `k`, `resource.new` `k+1`,
`resource.rep` `k+2`, program `make` `k+3`, `t-encode` `k+4`, `cabi_realloc` `k+5`. Component types —
import-instance-type 0, resource 1, `own<t>` 2, make-ft 3, `borrow<t>` 4, `list u8` 5, encode-ft 6.
Component funcs — aliased ops `0..k`, make-lift `k`, encode-lift `k+1`. Inner re-export component
(`resource_inner_component_borrow`) re-exports `t` + `make` + `encode`.

**Each extra method M appends, in order:**
- core: the program core module exports `t-M` (a new core func after `t-encode`); the boundary alias
  section aliases it off the program instance (one more `core_alias_item`).
- component types: a fresh `borrow<t>` defined type + M's functype (a scalar-result method needs no
  `list u8`/Memory/Realloc; a `list<u8>`/`string`-result method needs them like `encode`).
- canon: one `canon_lift_item` (scalar/handle result) or `canon_lift_list_item` (list/string result,
  Memory 0 + Realloc) → the next component func.
- inner component: re-declare M against the imported abstract resource AND re-export it against the
  exported resource (mirror `encode`'s two-sided re-typing).
- the inner-component INSTANTIATE item passes one more lifted func.

The proven oracle helpers (`tuple_methods_core`, `oracle_tuple_methods`,
`inner_reexport_component_methods`) are the exact reference to mirror — a method is added by pattern,
not by novel reasoning. Byte-gate each addition against a matching ComponentBuilder oracle before
wiring the hand-emit.

## Type-directed emission (the front-end seam)

`mod.rs::emit` already routes a single-nullary compound export to `emit_runtime_resource` /
`emit_runtime_bytes_resource` / `emit_runtime_sum_resource` by result `Ty`. The method SET is chosen
there by `Ty`: `Ty::Bytes` → `[encode, len, to-bytes]`; `Ty::String` → `[encode, len, to-string]`;
`Ty::List` → `[encode, len]`; tuple/record/sum → `[encode]`. The serializer's core-module builder
grows one `t-M` body per method in the set (a `bytes-len`/`str-get`/`vec-len` one-liner over the
borrow rep); `assemble_runtime_resource` grows a `methods: &[Method]` parameter (the per-method
boundary functype + core-export name + lift kind), so the envelope is generic over the set.

## Increment plan (each a gated landing to `spec`)

- **VM-1** — `len` on the runtime-Bytes resource. The smallest real method: a scalar-result borrow
  method `len : borrow<t> -> u32` = `bytes-len(rep)`. Oracle-gate the 3-method envelope, wire
  `assemble_runtime_resource(methods)`, emit `t-len` in the bytes core, choose `[encode, len]` for
  `Ty::Bytes` in `emit`. A `cdz-run` test calls `len` on a runtime-Bytes escape.
- **VM-2** — `len` generalized to String + List (same scalar method, different core op:
  `bytes-len`/`vec-len`; String's rope is `bytes-len` too).
- **VM-3** — `to-bytes : borrow<t> -> list<u8>` (Bytes) — a list-result method (Memory/Realloc), body
  = `bytes-compact(rep)` then copy the contiguous bytes into the retarea (like `encode` but the raw
  payload, no value-form framing).
- **VM-4** — `to-string : borrow<t> -> string` (String) — a string-result method, body = `str-get`
  materializing the `(ptr,len)` the canonical ABI reads a `string` from.
- **VM-5** — `char-at`/`byte-at`/`at` per-element access (option-returning) — the richest surface.
- **VM-6** — REMOVE the native WIT `string` (host-call string arg in mod.rs) + any `list<u8>`
  result shortcut, so every value crosses as a resource. The uniformity payoff.

Each increment: oracle-gate the new envelope shape, wire the hand-emit byte-identical, add a
`cdz-run`/wasmtime test driving the new method, run the three gates, land via guarded CAS.

## Constraints / traps

- The closure `call` own→borrow is **C-HOST-5's** concern (the parallel closures-across-host
  worktree). Do NOT migrate `call` here — collision risk.
- `assemble_runtime_resource` is byte-gated: NEVER change its emitted bytes without re-pinning the
  oracle in the SAME landing (dump the oracle, mirror section-by-section).
- A scalar/handle-result method needs NO Memory/Realloc canon options; a `list<u8>`/`string`-result
  method needs Memory 0 + Realloc (like `encode`) — getting this wrong is an invalid component.
- `cdz-run`'s resource-escape path calls `make` + `encode` today; exposing the extra methods to a host
  is additive (the host reaches them by name inside `cadenza:run/run`), so `cdz-run`'s default render
  stays `encode`-driven — the new methods are opt-in for a method-aware host.
