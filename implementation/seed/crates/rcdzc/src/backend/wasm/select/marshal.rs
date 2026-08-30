use super::*;

/// Marshal a value-heap `List<T>` host ARG (handle in `list_slot`) into the shared `mem`: an OUTER array of
/// `count` element slots (each `stride(T)` bytes) laid at the running `cursor`; leaves `(outer-ptr, count)` on
/// the operand stack (the core `(ptr,count)` the `(list <T>)` param lowers to). Per element (`vec-get`):
///  • a `Bytes` element (`list<u8>`, stride 8) copies its rope into `mem` AFTER the array + writes `(ptr,len)`
///    into the slot (the `set-edges` `list<list<u8>>` shape) — the `cursor` advances past each rope;
///  • a SCALAR element (stride = its canonical size 1/2/4/8) unboxes (`get-int`/`bool`/`float`/`float32`) and
///    writes the value INLINE into the slot with the width store (`i32.store`/`i64.store`/`f*.store`/store16/8,
///    narrowing a `get-int` i64 to a sub-64 int slot) — no extra region.
///  • a NESTED list element (`list<list<T>>`, stride 8) RECURSES — marshals the inner list into `mem` at the
///    running `cursor` (its own outer array + data, past this level's), then writes its `(ptr, count)` header
///    into the slot (arbitrary nesting depth rides this recursion).
/// The arg-side inverse of `emit_result_lift`'s `List` arm (which READS this exact layout). `list_slot` is
/// BORROWED. A non-`Bytes`/non-scalar/non-list element (a record/tuple/variant) DECLINES (a later increment).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_list_arg_marshal(
    db: &mut Db,
    elem: &Ty,
    // The ELEMENT's declared WIT type (from the list param's `WitType::List(elem)`), when known — needed to
    // order a RECORD element's fields to the host's DECLARATION order (`emit_record_to_mem`), and threaded to
    // the inner list on a nested `list<list<…>>`. `None` for a scalar/bytes element (offset-agnostic).
    elem_wit: Option<&crate::wit_world::WitType>,
    list_slot: u32,
    cursor: u32,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let is_bytes = matches!(elem.strip_nominal(), Ty::Bytes | Ty::String);
    // A NESTED list element (`list<list<T>>`): the element itself crosses as an 8-byte `(ptr, count)` header
    // written into the outer slot, its backing array + element data laid recursively AFTER the outer array
    // (a recursive `emit_list_arg_marshal` on the inner element). Neither a bytes-rope copy nor a scalar
    // inline store — this is the recursion that generalizes the arg-side list marshal to arbitrary depth.
    let nested_list_inner: Option<Ty> = if is_bytes {
        None
    } else if let Ty::List(inner) = elem.strip_nominal() {
        Some((**inner).clone())
    } else {
        None
    };
    let is_nested_list = nested_list_inner.is_some();
    // A RECORD element (`list<record{…}>`, the reducer `list<request>` shape): the element is written IN PLACE
    // into the outer array slot at its canonical record layout (each field at its offset; a Bytes field's rope
    // spills after the array), via `emit_record_to_mem`. Not a scalar inline store — a distinct writer.
    let record_elem: Option<std::collections::BTreeMap<crate::resolved::Symbol, Ty>> =
        if is_bytes || is_nested_list {
            None
        } else if let Ty::Record(fs) = elem.strip_nominal() {
            Some((**fs).clone())
        } else {
            None
        };
    let is_record = record_elem.is_some();
    // A TUPLE element (`list<tuple<…>>`): the POSITIONAL product — written in place at its canonical layout
    // (cell i = element i), via `emit_tuple_to_mem`. Same in-place write as a record element, no name reorder.
    let tuple_elem: Option<Vec<Ty>> = if is_bytes || is_nested_list || is_record {
        None
    } else if let Ty::Tuple(es) = elem.strip_nominal() {
        Some(es.iter().cloned().collect())
    } else {
        None
    };
    let is_tuple = tuple_elem.is_some();
    // An `option<scalar>` element (`list<option<s64>>`): written in place at its canonical option layout (disc
    // byte + payload scalar) by `emit_option_to_mem`. Carries the payload `Ty` + the guest decl's some-disc.
    let option_elem: Option<(Ty, i32)> = if is_bytes || is_nested_list || is_record || is_tuple {
        None
    } else if let Some(payload) =
        crate::backend::wasm::host::option_payload_ty(db, elem).filter(|p| valtype_of(p).is_some())
    {
        let crate::ty::Ty::Sum { decl, .. } = elem.strip_nominal() else {
            unreachable!("option is a Sum")
        };
        let some_disc = db
            .type_decl_by_occ(*decl)
            .and_then(|d| d.variants.iter().position(|v| v.payloads.len() == 1))
            .map(|i| i as i32);
        some_disc.map(|sd| (payload, sd))
    } else {
        None
    };
    let is_option = option_elem.is_some();
    // A general `variant<scalar>` element (`list<variant{a, b(s64), …}>`): written in place at its canonical
    // variant layout (disc + uniform scalar payload) by `emit_variant_to_mem` — the N-case generalization of
    // the option element. Detected AFTER option/result (they take their own arms), so this is the residual
    // general variant. Scoped to a uniform single scalar payload (the #3368 record-field variant shape).
    let is_variant: bool = if is_bytes || is_nested_list || is_record || is_tuple || is_option {
        false
    } else {
        crate::backend::wasm::host::variant_scalar_payload_cases(db, elem).is_some()
    };
    // The store for a scalar element, by its slot valtype + canonical size: i64→i64.store, f64→f64.store,
    // f32→f32.store, i32 → 4-byte i32.store / 2-byte store16 / 1-byte store8.
    let scalar_store: Option<Lir> = if is_bytes
        || is_nested_list
        || is_record
        || is_tuple
        || is_option
        || is_variant
    {
        None
    } else {
        let (size, _) = canonical_layout(db, elem);
        Some(match (valtype_of(elem), size) {
            (Some(ValType::I64), _) => Lir::I64Store { offset: 0 },
            (Some(ValType::F64), _) => Lir::F64Store { offset: 0 },
            (Some(ValType::F32), _) => Lir::F32Store { offset: 0 },
            (Some(ValType::I32), 4) => Lir::I32Store { offset: 0 },
            (Some(ValType::I32), 2) => Lir::I32Store16 { offset: 0 },
            (Some(ValType::I32), 1) => Lir::I32Store8 { offset: 0 },
            _ => {
                return Err(Reject::decline(
                    "a `list<T>` host-arg with a non-`list<u8>`/non-scalar/non-list element is not yet \
                     marshaled (a later increment)",
                ));
            }
        })
    };
    // A `list<T>` element (incl. a nested list) crosses as an 8-byte `(ptr, count)` header; `canonical_layout`
    // gives (8, 4) for `List`, so `stride`/`elem_align` cover the nested-list case with no special value.
    let stride: u32 = if is_bytes {
        8
    } else {
        canonical_layout(db, elem).0
    };
    // The outer element array's alignment: a `list<u8>`/`list<String>` element crosses as an
    // `(ptr, len)` header (align 4); a scalar element as its canonical width's alignment. The canonical
    // ABI requires a `list<T>`'s pointer to be aligned to `alignment(T)`, so the running byte-granular
    // cursor must be rounded up before the outer array is placed (see the cursor alignment below).
    let elem_align: u32 = if is_bytes {
        4
    } else {
        canonical_layout(db, elem).1
    };
    let read = if is_bytes || is_nested_list || is_record || is_tuple || is_option || is_variant {
        None
    } else {
        Some(get_op_ty(db, elem)?.ok_or_else(|| {
            Reject::decline("a `list<T>` scalar element has no unbox op (not a value-heap scalar)")
        })?)
    };
    let count = work_base;
    let (outer, i, eh, ilen, ipos, slotaddr) = (
        count + 1,
        count + 2,
        count + 3,
        count + 4,
        count + 5,
        count + 6,
    );
    *high = (*high).max(slotaddr + 1);
    for s in [count, outer, i, eh, ilen, ipos, slotaddr] {
        scratch_ty.insert(s, ValType::I32);
    }
    // count = vec-len(list); outer = cursor; cursor += count * stride (reserve the outer element array).
    out.push(Lir::LocalGet(list_slot));
    out.push(Lir::CallImport(OP_VEC_LEN));
    out.push(Lir::LocalSet(count));
    // Align the byte-granular cursor UP to the element alignment before placing the outer array — the
    // canonical ABI rejects a `list<T>` whose pointer is not `alignment(T)`-aligned ("list pointer is
    // not aligned"). Prior scalar/bytes args leave the cursor at an arbitrary byte offset, so a
    // `list<list<u8>>`/`list<scalar>` arg placed straight at it would trap at the host's list.lift.
    if elem_align > 1 {
        out.push(Lir::LocalGet(cursor));
        out.push(Lir::ConstI32((elem_align - 1) as i32));
        out.push(Lir::I32Add);
        out.push(Lir::ConstI32(!((elem_align - 1) as i32)));
        out.push(Lir::I32And);
        out.push(Lir::LocalSet(cursor));
    }
    out.push(Lir::LocalGet(cursor));
    out.push(Lir::LocalSet(outer));
    out.push(Lir::LocalGet(cursor));
    out.push(Lir::LocalGet(count));
    out.push(Lir::ConstI32(stride as i32));
    out.push(Lir::I32Mul);
    out.push(Lir::I32Add);
    out.push(Lir::LocalSet(cursor));
    out.push(Lir::ConstI32(0));
    out.push(Lir::LocalSet(i));
    out.push(Lir::Block(BlockType::Empty)); // $elems-done
    out.push(Lir::Loop(BlockType::Empty)); // $elems
    out.push(Lir::LocalGet(i));
    out.push(Lir::LocalGet(count));
    out.push(Lir::I32GeS);
    out.push(Lir::BrIf(1)); // i >= count → $elems-done
    // eh = vec-get(list, i); slotaddr = outer + i*stride.
    out.push(Lir::LocalGet(list_slot));
    out.push(Lir::LocalGet(i));
    out.push(Lir::CallImport(OP_VEC_GET));
    out.push(Lir::LocalSet(eh));
    out.push(Lir::LocalGet(outer));
    out.push(Lir::LocalGet(i));
    out.push(Lir::ConstI32(stride as i32));
    out.push(Lir::I32Mul);
    out.push(Lir::I32Add);
    out.push(Lir::LocalSet(slotaddr));
    if is_bytes {
        // ilen = bytes-len(eh); the rope copies to `cursor` (its inner ptr).
        out.push(Lir::LocalGet(eh));
        out.push(Lir::CallImport(OP_BYTES_LEN));
        out.push(Lir::LocalSet(ilen));
        out.push(Lir::ConstI32(0));
        out.push(Lir::LocalSet(ipos));
        out.push(Lir::Block(BlockType::Empty)); // $copy-done
        out.push(Lir::Loop(BlockType::Empty)); // $copy
        out.push(Lir::LocalGet(ipos));
        out.push(Lir::LocalGet(ilen));
        out.push(Lir::I32GeS);
        out.push(Lir::BrIf(1)); // pos >= ilen → $copy-done
        out.push(Lir::LocalGet(cursor));
        out.push(Lir::LocalGet(ipos));
        out.push(Lir::I32Add); // addr = cursor + pos
        out.push(Lir::LocalGet(eh));
        out.push(Lir::LocalGet(ipos));
        out.push(Lir::CallImport(OP_BYTES_GET)); // byte
        out.push(Lir::I32Store8 { offset: 0 });
        out.push(Lir::LocalGet(ipos));
        out.push(Lir::ConstI32(1));
        out.push(Lir::I32Add);
        out.push(Lir::LocalSet(ipos));
        out.push(Lir::Br(0)); // → $copy
        out.push(Lir::End); // $copy
        out.push(Lir::End); // $copy-done
        // outer[i] = (inner-ptr = cursor, ilen); cursor += ilen.
        out.push(Lir::LocalGet(slotaddr));
        out.push(Lir::LocalGet(cursor));
        out.push(Lir::I32Store { offset: 0 }); // ptr
        out.push(Lir::LocalGet(slotaddr));
        out.push(Lir::LocalGet(ilen));
        out.push(Lir::I32Store { offset: 4 }); // len
        out.push(Lir::LocalGet(cursor));
        out.push(Lir::LocalGet(ilen));
        out.push(Lir::I32Add);
        out.push(Lir::LocalSet(cursor));
    } else if let Some(inner) = &nested_list_inner {
        // NESTED list element: recurse to marshal the inner `List<T>` into `mem` (its own outer array + element
        // data laid at the SHARED running `cursor`, past this level's outer array), then store the
        // `(inner-ptr, inner-count)` header it leaves on the stack into the outer slot at `slotaddr`
        // (`ptr@0`, `count@4`) — the exact `(ptr, count)` layout `emit_result_lift`'s List arm reads back. The
        // recursion takes a work_base ABOVE this level's scratch (`+9`) so a nested marshal never reuses a
        // live slot; `eh` (the element handle, a borrowed inner `List`) is its `list_slot`.
        let nl_ptr = work_base + 7;
        let nl_count = work_base + 8;
        for s in [nl_ptr, nl_count] {
            scratch_ty.insert(s, ValType::I32);
        }
        *high = (*high).max(work_base + 9);
        // The inner list's own element WIT (a `list<list<…>>`'s `WitType::List(inner)`), threaded so a
        // record deeper in the nest still orders its fields to the host declaration order.
        let inner_wit = match elem_wit {
            Some(crate::wit_world::WitType::List(iw)) => Some(iw.as_ref()),
            _ => None,
        };
        emit_list_arg_marshal(
            db,
            inner,
            inner_wit,
            eh,
            cursor,
            work_base + 9,
            high,
            scratch_ty,
            out,
        )?;
        // stack: [inner-ptr, inner-count] → pop count (top) then ptr, store the header at slotaddr.
        out.push(Lir::LocalSet(nl_count));
        out.push(Lir::LocalSet(nl_ptr));
        out.push(Lir::LocalGet(slotaddr));
        out.push(Lir::LocalGet(nl_ptr));
        out.push(Lir::I32Store { offset: 0 });
        out.push(Lir::LocalGet(slotaddr));
        out.push(Lir::LocalGet(nl_count));
        out.push(Lir::I32Store { offset: 4 });
    } else if let Some(rec_fields) = &record_elem {
        // RECORD element (`list<record{…}>`): write the value-heap record IN PLACE into the outer slot at
        // `slotaddr` per its canonical record layout — each field at its declaration offset, a Bytes field's
        // rope spilling after the array at the shared `cursor`. `eh` is the borrowed element record handle;
        // the field ORDER follows the host WIT record decl order. work_base ABOVE this level's scratch (`+9`).
        let ew = elem_wit.ok_or_else(|| {
            Reject::decline(
                "a `list<record>` host-arg needs the element record's WIT type to order its fields",
            )
        })?;
        emit_record_to_mem(
            db,
            eh,
            slotaddr,
            rec_fields,
            ew,
            cursor,
            work_base + 9,
            high,
            scratch_ty,
            out,
        )?;
    } else if let Some(tup_elems) = &tuple_elem {
        // TUPLE element (`list<tuple<…>>`): write the value-heap tuple IN PLACE into the outer slot at
        // `slotaddr` per its canonical (positional) layout — element i at its offset; a Bytes element's rope
        // spills after the array at the shared `cursor`. `eh` is the borrowed element tuple handle. A tuple's
        // WIT order IS its element order (no name reorder), so `elem_wit` is not needed for ordering.
        emit_tuple_to_mem(
            db,
            eh,
            slotaddr,
            tup_elems,
            cursor,
            work_base + 9,
            high,
            scratch_ty,
            out,
        )?;
    } else if let Some((payload_ty, some_disc)) = &option_elem {
        // OPTION element (`list<option<scalar>>`): write the value-heap Option IN PLACE into the outer slot at
        // `slotaddr` per its canonical option layout (disc byte + payload scalar), via `emit_option_to_mem`.
        // `eh` is the borrowed element Option handle. work_base ABOVE this level's scratch (`+9`).
        emit_option_to_mem(
            db,
            eh,
            slotaddr,
            payload_ty,
            *some_disc,
            work_base + 9,
            high,
            scratch_ty,
            out,
        )?;
    } else if is_variant {
        // VARIANT<scalar> element (`list<variant{a, b(s64), …}>`): write the value-heap variant IN PLACE into
        // the outer slot at `slotaddr` per its canonical variant layout (disc + uniform scalar payload), via
        // `emit_variant_to_mem`. `eh` is the borrowed element variant handle. work_base ABOVE this level (`+9`).
        emit_variant_to_mem(db, eh, slotaddr, elem, work_base + 9, high, scratch_ty, out)?;
    } else {
        // scalar element: outer[i] = unbox(eh), width-narrowed, stored INLINE (no cursor advance).
        let read = read.expect("a scalar element has an unbox op");
        out.push(Lir::LocalGet(slotaddr)); // store addr
        out.push(Lir::LocalGet(eh));
        out.push(Lir::CallImport(read)); // unboxed value (i64 for get-int, else its width)
        if read == OP_GET_INT && matches!(valtype_of(elem), Some(ValType::I32)) {
            out.push(Lir::I32WrapI64); // a narrow int / char boxes as i64 → narrow to its i32 slot
        }
        out.push(
            scalar_store
                .clone()
                .expect("a scalar element has a width store"),
        );
    }
    // i++.
    out.push(Lir::LocalGet(i));
    out.push(Lir::ConstI32(1));
    out.push(Lir::I32Add);
    out.push(Lir::LocalSet(i));
    out.push(Lir::Br(0)); // → $elems
    out.push(Lir::End); // $elems
    out.push(Lir::End); // $elems-done
    // Push (outer-ptr, count) — the core `(list <T>)` param.
    out.push(Lir::LocalGet(outer));
    out.push(Lir::LocalGet(count));
    Ok(())
}

/// The shared write-loop for a PRODUCT (record or tuple) host `list<…>` ELEMENT — writes each field IN PLACE
/// into linear memory at `dest_addr` per the canonical layout: a SCALAR inline at its offset, a `Bytes` field's
/// rope copied after the outer array at the running `cursor` with its `(ptr,len)` written inline. `layout` is
/// the fields in CANONICAL (memory) order as `(value-heap cell index, field type)` — a RECORD resolves it from
/// the WIT DECLARATION order (each field's name-lex cell), a TUPLE positionally (cell i = element i). Reads
/// BORROW `agg_slot` (`arr-get`; the element handle is not consumed). Scratch (rope/len/pos) at
/// `work_base..work_base+3`. A field that is not a scalar or `Bytes` (a nested record/list/tuple) declines —
/// a later increment (each nesting level takes a disjoint scratch region above its parent's).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_product_to_mem(
    db: &mut Db,
    agg_slot: u32,
    dest_addr: u32,
    layout: &[(usize, Ty)],
    cursor: u32,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let rope = work_base;
    let len = work_base + 1;
    let pos = work_base + 2;
    for s in [rope, len, pos] {
        scratch_ty.insert(s, ValType::I32);
    }
    *high = (*high).max(work_base + 3);
    // Walk the fields in CANONICAL layout order, accumulating each field's offset (aligned to its own
    // alignment) — the exact offset walk `emit_result_lift`'s product arm reads back.
    let mut foff: u32 = 0;
    for (cell, fty) in layout {
        let cell = *cell;
        let (fs, fa) = canonical_layout(db, fty);
        foff = align_up_u32(foff, fa);
        match get_op_ty(db, fty)? {
            // A SCALAR field: `mem[dest_addr + foff] = narrow(unbox(arr-get(agg, cell)))` at its canonical width.
            Some(read) => {
                let store = match (valtype_of(fty), fs) {
                    (Some(ValType::I64), _) => Lir::I64Store { offset: foff },
                    (Some(ValType::F64), _) => Lir::F64Store { offset: foff },
                    (Some(ValType::F32), _) => Lir::F32Store { offset: foff },
                    (Some(ValType::I32), 4) => Lir::I32Store { offset: foff },
                    (Some(ValType::I32), 2) => Lir::I32Store16 { offset: foff },
                    (Some(ValType::I32), 1) => Lir::I32Store8 { offset: foff },
                    _ => {
                        return Err(Reject::decline(
                            "a product host-arg element's scalar field has no width store",
                        ));
                    }
                };
                out.push(Lir::LocalGet(dest_addr)); // store addr base (offset immediate = foff)
                out.push(Lir::LocalGet(agg_slot));
                out.push(Lir::ConstI32(cell as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [addr, field] (borrows agg)
                out.push(Lir::CallImport(read)); // [addr, scalar]
                if read == OP_GET_INT && matches!(valtype_of(fty), Some(ValType::I32)) {
                    out.push(Lir::I32WrapI64); // a narrow int / char boxes as i64 → narrow to its i32 slot
                }
                out.push(store);
            }
            // A `Bytes` field: copy its rope into `mem` at `cursor`, write `(ptr@foff, len@foff+4)`, advance cursor.
            None if matches!(fty.strip_nominal(), Ty::Bytes | Ty::String) => {
                out.push(Lir::LocalGet(agg_slot));
                out.push(Lir::ConstI32(cell as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [rope handle] (borrows agg)
                out.push(Lir::LocalSet(rope));
                out.push(Lir::LocalGet(rope));
                out.push(Lir::CallImport(OP_BYTES_LEN));
                out.push(Lir::LocalSet(len));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(pos));
                out.push(Lir::Block(BlockType::Empty));
                out.push(Lir::Loop(BlockType::Empty));
                out.push(Lir::LocalGet(pos));
                out.push(Lir::LocalGet(len));
                out.push(Lir::I32GeS);
                out.push(Lir::BrIf(1));
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(pos));
                out.push(Lir::I32Add); // addr = cursor + pos
                out.push(Lir::LocalGet(rope));
                out.push(Lir::LocalGet(pos));
                out.push(Lir::CallImport(OP_BYTES_GET));
                out.push(Lir::I32Store8 { offset: 0 });
                out.push(Lir::LocalGet(pos));
                out.push(Lir::ConstI32(1));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(pos));
                out.push(Lir::Br(0));
                out.push(Lir::End);
                out.push(Lir::End);
                out.push(Lir::LocalGet(dest_addr));
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::I32Store { offset: foff }); // ptr
                out.push(Lir::LocalGet(dest_addr));
                out.push(Lir::LocalGet(len));
                out.push(Lir::I32Store { offset: foff + 4 }); // len
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(len));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(cursor)); // cursor += len
            }
            // An `option<scalar>` field: write it at `dest_addr + foff` per the canonical option layout (disc
            // byte + payload) via `emit_option_to_mem`. Its base address is computed into a temp (the writer's
            // store offsets are relative to that base). Reuses the option memory-writer wholesale.
            None if crate::backend::wasm::host::option_payload_ty(db, fty)
                .is_some_and(|p| valtype_of(&p).is_some()) =>
            {
                let payload = crate::backend::wasm::host::option_payload_ty(db, fty)
                    .expect("option-shaped by the guard");
                let crate::ty::Ty::Sum { decl, .. } = fty.strip_nominal() else {
                    unreachable!("option is a Sum")
                };
                let some_disc = db
                    .type_decl_by_occ(*decl)
                    .and_then(|d| d.variants.iter().position(|v| v.payloads.len() == 1))
                    .ok_or_else(|| Reject::decline("the option field has no payload variant"))?
                    as i32;
                let opt_slot = work_base + 3;
                let field_addr = work_base + 4;
                scratch_ty.insert(opt_slot, ValType::I32);
                scratch_ty.insert(field_addr, ValType::I32);
                *high = (*high).max(work_base + 5);
                out.push(Lir::LocalGet(agg_slot));
                out.push(Lir::ConstI32(cell as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [option handle] (borrows agg)
                out.push(Lir::LocalSet(opt_slot));
                out.push(Lir::LocalGet(dest_addr));
                out.push(Lir::ConstI32(foff as i32));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(field_addr)); // field_addr = dest_addr + foff
                emit_option_to_mem(
                    db,
                    opt_slot,
                    field_addr,
                    &payload,
                    some_disc,
                    work_base + 5,
                    high,
                    scratch_ty,
                    out,
                )?;
            }
            // A general `variant<scalar>` field: write it at `dest_addr + foff` per the canonical variant
            // layout (disc + uniform scalar payload) via `emit_variant_to_mem` — the N-case generalization of
            // the option field arm. Its base address is computed into a temp (the writer's store offsets are
            // relative to that base). Reuses the proven variant memory-writer wholesale.
            None if crate::backend::wasm::host::variant_scalar_payload_cases(db, fty).is_some() => {
                let var_slot = work_base + 3;
                let field_addr = work_base + 4;
                scratch_ty.insert(var_slot, ValType::I32);
                scratch_ty.insert(field_addr, ValType::I32);
                *high = (*high).max(work_base + 5);
                out.push(Lir::LocalGet(agg_slot));
                out.push(Lir::ConstI32(cell as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [variant handle] (borrows agg)
                out.push(Lir::LocalSet(var_slot));
                out.push(Lir::LocalGet(dest_addr));
                out.push(Lir::ConstI32(foff as i32));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(field_addr)); // field_addr = dest_addr + foff
                emit_variant_to_mem(
                    db,
                    var_slot,
                    field_addr,
                    fty,
                    work_base + 5,
                    high,
                    scratch_ty,
                    out,
                )?;
            }
            _ => {
                return Err(Reject::decline(
                    "a product host-arg element field that is not a scalar, `Bytes`, option<scalar>, or \
                     variant<scalar> is a later increment",
                ));
            }
        }
        foff += fs;
    }
    Ok(())
}

/// Marshal a value-heap RECORD as a host `list<record>` ELEMENT — writing it IN PLACE at `dest_addr`. Resolves
/// the canonical field order from the host WIT record's DECLARATION order (`wit`) — each field's NAME-LEX cell
/// index — so the emitted bytes match the host's `record` layout, then delegates to [`emit_product_to_mem`].
/// The memory-writing analogue of [`emit_record_arg_marshal`] (that FLATTENS a top-level record arg; this
/// writes a by-reference record element). Reads BORROW `rec_slot`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_record_to_mem(
    db: &mut Db,
    rec_slot: u32,
    dest_addr: u32,
    fields: &std::collections::BTreeMap<crate::resolved::Symbol, Ty>,
    wit: &crate::wit_world::WitType,
    cursor: u32,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let crate::wit_world::WitType::Record(wit_fields) = wit else {
        return Err(Reject::decline(
            "a `list<record>` element's declared WIT type is not a record",
        ));
    };
    let names: Vec<String> = fields.keys().map(|s| s.name.to_string()).collect();
    let mut layout: Vec<(usize, Ty)> = Vec::with_capacity(wit_fields.len());
    for (fname, _fwit) in wit_fields {
        let i = names.iter().position(|n| n == fname).ok_or_else(|| {
            Reject::decline(
                "a host WIT record field is absent from the guest `list<record>` element type",
            )
        })?;
        let fty = fields
            .values()
            .nth(i)
            .expect("name-lex index in range")
            .clone();
        layout.push((i, fty));
    }
    emit_product_to_mem(
        db, rec_slot, dest_addr, &layout, cursor, work_base, high, scratch_ty, out,
    )
}

/// Marshal a value-heap TUPLE as a host `list<tuple>` ELEMENT — the POSITIONAL product: cell `i` is element
/// `i` (a tuple's WIT order IS its element order, no name reorder), then delegates to [`emit_product_to_mem`].
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_tuple_to_mem(
    db: &mut Db,
    tup_slot: u32,
    dest_addr: u32,
    elems: &[Ty],
    cursor: u32,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let layout: Vec<(usize, Ty)> = elems
        .iter()
        .enumerate()
        .map(|(i, t)| (i, t.clone()))
        .collect();
    emit_product_to_mem(
        db, tup_slot, dest_addr, &layout, cursor, work_base, high, scratch_ty, out,
    )
}

/// Marshal a value-heap `option<scalar>` (handle in `opt_slot`) as a host `list<option>` ELEMENT — writing it
/// IN PLACE into linear memory at `dest_addr` per the canonical option layout: a 1-byte discriminant at
/// offset 0 (WIT `option` some=1 / none=0) then the payload scalar at `align_up(1, align(payload))` — the
/// exact layout `emit_option_sum_lift` (the result side) reads back. `some_disc` is the guest decl's
/// single-payload variant index. Scratch (`is_some`) at `work_base`. A SCALAR payload only (no `cursor`; an
/// option<bytes>/compound element is a later slice).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_option_to_mem(
    db: &mut Db,
    opt_slot: u32,
    dest_addr: u32,
    payload_ty: &Ty,
    some_disc: i32,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let (psize, palign) = canonical_layout(db, payload_ty);
    let payload_off = align_up_u32(disc_size_for(2), palign); // disc is 1 byte for a 2-variant option
    let pv = valtype_of(payload_ty)
        .ok_or_else(|| Reject::decline("an option<scalar> element payload has no valtype"))?;
    let read = get_op_ty(db, payload_ty)?
        .ok_or_else(|| Reject::decline("an option<scalar> element payload has no unbox op"))?;
    let width_store = |offset: u32| -> Result<Lir, Reject> {
        Ok(match (pv, psize) {
            (ValType::I64, _) => Lir::I64Store { offset },
            (ValType::F64, _) => Lir::F64Store { offset },
            (ValType::F32, _) => Lir::F32Store { offset },
            (ValType::I32, 4) => Lir::I32Store { offset },
            (ValType::I32, 2) => Lir::I32Store16 { offset },
            (ValType::I32, 1) => Lir::I32Store8 { offset },
            _ => {
                return Err(Reject::decline(
                    "an option<scalar> element payload has no width store",
                ));
            }
        })
    };
    let zero = match pv {
        ValType::I64 => Lir::ConstI64(0),
        ValType::F64 => Lir::F64ConstBits(0),
        ValType::F32 => Lir::F32ConstBits(0),
        _ => Lir::ConstI32(0),
    };
    let is_some = work_base;
    scratch_ty.insert(is_some, ValType::I32);
    *high = (*high).max(work_base + 1);
    // is_some = (guest sum-disc == some_disc); this IS the WIT option disc (some=1 / none=0).
    out.push(Lir::LocalGet(opt_slot));
    out.push(Lir::CallImport(OP_SUM_DISC));
    out.push(Lir::ConstI32(some_disc));
    out.push(Lir::I32Eq);
    out.push(Lir::LocalSet(is_some));
    out.push(Lir::LocalGet(dest_addr));
    out.push(Lir::LocalGet(is_some));
    out.push(Lir::I32Store8 { offset: 0 }); // 1-byte disc
    out.push(Lir::LocalGet(is_some));
    out.push(Lir::If(BlockType::Empty)); // Some: dest[payload_off] = unbox(sum-payload)
    out.push(Lir::LocalGet(dest_addr));
    out.push(Lir::LocalGet(opt_slot));
    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
    out.push(Lir::CallImport(read));
    if read == OP_GET_INT && matches!(pv, ValType::I32) {
        out.push(Lir::I32WrapI64);
    }
    out.push(width_store(payload_off)?);
    out.push(Lir::Else); // None: dest[payload_off] = 0
    out.push(Lir::LocalGet(dest_addr));
    out.push(zero);
    out.push(width_store(payload_off)?);
    out.push(Lir::End);
    Ok(())
}

/// Write a value-heap general `variant { c0, c1(scalar), … }` ELEMENT (in `var_slot`) IN PLACE into linear
/// memory at `dest_addr` per its canonical variant layout: the discriminant at offset 0 (its width by case
/// count) followed by the uniform scalar payload at `align_up(disc_size, payload_align)`. The guest's
/// `sum-disc` IS the component discriminant (cases in declaration order, like an enum), so the RAW disc is
/// stored (not remapped as the 2-case option does). A payload case (`disc` in the payload-case set) stores
/// `unbox(sum-payload)`; a nullary case the payload-width zero. The N-case generalization of
/// [`emit_option_to_mem`], scoped (like the record-field variant arm, #3368) to a uniform single scalar
/// payload — a mixed-width / `Bytes` / compound payload join is a later increment.
#[allow(clippy::too_many_arguments)]
/// The core JOIN valtype of a scalar-payload variant's flattened payload slot (canonical ABI `flatten_variant`
/// join, the REGISTER path): `I64` if any payload case is a 64-bit int, else `I32` for all int/bool/char
/// payloads; the single float valtype for a uniform-float variant. `None` if the payload set mixes int and
/// float (the reinterpret join lattice — a later increment; the detector already excludes it). Mirrors
/// `wit_ctype::flatten_variant`'s join so the guest push matches the import's core param slot. DIVERGES from
/// the MEMORY max-natural width (`emit_variant_to_mem`): e.g. `variant{u8, u16}` joins to `I32` (register)
/// but its memory payload area is 2 bytes.
pub(super) fn variant_register_join_vt(
    db: &mut Db,
    variant_ty: &Ty,
    payload_discs: &[i32],
) -> Result<ValType, Reject> {
    let mut join: Option<ValType> = None;
    for &pd in payload_discs {
        let pty = variant_payload_ty_at(db, variant_ty, pd as u32)
            .ok_or_else(|| Reject::decline("a variant payload type could not be resolved"))?;
        let vt =
            valtype_of(&pty).ok_or_else(|| Reject::decline("a variant payload has no valtype"))?;
        join = Some(match join {
            None => vt,
            Some(prev) if prev == vt => vt,
            Some(ValType::I32) if vt == ValType::I64 => ValType::I64,
            Some(ValType::I64) if vt == ValType::I32 => ValType::I64,
            _ => {
                return Err(Reject::decline(
                    "a variant mixes int and float payloads (the reinterpret join is a later increment)",
                ));
            }
        });
    }
    join.ok_or_else(|| Reject::decline("a variant has no payload case to join"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_variant_to_mem(
    db: &mut Db,
    var_slot: u32,
    dest_addr: u32,
    variant_ty: &Ty,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let cases = crate::backend::wasm::host::variant_scalar_payload_cases(db, variant_ty)
        .ok_or_else(|| Reject::decline("a variant element is not a scalar-payload variant"))?;
    let ncases = cases.len();
    let payload_discs: Vec<i32> = cases
        .iter()
        .enumerate()
        .filter_map(|(d, (_, p))| p.map(|_| d as i32))
        .collect();
    let first_pd = *payload_discs
        .first()
        .ok_or_else(|| Reject::decline("a variant element has no payload case"))?;
    // The MEMORY payload area = the MAX natural (size, align) over the payload cases (canonical variant
    // layout): each case stores at its natural width into this max-sized slot, and little-endian + the host
    // reading the SELECTED case's width from the low bytes makes ONE max-width store correct. This DIVERGES
    // from the register-flatten join valtype for a mixed-width variant (e.g. variant{u8, u16}: the register
    // join is i32 = 4 bytes, but the memory max-natural is 2 bytes). Integer cases all read via `get-int`
    // (i64); a uniform-float variant reads `get-float` (a mixed int/float payload is declined by the detector).
    let mut psize = 0u32;
    let mut palign = 1u32;
    for &pd in &payload_discs {
        let pty = variant_payload_ty_at(db, variant_ty, pd as u32).ok_or_else(|| {
            Reject::decline("a variant element payload type could not be resolved")
        })?;
        let (s, a) = canonical_layout(db, &pty);
        psize = psize.max(s);
        palign = palign.max(a);
    }
    let payload_ty = variant_payload_ty_at(db, variant_ty, first_pd as u32)
        .ok_or_else(|| Reject::decline("a variant element payload type could not be resolved"))?;
    let disc_size = disc_size_for(ncases);
    let payload_off = align_up_u32(disc_size, palign);
    let pv = valtype_of(&payload_ty)
        .ok_or_else(|| Reject::decline("a variant element scalar payload has no valtype"))?;
    let is_float = matches!(pv, ValType::F32 | ValType::F64);
    let read = get_op_ty(db, &payload_ty)?
        .ok_or_else(|| Reject::decline("a variant element payload has no unbox op"))?;
    let width_store = |offset: u32| -> Result<Lir, Reject> {
        Ok(match (is_float, psize) {
            (true, 8) => Lir::F64Store { offset },
            (true, 4) => Lir::F32Store { offset },
            (false, 8) => Lir::I64Store { offset },
            (false, 4) => Lir::I32Store { offset },
            (false, 2) => Lir::I32Store16 { offset },
            (false, 1) => Lir::I32Store8 { offset },
            _ => {
                return Err(Reject::decline(
                    "a variant element payload has no width store",
                ));
            }
        })
    };
    let disc_store = |offset: u32| -> Lir {
        match disc_size {
            1 => Lir::I32Store8 { offset },
            2 => Lir::I32Store16 { offset },
            _ => Lir::I32Store { offset },
        }
    };
    let zero = match (is_float, psize) {
        (true, 8) => Lir::F64ConstBits(0),
        (true, 4) => Lir::F32ConstBits(0),
        (false, 8) => Lir::ConstI64(0),
        _ => Lir::ConstI32(0),
    };
    let disc = work_base;
    let is_payload = work_base + 1;
    scratch_ty.insert(disc, ValType::I32);
    scratch_ty.insert(is_payload, ValType::I32);
    *high = (*high).max(work_base + 2);
    // disc = guest sum-disc (= the component discriminant, decl order); store it at offset 0.
    out.push(Lir::LocalGet(var_slot));
    out.push(Lir::CallImport(OP_SUM_DISC));
    out.push(Lir::LocalSet(disc));
    out.push(Lir::LocalGet(dest_addr));
    out.push(Lir::LocalGet(disc));
    out.push(disc_store(0));
    // is_payload = OR over the payload-case discs of (disc == pd).
    for (k, pd) in payload_discs.iter().enumerate() {
        out.push(Lir::LocalGet(disc));
        out.push(Lir::ConstI32(*pd));
        out.push(Lir::I32Eq);
        if k > 0 {
            out.push(Lir::I32Or);
        }
    }
    out.push(Lir::LocalSet(is_payload));
    out.push(Lir::LocalGet(is_payload));
    out.push(Lir::If(BlockType::Empty)); // a payload case → dest[payload_off] = unbox(sum-payload)
    out.push(Lir::LocalGet(dest_addr));
    out.push(Lir::LocalGet(var_slot));
    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
    out.push(Lir::CallImport(read));
    if read == OP_GET_INT && psize <= 4 {
        out.push(Lir::I32WrapI64); // narrow the i64 heap cell to the i32 store slot (<=4-byte payload area)
    }
    out.push(width_store(payload_off)?);
    out.push(Lir::Else); // a nullary case → dest[payload_off] = 0
    out.push(Lir::LocalGet(dest_addr));
    out.push(zero);
    out.push(width_store(payload_off)?);
    out.push(Lir::End);
    Ok(())
}

/// Push the canonical register-flatten `(disc: i32, payload: join)` of a scalar-payload VARIANT whose
/// value-heap handle is in `var_slot`, onto the operand stack — the guest side of a component `variant`
/// param/field. The guest `sum-disc` IS the component discriminant (declaration order, like an enum); the
/// payload slot is the canonical JOIN valtype (`variant_register_join_vt`) so a MIXED-width variant reads back
/// correctly — a payload case unboxes its value (narrowing to the join slot's low bits when the join is i32), a
/// nullary case pushes the payload-width zero. Uses `work_base..work_base+3` as scratch. Shared by the
/// record-FIELD variant marshal and the bare-VARIANT-param marshal (the top-level param position).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_variant_reg_flatten(
    db: &mut Db,
    var_slot: u32,
    fty: &Ty,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let cases = crate::backend::wasm::host::variant_scalar_payload_cases(db, fty)
        .ok_or_else(|| Reject::decline("a variant element is not a scalar-payload variant"))?;
    let payload_discs: Vec<i32> = cases
        .iter()
        .enumerate()
        .filter_map(|(d, (_, p))| p.map(|_| d as i32))
        .collect();
    let first_pd = *payload_discs
        .first()
        .expect("the detector guarantees ≥1 payload case");
    let payload_ty = variant_payload_ty_at(db, fty, first_pd as u32)
        .ok_or_else(|| Reject::decline("a variant payload type could not be resolved"))?;
    let pv = variant_register_join_vt(db, fty, &payload_discs)?;
    let read = get_op_ty(db, &payload_ty)?
        .ok_or_else(|| Reject::decline("a variant payload has no unbox op"))?;
    let disc_out = work_base;
    let is_payload = work_base + 1;
    let pval = work_base + 2;
    scratch_ty.insert(disc_out, ValType::I32);
    scratch_ty.insert(is_payload, ValType::I32);
    scratch_ty.insert(pval, pv);
    *high = (*high).max(work_base + 3);
    let zero = match pv {
        ValType::I64 => Lir::ConstI64(0),
        ValType::F64 => Lir::F64ConstBits(0),
        ValType::F32 => Lir::F32ConstBits(0),
        _ => Lir::ConstI32(0),
    };
    out.push(Lir::LocalGet(var_slot));
    out.push(Lir::CallImport(OP_SUM_DISC)); // [disc] (= component disc, decl order)
    out.push(Lir::LocalSet(disc_out));
    // is_payload = OR over the payload-case discs of (disc == pd).
    for (k, pd) in payload_discs.iter().enumerate() {
        out.push(Lir::LocalGet(disc_out));
        out.push(Lir::ConstI32(*pd));
        out.push(Lir::I32Eq);
        if k > 0 {
            out.push(Lir::I32Or);
        }
    }
    out.push(Lir::LocalSet(is_payload));
    out.push(Lir::LocalGet(is_payload));
    out.push(Lir::If(BlockType::Empty)); // a payload case → unbox the payload
    out.push(Lir::LocalGet(var_slot));
    out.push(Lir::CallImport(OP_SUM_PAYLOAD));
    out.push(Lir::CallImport(read));
    if read == OP_GET_INT && matches!(pv, ValType::I32) {
        out.push(Lir::I32WrapI64); // a narrow int / char payload narrows to its i32 slot
    }
    out.push(Lir::LocalSet(pval));
    out.push(Lir::Else); // a nullary case → the payload-width zero
    out.push(zero);
    out.push(Lir::LocalSet(pval));
    out.push(Lir::End);
    out.push(Lir::LocalGet(disc_out)); // push (disc, payload)
    out.push(Lir::LocalGet(pval));
    Ok(())
}

/// Marshal a value-heap RECORD host argument whose handle is in `rec_slot` into the FLATTENED core slots the
/// component `record` param lowers to, pushing them onto the operand stack in NAME-LEX field order (= the
/// component record's field declaration order = the core flatten order). Per field: a SCALAR reads back
/// `arr-get` (borrows the aggregate) + its wrap-free get-op (1 slot); a `Bytes` field's rope is copied into
/// `mem` at the running `cursor` and pushed as `(ptr,len)` (2 slots, the same copy a Bytes ARG does); a
/// NESTED record (d3, the message envelope's `sender: origin`) is projected (`arr-get` its sub-handle) and
/// marshalled RECURSIVELY — its fields flatten inline. All reads BORROW `rec_slot` (no consume/drop; the
/// owner reclaims it). `work_base` is the first free scratch slot for THIS level (rope/len/pos + a sub-record
/// handle at `work_base..work_base+4`); each nesting level takes a disjoint region above its parent's.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_record_arg_marshal(
    db: &mut Db,
    rec_slot: u32,
    fields: &std::collections::BTreeMap<crate::resolved::Symbol, Ty>,
    wit: &crate::wit_world::WitType,
    cursor: Option<u32>,
    work_base: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let rope_slot = work_base;
    let len_slot = work_base + 1;
    let pos_slot = work_base + 2;
    let sub_rec_slot = work_base + 3;
    for s in [rope_slot, len_slot, pos_slot, sub_rec_slot] {
        scratch_ty.insert(s, ValType::I32);
    }
    *high = (*high).max(work_base + 4);
    // Marshal the fields in the host WIT record's DECLARATION order (not the guest's name-lex `Ty::Record`
    // order) — the component-linker requires the flattened core args (and the import record type) to match the
    // host's field order, and the two orders differ (e.g. `message{contract, sender, payload, token}` vs
    // name-lex `contract, payload, sender, token`). For each WIT field we `arr-get` its NAME-LEX cell index
    // (the value-heap record cell is name-lex), so the read stays correct while the PUSH order is WIT's.
    let crate::wit_world::WitType::Record(wit_fields) = wit else {
        return Err(Reject::decline(
            "a record host-arg's declared WIT type is not a record",
        ));
    };
    let names: Vec<String> = fields.keys().map(|s| s.name.to_string()).collect();
    for (fname, fwit) in wit_fields {
        let Some(i) = names.iter().position(|n| n == fname) else {
            return Err(Reject::decline(
                "a host WIT record field is absent from the guest record type",
            ));
        };
        let fty = fields
            .values()
            .nth(i)
            .expect("name-lex index in range")
            .clone();
        let fty = &fty;
        match get_op_ty(db, fty)? {
            // A SCALAR field: arr-get + unbox → one core slot.
            Some(read) => {
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [field] (borrows rec)
                out.push(Lir::CallImport(read)); // [scalar]
                // A NARROW int / char / enum-disc field boxes into the i64 int cell, so `get-int` returns an
                // i64 — but its core slot is i32 (its aliased width), so narrow it. A 64-bit int (`get-int`
                // i64 → i64 slot), a bool (`get-bool` i32), or a float (`get-float`) needs no narrow.
                if read == OP_GET_INT && matches!(valtype_of(fty), Some(ValType::I32)) {
                    out.push(Lir::I32WrapI64);
                }
            }
            // A `Bytes` field: arr-get its list<u8> handle → copy rope→mem at the cursor → push (ptr,len).
            None if matches!(fty, Ty::Bytes) => {
                let cursor =
                    cursor.expect("a bytes-field record reserves the scratch cursor (pre-scan)");
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [field list<u8> handle]
                out.push(Lir::LocalSet(rope_slot));
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::CallImport(OP_BYTES_LEN)); // [len]
                out.push(Lir::LocalSet(len_slot));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty));
                out.push(Lir::Loop(BlockType::Empty));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32GeS);
                out.push(Lir::BrIf(1));
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::CallImport(OP_BYTES_GET));
                out.push(Lir::I32Store8 { offset: 0 });
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI32(1));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0));
                out.push(Lir::End);
                out.push(Lir::End);
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(len_slot)); // push (ptr, len)
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(cursor)); // cursor += len
            }
            // A NESTED record field (d3): arr-get its sub-record handle → recurse (fields flatten inline). The
            // nested record marshals in ITS OWN WIT declaration order (`fwit`, the nested WIT record type).
            None if matches!(fty, Ty::Record(_)) => {
                let Ty::Record(sub) = fty else { unreachable!() };
                let sub = sub.clone();
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [sub-record handle] (borrows rec)
                out.push(Lir::LocalSet(sub_rec_slot));
                emit_record_arg_marshal(
                    db,
                    sub_rec_slot,
                    &sub,
                    fwit,
                    cursor,
                    work_base + 4,
                    high,
                    scratch_ty,
                    out,
                )?;
            }
            // A `result<list<u8>, enum>` field flattens to `(disc, i32, i32)`. Branch on the value-heap
            // Result sum's disc (Ok=0 / Err=1, decl order = the component result disc): Ok → the Bytes
            // payload copied rope→mem at the cursor gives `(ptr,len)`; Err → the enum payload's disc + a
            // 0-pad. BlockType is SINGLE-value (no multi-value block), so the `if` arms SIDE-EFFECT into
            // scratch slots and we push the 3 flattened values (disc, p0, p1) AFTER the `if`.
            None if crate::backend::wasm::host::result_bytes_enum(db, fty).is_some() => {
                let cursor =
                    cursor.expect("a result-field record reserves the scratch cursor (pre-scan)");
                let ans = work_base + 4;
                let disc = work_base + 5;
                let p0 = work_base + 6;
                let p1 = work_base + 7;
                for s in [ans, disc, p0, p1] {
                    scratch_ty.insert(s, ValType::I32);
                }
                *high = (*high).max(work_base + 8);
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [result handle] (borrows rec)
                out.push(Lir::LocalSet(ans));
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_DISC)); // [disc]
                out.push(Lir::LocalSet(disc));
                out.push(Lir::LocalGet(disc));
                out.push(Lir::If(BlockType::Empty)); // disc != 0 → Err
                // Err arm: p0 = the err enum's disc, p1 = 0.
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [err enum handle]
                out.push(Lir::CallImport(OP_SUM_DISC)); // [enum disc]
                out.push(Lir::LocalSet(p0));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(p1));
                out.push(Lir::Else); // disc == 0 → Ok
                // Ok arm: the Bytes payload copied rope→mem at the cursor → p0=ptr, p1=len.
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [Bytes handle]
                out.push(Lir::LocalSet(rope_slot));
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::CallImport(OP_BYTES_LEN));
                out.push(Lir::LocalSet(len_slot));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty));
                out.push(Lir::Loop(BlockType::Empty));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32GeS);
                out.push(Lir::BrIf(1));
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::CallImport(OP_BYTES_GET));
                out.push(Lir::I32Store8 { offset: 0 });
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI32(1));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0));
                out.push(Lir::End); // loop
                out.push(Lir::End); // block
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalSet(p0)); // ptr = cursor (before advance)
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::LocalSet(p1)); // len
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(cursor)); // cursor += len
                out.push(Lir::End); // if
                out.push(Lir::LocalGet(disc)); // push the 3 flattened core values
                out.push(Lir::LocalGet(p0));
                out.push(Lir::LocalGet(p1));
            }
            // A `list<T>` field (NON-`Bytes`; a `list<u8>` field is `Ty::Bytes`, handled above): `arr-get` its
            // `List` handle → marshal the list into `mem` (its backing array + elements at the running cursor)
            // and push `(ptr, count)` — the same 2 flattened core slots a `list<T>` ARG lowers to. The whole
            // list marshal (`emit_list_arg_marshal`, incl. its own Block/Loop) rides here mid-flatten exactly as
            // the `Bytes`/`result` arms do; `sub_rec_slot` briefly holds the field's list handle, and the list
            // marshal takes a work_base ABOVE this level's scratch (`*high`). The element WIT comes from `fwit`
            // (`WitType::List(elem)`), so a record/nested element inside the field still orders correctly.
            None if matches!(fty, Ty::List(_)) => {
                let cursor =
                    cursor.expect("a list-field record reserves the scratch cursor (pre-scan)");
                let Ty::List(elem) = fty else { unreachable!() };
                let elem = (**elem).clone();
                let elem_wit = match fwit {
                    crate::wit_world::WitType::List(ew) => Some(ew.as_ref()),
                    _ => None,
                };
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [field List handle] (borrows rec)
                out.push(Lir::LocalSet(sub_rec_slot));
                let lwb = *high;
                emit_list_arg_marshal(
                    db,
                    &elem,
                    elem_wit,
                    sub_rec_slot,
                    cursor,
                    lwb,
                    high,
                    scratch_ty,
                    out,
                )?;
                // (outer-ptr, count) left on the stack = the list field's 2 flattened core slots.
            }
            // A `tuple<…>` field flattens its elements INLINE (positional), each element's flattened slots
            // joining the parent's core run — the canonical tuple flatten. Per element (read from the tuple
            // cell by position): a SCALAR pushes one slot; a `Bytes` element copies its rope into `mem` at the
            // cursor and pushes `(ptr,len)`. `sub_rec_slot` holds the tuple handle. A nested compound element is
            // a later increment (declines). Mirrors the scalar/`Bytes` field arms, indexed by tuple position.
            None if matches!(fty, Ty::Tuple(_)) => {
                let Ty::Tuple(elems) = fty else {
                    unreachable!()
                };
                let elems: Vec<Ty> = elems.iter().cloned().collect();
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [tuple handle] (borrows rec)
                out.push(Lir::LocalSet(sub_rec_slot));
                for (j, ety) in elems.iter().enumerate() {
                    match get_op_ty(db, ety)? {
                        // A SCALAR element: arr-get + unbox → one core slot, pushed inline.
                        Some(read) => {
                            out.push(Lir::LocalGet(sub_rec_slot));
                            out.push(Lir::ConstI32(j as i32));
                            out.push(Lir::CallImport(OP_ARR_GET)); // [element] (borrows tuple)
                            out.push(Lir::CallImport(read));
                            if read == OP_GET_INT && matches!(valtype_of(ety), Some(ValType::I32)) {
                                out.push(Lir::I32WrapI64);
                            }
                        }
                        // A `Bytes` element: copy its rope → `mem` at the cursor, push `(ptr,len)`.
                        None if matches!(ety.strip_nominal(), Ty::Bytes | Ty::String) => {
                            let cursor = cursor.expect(
                                "a tuple-field record reserves the scratch cursor (pre-scan)",
                            );
                            out.push(Lir::LocalGet(sub_rec_slot));
                            out.push(Lir::ConstI32(j as i32));
                            out.push(Lir::CallImport(OP_ARR_GET)); // [element list<u8> handle]
                            out.push(Lir::LocalSet(rope_slot));
                            out.push(Lir::LocalGet(rope_slot));
                            out.push(Lir::CallImport(OP_BYTES_LEN));
                            out.push(Lir::LocalSet(len_slot));
                            out.push(Lir::ConstI32(0));
                            out.push(Lir::LocalSet(pos_slot));
                            out.push(Lir::Block(BlockType::Empty));
                            out.push(Lir::Loop(BlockType::Empty));
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::LocalGet(len_slot));
                            out.push(Lir::I32GeS);
                            out.push(Lir::BrIf(1));
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::I32Add);
                            out.push(Lir::LocalGet(rope_slot));
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::CallImport(OP_BYTES_GET));
                            out.push(Lir::I32Store8 { offset: 0 });
                            out.push(Lir::LocalGet(pos_slot));
                            out.push(Lir::ConstI32(1));
                            out.push(Lir::I32Add);
                            out.push(Lir::LocalSet(pos_slot));
                            out.push(Lir::Br(0));
                            out.push(Lir::End);
                            out.push(Lir::End);
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(len_slot)); // push (ptr, len)
                            out.push(Lir::LocalGet(cursor));
                            out.push(Lir::LocalGet(len_slot));
                            out.push(Lir::I32Add);
                            out.push(Lir::LocalSet(cursor)); // cursor += len
                        }
                        _ => {
                            return Err(Reject::decline(
                                "a record host-arg tuple field with a non-scalar/non-`Bytes` element is a \
                                 later increment",
                            ));
                        }
                    }
                }
            }
            // An `option<bytes>` field flattens to `(disc:i32, ptr:i32, len:i32)`. Some → `(1, ptr, len)` with
            // the payload rope copied into `mem` at the cursor (the same copy the `result` Ok arm / a Bytes
            // field does); None → `(0, 0, 0)`. Side-effect scratch in the `if`, push the 3 values after.
            None if crate::backend::wasm::host::option_payload_ty(db, fty)
                .is_some_and(|p| matches!(p.strip_nominal(), Ty::Bytes | Ty::String)) =>
            {
                let cursor =
                    cursor.expect("an option<bytes> field reserves the scratch cursor (pre-scan)");
                let crate::ty::Ty::Sum { decl, .. } = fty.strip_nominal() else {
                    unreachable!("option is a Sum")
                };
                let some_disc = {
                    let d = db.type_decl_by_occ(*decl).ok_or_else(|| {
                        Reject::decline("the option field's sum decl was not found")
                    })?;
                    d.variants
                        .iter()
                        .position(|v| v.payloads.len() == 1)
                        .ok_or_else(|| Reject::decline("the option field has no payload variant"))?
                        as i32
                };
                let ans = work_base + 4;
                let disc_out = work_base + 5;
                let p0 = work_base + 6;
                let p1 = work_base + 7;
                for s in [ans, disc_out, p0, p1] {
                    scratch_ty.insert(s, ValType::I32);
                }
                *high = (*high).max(work_base + 8);
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [option handle] (borrows rec)
                out.push(Lir::LocalSet(ans));
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_DISC));
                out.push(Lir::ConstI32(some_disc));
                out.push(Lir::I32Eq);
                out.push(Lir::If(BlockType::Empty)); // Some: copy the payload rope → (ptr,len)
                out.push(Lir::ConstI32(1));
                out.push(Lir::LocalSet(disc_out));
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [Bytes handle]
                out.push(Lir::LocalSet(rope_slot));
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::CallImport(OP_BYTES_LEN));
                out.push(Lir::LocalSet(len_slot));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Block(BlockType::Empty));
                out.push(Lir::Loop(BlockType::Empty));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32GeS);
                out.push(Lir::BrIf(1));
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalGet(rope_slot));
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::CallImport(OP_BYTES_GET));
                out.push(Lir::I32Store8 { offset: 0 });
                out.push(Lir::LocalGet(pos_slot));
                out.push(Lir::ConstI32(1));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(pos_slot));
                out.push(Lir::Br(0));
                out.push(Lir::End);
                out.push(Lir::End);
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalSet(p0)); // ptr = cursor (before advance)
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::LocalSet(p1)); // len
                out.push(Lir::LocalGet(cursor));
                out.push(Lir::LocalGet(len_slot));
                out.push(Lir::I32Add);
                out.push(Lir::LocalSet(cursor)); // cursor += len
                out.push(Lir::Else); // None: (0, 0, 0)
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(disc_out));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(p0));
                out.push(Lir::ConstI32(0));
                out.push(Lir::LocalSet(p1));
                out.push(Lir::End);
                out.push(Lir::LocalGet(disc_out)); // push (disc, ptr, len)
                out.push(Lir::LocalGet(p0));
                out.push(Lir::LocalGet(p1));
            }
            // An `option<scalar>` field flattens (canonical variant flatten) to `(disc:i32, payload)`. Branch on
            // the value-heap Option's discriminant: Some (the guest decl's single-payload arm) → `(1, unbox(
            // payload))`; None → `(0, 0)`. `BlockType` is single-value, so the `if` arms SIDE-EFFECT into scratch
            // slots (disc_out, pval) and we push the 2 flattened values AFTER the `if` — the same shape as the
            // `result<list<u8>, enum>` field arm. WIT `option` disc is some=1 / none=0.
            None if crate::backend::wasm::host::option_payload_ty(db, fty)
                .is_some_and(|p| valtype_of(&p).is_some()) =>
            {
                // An option<scalar> field touches no `mem` (it flattens to core slots) — no `cursor` needed.
                let payload_ty = crate::backend::wasm::host::option_payload_ty(db, fty)
                    .expect("option-shaped by the guard");
                let pv = valtype_of(&payload_ty).expect("a scalar option payload has a valtype");
                let read = get_op_ty(db, &payload_ty)?
                    .ok_or_else(|| Reject::decline("an option payload scalar has no unbox op"))?;
                // The guest decl's SOME discriminant = the single-payload variant's index.
                let crate::ty::Ty::Sum { decl, .. } = fty.strip_nominal() else {
                    unreachable!("option is a Sum")
                };
                let some_disc = {
                    let d = db.type_decl_by_occ(*decl).ok_or_else(|| {
                        Reject::decline("the option field's sum decl was not found")
                    })?;
                    d.variants
                        .iter()
                        .position(|v| v.payloads.len() == 1)
                        .ok_or_else(|| Reject::decline("the option field has no payload variant"))?
                        as i32
                };
                let ans = work_base + 4;
                let disc_out = work_base + 5;
                let pval = work_base + 6;
                scratch_ty.insert(ans, ValType::I32);
                scratch_ty.insert(disc_out, ValType::I32);
                scratch_ty.insert(pval, pv);
                *high = (*high).max(work_base + 7);
                let zero = match pv {
                    ValType::I64 => Lir::ConstI64(0),
                    ValType::F64 => Lir::F64ConstBits(0),
                    ValType::F32 => Lir::F32ConstBits(0),
                    _ => Lir::ConstI32(0),
                };
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [option handle] (borrows rec)
                out.push(Lir::LocalSet(ans));
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_DISC)); // [guest disc]
                out.push(Lir::ConstI32(some_disc));
                out.push(Lir::I32Eq);
                out.push(Lir::If(BlockType::Empty)); // guest disc == some_disc → Some
                out.push(Lir::ConstI32(1)); // WIT `option` some = 1
                out.push(Lir::LocalSet(disc_out));
                out.push(Lir::LocalGet(ans));
                out.push(Lir::CallImport(OP_SUM_PAYLOAD)); // [payload handle]
                out.push(Lir::CallImport(read)); // [payload scalar]
                if read == OP_GET_INT && matches!(pv, ValType::I32) {
                    out.push(Lir::I32WrapI64); // a narrow int / char payload narrows to its i32 slot
                }
                out.push(Lir::LocalSet(pval));
                out.push(Lir::Else); // None
                out.push(Lir::ConstI32(0)); // WIT `option` none = 0
                out.push(Lir::LocalSet(disc_out));
                out.push(zero);
                out.push(Lir::LocalSet(pval));
                out.push(Lir::End);
                out.push(Lir::LocalGet(disc_out)); // push (disc, payload)
                out.push(Lir::LocalGet(pval));
            }
            // A general `variant { c0, c1(scalar), … }` field flattens (canonical variant flatten) to
            // `(disc:i32, payload)`. The guest's `sum-disc` IS the component discriminant (cases in
            // declaration order, like an enum). The payload slot: if `disc` is a PAYLOAD case → unbox(
            // sum-payload); else the payload-width zero. Side-effect scratch in the `if`, push `(disc,
            // payload)` after — the option-field shape generalized to N cases (payload-case-set membership).
            // A general `variant { c0, c1(scalar), … }` field flattens (canonical variant flatten) to
            // `(disc:i32, payload)` via the shared `emit_variant_reg_flatten`. Read the field's variant handle
            // (arr-get, borrows the record) into a slot, then decompose it — the SAME helper the bare-variant
            // PARAM position uses, so the field and top-level variant marshals stay in lockstep.
            None if crate::backend::wasm::host::variant_scalar_payload_cases(db, fty).is_some() => {
                let ans = work_base + 4;
                scratch_ty.insert(ans, ValType::I32);
                *high = (*high).max(work_base + 5);
                out.push(Lir::LocalGet(rec_slot));
                out.push(Lir::ConstI32(i as i32));
                out.push(Lir::CallImport(OP_ARR_GET)); // [variant handle] (borrows rec)
                out.push(Lir::LocalSet(ans));
                emit_variant_reg_flatten(db, ans, fty, work_base + 5, high, scratch_ty, out)?;
            }
            None => {
                return Err(Reject::decline(
                    "a record host-arg field has no boundary read (only scalar, list<u8>, list<T>, \
                     option<scalar>, variant<scalar>, nested-record, and result<list<u8>, enum> fields cross \
                     this increment)",
                ));
            }
        }
    }
    Ok(())
}
