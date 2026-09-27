//! `list<…>` entry-param element READERS (peeled from `mod.rs`, a pure code move — the 512 KiB file-size
//! mandate split). Each function classifies one list element shape into a [`serialize::ListElem`] descriptor
//! the wrapper uses to lift the element out of linear memory into a value-heap handle: `list_scalar_elem`
//! (a flat/nested scalar, byte-leaf, or scalar-fielded compound element), `list_sum_elem` (an `option<scalar>`
//! element), and `list_flags_elem` (a WIT `flags` element, unpacked into a record-of-bools). `scalar_read_box`
//! / `scalar_read_box_ty` are the shared per-scalar read+box descriptor helpers. Called from `mod.rs`'s
//! entry-param routing and `param_field.rs`'s list-field rebuild (hence the classifiers are `pub(super)`).

use super::*;

/// The per-element read+box descriptor for a `list<scalar>` (or NESTED `list<list<…<scalar>>>`) entry param
/// — the load op / natural-align / canonical stride / narrow-int extend / box op of the SCALAR LEAF, plus the
/// `nest_lists` depth of enclosing list levels (0 = flat `list<scalar>`; k>0 = the element is a `(ptr,len)`
/// sub-list recursively lifted k levels to the scalar — el8/eln). `None` for a compound leaf (list<record>,
/// list<tuple>) — a later slice.
pub(super) fn list_scalar_elem(
    elem: &crate::ty::Ty,
) -> Option<crate::backend::wasm::serialize::ListElem> {
    use crate::backend::wasm::serialize::ListElem;
    use crate::ty::Ty;
    // Descend nested list levels: a `list<list<…<scalar>>>` element is `nest` list-levels then a scalar leaf.
    // Every intermediate `list<T>` has the SAME `(ptr,len)` boundary rep, so the nesting is a uniform depth.
    let mut nest = 0u32;
    let mut leaf = elem.strip_nominal().clone();
    while let Ty::List(inner) = &leaf {
        nest += 1;
        leaf = inner.strip_nominal().clone();
    }
    // A `String`/`Bytes` leaf: a `list<string>`/`list<bytes>` element crosses as a `(ptr, len)` descriptor
    // (canonical stride 8, like a nested sub-list) and lifts by copying its bytes out of linear memory 0 into a
    // value-heap byte-leaf (`bytes-alloc`/`bytes-set`), not a scalar load+box. `nest_lists` carries the
    // enclosing list depth (0 = a flat `list<string>`; k>0 = a `list<list<…<string>>>` whose elements are
    // `(ptr,len)` sub-lists recursively descended k levels to the byte-leaf copy-in) — the byte-leaf twin of
    // the nested-scalar `list<list<…<scalar>>>` recursion; `emit_list_level` descends `levels` then fires the
    // byte-leaf branch at level 0, allocating fresh copy-in scratch at each level via `next_local`. The scalar
    // read/box fields are unused for a byte-leaf, so they carry inert placeholders.
    if matches!(leaf, Ty::String | Ty::Bytes) {
        return Some(ListElem {
            load_op: 0,
            load_align: 0,
            stride: 8,
            extend: None,
            box_op: "",
            nest_lists: nest,
            byte_leaf: Some(matches!(leaf, Ty::String)),
            compound: None,
            sum: None,
            flags: None,
        });
    }
    // A COMPOUND leaf — a scalar-fielded `tuple<…>`/`record<…>` — crosses as `stride` contiguous bytes per
    // element (the element's `canonical_size`), each field at its canonical offset. Only a FLAT (`nest == 0`)
    // element whose fields are ALL aliased-width scalars is admitted here (a nested-compound / byte-leaf / sum
    // field is a later slice); read each field into a value-heap cell (`arr-alloc`/`arr-set`) per element.
    if matches!(leaf, Ty::Tuple(_) | Ty::Record(_)) {
        if nest != 0 {
            return None;
        }
        use crate::backend::wasm::wit_ctype::{canonical_size, record_field_offsets};
        // Field types in CELL-SLOT order: a tuple is positional; a record's cell slots are name-lex, which is
        // also `ty_natural_wit`'s sorted `WitType::Record` order — so the field WITs, their offsets, and the
        // cell slots all share one order.
        let field_wits: Vec<crate::wit_world::WitType> =
            match crate::wit_world::ty_natural_wit(&leaf)? {
                crate::wit_world::WitType::Tuple(ws) => ws,
                crate::wit_world::WitType::Record(fs) => fs.into_iter().map(|(_n, w)| w).collect(),
                _ => return None,
            };
        let offsets = record_field_offsets(&field_wits);
        let mut fields = Vec::with_capacity(field_wits.len());
        for (fw, off) in field_wits.iter().zip(offsets) {
            let (load_op, load_align, _stride, extend, box_op) = scalar_read_box(fw)?;
            fields.push(crate::backend::wasm::serialize::CompoundListField {
                offset: off,
                load_op,
                load_align,
                extend,
                box_op,
            });
        }
        return Some(ListElem {
            load_op: 0,
            load_align: 0,
            stride: canonical_size(&crate::wit_world::ty_natural_wit(&leaf)?),
            extend: None,
            box_op: "",
            nest_lists: 0,
            byte_leaf: None,
            compound: Some(fields),
            sum: None,
            flags: None,
        });
    }
    // The scalar leaf's read+box: (load_op, natural-align, stride, narrow-extend, box_op).
    let (load_op, load_align, stride, extend, box_op) = scalar_read_box_ty(&leaf)?;
    Some(ListElem {
        load_op,
        load_align,
        stride,
        extend,
        box_op,
        nest_lists: nest,
        byte_leaf: None,
        compound: None,
        sum: None,
        flags: None,
    })
}

/// The per-element descriptor for a `list<option<scalar>>` entry param (lpo1): each element is a canonical
/// `option<T>` — a 1-byte discriminant then the scalar payload — lifted into a value-heap sum cell per element.
/// Reuses [`crate::backend::wasm::arg_boundary::fixed_shape_option_scalar_arg`] for the disc convention
/// (`boundary_true_disc` + the per-arm decl discs) and the canonical layout helpers for the payload's in-memory
/// offset + load op. Only a FLAT `option<SCALAR>` element is admitted; a `result<…>` (two payload arms), a
/// compound/`list`/`Bytes` payload, or a nested `list<option<…>>` declines to a later slice — the caller then
/// declines the whole param. Complements [`list_scalar_elem`] (called via `.or_else` on the entry path).
pub(super) fn list_sum_elem(
    db: &mut Db,
    elem: &crate::ty::Ty,
) -> Option<crate::backend::wasm::serialize::ListElem> {
    use crate::backend::wasm::serialize::{ListElem, SumArgArm, SumArmPayload, SumListElem};
    use crate::ty::Ty;
    let leaf = elem.strip_nominal();
    // Only a DIRECT sum element (no nested-list wrapping) is admitted here — a `list<list<option<…>>>` is a
    // later slice.
    if !matches!(leaf, Ty::Sum { .. }) {
        return None;
    }
    // Classify the two-variant sum. Only the OPTION shape (one nullary + one SCALAR payload) is admitted: the
    // rebuild's `arm_true` (Some) is a single boxed scalar, `arm_false` (None) is nullary. A `result<…>`
    // classifies as `ArgSlot::Result` (two payload arms) and declines here — a later slice.
    let (_slot, _vts, rebuild) =
        crate::backend::wasm::arg_boundary::fixed_shape_option_scalar_arg(db, leaf)?;
    let SumArgArm {
        decl_disc: some_decl_disc,
        payload: SumArmPayload::Scalar { .. },
    } = rebuild.arm_true
    else {
        return None;
    };
    let SumArgArm {
        decl_disc: none_decl_disc,
        payload: SumArmPayload::Nullary,
    } = rebuild.arm_false
    else {
        return None; // not the option shape (a result<…> Err arm carries a scalar, not nullary)
    };
    // The element WIT is `option<payload>`; derive the payload's canonical in-memory load op + offset from it,
    // keyed on the WIT (so the memarg alignment matches the canonical layout the disc/payload placement uses).
    // The disc is 1 byte at offset 0; the payload sits at `align_to(1, canonical_align(payload))`.
    let crate::wit_world::WitType::Option(payload_wit) = natural_wit_bare(db, leaf)? else {
        return None;
    };
    let (payload_load_op, payload_load_align, _pstride, payload_extend, payload_box) =
        scalar_read_box(&payload_wit)?;
    // The payload offset is `align_to(disc_size=1, canonical_align(payload))`, which for a 1-byte disc is just
    // the payload's own alignment (every alignment is ≥ 1). The stride is the option's whole `canonical_size`.
    let payload_offset = crate::backend::wasm::wit_ctype::canonical_align(&payload_wit);
    let stride = crate::backend::wasm::wit_ctype::canonical_size(
        &crate::wit_world::WitType::Option(payload_wit.clone()),
    );
    Some(ListElem {
        load_op: 0,
        load_align: 0,
        stride,
        extend: None,
        box_op: "",
        nest_lists: 0,
        byte_leaf: None,
        compound: None,
        sum: Some(SumListElem {
            boundary_true_disc: rebuild.boundary_true_disc,
            some_decl_disc,
            none_decl_disc,
            payload_offset,
            payload_load_op,
            payload_load_align,
            payload_extend,
            payload_box,
        }),
        flags: None,
    })
}

/// The per-element descriptor for a `list<flags>` entry param: each element is a PACKED bitset (canonical width
/// 1/2/4 bytes by label count) lifted into a value-heap record-of-bools cell per element. The guest models a WIT
/// `flags` as `record{ label: bool, … }` (the operator's PRODUCT ruling), so `elem_ty` is a `Ty::Record` of
/// bools that must match the WIT flags `labels` BY NAME. Builds the `(cell_slot, bitset_bit)` pairing — the list
/// twin of the record-field [`param_field::param_field_rebuild`]'s flags arm and the top-level flags param arm.
/// Declines >32 labels, an arity mismatch, a non-bool field, or a label with no matching field. WIT-AWARE: it
/// needs the flags labels (the guest `Ty::Record` alone cannot tell a flags element from an ordinary
/// record-of-bools), so unlike [`list_scalar_elem`] it is called with the element's WIT.
pub(super) fn list_flags_elem(
    elem_ty: &crate::ty::Ty,
    labels: &[String],
) -> Option<crate::backend::wasm::serialize::ListElem> {
    use crate::backend::common::export_name::kebab_extern_name;
    use crate::backend::wasm::serialize::{FlagsListElem, ListElem};
    use crate::ty::Ty;
    let Ty::Record(map) = elem_ty.strip_nominal() else {
        return None;
    };
    if labels.len() > 32 || map.len() != labels.len() {
        return None;
    }
    let label_kebab: Vec<String> = labels.iter().map(|l| kebab_extern_name(l)).collect();
    let mut field_bits: Vec<(u32, u32)> = Vec::with_capacity(labels.len());
    for (slot, (fname, fty)) in map.iter().enumerate() {
        if !matches!(fty.strip_nominal(), Ty::Bool) {
            return None;
        }
        let fk = kebab_extern_name(fname.name.as_ref());
        let bit = label_kebab.iter().position(|l| *l == fk)?;
        field_bits.push((slot as u32, bit as u32));
    }
    // The bitset's canonical width (1/2/4 bytes) sets the packed-read load op + the per-element stride; its
    // natural-align memarg is `log2(size)` (`size.trailing_zeros()`: 1→0, 2→1, 4→2).
    let size = crate::backend::wasm::wit_ctype::canonical_size(&crate::wit_world::WitType::Flags(
        labels.to_vec(),
    ));
    Some(ListElem {
        load_op: 0,
        load_align: 0,
        stride: size,
        extend: None,
        box_op: "",
        nest_lists: 0,
        byte_leaf: None,
        compound: None,
        sum: None,
        flags: Some(FlagsListElem {
            field_bits,
            load_op: disc_load_of(size),
            load_align: size.trailing_zeros(),
        }),
    })
}

/// The wasm read+box descriptor `(load_op, natural-align log2, canonical stride, narrow-int extend, box op)`
/// for an aliased-width SCALAR leaf `Ty` (`Int*`/`UInt*`/`Bool`/`Float*`). `None` for a non-scalar. Shared by
/// [`list_scalar_elem`]'s flat-scalar and compound-element paths.
fn scalar_read_box_ty(leaf: &crate::ty::Ty) -> Option<(u8, u32, u32, Option<bool>, &'static str)> {
    use crate::backend::wasm::wasm_abi::op;
    use crate::ty::Ty;
    Some(match leaf {
        Ty::Int(it) => {
            let signed = it.ground_signed();
            match it.ground_width() {
                64 => (op::I64_LOAD, 3, 8, None, "box-int"),
                32 => (op::I32_LOAD, 2, 4, Some(signed), "box-int"),
                16 => (
                    if signed {
                        op::I32_LOAD16_S
                    } else {
                        op::I32_LOAD16_U
                    },
                    1,
                    2,
                    Some(signed),
                    "box-int",
                ),
                8 => (
                    if signed {
                        op::I32_LOAD8_S
                    } else {
                        op::I32_LOAD8_U
                    },
                    0,
                    1,
                    Some(signed),
                    "box-int",
                ),
                _ => return None,
            }
        }
        Ty::Bool => (op::I32_LOAD8_U, 0, 1, None, "box-bool"),
        Ty::Float(ft) => match ft.ground_width() {
            64 => (op::F64_LOAD, 3, 8, None, "box-float"),
            32 => (op::F32_LOAD, 2, 4, None, "box-float32"),
            _ => return None,
        },
        _ => return None,
    })
}

/// The read+box descriptor for a compound field, keyed on its canonical `WitType` (so the memarg alignment
/// matches the canonical layout `record_field_offsets` computed). Maps each scalar WIT to the same
/// load/extend/box the value rep uses.
fn scalar_read_box(
    wt: &crate::wit_world::WitType,
) -> Option<(u8, u32, u32, Option<bool>, &'static str)> {
    use crate::backend::wasm::wasm_abi::op;
    use crate::wit_world::WitType;
    Some(match wt {
        WitType::S64 | WitType::U64 => (op::I64_LOAD, 3, 8, None, "box-int"),
        WitType::S32 => (op::I32_LOAD, 2, 4, Some(true), "box-int"),
        WitType::U32 => (op::I32_LOAD, 2, 4, Some(false), "box-int"),
        WitType::S16 => (op::I32_LOAD16_S, 1, 2, Some(true), "box-int"),
        WitType::U16 => (op::I32_LOAD16_U, 1, 2, Some(false), "box-int"),
        WitType::S8 => (op::I32_LOAD8_S, 0, 1, Some(true), "box-int"),
        WitType::U8 => (op::I32_LOAD8_U, 0, 1, Some(false), "box-int"),
        WitType::Bool => (op::I32_LOAD8_U, 0, 1, None, "box-bool"),
        WitType::F64 => (op::F64_LOAD, 3, 8, None, "box-float"),
        WitType::F32 => (op::F32_LOAD, 2, 4, None, "box-float32"),
        _ => return None,
    })
}
