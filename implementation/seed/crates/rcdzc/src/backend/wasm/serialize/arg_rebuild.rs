//! Argument / cell REBUILD + sum-arm emit helpers for the resource-escape `make` bodies, split out of
//! `serialize/mod.rs` to keep that file under the 512 KiB source mandate. Pure code motion — every item
//! keeps its behavior; the make/closure/escape emitters call these via `pub(super)`. Types + ABI consts
//! (`SumArgRebuild`, `FieldRebuild`, `MakeCoreSlot`, `op`, `uleb128`, …) live in the parent and reach here
//! through `use super::*`.
#![allow(clippy::too_many_arguments)]
use super::*;

/// Emit ONE arm's cell build: `sum-new(decl_disc, payload)` where the payload is the inline unit (nullary), a
/// boxed scalar leaf, or a rebuilt compound cell. Leaves `[sum-handle]` on stack. `payload_param` is the
/// flattened core-param index the payload's leaf/leaves start at (the sum's `base_param + 1`).
pub(super) fn emit_sum_arm(
    arm: &SumArgArm,
    payload_param: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    next_local: &mut u32,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(arm.decl_disc as i64, out); // [disc]
    match &arm.payload {
        SumArmPayload::Scalar {
            box_op,
            extend,
            wrap_join,
        } => {
            out.push(op::LOCAL_GET);
            uleb128(payload_param as u64, out); // [disc, payload-leaf]
            if *wrap_join {
                // The joined payload slot is the WIDER core (i64) but THIS arm's payload is narrow (i32-core):
                // the narrow value arrived widened into the join, so recover its low 32 bits before this arm's
                // own (re-)extend. `i32.wrap_i64` keeps the low 32 bits (the raw narrow value, sign/zero
                // irrelevant here since `extend` below re-applies the correct one). See the diff-width oracle.
                out.push(op::I32_WRAP_I64);
            }
            if let Some(signed) = extend {
                out.push(if *signed {
                    op::I64_EXTEND_I32_S
                } else {
                    op::I64_EXTEND_I32_U
                });
            }
            out.push(op::CALL);
            uleb128(imp(box_op), out); // [disc, payload-handle]
        }
        SumArmPayload::Compound(fields) => {
            // Rebuild the payload's value-heap cell from its recursively-flattened leaves, starting at the
            // payload base — exactly as a bare tuple arg rebuilds. Leaves the cell handle on the stack.
            let mut cursor = payload_param;
            emit_cell_rebuild(fields, &mut cursor, bulk_bytes, imp, None, None, out); // [disc, payload-cell-handle]
        }
        SumArmPayload::Nullary => {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(
                crate::backend::wasm::runtime_abi::IMM_UNIT as i64,
                out,
            ); // [disc, unit]
        }
        SumArmPayload::Bytes { ptr_from_i64 } => {
            // The `list<u8>`/`string` payload crossed as `(ptr, len)` at the payload base; copy it into a guest
            // `Bytes` (exactly like a top-level `BytesLeaf`; a `String` IS the same UTF-8 byte-leaf), leaving the
            // handle as this arm's payload. `ptr_from_i64` wraps the joined `i64` ptr slot to i32 first (the
            // different-width Result join — erp1).
            let (buf, ctr) = scratch.expect("a Bytes sum arm needs the wrapper's scratch locals");
            emit_bytes_leaf_copy_in(payload_param, *ptr_from_i64, buf, ctr, bulk_bytes, imp, out); // [disc, bytes-handle]
        }
        SumArmPayload::Enum => {
            // The enum payload crossed as ONE i32 disc leaf; build the inner all-nullary cell
            // `sum-new(disc, IMM_UNIT)` (boundary disc == guest decl disc — same case order) as this arm's
            // payload.
            out.push(op::LOCAL_GET);
            uleb128(payload_param as u64, out); // [disc, enum-disc]
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(
                crate::backend::wasm::runtime_abi::IMM_UNIT as i64,
                out,
            ); // [disc, enum-disc, unit]
            out.push(op::CALL);
            uleb128(imp("sum-new"), out); // [disc, enum-cell]
        }
        SumArmPayload::List(elem) => {
            // The `list<scalar>`/`list<string>`/`list<bytes>` payload crossed as `(ptr, len)` at the payload
            // base; build a value-heap vec (exactly like a top-level `MemLeafKind::List` lift), leaving the vec
            // handle as this arm's payload. Reuse the wrapper's scratch pair as the OUTER vec accumulator +
            // element cursor (like the Bytes arm). A BYTE-LEAF element (`list<string>`, eop3) additionally
            // reads each element's `(ptr, len)` descriptor and copies its bytes into a fresh guest byte-leaf —
            // `emit_list_leaf_lift` allocates four fresh locals for that from `next_local`, which is the
            // wrapper's real local cursor (declared in `n_locals`), NOT a throwaway. A FLAT list<scalar>
            // element allocates none (the count is byte-identical to before). Only a flat list
            // (`nest_lists == 0`) is admitted; a nested list-in-option is a later slice.
            let (buf, ctr) = scratch.expect("a list sum arm needs the wrapper's scratch locals");
            debug_assert_eq!(
                elem.nest_lists, 0,
                "only a flat list payload is admitted in a sum arm"
            );
            emit_list_leaf_lift(elem, payload_param, buf, ctr, next_local, imp, out); // [disc, vec-handle]
        }
        SumArmPayload::Flags { field_bits } => {
            // The flags payload crossed as ONE i32 bitset at `payload_param`; build the guest record-of-bools
            // cell (arr-alloc N + per (slot,bit) box-bool((bits>>bit)&1) arr-set), leaving the cell handle as
            // this arm's payload — the top-level flags reader, sourcing its one leaf from the sum payload base.
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(field_bits.len() as i64, out);
            out.push(op::CALL);
            uleb128(imp("arr-alloc"), out); // [disc, arr]
            for &(slot, bit) in field_bits {
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(slot as i64, out); // [disc, arr, slot]
                out.push(op::LOCAL_GET);
                uleb128(payload_param as u64, out);
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(bit as i64, out);
                out.push(op::I32_SHR_U);
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(1, out);
                out.push(op::I32_AND); // [disc, arr, slot, (bits>>bit)&1]
                out.push(op::CALL);
                uleb128(imp("box-bool"), out); // [disc, arr, slot, bool]
                out.push(op::CALL);
                uleb128(imp("arr-set"), out); // [disc, arr]
            }
        }
    }
    out.push(op::CALL);
    uleb128(imp("sum-new"), out); // [sum-handle]
}

/// Emit the SUM-arg CELL REBUILD for a closure `call` body: reassemble the single fixed-shape sum argument
/// (which crossed FLATTENED as `(disc, payload)` core params) into the one i32 sum-cell handle the lifted
/// body expects, leaving the handle on the stack AND stashed in `sum_local` (dropped after `call_indirect`).
/// `if disc == boundary_true_disc { arm_true } else { arm_false }`, each arm via [`emit_sum_arm`].
pub(super) fn emit_sum_arg_rebuild(
    rebuild: &SumArgRebuild,
    sum_local: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let disc_param = rebuild.base_param;
    let payload_param = rebuild.base_param + 1;
    // Branch on the BOUNDARY disc (the component-model convention), NOT the decl disc.
    out.push(op::LOCAL_GET);
    uleb128(disc_param as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(rebuild.boundary_true_disc as i64, out);
    out.push(op::I32_EQ);
    out.push(op::IF);
    out.push(wasm_abi::CORE_I32); // block type: → i32 (the sum handle)
    // Closure sum args carry scalar/nullary/compound payloads only (no `list<u8>`/enum arm) — no scratch.
    // Closure sum-arg rebuild: no shared allocator at lower-time → per-byte (bulk_bytes=false). A flat
    // scalar-list arm allocates no fresh locals, so a throwaway `next_local` suffices here (a byte-leaf-list
    // arm — which needs real locals — never reaches the closure path; it is a top-level entry param only).
    let mut nl = 0u32;
    emit_sum_arm(
        &rebuild.arm_true,
        payload_param,
        false,
        imp,
        None,
        &mut nl,
        out,
    );
    out.push(op::ELSE);
    emit_sum_arm(
        &rebuild.arm_false,
        payload_param,
        false,
        imp,
        None,
        &mut nl,
        out,
    );
    out.push(op::END);
    // stash for the post-dispatch drop; leaves [sum-handle] on the stack.
    out.push(op::LOCAL_TEE);
    uleb128(sum_local as u64, out);
}

/// Emit the sum rebuild for a record-cell FIELD ([`FieldRebuild::Sum`]): like [`emit_sum_arg_rebuild`] but the
/// disc/payload are read at the record's running cursor (NOT the closure `base_param`), and the handle is left
/// on the stack for the parent `arr-set` (no drop-stash — the parent record owns it). Advances `*cursor` past
/// the sum's flattened `(disc, payload…)`.
pub(super) fn emit_sum_field(
    rebuild: &SumArgRebuild,
    cursor: &mut u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    next_local: &mut u32,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let disc_param = *cursor;
    let payload_param = *cursor + 1;
    out.push(op::LOCAL_GET);
    uleb128(disc_param as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(rebuild.boundary_true_disc as i64, out);
    out.push(op::I32_EQ);
    out.push(op::IF);
    out.push(wasm_abi::CORE_I32); // block type: → i32 (the sum handle)
    emit_sum_arm(
        &rebuild.arm_true,
        payload_param,
        bulk_bytes,
        imp,
        scratch,
        next_local,
        out,
    );
    out.push(op::ELSE);
    emit_sum_arm(
        &rebuild.arm_false,
        payload_param,
        bulk_bytes,
        imp,
        scratch,
        next_local,
        out,
    );
    out.push(op::END); // → [sum-handle]
    *cursor += rebuild.flattened_param_count();
}

/// Emit the tuple-arg CELL REBUILD for a closure `call` body: reassemble the single fixed-shape tuple/record
/// argument (which crossed the boundary FLATTENED into its N scalar fields at core params `1..1+N`) into the
/// one i32 cell handle the lifted body expects — `arr-alloc N` + per field (index, the flattened param, box,
/// `arr-set`; the FBIP array threaded on the stack) — leaving the handle on the stack AND stashed in
/// `tuple_local`. Caller must have pushed nothing between (the array threads from `arr-alloc`), and drops the
/// cell after `call_indirect` via [`emit_tuple_rebuilt_drop`]. Shared by every list-result `call` body (bytes/
/// value-form/value-encode) + the scalar body; `imp(name) -> import index`. See [`TupleArgRebuild`].
pub(super) fn emit_tuple_rebuild(
    rebuild: &TupleArgRebuild,
    tuple_local: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    // Rebuild the TOP-LEVEL cell, threading the leaf cursor from `base_param`; `local.tee` the resulting
    // handle into `tuple_local` for the post-dispatch drop (only the OUTER cell is dropped — its nested
    // sub-cells are its elements, reclaimed with it).
    let mut cursor = rebuild.base_param;
    // Closure tuple-arg rebuild: no shared allocator at lower-time → per-byte (bulk_bytes=false).
    emit_cell_rebuild(&rebuild.fields, &mut cursor, false, imp, None, None, out);
    out.push(crate::backend::wasm::wasm_abi::op::LOCAL_TEE);
    uleb128(tuple_local as u64, out); // stash for the post-dispatch drop; leaves [arr] on the stack
}

/// Emit ONE value-heap cell for a run of [`FieldRebuild`] fields, consuming flattened leaf params from
/// `*cursor` (advanced past each leaf, depth-first). `arr-alloc N` + per field: index, then either the boxed
/// scalar leaf OR a recursively-rebuilt nested sub-cell handle OR a `list<u8>` copied out of memory, then
/// `arr-set` (FBIP array threaded on the stack). Leaves the cell handle on the stack. Recursion mirrors
/// `Core::Tuple`/`Core::Record`. `scratch` = `Some((buf_local, i_local))` when this cell (or a nested one)
/// carries a `BytesLeaf` — the two reusable scratch locals its copy-in loop threads (`buf` handle + counter);
/// `None` for any rebuild with no bytes leaf (every non-wrapper caller — closure/tuple/sum rebuilds — which
/// never carry one). A `BytesLeaf` reached with `scratch == None` is a caller bug (`.expect`).
pub(super) fn emit_cell_rebuild(
    fields: &[FieldRebuild],
    cursor: &mut u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    scratch: Option<(u32, u32)>,
    slots: Option<&[u32]>,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(fields.len() as i64, out);
    out.push(op::CALL);
    uleb128(imp("arr-alloc"), out); // [arr]
    // `fields` are consumed in WIT/flattened-param order (the cursor advances sequentially); each is stored
    // at its cell SLOT — `slots[i]` when the record permutes (a WIT field whose name-lex slot ≠ its WIT
    // position), else the position `i` (a name-lex-ordered record / a nested rebuild, identity). This is how
    // a declaration-ordered WIT record param (the real `message`) lands its fields in the value-heap cell's
    // name-lex slots the def reads.
    for (i, field) in fields.iter().enumerate() {
        let slot = slots.map(|s| s[i]).unwrap_or(i as u32);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(slot as i64, out); // [arr, slot]
        match field {
            FieldRebuild::Scalar { box_op, extend } => {
                out.push(op::LOCAL_GET);
                uleb128(*cursor as u64, out); // the flattened leaf param → [arr, i, leaf]
                *cursor += 1;
                if let Some(signed) = extend {
                    out.push(if *signed {
                        op::I64_EXTEND_I32_S
                    } else {
                        op::I64_EXTEND_I32_U
                    });
                }
                out.push(op::CALL);
                uleb128(imp(box_op), out);
            }
            FieldRebuild::Nested(sub, sub_slots) => {
                // Rebuild the nested sub-cell (consumes its own leaves in WIT order) → an i32 handle stored
                // AS-IS. Its own fields permute into their name-lex slots via `sub_slots`.
                emit_cell_rebuild(sub, cursor, bulk_bytes, imp, scratch, Some(sub_slots), out); // → [arr, i, sub-handle]
            }
            FieldRebuild::BytesLeaf => {
                let (buf, ctr) = scratch.expect("a BytesLeaf needs the wrapper's scratch locals");
                emit_bytes_leaf_copy_in(*cursor, false, buf, ctr, bulk_bytes, imp, out); // → [arr, i, buf]
                *cursor += 2; // the list flattened to (ptr, len)
            }
            FieldRebuild::ListLeaf(elem) => {
                // The `list<scalar>` field crossed as `(ptr, len)` at `*cursor`; build a value-heap vec into the
                // wrapper's scratch pair (exactly like the sum-arm `SumArmPayload::List` lift), leaving the vec
                // handle for the parent `arr-set`. A FLAT list allocates no fresh locals, so a throwaway
                // `next_local` suffices (the classifier admits only `nest_lists == 0` here).
                let (buf, ctr) = scratch.expect("a ListLeaf needs the wrapper's scratch locals");
                debug_assert_eq!(elem.nest_lists, 0, "only a flat list field is admitted");
                let mut nl = 0u32;
                emit_list_leaf_lift(elem, *cursor, buf, ctr, &mut nl, imp, out); // → [arr, i, vec]
                *cursor += 2; // the list flattened to (ptr, len)
            }
            FieldRebuild::Sum(rebuild) => {
                // A record-field sum's arms carry scalar/nullary/compound/Bytes/scalar-list payloads only (a
                // byte-leaf-list arm is a top-level entry param only — this field's classifier declines it), so
                // a flat-list arm allocates no fresh locals and a throwaway `next_local` suffices.
                let mut nl = 0u32;
                emit_sum_field(rebuild, cursor, bulk_bytes, imp, scratch, &mut nl, out); // → [arr, i, sum-handle]
            }
            FieldRebuild::Flags { field_bits } => {
                // A WIT `flags{…}` field: the packed i32 bitset is at `*cursor`. Build the nested record-of-
                // bools cell (`arr-alloc N` + per `(fslot, bit)` `box-bool((bits>>bit)&1)` `arr-set`) and leave
                // its handle for the parent `arr-set` — the record-FIELD twin of the top-level flags reader.
                let bits_leaf = *cursor;
                out.push(op::I32_CONST);
                crate::backend::wasm::encode::sleb128(field_bits.len() as i64, out);
                out.push(op::CALL);
                uleb128(imp("arr-alloc"), out); // [arr, i, nested-arr]
                for &(fslot, bit) in field_bits {
                    out.push(op::I32_CONST);
                    crate::backend::wasm::encode::sleb128(fslot as i64, out); // [.., nested-arr, fslot]
                    out.push(op::LOCAL_GET);
                    uleb128(bits_leaf as u64, out);
                    out.push(op::I32_CONST);
                    crate::backend::wasm::encode::sleb128(bit as i64, out);
                    out.push(op::I32_SHR_U);
                    out.push(op::I32_CONST);
                    crate::backend::wasm::encode::sleb128(1, out);
                    out.push(op::I32_AND); // [.., nested-arr, fslot, (bits>>bit)&1]
                    out.push(op::CALL);
                    uleb128(imp("box-bool"), out); // [.., nested-arr, fslot, bool]
                    out.push(op::CALL);
                    uleb128(imp("arr-set"), out); // [.., nested-arr]
                }
                *cursor += 1; // consumed the ONE packed-bitset leaf
            }
        }
        out.push(op::CALL);
        uleb128(imp("arr-set"), out); // → [arr]
    }
}

/// Emit the copy-in for one `BytesLeaf`: the list crossed the boundary as `(ptr, len)` at flattened core
/// params `ptr_leaf` (= `*cursor`) and `ptr_leaf + 1`. Build the guest `Bytes` in ONE bulk call —
/// `bytes-new((ptr, len))` — instead of `bytes-alloc` + a per-byte `bytes-set` loop (the ~1.9us/byte
/// reducer-fold marshaling cost, operator seq 916). The marshaling-boundary bytes are already CONTIGUOUS in
/// linear memory 0 (the core module owns it under `wrapper_needs_memory`), so the `list<u8>` arg lowers to
/// exactly the `(ptr_leaf, len_leaf)` core params the canon adapter copies across into the runtime heap.
/// Leaves the fresh `Bytes` handle on the stack (the caller `arr-set`s it AS-IS); the surrounding stack
/// (`[arr, i]`) is untouched — this is a balanced `[] -> [handle]`. An empty list (`len == 0`) yields the
/// shared immortal empty-`Bytes` singleton. The `buf`/`ctr` scratch locals are no longer needed (the bulk
/// call carries no loop), kept in the signature so callers need not renumber their reserved scratch.
pub(super) fn emit_bytes_leaf_copy_in(
    ptr_leaf: u32,
    ptr_from_i64: bool,
    buf: u32,
    ctr: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let len_leaf = ptr_leaf + 1;
    // Read the `ptr` core-param slot, wrapping i64→i32 when the variant JOIN widened it (a different-width
    // Result whose OTHER arm carries an i64 scalar at slot0 — erp1). The `len` slot is always i32.
    let get_ptr = |out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(ptr_leaf as u64, out);
        if ptr_from_i64 {
            out.push(op::I32_WRAP_I64);
        }
    };
    if bulk_bytes {
        // handle = bytes-new(ptr, len) — the `list<u8>` arg is canon-lowered to the `(ptr, len)` pair, read
        // straight out of linear memory 0; one cross-component call replaces the alloc + per-byte-set loop.
        // Only valid where the envelope provides a shared allocator at lower-time (the host-`_mem` assembler).
        get_ptr(out);
        out.push(op::LOCAL_GET);
        uleb128(len_leaf as u64, out);
        out.push(op::CALL);
        uleb128(imp("bytes-new"), out);
        // leaves the Bytes handle on the stack for the caller's arr-set ([] -> [handle]).
        return;
    }
    // PER-BYTE FALLBACK (no shared allocator at lower-time — e.g. a non-host typed interface, whose
    // runtime-op canon-lower cannot carry the Memory option `bytes-new` needs): `buf = bytes-alloc(len)`,
    // then loop `j in 0..len` copying `bytes-set(buf, j, i32.load8_u(ptr + j))` out of linear memory 0.
    // buf = bytes-alloc(len)
    out.push(op::LOCAL_GET);
    uleb128(len_leaf as u64, out);
    out.push(op::CALL);
    uleb128(imp("bytes-alloc"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    // ctr = 0
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(0, out);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    // block { loop { if ctr >= len br 1; buf = bytes-set(buf, ctr, load8(ptr + ctr)); ctr += 1; br 0 } }
    out.push(op::BLOCK);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(len_leaf as u64, out);
    out.push(op::I32_GE_U);
    out.push(op::BR_IF);
    uleb128(1, out);
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    get_ptr(out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_ADD);
    out.push(op::I32_LOAD8_U);
    out.push(0x00); // align 2^0
    out.push(0x00); // offset 0
    out.push(op::CALL);
    uleb128(imp("bytes-set"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(1, out);
    out.push(op::I32_ADD);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    out.push(op::BR);
    uleb128(0, out);
    out.push(op::END); // end loop
    out.push(op::END); // end block
    // leave buf on the stack for the caller's arr-set
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
}

/// Emit the lift for one top-level VALUE-FORM leaf param (`BigInt`/`Rational`/`Symbol`) — a type with no
/// scalar boundary rep that crosses as the canonical `list<u8>` value-form at flattened core params
/// `ptr_leaf` / `ptr_leaf + 1`. Copies those bytes out of linear memory 0 into a value-heap byte-leaf, bakes
/// the type's shape `desc` into a second byte-leaf, then `value-decode(bytes, desc)` reconstructs the
/// value-heap handle (the PARAM twin of `Core::ValueDecode`, R2). `value-decode` BORROWS both leaves, so the
/// wrapper (their owner) drops both here; the decoded handle is left on the stack as the def arg (`[]->[h]`).
/// `buf`/`ctr` are the two reusable byte-copy scratch locals; `desc_local`/`res_local` are two fresh i32
/// locals (the baked descriptor handle + the decoded-result stash). A NULL decode (a host contract
/// violation — a non-`Option` param's bytes are a valid encoding by contract) is left as-is; the borrowed
/// slice's post-call reclaim drops it like any handle.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_value_form_lift(
    desc: &[u8],
    ptr_leaf: u32,
    buf: u32,
    ctr: u32,
    desc_local: u32,
    res_local: u32,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // 1. Bake the descriptor into `desc_local` first (clean stack for the operand copy). `desc-buf =
    //    bytes-alloc(len)`, then thread it through one `bytes-set(buf, j, byte)` per descriptor byte (each
    //    returns the — possibly reallocated — handle, left on the stack for the next).
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(desc.len() as i64, out);
    out.push(op::CALL);
    uleb128(imp("bytes-alloc"), out); // [desc-buf]
    for (j, &byte) in desc.iter().enumerate() {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(j as i64, out);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(byte as i64, out);
        out.push(op::CALL);
        uleb128(imp("bytes-set"), out); // [desc-buf]
    }
    out.push(op::LOCAL_SET);
    uleb128(desc_local as u64, out); // [] descriptor stored
    // 2. Copy the boundary `(ptr, len)` value-form bytes into a value-heap byte-leaf; normalize into `buf`
    //    (the per-byte path already leaves it there, the bulk `bytes-new` path leaves it only on the stack).
    emit_bytes_leaf_copy_in(ptr_leaf, false, buf, ctr, bulk_bytes, imp, out); // → [bytes]
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out); // [] bytes stored in buf
    // 3. handle = value-decode(bytes, desc) — borrows both; stash the result (NULL on a mismatch).
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out); // [bytes]
    out.push(op::LOCAL_GET);
    uleb128(desc_local as u64, out); // [bytes, desc]
    out.push(op::CALL);
    uleb128(imp("value-decode"), out); // [handle-or-null]
    out.push(op::LOCAL_SET);
    uleb128(res_local as u64, out); // [] handle stashed
    // 4. Drop the borrowed-only temporaries (the copied bytes + the baked descriptor). The decoded value in
    //    `res_local` is independent of both, so this is safe before leaving it as the def arg.
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    out.push(op::LOCAL_GET);
    uleb128(desc_local as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    // 5. Leave the decoded handle on the stack as the def arg.
    out.push(op::LOCAL_GET);
    uleb128(res_local as u64, out); // [handle]
}

/// Emit the lift for one top-level `list<scalar>` param: the list crossed the boundary as `(ptr, len)` at
/// flattened core params `ptr_leaf` / `ptr_leaf + 1` (len = the ELEMENT count). Build a value-heap vec
/// (`vec-empty`) and loop `j in 0..len` pushing `box(load(ptr + j*stride))` per the [`ListElem`] — each
/// element read at its natural canonical stride, optionally i32→i64 extended (a narrow int), boxed, and
/// `vec-push`ed. Leaves the vec handle on the stack (the caller passes it as the def arg directly).
/// `buf`/`ctr` are the two reusable scratch locals; stack-balanced (`[]->[handle]`). An empty list yields
/// the fresh empty vec.
pub(super) fn emit_list_leaf_lift(
    elem: &ListElem,
    ptr_leaf: u32,
    buf: u32,
    ctr: u32,
    next_local: &mut u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // Build the outer list at boundary leaves (ptr_leaf, ptr_leaf+1) into `buf`, descending `nest_lists`
    // sub-list levels; then leave the outer vec handle on the stack for the def call.
    emit_list_level(
        elem,
        elem.nest_lists,
        ptr_leaf,
        ptr_leaf + 1,
        buf,
        ctr,
        next_local,
        imp,
        out,
    );
    out.push(op::LOCAL_GET);
    uleb128(buf as u64, out);
}

/// Build ONE list level into `buf` from the `(ptr_local, len_local)` descriptor, leaving the stack balanced
/// (the result is left in `buf`, NOT on the stack — the caller reads `buf`). `levels` = remaining nested
/// list depth: `0` → each element is the scalar leaf (load + box + push); `k>0` → each element is a
/// `(ptr, len)` sub-list at canonical stride 8 (load its ptr/len into fresh locals, recursively build it into
/// a fresh inner `buf`, then push that vec handle). `next_local` hands each nested level its own scratch
/// locals so no level clobbers another's cursor/accumulator.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_list_level(
    elem: &ListElem,
    levels: u32,
    ptr_local: u32,
    len_local: u32,
    buf: u32,
    ctr: u32,
    next_local: &mut u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    // A nested element is a `(ptr,len)` sub-list = 8 canonical bytes; a scalar leaf uses its own stride.
    let stride: u32 = if levels > 0 { 8 } else { elem.stride };
    // For a nested level, pre-allocate this level's per-element scratch (inner ptr/len + inner vec/cursor).
    // A BYTE-LEAF element (`list<string>`/`list<bytes>`) at the leaf level (levels == 0) also needs four
    // fresh locals: inner_ptr/inner_len read the element's `(ptr, len)` descriptor out of memory, and
    // inner_buf/inner_ctr are the byte copy-in's scratch pair (`emit_bytes_leaf_copy_in`). inner_len MUST be
    // inner_ptr + 1 (the copy-in reads its len at ptr_leaf + 1).
    let (inner_ptr, inner_len, inner_buf, inner_ctr) = if levels > 0 || elem.byte_leaf.is_some() {
        let base = *next_local;
        *next_local += 4;
        (base, base + 1, base + 2, base + 3)
    } else {
        (0, 0, 0, 0)
    };
    // buf = vec-empty()
    out.push(op::CALL);
    uleb128(imp("vec-empty"), out);
    out.push(op::LOCAL_SET);
    uleb128(buf as u64, out);
    // ctr = 0
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(0, out);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    // block { loop { if ctr >= len br 1; <build+push element>; ctr += 1; br 0 } }
    out.push(op::BLOCK);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(crate::backend::wasm::wasm_abi::BLOCK_EMPTY);
    // if ctr >= len -> br 1 (exit)
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::LOCAL_GET);
    uleb128(len_local as u64, out);
    out.push(op::I32_GE_U);
    out.push(op::BR_IF);
    uleb128(1, out);
    // addr = ptr + ctr*stride  (helper: pushes the element address onto the stack)
    let emit_addr = |out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(ptr_local as u64, out);
        out.push(op::LOCAL_GET);
        uleb128(ctr as u64, out);
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(stride as i64, out);
        out.push(op::I32_MUL);
        out.push(op::I32_ADD);
    };
    if levels == 0 && elem.byte_leaf.is_some() {
        // BYTE-LEAF element (`list<string>`/`list<bytes>`): the element at `addr` is a `(ptr, len)`
        // descriptor (8 bytes). Read it into inner_ptr/inner_len, then copy those bytes out of linear memory
        // 0 into a fresh value-heap byte-leaf (`emit_bytes_leaf_copy_in`, per-byte — no shared allocator on
        // the bare entry path) and push the handle. A String and a Bytes element share this lift (a Cadenza
        // String IS a flat UTF-8 byte-leaf), so `byte_leaf`'s bool is not consulted here.
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out); // align log2(4)
        uleb128(0, out); // offset 0 (ptr)
        out.push(op::LOCAL_SET);
        uleb128(inner_ptr as u64, out);
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out);
        uleb128(4, out); // offset 4 (len)
        out.push(op::LOCAL_SET);
        uleb128(inner_len as u64, out);
        // buf = vec-push(buf, bytes-copy-in(inner_ptr, inner_len))
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        emit_bytes_leaf_copy_in(inner_ptr, false, inner_buf, inner_ctr, false, imp, out); // [buf, handle]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else if levels == 0 && elem.compound.is_some() {
        // COMPOUND element (`list<tuple>`/`list<record>` of scalars): the element at `addr` occupies `stride`
        // (= its canonical_size) contiguous bytes. Build a fresh value-heap cell (`arr-alloc`), read each
        // field at its canonical offset from `addr`, box it, and `arr-set` it at its slot; then push the cell.
        // The stack threads `[buf, arr]` across the field loop (exactly `emit_cell_rebuild`'s convention),
        // then `vec-push(buf, arr)`.
        let fields = elem.compound.as_ref().unwrap();
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(fields.len() as i64, out);
        out.push(op::CALL);
        uleb128(imp("arr-alloc"), out); // [buf, arr]
        for (i, f) in fields.iter().enumerate() {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(i as i64, out); // [buf, arr, slot]
            emit_addr(out); // [buf, arr, slot, base]
            out.push(f.load_op);
            uleb128(f.load_align as u64, out);
            uleb128(f.offset as u64, out); // [buf, arr, slot, raw] (load at base + offset)
            if let Some(signed) = f.extend {
                out.push(if signed {
                    op::I64_EXTEND_I32_S
                } else {
                    op::I64_EXTEND_I32_U
                });
            }
            out.push(op::CALL);
            uleb128(imp(f.box_op), out); // [buf, arr, slot, boxed]
            out.push(op::CALL);
            uleb128(imp("arr-set"), out); // [buf, arr]
        }
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else if levels == 0 && elem.sum.is_some() {
        // SUM element (`list<option<scalar>>`): the element at `addr` is a canonical `option<T>` — a 1-byte
        // disc at offset 0 (None=0, Some=1), then the scalar payload at `payload_offset`. Branch on the disc
        // and build the guest sum cell: the Some arm reads+boxes the payload and `sum-new`s the payload arm;
        // the None arm `sum-new`s the nullary arm (inline unit). The result-typed `if` leaves the cell handle
        // on the stack (exactly `emit_sum_field`'s convention), which `vec-push` then appends to `buf`. The
        // memory-reading twin of the param-fed `emit_sum_arm`.
        let Some(s) = elem.sum.as_ref() else {
            unreachable!("guarded by elem.sum.is_some()")
        };
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        // disc = i32.load8_u(addr + 0)
        emit_addr(out); // [buf, addr]
        out.push(op::I32_LOAD8_U);
        uleb128(0, out); // align log2(1)
        uleb128(0, out); // offset 0 (disc)
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(s.boundary_true_disc as i64, out);
        out.push(op::I32_EQ);
        out.push(op::IF);
        out.push(crate::backend::wasm::wasm_abi::CORE_I32); // block type: → i32 (the sum handle)
        // Some arm: sum-new(some_decl_disc, box(load(addr + payload_offset)))
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(s.some_decl_disc as i64, out); // [buf, disc]
        emit_addr(out); // [buf, disc, addr]
        out.push(s.payload_load_op);
        uleb128(s.payload_load_align as u64, out);
        uleb128(s.payload_offset as u64, out); // [buf, disc, raw]
        if let Some(signed) = s.payload_extend {
            out.push(if signed {
                op::I64_EXTEND_I32_S
            } else {
                op::I64_EXTEND_I32_U
            });
        }
        out.push(op::CALL);
        uleb128(imp(s.payload_box), out); // [buf, disc, boxed]
        out.push(op::CALL);
        uleb128(imp("sum-new"), out); // [buf, some-cell]
        out.push(op::ELSE);
        // None arm: sum-new(none_decl_disc, IMM_UNIT)
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(s.none_decl_disc as i64, out); // [buf, disc]
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(
            crate::backend::wasm::runtime_abi::IMM_UNIT as i64,
            out,
        ); // [buf, disc, unit]
        out.push(op::CALL);
        uleb128(imp("sum-new"), out); // [buf, none-cell]
        out.push(op::END); // [buf, cell]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else if levels == 0 && elem.flags.is_some() {
        // FLAGS element (`list<flags>`): the element at `addr` is a PACKED bitset (`stride` = its canonical
        // width). Build a fresh value-heap record-of-bools cell (`arr-alloc`), then per `(slot, bit)` re-load
        // the packed bitset, mask `(bits >> bit) & 1`, `box-bool` it, and `arr-set` it at `slot`; then push the
        // cell. Re-loading the bitset per field (like the compound branch re-emits `emit_addr`) needs NO fresh
        // scratch local. The memory-reading twin of the top-level flags param unpack + the param-field
        // `FieldRebuild::Flags` cell rebuild.
        let f = elem.flags.as_ref().unwrap();
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(f.field_bits.len() as i64, out);
        out.push(op::CALL);
        uleb128(imp("arr-alloc"), out); // [buf, arr]
        for &(slot, bit) in &f.field_bits {
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(slot as i64, out); // [buf, arr, slot]
            emit_addr(out); // [buf, arr, slot, addr]
            out.push(f.load_op);
            uleb128(f.load_align as u64, out);
            uleb128(0, out); // [buf, arr, slot, bits] (packed bitset at offset 0)
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(bit as i64, out);
            out.push(op::I32_SHR_U); // [buf, arr, slot, bits>>bit]
            out.push(op::I32_CONST);
            crate::backend::wasm::encode::sleb128(1, out);
            out.push(op::I32_AND); // [buf, arr, slot, (bits>>bit)&1]
            out.push(op::CALL);
            uleb128(imp("box-bool"), out); // [buf, arr, slot, bool-handle]
            out.push(op::CALL);
            uleb128(imp("arr-set"), out); // [buf, arr]
        }
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else if levels == 0 {
        // buf = vec-push(buf, box(load(addr)))
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        emit_addr(out); // [buf, addr]
        out.push(elem.load_op);
        uleb128(elem.load_align as u64, out);
        uleb128(0, out); // offset 0 → [buf, elem]
        if let Some(signed) = elem.extend {
            out.push(if signed {
                op::I64_EXTEND_I32_S
            } else {
                op::I64_EXTEND_I32_U
            });
        }
        out.push(op::CALL);
        uleb128(imp(elem.box_op), out); // [buf, boxed]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    } else {
        // inner_ptr = i32.load(addr + 0); inner_len = i32.load(addr + 4) — the canonical list descriptor.
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out); // align log2(4)
        uleb128(0, out); // offset 0 (ptr)
        out.push(op::LOCAL_SET);
        uleb128(inner_ptr as u64, out);
        emit_addr(out);
        out.push(op::I32_LOAD);
        uleb128(2, out);
        uleb128(4, out); // offset 4 (len)
        out.push(op::LOCAL_SET);
        uleb128(inner_len as u64, out);
        // Recursively build the inner list into inner_buf (leaves the stack balanced), then push its handle.
        emit_list_level(
            elem,
            levels - 1,
            inner_ptr,
            inner_len,
            inner_buf,
            inner_ctr,
            next_local,
            imp,
            out,
        );
        out.push(op::LOCAL_GET);
        uleb128(buf as u64, out); // [buf]
        out.push(op::LOCAL_GET);
        uleb128(inner_buf as u64, out); // [buf, inner-vec]
        out.push(op::CALL);
        uleb128(imp("vec-push"), out); // [buf']
        out.push(op::LOCAL_SET);
        uleb128(buf as u64, out);
    }
    // ctr += 1
    out.push(op::LOCAL_GET);
    uleb128(ctr as u64, out);
    out.push(op::I32_CONST);
    crate::backend::wasm::encode::sleb128(1, out);
    out.push(op::I32_ADD);
    out.push(op::LOCAL_SET);
    uleb128(ctr as u64, out);
    out.push(op::BR);
    uleb128(0, out);
    out.push(op::END); // end loop
    out.push(op::END); // end block
}

/// Emit the RESULT-SPILL for a wrapper whose def returns a value-heap compound HANDLE: store the handle
/// (on the stack from the def `call`) to `rec`, allocate a `size`-byte return area (`cabi_realloc(0, 0,
/// align, size)`) into `retptr`, write the value's canonical form there via [`emit_canon_write`], and leave
/// `retptr` on the stack as the boundary result. `next_local` hands the writer fresh scratch locals. The canon
/// lift reads the value back out of memory from that pointer.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_result_spill(
    rec: u32,
    retptr: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    size: u32,
    align: u32,
    write: &CanonWrite,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    // rec = def result handle (currently on the stack).
    out.push(op::LOCAL_SET);
    uleb128(rec as u64, out);
    // retptr = cabi_realloc(old_ptr=0, old_size=0, align, size)
    const_i32(0, out);
    const_i32(0, out);
    const_i32(align as i64, out);
    const_i32(size as i64, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    out.push(op::LOCAL_SET);
    uleb128(retptr as u64, out);
    // Write the value's canonical form at retptr + 0.
    emit_canon_write(
        write,
        rec,
        retptr,
        0,
        next_local,
        realloc_abs,
        bulk_bytes,
        imp,
        out,
    );
    // Reclaim the def's RESULT handle: the def returned an OWNED compound (callee-owns-args → the caller, this
    // wrapper, owns the result), and the canonical writer only BORROWED it (arr-get/vec-get/sum-disc/bytes-len
    // are borrowing reads that retain nothing), so after the write `rec` holds the sole reference to the whole
    // value tree — `drop` deep-reclaims it (the tree's children too). Without this the spilled result cell
    // (+ its boxed children) LEAKED one per call (the SpillRecord-result known-leak class, SHAPE 60/62/63).
    out.push(op::LOCAL_GET);
    uleb128(rec as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
    // Return the area pointer.
    out.push(op::LOCAL_GET);
    uleb128(retptr as u64, out);
}

/// Lower a `list<u8>`/`Bytes` RESULT member (a def returning a value-heap Bytes handle) to the canonical
/// `list<u8>` return in ONE bulk call — `bytes-read(rec)` — instead of `bytes-len` + a per-byte `bytes-get`
/// copy loop (the ~1.9us/byte reducer-fold marshaling cost, operator seq 916). `bytes-read` is canon-lowered
/// with Memory+Realloc options, so its `list<u8>` result (2 flats > `MAX_FLAT_RESULTS`) is written by the
/// adapter — allocating the buffer via the guest's OWN `cabi_realloc` in guest memory 0 — into a
/// caller-provided 8-byte return area as `(ptr, len)` at `[+0]`/`[+4]`. That area IS the member's canonical
/// `list<u8>` return, so we allocate it, hand it to `bytes-read` as the trailing retptr arg, and return it
/// directly — no second buffer, no copy loop. `bytes-read` BORROWS `rec` (rc unchanged), so we still DROP the
/// owned def-result handle afterwards, exactly as the old `bytes-get` path did. `rec`/`retptr` are the two
/// scratch i32 locals the caller reserved; `next_local` is no longer drawn from (the bulk call has no loop
/// scratch), kept in the signature so callers need not change.
pub(super) fn emit_result_copy_bytes(
    rec: u32,
    retptr: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    let call = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(imp(name), out);
    };
    if bulk_bytes {
        // rec = the def's result Bytes handle (currently on the stack).
        set(rec, out);
        // retptr = cabi_realloc(0, 0, align=4, size=8) — the (ptr,len) return area bytes-read fills. Only
        // valid where the envelope provides a shared allocator at lower-time (the host-`_mem` assembler).
        const_i32(0, out);
        const_i32(0, out);
        const_i32(4, out);
        const_i32(8, out);
        out.push(op::CALL);
        uleb128(realloc_abs, out);
        set(retptr, out);
        // bytes-read(rec, retptr) -> () : one bulk call writes retptr[0]=guest ptr, retptr[4]=len (the buffer
        // allocated in guest memory 0 by the canon adapter's realloc). Args: buf first, then trailing retptr.
        get(rec, out);
        get(retptr, out);
        call("bytes-read", out);
        // Drop the owned def result handle (bytes-read only BORROWED it).
        get(rec, out);
        call("drop", out);
        // Return the area pointer (the member's canonical (ptr,len) list<u8> return).
        get(retptr, out);
        return;
    }
    // PER-BYTE FALLBACK (no shared allocator at lower-time): `bytes-len` + a `bytes-get` per-byte copy loop
    // into a `cabi_realloc`'d buffer, then write `(ptr, len)` into a `cabi_realloc`'d 8-byte return area.
    let (n, buf, i) = (*next_local, *next_local + 1, *next_local + 2);
    *next_local += 3;
    // rec = the def's result Bytes handle (currently on the stack).
    set(rec, out);
    // n = bytes-len(rec)
    get(rec, out);
    call("bytes-len", out);
    set(n, out);
    // buf = cabi_realloc(orig=0, orig_size=0, align=1, size=n)
    const_i32(0, out);
    const_i32(0, out);
    const_i32(1, out);
    get(n, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    set(buf, out);
    // COPY LOOP: i = 0; while i < n { store8(buf + i, bytes-get(rec, i)); i++ }
    const_i32(0, out);
    set(i, out);
    out.push(op::BLOCK);
    out.push(wasm_abi::BLOCK_EMPTY);
    out.push(op::LOOP);
    out.push(wasm_abi::BLOCK_EMPTY);
    {
        get(i, out);
        get(n, out);
        out.push(op::I32_GE_U);
        out.push(op::BR_IF);
        uleb128(1, out);
        get(buf, out);
        get(i, out);
        out.push(op::I32_ADD);
        get(rec, out);
        get(i, out);
        call("bytes-get", out);
        out.push(op::I32_STORE8);
        out.push(0x00);
        out.push(0x00);
        get(i, out);
        const_i32(1, out);
        out.push(op::I32_ADD);
        set(i, out);
        out.push(op::BR);
        uleb128(0, out);
    }
    out.push(op::END);
    out.push(op::END);
    // retptr = cabi_realloc(0, 0, align=4, size=8) — the (ptr,len) return area.
    const_i32(0, out);
    const_i32(0, out);
    const_i32(4, out);
    const_i32(8, out);
    out.push(op::CALL);
    uleb128(realloc_abs, out);
    set(retptr, out);
    // retptr[0] = buf (ptr), retptr[4] = n (len) — i32 stores, 4-byte aligned.
    get(retptr, out);
    get(buf, out);
    out.push(op::I32_STORE);
    out.push(0x02);
    out.push(0x00);
    get(retptr, out);
    get(n, out);
    out.push(op::I32_STORE);
    out.push(0x02);
    out.push(0x04);
    // Drop the def result handle (the wrapper consumed it into the buffer).
    get(rec, out);
    call("drop", out);
    // Return the area pointer.
    get(retptr, out);
}

/// Recursively write ONE value-heap value's canonical-ABI form into linear memory at `dst_base + offset`.
/// `handle` is the local holding the value's runtime handle; `dst_base` a local holding the base address;
/// `offset` a static byte offset. `next_local` hands out fresh i32 scratch locals (per-level handles, list
/// loop counters). See [`CanonWrite`] for the per-kind plan.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_canon_write(
    cw: &CanonWrite,
    handle: u32,
    dst_base: u32,
    offset: u32,
    next_local: &mut u32,
    realloc_abs: u64,
    bulk_bytes: bool,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let const_i32 = |v: i64, out: &mut Vec<u8>| {
        out.push(op::I32_CONST);
        crate::backend::wasm::encode::sleb128(v, out);
    };
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    let set = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_SET);
        uleb128(l as u64, out);
    };
    let call = |name: &str, out: &mut Vec<u8>| {
        out.push(op::CALL);
        uleb128(imp(name), out);
    };
    // Emit an `i32.store` (align hint 4, offset `off`) — the caller pushed [addr, value] first.
    let store_i32_at = |off: u32, out: &mut Vec<u8>| {
        out.push(op::I32_STORE);
        out.push(0x02);
        uleb128(off as u64, out);
    };
    match cw {
        CanonWrite::Scalar {
            read,
            wrap_i64,
            store,
        } => {
            // store(dst_base + offset) = [wrap] read(handle)
            get(dst_base, out);
            get(handle, out);
            call(read, out);
            if *wrap_i64 {
                out.push(op::I32_WRAP_I64);
            }
            out.push(*store);
            out.push(0x00); // align hint (conservative)
            uleb128(offset as u64, out);
        }
        CanonWrite::EnumDisc { store, remap } => {
            // store(dst_base + offset) = the WIT disc. `handle` is the BOXED enum value in the arr cell, so
            // UNBOX it (`get-int` → i64, `i32.wrap` → the guest disc). Then, on a reorder, remap guest->WIT.
            get(dst_base, out); // [base] (store addr, stays at the stack bottom)
            match remap {
                // Order matches: the guest disc IS the WIT case index — unbox and store it.
                None => {
                    get(handle, out); // [base, box]
                    call("get-int", out); // [base, i64 disc]
                    out.push(op::I32_WRAP_I64); // [base, i32 guest disc = WIT index]
                }
                // Reorder: unbox into a temp local `g`, then remap g -> WIT disc BY NAME via a `select`-fold.
                // Seed acc=0; for each guest disc `gi`: acc = (g != gi) ? acc : guest_to_wit[gi] (select pops
                // [a,b,cond] → cond?a:b). Exactly one `gi` matches a valid disc, so the seed is irrelevant.
                Some(guest_to_wit) => {
                    let g = *next_local;
                    *next_local += 1;
                    get(handle, out);
                    call("get-int", out);
                    out.push(op::I32_WRAP_I64);
                    set(g, out); // g = guest disc (i32)
                    const_i32(0, out); // [base, acc=0]
                    for (gi, &wit_disc) in guest_to_wit.iter().enumerate() {
                        const_i32(wit_disc as i64, out); // [base, acc, wit_gi]
                        get(g, out); // [base, acc, wit_gi, g]
                        const_i32(gi as i64, out); // [base, acc, wit_gi, g, gi]
                        out.push(op::I32_NE); // [base, acc, wit_gi, cond=(g!=gi)]
                        out.push(op::SELECT); // [base, acc']
                    }
                    // [base, remapped WIT disc]
                }
            }
            out.push(*store);
            out.push(0x00); // align hint (conservative)
            uleb128(offset as u64, out);
        }
        CanonWrite::Flags { field_bits, store } => {
            // store(dst_base + offset) = the packed bitset. `handle` is the guest record-of-bools cell; pack
            // per `(slot, bit)`: `get-bool(arr-get(handle, slot))` shifted into `bit`, OR-ed onto a seeded 0.
            // No unbox `read` (bools come from `get-bool`); no fresh locals (a pure stack expression).
            get(dst_base, out); // [base]
            const_i32(0, out); // [base, acc=0]
            for &(slot, bit) in field_bits {
                get(handle, out); // [.., acc, handle]
                const_i32(slot as i64, out);
                call("arr-get", out); // [.., acc, bool-box] (borrows handle)
                call("get-bool", out); // [.., acc, 0/1]
                const_i32(bit as i64, out);
                out.push(op::I32_SHL); // [.., acc, (0/1)<<bit]
                out.push(op::I32_OR); // [.., acc']
            }
            out.push(*store); // [base, bitset] → store
            out.push(0x00); // align hint (conservative)
            uleb128(offset as u64, out);
        }
        CanonWrite::Record { fields } => {
            for f in fields {
                let fh = *next_local;
                *next_local += 1;
                get(handle, out);
                const_i32(f.index as i64, out);
                call("arr-get", out);
                set(fh, out);
                emit_canon_write(
                    &f.write,
                    fh,
                    dst_base,
                    offset + f.offset,
                    next_local,
                    realloc_abs,
                    bulk_bytes,
                    imp,
                    out,
                );
            }
        }
        CanonWrite::Bytes if bulk_bytes => {
            // BULK (seq-916 loop-ii): `bytes-read(handle, dst_base+offset)` writes the (ptr,len) pair straight
            // into this field's canonical `list<u8>` slot — retptr[0]=guest ptr, retptr[4]=len — in ONE call,
            // replacing `bytes-len` + `cabi_realloc` + the per-byte `bytes-get` copy loop the fallback below
            // does (the nested-Bytes LOWER twin of `emit_result_copy_bytes`'s bare-result bulk). The guest's
            // canon adapter allocates the buffer in memory 0 via its own realloc, exactly like the fallback's
            // `cabi_realloc(0,0,1,count)`, so the resulting (ptr,len) slot is identical downstream. `bytes-read`
            // BORROWS `handle` (rc unchanged) and `handle` is itself a borrowing `arr-get` read the enclosing
            // `emit_result_spill` deep-drops with the whole tree — so, exactly like the per-byte `bytes-get`
            // path, NO drop is emitted here. Only valid where the assembler canon-lowers `bytes-read` + provides
            // the shared allocator (the `import_realloc`/`_mem` mode the caller gates on).
            get(handle, out); // buf
            get(dst_base, out);
            const_i32(offset as i64, out);
            out.push(op::I32_ADD); // retptr = dst_base + offset (this field's (ptr,len) slot)
            call("bytes-read", out);
        }
        CanonWrite::Bytes => {
            // PER-BYTE FALLBACK (no shared allocator / no bytes-read canon-lower at lower-time):
            // count = bytes-len(handle); ptr = cabi_realloc(0,0,1,count); copy loop; store (ptr, count).
            let count = *next_local;
            let ptr = *next_local + 1;
            let i = *next_local + 2;
            *next_local += 3;
            get(handle, out);
            call("bytes-len", out);
            set(count, out);
            const_i32(0, out);
            const_i32(0, out);
            const_i32(1, out); // align 1 for bytes
            get(count, out);
            out.push(op::CALL);
            uleb128(realloc_abs, out);
            set(ptr, out);
            // store (ptr, count) at (dst_base+offset, dst_base+offset+4)
            get(dst_base, out);
            get(ptr, out);
            store_i32_at(offset, out);
            get(dst_base, out);
            get(count, out);
            store_i32_at(offset + 4, out);
            // copy loop: i=0; while i<count: store8(ptr+i, bytes-get(handle,i)); i++
            const_i32(0, out);
            set(i, out);
            out.push(op::BLOCK);
            out.push(wasm_abi::BLOCK_EMPTY);
            out.push(op::LOOP);
            out.push(wasm_abi::BLOCK_EMPTY);
            get(i, out);
            get(count, out);
            out.push(op::I32_GE_U);
            out.push(op::BR_IF);
            uleb128(1, out);
            // store8(ptr + i, bytes-get(handle, i))
            get(ptr, out);
            get(i, out);
            out.push(op::I32_ADD);
            get(handle, out);
            get(i, out);
            call("bytes-get", out);
            out.push(op::I32_STORE8);
            out.push(0x00);
            uleb128(0, out);
            get(i, out);
            const_i32(1, out);
            out.push(op::I32_ADD);
            set(i, out);
            out.push(op::BR);
            uleb128(0, out);
            out.push(op::END); // loop
            out.push(op::END); // block
        }
        CanonWrite::List {
            elem_size,
            elem_align,
            elem,
        } => {
            // count = vec-len(handle); base = cabi_realloc(0,0,elem_align, count*elem_size);
            let count = *next_local;
            let base = *next_local + 1;
            let i = *next_local + 2;
            let eh = *next_local + 3;
            let edst = *next_local + 4;
            *next_local += 5;
            get(handle, out);
            call("vec-len", out);
            set(count, out);
            const_i32(0, out);
            const_i32(0, out);
            const_i32(*elem_align as i64, out);
            get(count, out);
            const_i32(*elem_size as i64, out);
            out.push(op::I32_MUL);
            out.push(op::CALL);
            uleb128(realloc_abs, out);
            set(base, out);
            // store (base, count) at (dst_base+offset, +4)
            get(dst_base, out);
            get(base, out);
            store_i32_at(offset, out);
            get(dst_base, out);
            get(count, out);
            store_i32_at(offset + 4, out);
            // loop i in 0..count: eh = vec-get(handle,i); edst = base + i*elem_size; write elem at (edst,0)
            const_i32(0, out);
            set(i, out);
            out.push(op::BLOCK);
            out.push(wasm_abi::BLOCK_EMPTY);
            out.push(op::LOOP);
            out.push(wasm_abi::BLOCK_EMPTY);
            get(i, out);
            get(count, out);
            out.push(op::I32_GE_U);
            out.push(op::BR_IF);
            uleb128(1, out);
            get(handle, out);
            get(i, out);
            call("vec-get", out);
            set(eh, out);
            get(base, out);
            get(i, out);
            const_i32(*elem_size as i64, out);
            out.push(op::I32_MUL);
            out.push(op::I32_ADD);
            set(edst, out);
            emit_canon_write(
                elem,
                eh,
                edst,
                0,
                next_local,
                realloc_abs,
                bulk_bytes,
                imp,
                out,
            );
            get(i, out);
            const_i32(1, out);
            out.push(op::I32_ADD);
            set(i, out);
            out.push(op::BR);
            uleb128(0, out);
            out.push(op::END); // loop
            out.push(op::END); // block
        }
        CanonWrite::Variant {
            disc_store,
            payload_offset,
            arms,
        } => {
            // d = sum-disc(handle) — the guest's DECL disc.
            let d = *next_local;
            *next_local += 1;
            get(handle, out);
            call("sum-disc", out);
            set(d, out);
            // Per arm k: if d == k, store its boundary disc at dst_base+offset, then (if payload) write it.
            for (k, arm) in arms.iter().enumerate() {
                get(d, out);
                const_i32(k as i64, out);
                out.push(op::I32_EQ);
                out.push(op::IF);
                out.push(wasm_abi::BLOCK_EMPTY);
                // store boundary disc
                get(dst_base, out);
                const_i32(arm.boundary_disc as i64, out);
                out.push(*disc_store);
                out.push(0x00); // align hint
                uleb128(offset as u64, out);
                if let Some(pw) = &arm.payload {
                    let ph = *next_local;
                    *next_local += 1;
                    get(handle, out);
                    call("sum-payload", out);
                    set(ph, out);
                    emit_canon_write(
                        pw,
                        ph,
                        dst_base,
                        offset + payload_offset,
                        next_local,
                        realloc_abs,
                        bulk_bytes,
                        imp,
                        out,
                    );
                }
                out.push(op::END); // if
            }
        }
    }
}

/// Push a closure `call`'s arguments onto the stack in the lifted body's order, threading ZERO OR MORE
/// fixed-shape tuple-arg REBUILDS among the scalar core params. `tuples` are in ascending `base_param` order
/// (each rebuild carries the core-param index its flattened leaves start at); the flattened core params run
/// `1..1+arity` (after `self`=0). Walks the core-param range: a param that starts a tuple's leaves emits that
/// tuple's rebuilt cell (via [`emit_tuple_rebuild`], stashed at `tuple_local + tuple_index` for the
/// post-dispatch drop) and skips its leaves; any other param is a plain scalar `local.get`. With `tuples`
/// empty, byte-identical to the raw scalar push; with one tuple, byte-identical to the prior single-tuple
/// interleave. Shared by the scalar `call` body + every list-result `call` body.
pub(super) fn emit_closure_call_args(
    tuples: &[TupleArgRebuild],
    tuple_local: u32,
    arity: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    emit_closure_call_args_with_sums(tuples, tuple_local, &[], 0, arity, imp, out)
}

/// [`emit_closure_call_args`] with ZERO OR MORE fixed-shape SUM-arg rebuilds interleaved among the scalars +
/// tuples. Each sum consumes `1` (nullary payload variant) or `2` (disc + one scalar payload) flattened core
/// params from its `base_param`; at a sum's `base_param` the walk emits its rebuilt cell (stashed at
/// `sum_local + i`) and skips its params. Sums + tuples are non-overlapping. With `sums` empty, byte-identical
/// to [`emit_closure_call_args`]. (This increment wires only sums; a tuple + sum together is a later widening.)
pub(super) fn emit_closure_call_args_with_sums(
    tuples: &[TupleArgRebuild],
    tuple_local: u32,
    sums: &[SumArgRebuild],
    sum_local: u32,
    arity: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    let get = |l: u32, out: &mut Vec<u8>| {
        out.push(op::LOCAL_GET);
        uleb128(l as u64, out);
    };
    // Walk the flattened core params `1..1+arity`; at each tuple's/sum's `base_param` emit its rebuild + skip
    // its params, else push the scalar. `tuples`/`sums` are ascending by `base_param` + non-overlapping.
    let mut a = 1u32;
    while a < 1 + arity {
        if let Some((ti, rebuild)) = tuples.iter().enumerate().find(|(_, t)| t.base_param == a) {
            emit_tuple_rebuild(rebuild, tuple_local + ti as u32, imp, out);
            a += rebuild
                .fields
                .iter()
                .map(FieldRebuild::leaf_count)
                .sum::<u32>();
        } else if let Some((si, rebuild)) = sums.iter().enumerate().find(|(_, s)| s.base_param == a)
        {
            emit_sum_arg_rebuild(rebuild, sum_local + si as u32, imp, out);
            // Skip the sum's flattened params: disc (1) + the payload's leaves. A scalar `option`/`result`
            // flattens to `(disc, payload)` = 2; a COMPOUND (Option-of-tuple) payload spans `1 + its leaves`.
            a += rebuild.flattened_param_count();
        } else {
            get(a, out);
            a += 1;
        }
    }
}

/// Drop the REBUILT tuple-arg cell (an owned per-call temporary the `call` fabricated) after `call_indirect`.
/// Unconditional (both own + borrow — the host owns only the closure handle, never this fabricated arg cell),
/// balancing the `arr-alloc` in [`emit_tuple_rebuild`]. Leaves the stack unchanged (drop returns nothing).
pub(super) fn emit_tuple_rebuilt_drop(
    tuple_local: u32,
    imp: &dyn Fn(&str) -> u64,
    out: &mut Vec<u8>,
) {
    use crate::backend::wasm::wasm_abi::op;
    out.push(op::LOCAL_GET);
    uleb128(tuple_local as u64, out);
    out.push(op::CALL);
    uleb128(imp("drop"), out);
}
