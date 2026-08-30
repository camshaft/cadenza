use super::*;

/// Round `x` up to the next multiple of the power-of-two alignment `a`.
pub(super) fn align_up_u32(x: u32, a: u32) -> u32 {
    debug_assert!(a.is_power_of_two());
    (x + a - 1) & !(a - 1)
}

/// The component-model discriminant SIZE (bytes) for a variant/enum of `nvar` cases: 1 for ≤256, 2 for
/// ≤65536, else 4 — the canonical ABI's `discriminant_type` widths.
pub(super) fn disc_size_for(nvar: usize) -> u32 {
    if nvar <= 256 {
        1
    } else if nvar <= 65536 {
        2
    } else {
        4
    }
}

/// The component-model canonical-ABI linear-memory `(size, align)` of a value of type `ty` as the host's
/// canon `Lower` STORES it into a caller-provided return area for a SPILLED result — a compound whose
/// flattened core form is more than one value, so the canonical ABI returns it by pointer. This is the
/// WIT-type-driven layout [`emit_result_lift`] reads back and the size the guest reallocs the return area
/// to, so the two MUST agree. Recurses over the type exactly as the canonical ABI lays it out: a
/// `Bytes`/`String`/`List` crosses as an 8-byte `(ptr, len/count)` header (align 4); a scalar as its own
/// component width; a `Record`/`Tuple` as its fields laid out with C-style alignment padding; a `Sum` as a
/// discriminant (1/2/4 bytes by variant count) followed by the payload area (max over the arms), the whole
/// rounded up to its alignment. Mirrors the canonical ABI's `alignment`/`elem_size`/`store`.
pub(super) fn canonical_layout(db: &mut Db, ty: &Ty) -> (u32, u32) {
    match ty.strip_nominal().clone() {
        Ty::Bool => (1, 1),
        // A `char` is a Unicode scalar — 4 bytes.
        Ty::Char => (4, 4),
        Ty::Int(it) => match it.ground_width() {
            8 => (1, 1),
            16 => (2, 2),
            32 => (4, 4),
            _ => (8, 8), // 64 (and any wider aliased width the boundary admits)
        },
        Ty::Float(ft) => match ft.ground_width() {
            32 => (4, 4),
            _ => (8, 8),
        },
        // `list<u8>` (`Bytes`), `string`, and any `list<T>` all cross as an 8-byte `(ptr, len/count)` pair.
        Ty::Bytes | Ty::String | Ty::List(_) => (8, 4),
        Ty::Tuple(elems) => product_layout(db, elems.iter().cloned()),
        Ty::Record(fields) => product_layout(db, fields.values().cloned()),
        Ty::Sum { decl, .. } => {
            let nvar = db
                .type_decl_by_occ(decl)
                .map(|d| d.variants.len())
                .unwrap_or(1);
            let ds = disc_size_for(nvar);
            let mut case_size = 0u32;
            let mut case_align = 1u32;
            for disc in 0..nvar {
                if let Some(pty) = variant_payload_ty_at(db, ty, disc as u32) {
                    let (s, a) = canonical_layout(db, &pty);
                    case_size = case_size.max(s);
                    case_align = case_align.max(a);
                }
            }
            let align = ds.max(case_align);
            let payload_off = align_up_u32(ds, case_align);
            (align_up_u32(payload_off + case_size, align), align)
        }
        // Any type with no spilled boundary form: a defensive 8-byte slot. Unreached for an admitted result
        // shape (the HostCall lift gate restricts entry to a spilled compound).
        _ => (8, 4),
    }
}

/// The `(size, align)` of a PRODUCT (tuple / record) whose fields (in layout order) are `tys` — each field
/// aligned up to its own alignment, the total rounded to the max field alignment.
pub(super) fn product_layout(db: &mut Db, tys: impl Iterator<Item = Ty>) -> (u32, u32) {
    let mut size = 0u32;
    let mut align = 1u32;
    for t in tys {
        let (s, a) = canonical_layout(db, &t);
        size = align_up_u32(size, a) + s;
        align = align.max(a);
    }
    (align_up_u32(size, align), align)
}

/// The GENERAL, WIT-type-driven result LIFT. Given a host op's SPILLED compound result of type `ty` that the
/// host wrote (canonical-ABI `store`) into linear memory at `mem[ptr_slot] + offset`, emit the value-heap
/// construction that lifts it into a Cadenza value, leaving the resulting handle on the operand stack. This
/// ONE recursion REPLACES the former per-shape lift blocks (`option<list<u8>>`, bare `list<u8>`,
/// `list<tuple<list<u8>,list<u8>>>`) — the general shape mechanism, not a per-shape shortcut. It recurses
/// over the type exactly as [`canonical_layout`] sizes it:
///  • `Bytes`/`String` → `bytes-alloc` + copy the `(ptr,len)` bytes out of the host's memory,
///  • `List<T>` → `arr-alloc(count)` + per element at `list-ptr + i*stride(T)` recurse, then `vec-of-arr`,
///  • `Tuple`/`Record` → `arr-alloc(n)` + lift each field at its canonical offset (a Tuple/Record is a raw arr),
///  • an option-shaped `Sum` → read the discriminant and lift the present arm's payload / build the nullary arm.
/// A leaf this increment does not yet wire (a bare scalar, a wider variant/enum/`result`, a multi-payload arm)
/// DECLINES honestly rather than miscompiling — the same incremental-leaf discipline the ARG-side marshal holds.
///
/// `ptr_slot` is an i32 local holding the base address; `offset` the static byte offset of this value within
/// it. Scratch slots are allocated ABOVE `*high` (which is bumped past them), so a nested recursion never
/// reuses a live slot.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_result_lift(
    db: &mut Db,
    ty: &Ty,
    // The value's declared WIT type (the host's canonical layout), when known — drives a `record`'s field
    // ORDER (the host writes fields in WIT DECLARATION order; the guest cell is name-lex, so the lift reads
    // each WIT field at its declaration offset and arr-sets it to the field's name-lex slot). `None` (or a
    // structurally-mismatched wit) falls back to the guest's name-lex order — byte-identical for a
    // list/tuple/option/result/scalar (all POSITIONAL: their offsets are order-agnostic), so the only behavior
    // this changes is a `record` whose WIT declaration order differs from name-lex. The result-side analogue
    // of the ARG-side `reorder_record_fields_to_wit` (#3223): the emitted read MUST follow the host's layout.
    wit: Option<&crate::wit_world::WitType>,
    ptr_slot: u32,
    offset: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    use crate::wit_world::WitType;
    match ty.strip_nominal().clone() {
        // A `list<u8>` (Bytes) / `string`: `(data-ptr@offset, len@offset+4)`. Copy the bytes out of the
        // host's linear memory into a value-heap `Bytes`; leave the handle.
        Ty::Bytes | Ty::String => {
            let lptr = *high;
            let (llen, handle, ii) = (lptr + 1, lptr + 2, lptr + 3);
            *high = (*high).max(ii + 1);
            for s in [lptr, llen, handle, ii] {
                scratch_ty.insert(s, ValType::I32);
            }
            out.push(Lir::LocalGet(ptr_slot));
            out.push(Lir::I32Load { offset });
            out.push(Lir::LocalSet(lptr));
            out.push(Lir::LocalGet(ptr_slot));
            out.push(Lir::I32Load { offset: offset + 4 });
            out.push(Lir::LocalSet(llen));
            emit_host_bytes_to_value_heap(out, llen, lptr, handle, ii);
            out.push(Lir::LocalGet(handle));
            Ok(())
        }
        // A `list<T>`: `(list-ptr@offset, count@offset+4)`. arr-alloc(count), then per element at
        // `list-ptr + i*stride(T)` recurse; `arr-set` into the aggregate; finally `vec-of-arr` → a persistent
        // `List` (WITHOUT this the lift returns a raw arr, which `List.len`/`vec-*` misread as empty).
        Ty::List(elem) => {
            let (stride, _) = canonical_layout(db, &elem);
            let list_ptr = *high;
            let (count, i, elem_ptr, elem_h, outer) = (
                list_ptr + 1,
                list_ptr + 2,
                list_ptr + 3,
                list_ptr + 4,
                list_ptr + 5,
            );
            *high = (*high).max(outer + 1);
            for s in [list_ptr, count, i, elem_ptr, elem_h, outer] {
                scratch_ty.insert(s, ValType::I32);
            }
            out.push(Lir::LocalGet(ptr_slot));
            out.push(Lir::I32Load { offset });
            out.push(Lir::LocalSet(list_ptr));
            out.push(Lir::LocalGet(ptr_slot));
            out.push(Lir::I32Load { offset: offset + 4 });
            out.push(Lir::LocalSet(count));
            out.push(Lir::LocalGet(count));
            out.push(Lir::CallImport(OP_ARR_ALLOC));
            out.push(Lir::LocalSet(outer));
            out.push(Lir::ConstI32(0));
            out.push(Lir::LocalSet(i));
            out.push(Lir::Block(BlockType::Empty));
            out.push(Lir::Loop(BlockType::Empty));
            out.push(Lir::LocalGet(i));
            out.push(Lir::LocalGet(count));
            out.push(Lir::I32GeU);
            out.push(Lir::BrIf(1)); // i >= count → exit the outer block
            // elem_ptr = list-ptr + i*stride
            out.push(Lir::LocalGet(list_ptr));
            out.push(Lir::LocalGet(i));
            out.push(Lir::ConstI32(stride as i32));
            out.push(Lir::I32Mul);
            out.push(Lir::I32Add);
            out.push(Lir::LocalSet(elem_ptr));
            // Lift the element (leaves its handle) on a clean stack, stash it, then arr-set(outer, i, elem).
            let elem_wit = match wit {
                Some(WitType::List(e)) => Some(&**e),
                _ => None,
            };
            emit_result_lift(db, &elem, elem_wit, elem_ptr, 0, high, scratch_ty, out)?;
            out.push(Lir::LocalSet(elem_h));
            out.push(Lir::LocalGet(outer));
            out.push(Lir::LocalGet(i));
            out.push(Lir::LocalGet(elem_h));
            out.push(Lir::CallImport(OP_ARR_SET));
            out.push(Lir::LocalSet(outer));
            out.push(Lir::LocalGet(i));
            out.push(Lir::ConstI32(1));
            out.push(Lir::I32Add);
            out.push(Lir::LocalSet(i));
            out.push(Lir::Br(0));
            out.push(Lir::End); // loop
            out.push(Lir::End); // block
            out.push(Lir::LocalGet(outer));
            out.push(Lir::CallImport(OP_VEC_OF_ARR));
            Ok(())
        }
        // A `tuple<…>` / `record{…}`: a value-heap aggregate (a raw arr — a Tuple/Record IS an arr, no
        // vec conversion). Lift each field at its canonical offset in layout order (a record's name-lex
        // order = its value-heap cell layout = the canonical field order).
        // A `tuple<…>` — POSITIONAL, so the WIT order == the guest order; lift each field at its canonical
        // offset (thread each element's WIT via the `tuple<…>` wit).
        Ty::Tuple(elems) => {
            let field_tys: Vec<Ty> = elems.iter().cloned().collect();
            let field_wits: Vec<Option<&WitType>> = (0..field_tys.len())
                .map(|j| match wit {
                    Some(WitType::Tuple(ws)) => ws.get(j),
                    _ => None,
                })
                .collect();
            emit_product_lift(
                db,
                &field_tys,
                &field_wits,
                ptr_slot,
                offset,
                high,
                scratch_ty,
                out,
            )
        }
        // A `record{…}` — a value-heap aggregate whose cell slots are the guest's NAME-LEX order, but the host
        // wrote the fields in the WIT record's DECLARATION order (their canonical offsets accumulate in that
        // order). So lift each WIT field at its declaration offset and arr-set it to that field's name-lex
        // slot; a `None`/non-record wit falls back to the name-lex order (byte-identical). The result-side
        // analogue of the ARG marshal's `reorder_record_fields_to_wit`.
        Ty::Record(fields) => {
            if let Some(WitType::Record(wit_fields)) = wit {
                let names: Vec<String> = fields.keys().map(|s| s.name.to_string()).collect();
                let arr = *high;
                let fh = arr + 1;
                *high = (*high).max(fh + 1);
                scratch_ty.insert(arr, ValType::I32);
                scratch_ty.insert(fh, ValType::I32);
                out.push(Lir::ConstI32(fields.len() as i32));
                out.push(Lir::CallImport(OP_ARR_ALLOC));
                out.push(Lir::LocalSet(arr));
                let mut foff = offset;
                for (fname, fwit) in wit_fields {
                    let fty = fields
                        .iter()
                        .find(|(s, _)| s.name.as_ref() == fname.as_str())
                        .map(|(_, t)| t.clone())
                        .ok_or_else(|| {
                            Reject::decline(
                                "a host WIT record-result field is absent from the guest record type",
                            )
                        })?;
                    let slot = names
                        .iter()
                        .position(|n| n == fname)
                        .expect("field name in name-lex set");
                    let (fs, fa) = canonical_layout(db, &fty);
                    foff = align_up_u32(foff, fa);
                    emit_result_lift(db, &fty, Some(fwit), ptr_slot, foff, high, scratch_ty, out)?;
                    out.push(Lir::LocalSet(fh));
                    out.push(Lir::LocalGet(arr));
                    out.push(Lir::ConstI32(slot as i32));
                    out.push(Lir::LocalGet(fh));
                    out.push(Lir::CallImport(OP_ARR_SET));
                    out.push(Lir::LocalSet(arr));
                    foff += fs;
                }
                out.push(Lir::LocalGet(arr));
                Ok(())
            } else {
                let field_tys: Vec<Ty> = fields.values().cloned().collect();
                let field_wits: Vec<Option<&WitType>> = vec![None; field_tys.len()];
                emit_product_lift(
                    db,
                    &field_tys,
                    &field_wits,
                    ptr_slot,
                    offset,
                    high,
                    scratch_ty,
                    out,
                )
            }
        }
        // A `result<list<u8>, enum>` (both arms carry a payload — Ok: Bytes, Err: enum) vs an option-shaped
        // `Sum` (2 variants: one single-payload + one nullary). Distinguished by shape.
        Ty::Sum { decl, .. } => {
            if crate::backend::wasm::host::result_bytes_enum(db, ty).is_some() {
                emit_result_sum_lift(db, ty, decl, wit, ptr_slot, offset, high, scratch_ty, out)
            } else if crate::backend::wasm::host::option_payload_ty(db, ty).is_some() {
                emit_option_sum_lift(db, ty, decl, wit, ptr_slot, offset, high, scratch_ty, out)
            } else if crate::backend::wasm::host::variant_liftable_payload_cases(db, ty).is_some() {
                // A general VARIANT result (N cases): read the disc + the selected case's payload (scalar OR a
                // liftable compound) from the spilled region and rebuild the guest Sum — the N-case
                // generalization of the option lift, the result-side twin of the bare-variant ARG marshal.
                let _ = decl;
                emit_variant_sum_lift(db, ty, wit, ptr_slot, offset, high, scratch_ty, out)
            } else {
                emit_option_sum_lift(db, ty, decl, wit, ptr_slot, offset, high, scratch_ty, out)
            }
        }
        // A SCALAR leaf (bool/char/aliased int/float, or a `Qty` over one) — load width-correct + box. Only
        // reached as an ELEMENT/FIELD/PAYLOAD of a compound (a top-level scalar result crosses by value).
        Ty::Bool | Ty::Char | Ty::Int(_) | Ty::Float(_) | Ty::Qty { .. } => {
            emit_scalar_leaf_lift(db, ty, ptr_slot, offset, out)
        }
        other => Err(Reject::unsupported(format!(
            "the general result-lift does not support a `{}` spilled-result leaf",
            other.render_name(&db.name_ctx())
        ))),
    }
}

/// The product (tuple / record) arm of [`emit_result_lift`]: `arr-alloc(n)`, then lift each field at its
/// canonical byte offset (from `offset`) and `arr-set` it in layout order; leave the raw arr handle.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_product_lift(
    db: &mut Db,
    field_tys: &[Ty],
    // Per-field WIT type (index-aligned with `field_tys`) for threading a nested record's field ORDER; a
    // POSITIONAL product (tuple, or a name-lex record fallback) passes `None` per field (byte-identical).
    field_wits: &[Option<&crate::wit_world::WitType>],
    ptr_slot: u32,
    offset: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let arr = *high;
    let fh = arr + 1;
    *high = (*high).max(fh + 1);
    scratch_ty.insert(arr, ValType::I32);
    scratch_ty.insert(fh, ValType::I32);
    out.push(Lir::ConstI32(field_tys.len() as i32));
    out.push(Lir::CallImport(OP_ARR_ALLOC));
    out.push(Lir::LocalSet(arr));
    let mut foff = offset;
    for (j, fty) in field_tys.iter().enumerate() {
        let (fs, fa) = canonical_layout(db, fty);
        foff = align_up_u32(foff, fa);
        // Lift the field (leaves its handle) on a clean stack, stash, then arr-set(arr, j, field).
        emit_result_lift(
            db,
            fty,
            field_wits.get(j).copied().flatten(),
            ptr_slot,
            foff,
            high,
            scratch_ty,
            out,
        )?;
        out.push(Lir::LocalSet(fh));
        out.push(Lir::LocalGet(arr));
        out.push(Lir::ConstI32(j as i32));
        out.push(Lir::LocalGet(fh));
        out.push(Lir::CallImport(OP_ARR_SET));
        out.push(Lir::LocalSet(arr));
        foff += fs;
    }
    out.push(Lir::LocalGet(arr));
    Ok(())
}

/// The SCALAR-LEAF arm of [`emit_result_lift`]: a scalar element/field of a spilled compound result (a
/// `bool`, `char`, aliased int of any width, or `f32`/`f64`, incl. a `Qty` over one) that the host stored at
/// its NATURAL width in linear memory. Load it width-correct (sign/zero-extending a narrow int into the i64
/// int cell `box-int` takes), then BOX it into a value-heap scalar cell (`box-int`/`box-bool`/`box-float`/
/// `box-float32`), leaving the handle — so it can be an `arr-set` element of the enclosing tuple/list/record
/// or an option payload. A top-level scalar RESULT never reaches here (it crosses by value, not spilled).
pub(super) fn emit_scalar_leaf_lift(
    db: &mut Db,
    ty: &Ty,
    ptr_slot: u32,
    offset: u32,
    out: &mut Emit,
) -> Result<(), Reject> {
    let box_op = box_op_ty(db, ty)?
        .ok_or_else(|| Reject::decline("a scalar-leaf result field is not a value-heap scalar"))?;
    // Peel `Qty`/`Nominal` to the effective scalar type (its runtime rep) for the width-correct load.
    let scalar = peel_qty_ty(ty.clone());
    out.push(Lir::LocalGet(ptr_slot));
    match &scalar {
        // A bool is a 1-byte 0/1; `box-bool` takes the i32 directly (no extend).
        Ty::Bool => out.push(Lir::I32Load8U { offset }),
        // A char is a 4-byte code point; zero-extend into the i64 `box-int` cell.
        Ty::Char => {
            out.push(Lir::I32Load { offset });
            out.push(Lir::I64ExtendI32U);
        }
        Ty::Int(it) => {
            let signed = it.ground_signed();
            match it.ground_width() {
                64 => out.push(Lir::I64Load { offset }),
                32 => {
                    out.push(Lir::I32Load { offset });
                    out.push(if signed {
                        Lir::I64ExtendI32S
                    } else {
                        Lir::I64ExtendI32U
                    });
                }
                16 => {
                    out.push(if signed {
                        Lir::I32Load16S { offset }
                    } else {
                        Lir::I32Load16U { offset }
                    });
                    out.push(if signed {
                        Lir::I64ExtendI32S
                    } else {
                        Lir::I64ExtendI32U
                    });
                }
                8 => {
                    out.push(if signed {
                        Lir::I32Load8S { offset }
                    } else {
                        Lir::I32Load8U { offset }
                    });
                    out.push(if signed {
                        Lir::I64ExtendI32S
                    } else {
                        Lir::I64ExtendI32U
                    });
                }
                _ => {
                    return Err(Reject::decline(
                        "a non-aliased int width has no scalar-leaf load",
                    ));
                }
            }
        }
        Ty::Float(ft) => match ft.ground_width() {
            64 => out.push(Lir::F64Load { offset }),
            32 => out.push(Lir::F32Load { offset }),
            _ => {
                return Err(Reject::decline(
                    "a non-aliased float width has no scalar-leaf load",
                ));
            }
        },
        _ => return Err(Reject::decline("not a scalar-leaf type")),
    }
    out.push(Lir::CallImport(box_op));
    Ok(())
}

/// The option-shaped-`Sum` arm of [`emit_result_lift`]: a 2-variant sum with one single-payload arm and one
/// nullary arm (`option<T>`). The canonical WIT `option` discriminant is fixed (none = 0, some = 1)
/// regardless of the Cadenza `Option` decl's variant order, so the PRESENT arm is selected by disc == 1 —
/// NOT the Cadenza payload-variant index. On it, lift the payload at its canonical offset and build the
/// value-heap sum at the Cadenza payload variant's own discriminant (`some_disc`); the absent arm builds the
/// Cadenza nullary variant (`none_disc`). A wider sum (an N-ary variant, a multi-payload arm, `result<_,_>`
/// with two payloads) DECLINES this increment.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_option_sum_lift(
    db: &mut Db,
    ty: &Ty,
    decl: StructId,
    // The `option<T>` WIT type, when known — its payload `T`'s WIT is threaded into the Some-arm payload lift
    // (so a `option<record>` payload reorders its fields). `None`/non-option falls back to name-lex.
    wit: Option<&crate::wit_world::WitType>,
    ptr_slot: u32,
    offset: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let variants: Vec<(u32, usize)> = {
        let d = db
            .type_decl_by_occ(decl)
            .ok_or_else(|| Reject::decline("the option-shaped result sum decl was not found"))?;
        d.variants
            .iter()
            .enumerate()
            .map(|(i, v)| (i as u32, v.payloads.len()))
            .collect()
    };
    let nvar = variants.len();
    let payload_arms: Vec<u32> = variants
        .iter()
        .filter(|(_, n)| *n == 1)
        .map(|(d, _)| *d)
        .collect();
    let nullary_arms: Vec<u32> = variants
        .iter()
        .filter(|(_, n)| *n == 0)
        .map(|(d, _)| *d)
        .collect();
    if !(nvar == 2 && payload_arms.len() == 1 && nullary_arms.len() == 1) {
        return Err(Reject::decline(
            "the general result-lift wires an option-shaped sum result (2 variants: one single-payload, \
             one nullary); a wider variant/enum/result is a later increment",
        ));
    }
    let some_disc = payload_arms[0];
    let none_disc = nullary_arms[0];
    let payload_ty = variant_payload_ty_at(db, ty, some_disc)
        .ok_or_else(|| Reject::decline("the option-result payload type could not be resolved"))?;
    let (_, payload_align) = canonical_layout(db, &payload_ty);
    let ds = disc_size_for(nvar); // 1 for a 2-variant option
    let payload_off = offset + align_up_u32(ds, payload_align);
    // disc = mem[ptr_slot + offset] (a `ds`-byte discriminant; ds == 1 for an option, read zero-extended).
    let disc = *high;
    *high = (*high).max(disc + 1);
    scratch_ty.insert(disc, ValType::I32);
    out.push(Lir::LocalGet(ptr_slot));
    out.push(Lir::I32Load8U { offset });
    out.push(Lir::LocalSet(disc));
    // if disc == 1 (WIT `option` "some") { Some(lift payload) } else { None } — leaves the sum handle.
    out.push(Lir::LocalGet(disc));
    out.push(Lir::ConstI32(1));
    out.push(Lir::I32Eq);
    out.push(Lir::If(BlockType::Val(ValType::I32)));
    {
        let payload_wit = match wit {
            Some(crate::wit_world::WitType::Option(p)) => Some(&**p),
            _ => None,
        };
        emit_result_lift(
            db,
            &payload_ty,
            payload_wit,
            ptr_slot,
            payload_off,
            high,
            scratch_ty,
            out,
        )?;
        let ph = *high;
        *high = (*high).max(ph + 1);
        scratch_ty.insert(ph, ValType::I32);
        out.push(Lir::LocalSet(ph));
        out.push(Lir::ConstI32(some_disc as i32));
        out.push(Lir::LocalGet(ph));
        out.push(Lir::CallImport(OP_SUM_NEW));
    }
    out.push(Lir::Else);
    {
        // The nullary arm: sum-new(none_disc, IMM_UNIT) — the 2-child nullary (payload = inline unit).
        out.push(Lir::ConstI32(none_disc as i32));
        out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32));
        out.push(Lir::CallImport(OP_SUM_NEW));
    }
    out.push(Lir::End);
    Ok(())
}

/// The general scalar-payload VARIANT arm of [`emit_result_lift`]: an N-case sum where each case is nullary or
/// carries ONE scalar payload (NOT option/result-shaped — those took their own arms). Reads the `ds`-byte
/// discriminant from the spilled retptr'd region, then a per-case chain rebuilds the guest Sum: on a payload
/// case, lift the scalar at the canonical payload offset (`align_up(ds, max-payload-align)`) and `sum-new(i,
/// payload)`; on a nullary case, `sum-new(i, IMM_UNIT)`. The case index `i` IS both the component discriminant
/// (declaration order) and the guest variant's own discriminant, so no remap is needed. The result-side twin of
/// the bare-variant ARG marshal (`emit_variant_reg_flatten`); the N-case generalization of `emit_option_sum_lift`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_variant_sum_lift(
    db: &mut Db,
    ty: &Ty,
    // The variant's WIT type, when known — its i-th case's payload WIT threads into the recursive payload lift
    // (so a `record` payload orders its fields to the host layout). `None`/non-variant falls back to name-lex.
    wit: Option<&crate::wit_world::WitType>,
    ptr_slot: u32,
    offset: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    let cases = crate::backend::wasm::host::variant_liftable_payload_cases(db, ty)
        .ok_or_else(|| Reject::decline("a variant result is not a liftable-payload variant"))?;
    let ncases = cases.len();
    // The payload region begins after the discriminant, aligned to the widest payload case's alignment (the
    // canonical variant memory layout — all payload cases share this one offset). `canonical_layout` gives a
    // compound payload's alignment too (a `list<u8>` payload is `(ptr,len)`, align 4).
    let mut max_align = 1u32;
    for (i, (_, hp)) in cases.iter().enumerate() {
        if *hp {
            let pt = variant_payload_ty_at(db, ty, i as u32).ok_or_else(|| {
                Reject::decline("a variant result payload type could not be resolved")
            })?;
            max_align = max_align.max(canonical_layout(db, &pt).1);
        }
    }
    let ds = disc_size_for(ncases);
    let payload_off = offset + align_up_u32(ds, max_align);
    // disc = mem[ptr_slot + offset], read zero-extended at the discriminant's byte width.
    let disc = *high;
    *high = (*high).max(disc + 1);
    scratch_ty.insert(disc, ValType::I32);
    out.push(Lir::LocalGet(ptr_slot));
    match ds {
        1 => out.push(Lir::I32Load8U { offset }),
        2 => out.push(Lir::I32Load16U { offset }),
        _ => out.push(Lir::I32Load { offset }),
    }
    out.push(Lir::LocalSet(disc));
    // A per-case chain `if disc==0 { build 0 } else if disc==1 { … } else { build last }`; each arm leaves the
    // rebuilt sum handle (an `If(Val I32)`), so the whole chain is one value on the stack.
    for (i, (_, hp)) in cases.iter().enumerate() {
        let is_last = i == ncases - 1;
        if !is_last {
            out.push(Lir::LocalGet(disc));
            out.push(Lir::ConstI32(i as i32));
            out.push(Lir::I32Eq);
            out.push(Lir::If(BlockType::Val(ValType::I32)));
        }
        if *hp {
            let pt = variant_payload_ty_at(db, ty, i as u32).ok_or_else(|| {
                Reject::decline("a variant result payload type could not be resolved")
            })?;
            // `emit_result_lift` dispatches the payload: a SCALAR leaf boxes (via emit_scalar_leaf_lift), a
            // liftable COMPOUND (`list<u8>`/`tuple`/`record`/…) recurses — leaving a value-heap handle either
            // way. The case's payload WIT (from the variant WIT) threads through for a record's field order.
            let payload_wit = match wit {
                Some(crate::wit_world::WitType::Variant(cs)) => {
                    cs.get(i).and_then(|(_, w)| w.as_ref())
                }
                _ => None,
            };
            emit_result_lift(
                db,
                &pt,
                payload_wit,
                ptr_slot,
                payload_off,
                high,
                scratch_ty,
                out,
            )?;
            let ph = *high;
            *high = (*high).max(ph + 1);
            scratch_ty.insert(ph, ValType::I32);
            out.push(Lir::LocalSet(ph));
            out.push(Lir::ConstI32(i as i32));
            out.push(Lir::LocalGet(ph));
            out.push(Lir::CallImport(OP_SUM_NEW));
        } else {
            out.push(Lir::ConstI32(i as i32));
            out.push(Lir::ConstI32(super::super::runtime_abi::IMM_UNIT as i32));
            out.push(Lir::CallImport(OP_SUM_NEW));
        }
        if !is_last {
            out.push(Lir::Else);
        }
    }
    for _ in 0..ncases.saturating_sub(1) {
        out.push(Lir::End);
    }
    Ok(())
}

/// The RESULT-shaped-`Sum` arm of [`emit_result_lift`]: `result<list<u8>, enum>` (run.run's `result<payload,
/// error>`) — a 2-variant sum where BOTH arms carry a payload (Ok: `Bytes`, Err: a PAYLOAD-LESS `enum`). The
/// canonical WIT `result` discriminant is FIXED (ok = 0, err = 1) regardless of the Cadenza `Result` decl's
/// variant order, so the arm is selected by disc == 0. On Ok, lift the `Bytes` payload and build the value-
/// heap `Ok` at the Cadenza `Ok` variant's own discriminant; on Err, read the enum's i32 discriminant and box
/// it as the guest's enum-disc value (`box-int` of the zero-extended disc — an enum-disc crosses as a boxed
/// int cell when it is a heap payload, the same store `Err(Error.X)` codegens) and build the `Err`. A payload
/// whose Ok arm is not `Bytes` or whose Err arm is not a payload-less enum declines upstream (`result_is_
/// liftable`/`result_bytes_enum`); a richer result is a later increment.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_result_sum_lift(
    db: &mut Db,
    ty: &Ty,
    decl: StructId,
    // The `result<ok, err>` WIT type, when known — the Ok arm's payload WIT is threaded into its lift.
    wit: Option<&crate::wit_world::WitType>,
    ptr_slot: u32,
    offset: u32,
    high: &mut u32,
    scratch_ty: &mut HashMap<u32, ValType>,
    out: &mut Emit,
) -> Result<(), Reject> {
    // The Cadenza `Result` decl's own Ok / Err variant discriminants (build the value-heap sum at these,
    // independent of the WIT-canonical ok=0/err=1 the host wrote).
    let (ok_disc, err_disc) = {
        let d = db
            .type_decl_by_occ(decl)
            .ok_or_else(|| Reject::decline("the result-shaped sum decl was not found"))?;
        let ok = d
            .variants
            .iter()
            .position(|v| v.name == "Ok")
            .ok_or_else(|| Reject::decline("the result sum has no Ok variant"))?
            as u32;
        let err = d
            .variants
            .iter()
            .position(|v| v.name == "Err")
            .ok_or_else(|| Reject::decline("the result sum has no Err variant"))?
            as u32;
        (ok, err)
    };
    let ok_payload_ty = variant_payload_ty_at(db, ty, ok_disc)
        .ok_or_else(|| Reject::decline("the result Ok payload type could not be resolved"))?;
    let ds = disc_size_for(2); // 1 for a 2-variant result
    // The payload area is the JOIN of the arms; both align to 4 (Bytes `(ptr,len)` / enum i32).
    let (_, ok_align) = canonical_layout(db, &ok_payload_ty);
    let payload_off = offset + align_up_u32(ds, ok_align.max(4));
    // disc = mem[ptr_slot + offset] (a 1-byte discriminant, read zero-extended; WIT `result` ok=0 / err=1).
    let disc = *high;
    *high = (*high).max(disc + 1);
    scratch_ty.insert(disc, ValType::I32);
    out.push(Lir::LocalGet(ptr_slot));
    out.push(Lir::I32Load8U { offset });
    out.push(Lir::LocalSet(disc));
    // if disc == 0 (WIT `result` "ok") { Ok(lift bytes) } else { Err(box enum disc) } — leaves the sum handle.
    out.push(Lir::LocalGet(disc));
    out.push(Lir::ConstI32(0));
    out.push(Lir::I32Eq);
    out.push(Lir::If(BlockType::Val(ValType::I32)));
    {
        let ok_wit = match wit {
            Some(crate::wit_world::WitType::Result { ok: Some(o), .. }) => Some(&**o),
            _ => None,
        };
        emit_result_lift(
            db,
            &ok_payload_ty,
            ok_wit,
            ptr_slot,
            payload_off,
            high,
            scratch_ty,
            out,
        )?;
        let ph = *high;
        *high = (*high).max(ph + 1);
        scratch_ty.insert(ph, ValType::I32);
        out.push(Lir::LocalSet(ph));
        out.push(Lir::ConstI32(ok_disc as i32));
        out.push(Lir::LocalGet(ph));
        out.push(Lir::CallImport(OP_SUM_NEW));
    }
    out.push(Lir::Else);
    {
        // The err arm's payload is the enum's i32 discriminant; box it as the guest's enum-disc value
        // (`box-int` of the zero-extended disc), then `Err(that)`.
        out.push(Lir::LocalGet(ptr_slot));
        out.push(Lir::I32Load {
            offset: payload_off,
        });
        out.push(Lir::I64ExtendI32U);
        out.push(Lir::CallImport(OP_BOX_INT));
        let eh = *high;
        *high = (*high).max(eh + 1);
        scratch_ty.insert(eh, ValType::I32);
        out.push(Lir::LocalSet(eh));
        out.push(Lir::ConstI32(err_disc as i32));
        out.push(Lir::LocalGet(eh));
        out.push(Lir::CallImport(OP_SUM_NEW));
    }
    out.push(Lir::End);
    Ok(())
}
