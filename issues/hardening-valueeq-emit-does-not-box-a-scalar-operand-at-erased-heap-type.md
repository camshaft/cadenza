# HARDENING (standing item, filed by v-memory-safety 2026-07-18, concierge-endorsed)

## The latent bug
The `Core::ValueEq` emit (`backend/wasm/select.rs` ~8890) does NOT box a SCALAR operand into a heap
handle before `champ_eq`/`value-eq`. It emits each operand and expects an **i32 boxed handle**
(the `slot_l`/`slot_r` scratch are `ValType::I32`), then `value-eq` (a physical-byte champ_eq compare).
When an operand's runtime repr is a **raw scalar** (an `i64.const`, e.g. a `ConstInt` at erased/deferred
type), the emit produces a raw `i64` where champ_eq expects a boxed `i32` handle → **invalid wasm**
("type mismatch: expected i32 found i64"), AND the compare itself is semantically wrong (a boxed handle
on one side vs a raw scalar on the other).

## How it was found + why it's currently masked
Surfaced while root-causing the v-iterators generic-closure-callback decline: `(= h 1)` where `h` is an
erased type-param value routed the compare to `value-eq`; the `1` (a scalar `ConstInt`) hit this gap.
- The OWNERSHIP half (`heap_operand_ownership` declining the boxed erased-int `ConstInt`) was the visible
  symptom (a clean decline).
- v-inference's **cd9be4379** fixes the ITERATOR shape by routing proven-scalar `=` operands AWAY from
  value-eq to a scalar `Core::Compare` (unification grounds the width from the scalar side) — so the
  SCALAR path no longer reaches ValueEq. That makes THIS bug unreachable *for a scalar-vs-scalar or
  scalar-vs-erased equality*.

## Why it's still worth fixing (not fully dead)
cd9be4379 only reroutes when a `=` operand is a **proven scalar**. A `value-eq` that legitimately stays
on the champ_eq path but receives a **scalar operand it must box** (a non-scalar-provable erased position,
a future heap-eq shape, a compound whose element compare bottoms out at a scalar constant) could still hit
the unboxed-scalar-operand gap → invalid wasm or a wrong compare. The ValueEq emit should DEFENSIVELY box a
scalar operand (via `box_op`) when it stays on the champ_eq path, mirroring how nested compound elements are
boxed at construction. Reject-don't-miscompile is the current floor; boxing is the completeness fix.

## Fix sketch
In the `Core::ValueEq` emit, after emitting each operand, if the operand's repr is scalar (has a `box_op`)
AND we're on the champ_eq/value-eq path, emit the `box_op` before the `LocalTee(slot_*)`. Width-coerce
(i32→i64 extend) as the existing element-box path does. Verify: no double-box of an already-heap operand;
the owned box is dropped after the borrowing compare (the existing `lo/ro == Owned` drop already handles
this once `heap_operand_ownership` classifies the boxed ConstInt Owned — which it should once boxed).
Gate: a value-eq that lands a scalar operand on the champ_eq path runs + nets live-objects 0 + is
value-correct; `gate --opt-sweep`; alloc bench.

## Ownership
v-memory-safety (rc/emit). PICK UP after the SumExpect + MatchSum owned-Some-shell leak twins land.
Related: the SumExpect leak fix (d4b77be35), the MatchSum twin (next), v-inference cd9be4379.
