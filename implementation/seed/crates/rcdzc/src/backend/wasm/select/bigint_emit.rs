//! BigInt operand/constant emit helpers, peeled from select.rs to hold it under the 512KiB
//! lint-mandates file-size cap (pure move, no logic change). `use super::*` pulls select's
//! shared imports + sibling helpers; the fns are `pub(super)` so select and its sibling
//! submodules (emit.rs, dispatch.rs) reach them via select's `pub(super) use bigint_emit::*`.
use super::*;

/// Emit a UNARY runtime BigInt op that BORROWS its handle operand and returns a scalar (`bigint-to-i64-
/// checked`). The op reads the operand without consuming it, so an OWNED-temporary operand must be
/// DROPPED after the call (a borrowed param/local is left to its owner) — the `value-eq` reclamation
/// discipline. `tee` the operand into a scratch slot (kept on the stack for the call AND remembered for a
/// possible drop), call the op (which pops the borrowed handle and pushes the scalar), then drop the
/// remembered handle if it was owned. Declines (via `heap_operand_ownership`) an operand whose ownership
/// cannot be proved — reject, never a leak or double-free.
/// Register the runtime ops the inline materialization of a CONSTANT BigInt operand emits, so
/// `collect_used_ops` imports them: `bigint-of-i64` for an i64-fitting constant, or `bytes-alloc` +
/// `bytes-set` + `bigint-of-bytes` for a beyond-i64 one (the baked-sign-magnitude-bytes path). Mirrors the
/// two branches of `emit_const_bigint_leaf`. A non-constant / non-BigInt operand registers nothing.
pub(super) fn insert_const_bigint_materialize_ops(
    db: &mut Db,
    operand: StructId,
    out: &mut std::collections::BTreeSet<&'static str>,
) {
    if let Core::ConstInt(v) = core_of(db, operand)
        && is_bigint_valued(db, operand)
    {
        if v.to_i64().is_some() {
            out.insert(OP_BIGINT_OF_I64);
        } else {
            out.insert(OP_BYTES_ALLOC);
            out.insert(OP_BYTES_SET);
            out.insert(OP_BIGINT_OF_BYTES);
        }
    }
}

/// The canonical sign-magnitude heap-leaf bytes of a constant integer — `[sign][LE magnitude, trailing
/// zero bytes stripped]`, zero → `[0x00]` — byte-IDENTICAL to `bigint::Big::to_sign_magnitude_bytes` in
/// `cdz-runtime`, so a leaf built from these bytes via `bigint-of-bytes` is the SAME rep `bigint-of-i64` /
/// runtime arithmetic produces (so `bigint-cmp`/`value-eq` compare it correctly). `IntValue.magnitude` is
/// big-endian with no leading zero bytes, so reversing yields little-endian with no trailing zero bytes;
/// zero is the empty magnitude → the single sign byte `[0]` (never negative-zero).
pub(super) fn const_bigint_sign_magnitude_bytes(v: &crate::ast::IntValue) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(1 + v.magnitude.len());
    // Zero is non-negative on the wire (matches the runtime canonical form + `IntValue::zero`).
    bytes.push((v.negative && !v.magnitude.is_empty()) as u8);
    bytes.extend(v.magnitude.iter().rev().copied()); // big-endian → little-endian
    bytes
}

/// Emit a CONSTANT BigInt as a fresh OWNED heap-leaf handle on the stack. Fits i64 → `bigint-of-i64`;
/// beyond i64 → bake its canonical sign-magnitude bytes as a Bytes leaf (`bytes-alloc` + per-byte
/// `bytes-set`, exactly as a constant string materializes) then re-tag as a BigInt via `bigint-of-bytes`
/// (which consumes the byte leaf). Shared by the in-body value materialization (`Core::ConstInt`-typed-
/// BigInt) and the operand path (`emit_bigint_operand`).
pub(super) fn emit_const_bigint_leaf(v: &crate::ast::IntValue, out: &mut Emit) {
    match v.to_i64() {
        Some(x) => {
            out.push(Lir::ConstI64(x));
            out.push(Lir::CallImport(OP_BIGINT_OF_I64)); // → [fresh owned BigInt handle : i32]
        }
        None => {
            let bytes = const_bigint_sign_magnitude_bytes(v);
            out.push(Lir::ConstI32(bytes.len() as i32)); // [len]
            out.push(Lir::CallImport(OP_BYTES_ALLOC)); // → [buf]
            for (i, &byte) in bytes.iter().enumerate() {
                out.push(Lir::ConstI32(i as i32)); // [buf, index]
                out.push(Lir::ConstI32(byte as i32)); // [buf, index, byte]
                out.push(Lir::CallImport(OP_BYTES_SET)); // → [buf]
            }
            out.push(Lir::CallImport(OP_BIGINT_OF_BYTES)); // consumes buf → [fresh owned BigInt handle : i32]
        }
    }
}

/// Emit ONE BigInt operand as a heap HANDLE on the stack and return its ownership (for a possible post-op
/// drop). A CONSTANT BigInt that fits `i64` has no heap leaf yet, so materialize it: push its `i64` value
/// then `bigint-of-i64` → a FRESH OWNED handle (the borrowing op drops it after). A constant BEYOND `i64`
/// range DECLINES (the arbitrary-magnitude constant leaf is a B4 concern — the sign-magnitude byte
/// builder). Any other operand emits via `emit` and is classified by `heap_operand_ownership`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_bigint_operand(
    db: &mut Db,
    operand: StructId,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<HandleOwnership, Reject> {
    if let Core::ConstInt(v) = core_of(db, operand)
        && is_bigint_valued(db, operand)
    {
        // A constant BigInt operand has no heap leaf of its own — materialize one (fits-i64 via
        // `bigint-of-i64`, beyond-i64 via `bigint-of-bytes` on its baked sign-magnitude bytes). A FRESH
        // OWNED handle either way; the borrowing op drops it after.
        emit_const_bigint_leaf(&v, out);
        return Ok(HandleOwnership::Owned);
    }
    let o = heap_operand_ownership(db, operand)?;
    let op_base = *high;
    emit(db, operand, slots, op_base, high, scratch_ty, layout, out)?; // [h : i32]
    Ok(o)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_bigint_borrow_unary(
    db: &mut Db,
    operand: StructId,
    import: &'static str,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let slot = *high;
    *high = slot + 1;
    scratch_ty.insert(slot, ValType::I32);
    let o = emit_bigint_operand(db, operand, high, slots, scratch_ty, layout, out)?; // [h : i32]
    out.push(Lir::LocalTee(slot));
    out.push(Lir::CallImport(import)); // pops the borrowed handle → [scalar]
    o.drop_slot_if_owned(slot, out);
    Ok(())
}

/// Emit a BINARY runtime BigInt op that BORROWS both handle operands and returns a FRESH owned result
/// handle (`bigint-add`/`-sub`/`-mul`/`-div`, and — the next slice — `bigint-cmp`, which returns a scalar
/// instead; both leave the operands to be reclaimed by this emit). Each OWNED-temporary operand is
/// dropped after the call while the result stays on the stack; a borrowed param/local is left to its
/// owner. Same shape as the `value-eq` emit, but the result (a handle or a scalar) is kept rather than
/// discarded. Two i32 scratch slots hold the operand handles for the possible drops; the operands emit
/// above the running high-water so neither reuses the other's transient scratch at a different width.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_bigint_borrow_binary(
    db: &mut Db,
    lhs: StructId,
    rhs: StructId,
    import: &'static str,
    high: &mut u32,
    slots: &HashMap<StructId, u32>,
    scratch_ty: &mut HashMap<u32, ValType>,
    layout: &Layout,
    out: &mut Emit,
) -> Result<(), Reject> {
    let slot_l = *high;
    let slot_r = *high + 1;
    *high = slot_r + 1;
    scratch_ty.insert(slot_l, ValType::I32);
    scratch_ty.insert(slot_r, ValType::I32);
    let lo = emit_bigint_operand(db, lhs, high, slots, scratch_ty, layout, out)?; // [a : i32]
    out.push(Lir::LocalTee(slot_l));
    let ro = emit_bigint_operand(db, rhs, high, slots, scratch_ty, layout, out)?; // [a, b : i32]
    out.push(Lir::LocalTee(slot_r));
    out.push(Lir::CallImport(import)); // pops both borrowed handles → [result]
    lo.drop_slot_if_owned(slot_l, out);
    ro.drop_slot_if_owned(slot_r, out);
    Ok(())
}
