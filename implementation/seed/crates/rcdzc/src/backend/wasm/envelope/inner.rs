//! `envelope::inner` — inner-component byte builders for the resource/closure envelopes.
//!
//! Extracted verbatim from the parent `envelope` module (the `resource_inner_component_*` +
//! `component_instantiate_*_item` family) to keep the source file under the size cap. Behavior
//! and byte grammar are unchanged; these build the nested embedded component that the outer
//! `assemble_*_resource` envelopes instantiate.

use super::*;
use crate::backend::wasm::encode::{section, uleb_bytes, uleb128, wasm_vec};

/// The nested RE-EXPORT component (a self-contained component blob, its own magic + sections). It
/// IMPORTS an abstract resource (`SubResource` bound) + the two funcs typed against it, then RE-EXPORTS
/// the resource DIRECTLY (no `SubResource` ascription — that would mint a fresh identity distinct from
/// the funcs' resource → "resource types are not the same") + the funcs re-typed against the exported
/// resource. This is the only way to export a resource-with-methods; the outer component instantiates it
/// with the real (rep-carrying) resource + lifted funcs. Inner index spaces: imported resource → type 0;
/// `own<0>` → type 1; make-ft → type 2; `list u8` → type 3; encode-ft → type 4; imported `make` → func
/// 0; imported `encode` → func 1; the RE-EXPORTED resource → type 5; `own<5>` → type 6; make-exp-ft →
/// type 7; `own<5>`,`list u8`,encode-exp-ft → types 8,9,10.
pub(super) fn resource_inner_component() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource `import-type-t` (Type, SubResource bound) → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) then the imported `make` functype `() -> own<0>` (type 2).
    let make_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `list u8` (type 3) then the imported `encode` functype `(self: own<0>) -> list<u8>` (type
    // 4).
    let encode_import_types = {
        let mut items = list_u8_defined_type();
        items.extend_from_slice(&self_own_to_list_functype(1, 3));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&encode_import_types);
    // sec 10: import `import-func-encode` as a func of type 4 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", 4)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY under the name `t` (no ascription — a
    // `SubResource` ascription would mint a fresh identity) → exported type 5.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<5>` (type 6) then the `make` functype re-typed against the exported resource (type 7).
    let make_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&nullary_result_functype(&owned_valtype(6)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to the exported functype 7.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, 7)),
    ));
    // sec 7: `own<5>` (type 8), `list u8` (type 9), then the `encode` functype re-typed against the
    // exported resource (type 10).
    let encode_export_types = {
        let mut items = own_item(5);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_own_to_list_functype(8, 9));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_export_types);
    // sec 11: export `encode` (func 1) ascribed to the exported functype 10.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, 10)),
    ));
    out
}

/// The nested RE-EXPORT component for a CLOSURE resource — like [`resource_inner_component`] but the
/// second method is `call : (self: own<t>, args…) -> R` (invoke the closure) instead of `encode : (self:
/// own<t>) -> list<u8>`. Imports the abstract resource + `make`/`call` typed against it, re-exports the
/// resource DIRECTLY + both funcs ascribed against the exported identity — the only way to export a
/// resource-with-methods (the outer component instantiates it with the real rep-carrying resource + the
/// lifted funcs). `arg_bytes`/`result_byte` are the closure's boundary valtypes (`AbiValType::comp_byte`).
/// Inner index spaces: imported resource → type 0; `own<0>` → 1; make-ft `()->own<0>` → 2; `own<0>` → 3;
/// call-ft `(self:own<3>, args…)->R` → 4; imported `make` → func 0; imported `call` → func 1; RE-EXPORTED
/// resource → type 5; `own<5>` → 6; make-exp-ft → 7; `own<5>` → 8; call-exp-ft `(self:own<8>, args…)->R` →
/// 9. (No `list u8` type as the encode variant has — a `call`'s result is a scalar valtype inline.)
#[allow(dead_code)]
pub(super) fn resource_inner_component_closure(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    resource_inner_component_closure_borrow(make_param_bytes, arg_bytes, result_byte, false)
}

/// [`resource_inner_component_closure`] with a `call_borrow` switch. `call`'s `self` handle type (both the
/// imported type 3 and the re-exported type 8) is `borrow<t>` when TRUE, `own<t>` when FALSE — matching the
/// outer lift in [`assemble_closure_resource_borrow`]. `make` stays `own<t>` (it hands ownership out). Only
/// the two `call` handle-type items differ; the functype/index layout is identical.
pub(super) fn resource_inner_component_closure_borrow(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
) -> Vec<u8> {
    resource_inner_component_closure_borrow_tuple(
        make_param_bytes,
        arg_bytes,
        result_byte,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`resource_inner_component_closure_borrow`] with an optional FIXED-SHAPE SCALAR tuple ARGUMENT. When
/// `tuple_arg_bytes` is `Some(field_bytes)`, `call`'s single argument is a `tuple<field_bytes…>` DEFINED type
/// (the direct-call compound-arg path) instead of `arg_bytes`'s inline scalar params — so a `tuple<…>` item
/// is minted just before the `call` functype on BOTH the import and export sides, shifting the `call`
/// functype's own index by 1 (import: 4→5; export: 9→11, since the re-exported resource + make types also
/// sit between). `None` reproduces the scalar path byte-for-byte. `arg_bytes` is ignored when `Some`.
///
/// `call_arg_slots` is the N-COMPOUND-ARGS override (see [`assemble_closure_resource_borrow_tuple`]): when
/// `Some(slots)`, the slot list drives type minting + the `call` functype on both the import and export sides
/// (each tuple slot mints its own `tuple<…>` type group, in arg order), subsuming the single-tuple
/// `tuple_arg_bytes`/prefix/suffix/`tuple_shape` inputs. `None` = byte-identical to the existing paths.
#[allow(clippy::too_many_arguments)]
pub(super) fn resource_inner_component_closure_borrow_tuple(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    tuple_shape: Option<&[TupleFieldShape]>,
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    // `call`'s self handle type: a `borrow<idx>` (repeatable) or `own<idx>` (single-use) defined-type item.
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) then the imported `make` functype `(export-params…) -> own<0>` (type 2).
    let make_import_types = {
        let mut items = own_item(0);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: `own<0>`/`borrow<0>` (type 3) then — for a tuple arg — the `tuple<…>` defined type (type 4),
    // then the imported `call` functype `(self: <handle<3>>, <args>) -> R`. With a tuple arg the call
    // functype's own index shifts to 5 (the tuple sits between); the scalar path keeps it at 4.
    // The number of tuple DEFINED types each side's `call` type block mints: 0 (scalar/no tuple), 1 (a flat
    // single tuple), `nested_tuple_type_count` (a nested tuple), or `call_arg_tuple_type_count` (the N-arg slot
    // model). Every post-`call`-type index (the re-exported resource + make/call export types) shifts by this.
    let tuple_types_minted: u32 = if let Some(slots) = call_arg_slots {
        call_arg_tuple_type_count(slots)
    } else if let Some(s) = tuple_shape {
        nested_tuple_type_count(s)
    } else if tuple_arg_bytes.is_some() {
        1
    } else {
        0
    };
    let call_import_ty_idx: u32;
    let call_import_types = {
        let mut items = call_handle(0);
        let n_items: usize;
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint every tuple slot's type(s) starting at type 4 (after handle 3), in arg
            // order; the `call` functype references each by index (a scalar slot inlines its byte).
            let mut next_type = 4u32;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                3,
                slots,
                &tup_idxs,
                result_byte,
            ));
            call_import_ty_idx = next_type;
            n_items = 1 + call_arg_tuple_type_count(slots) as usize + 1;
        } else if let Some(shape) = tuple_shape {
            // NESTED tuple arg: mint the tuple types starting at type 4 (after handle 3); the OUTERMOST tuple
            // index is what the `call` functype references, and the functype sits right after all of them.
            let mut next_type = 4u32;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                3,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                result_byte,
            ));
            call_import_ty_idx = next_type;
            n_items = 1 + nested_tuple_type_count(shape) as usize + 1;
        } else if let Some(fields) = tuple_arg_bytes {
            items.extend_from_slice(&tuple_defined_type(fields)); // type 4
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                3,
                tuple_prefix_bytes,
                4,
                tuple_suffix_bytes,
                result_byte,
            )); // type 5
            call_import_ty_idx = 5;
            n_items = 3;
        } else {
            items.extend_from_slice(&closure_call_functype(3, arg_bytes, result_byte)); // type 4
            call_import_ty_idx = 4;
            n_items = 2;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(n_items, &items))
    };
    out.extend_from_slice(&call_import_types);
    // sec 10: import `import-func-call` as a func of the call functype → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-call", call_import_ty_idx)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY as `t`. Its exported type index is the next
    // free component type after the import-side `call` type block: 5 (scalar, no tuple type) + one per minted
    // tuple type (1 flat / N nested / the slot model's total) — `4 + tuple_types_minted` (the call functype) + 1.
    let exp_res_ty: u32 = 5 + tuple_types_minted;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<exp_res_ty>` then the `make` functype re-typed against the exported resource.
    let make_own_ty = exp_res_ty + 1; // 6 (scalar) / 7 (tuple)
    let make_export_ft = make_own_ty + 1; // 7 (scalar) / 8 (tuple)
    let make_export_types = {
        let mut items = own_item(exp_res_ty);
        items.extend_from_slice(&params_result_functype(
            make_param_bytes,
            &owned_valtype(make_own_ty),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to the make export functype.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, make_export_ft),
        ),
    ));
    // sec 7: `own/borrow<exp_res_ty>` then — for a tuple arg — the `tuple<…>` defined type, then the `call`
    // functype re-typed against the exported resource.
    let call_handle_ty = make_export_ft + 1; // 8 (scalar) / 9 (tuple) [+ tuple_types_minted via make_export_ft]
    let call_export_ty_idx: u32;
    let call_export_types = {
        let mut items = call_handle(exp_res_ty);
        let n_items: usize;
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint every tuple slot's type(s) right after the export-side handle, in arg
            // order; the re-typed `call` functype references each by index against the exported resource.
            let mut next_type = call_handle_ty + 1;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                call_handle_ty,
                slots,
                &tup_idxs,
                result_byte,
            ));
            call_export_ty_idx = next_type;
            n_items = 1 + call_arg_tuple_type_count(slots) as usize + 1;
        } else if let Some(shape) = tuple_shape {
            let mut next_type = call_handle_ty + 1;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                call_handle_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                result_byte,
            ));
            call_export_ty_idx = next_type;
            n_items = 1 + nested_tuple_type_count(shape) as usize + 1;
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = call_handle_ty + 1; // 10
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                call_handle_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                result_byte,
            ));
            call_export_ty_idx = tup_ty + 1; // 11
            n_items = 3;
        } else {
            items.extend_from_slice(&closure_call_functype(
                call_handle_ty,
                arg_bytes,
                result_byte,
            ));
            call_export_ty_idx = call_handle_ty + 1; // 9
            n_items = 2;
        }
        section(sec::COMPONENT_TYPE, &wasm_vec(n_items, &items))
    };
    out.extend_from_slice(&call_export_types);
    // sec 11: export `call` (func 1) ascribed to the call export functype.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(CALL_BOUNDARY_NAME, 1, call_export_ty_idx),
        ),
    ));
    out
}

/// The inner re-export component for a COMPOUND-RESULT (`Bytes`→`list<u8>`) closure: like
/// [`resource_inner_component_closure`] but `call`'s result is a `list<u8>` defined type instead of a scalar
/// byte. Each `list<u8>` type is minted on both the import and export side (independent type spaces). Type
/// indices (import side): resource 0; make `own<0>` 1, make-ft 2; call `own<0>` 3, `list<u8>` 4, call-ft 5.
/// Export side: re-exported resource 6; make `own<6>` 7, make-ft 8; call `own<6>` 9, `list<u8>` 10, call-ft
/// 11. Imported funcs: make 0, call 1.
#[allow(dead_code)]
pub(super) fn resource_inner_component_closure_bytes(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
) -> Vec<u8> {
    resource_inner_component_closure_bytes_borrow(make_param_bytes, arg_bytes, false)
}

/// [`resource_inner_component_closure_bytes`] with a `call_borrow` switch. `call`'s self handle type (the
/// imported type 3 and the re-exported type 9) is `borrow<t>` when TRUE, `own<t>` when FALSE — matching the
/// outer lift in [`assemble_closure_bytes_resource_borrow`]. `make` stays `own<t>`.
pub(super) fn resource_inner_component_closure_bytes_borrow(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    call_borrow: bool,
) -> Vec<u8> {
    resource_inner_component_closure_bytes_borrow_tuple(
        make_param_bytes,
        arg_bytes,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`resource_inner_component_closure_bytes_borrow`] with an optional fixed-shape scalar tuple ARGUMENT.
/// When `tuple_arg_bytes` is `Some`, `call`'s single arg is a native `tuple<…>` minted just before the
/// `list<u8>` result type on BOTH the import and export sides — each side's `call` type block adds one extra
/// type (own/borrow + tuple + list + functype = 4, vs the scalar-arg own + list + functype = 3), which shifts
/// the re-exported resource type index + all export-side indices up by 1. `None` = the scalar-arg path
/// (byte-identical). A running type counter keeps both shapes' indices consistent.
///
/// `call_arg_slots` is the N-COMPOUND-ARGS override (see [`assemble_closure_bytes_resource_borrow_tuple`]):
/// `Some(slots)` mints one `tuple<…>` group per tuple slot (in arg order) before the `list<u8>` on each side.
#[allow(clippy::too_many_arguments)]
pub(super) fn resource_inner_component_closure_bytes_borrow_tuple(
    make_param_bytes: &[u8],
    arg_bytes: &[u8],
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    tuple_shape: Option<&[TupleFieldShape]>,
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    // Emit the `call` type block (self handle at `block_base` wrapping `resource_ty`; for a tuple arg the
    // `tuple<…>` type(s); the `list<u8>` result type; the `(self, <prefix…>, tuple, <suffix…>) -> list<u8>`
    // functype). Returns the emitted items + the call-functype index + how many types the block added
    // (3 scalar / 4 flat-tuple / 3 + nested-tuple-count for a NESTED tuple). Prefix/suffix scalar bytes
    // surround the tuple when it sits AMONG scalar args.
    let call_type_block = |resource_ty: u32, block_base: u32| -> (Vec<u8>, u32, u32) {
        let handle_ty = block_base;
        let mut items = call_handle(resource_ty);
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint each tuple slot's type(s) after the handle (in arg order), then `list<u8>`,
            // then the slot-model list-result functype. Added = handle + all tuple types + list + functype.
            let mut next_type = block_base + 1;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_functype_slots(
                handle_ty, slots, &tup_idxs, list_ty,
            ));
            let added = 1 + call_arg_tuple_type_count(slots) + 2;
            (items, next_type, added)
        } else if let Some(shape) = tuple_shape {
            let mut next_type = block_base + 1;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            let list_ty = next_type;
            items.extend_from_slice(&list_u8_defined_type());
            next_type += 1;
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                list_ty,
            ));
            let added = 1 + nested_tuple_type_count(shape) + 2; // handle + tuple types + list + functype
            (items, next_type, added)
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = block_base + 1;
            let list_ty = block_base + 2;
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                list_ty,
            ));
            (items, list_ty + 1, 4)
        } else {
            let list_ty = block_base + 1;
            items.extend_from_slice(&list_u8_defined_type());
            items.extend_from_slice(&closure_call_list_functype(handle_ty, arg_bytes, list_ty));
            (items, list_ty + 1, 3)
        }
    };
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: `own<0>` (type 1) + imported `make` functype `(export-params…) -> own<0>` (type 2).
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: the imported `call` type block, self handle at type 3 wrapping the imported resource (type 0).
    let (call_import_items, call_import_ft, import_call_types) = call_type_block(0, 3);
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(import_call_types as usize, &call_import_items),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-call", call_import_ft)),
    ));
    // sec 11: RE-EXPORT the resource type 0 DIRECTLY as `t` → exported type R = the next free type index
    // after the import side (3 + import_call_types = 6 scalar / 7 tuple).
    let r = 3 + import_call_types;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: `own<R>` (type R+1) + `make` functype re-typed against the exported resource (type R+2).
    out.extend_from_slice(&{
        let mut items = own_item(r);
        items.extend_from_slice(&params_result_functype(
            make_param_bytes,
            &owned_valtype(r + 1),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, r + 2)),
    ));
    // sec 7: the exported `call` type block, self handle at type R+3 wrapping the exported resource (type R).
    let (call_export_items, call_export_ft, export_call_types) = call_type_block(r, r + 3);
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(export_call_types as usize, &call_export_items),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(CALL_BOUNDARY_NAME, 1, call_export_ft),
        ),
    ));
    out
}

/// The MULTI-EXPORT inner re-export component: imports the abstract resource + N `import-func-make-<i>`
/// (each `(export-params…) -> own<t>`) + one shared `import-func-call`, then re-exports the resource type
/// directly + each make under its boundary name (`makes[i].name`) + the shared `call`, all ascribed
/// against the exported resource identity. The N=1 case is byte-identical to
/// [`resource_inner_component_closure`]. Type-index layout (N = makes.len()): imported resource → type 0;
/// per make i: `own<0>` (1+2i), make functype (2+2i), imported func i; then `own<0>` (1+2N), call functype
/// (2+2N), imported func N. Exported resource → type R = 2N+3; per make i: `own<R>` (R+1+2i), make functype
/// (R+2+2i), exported func i; then `own<R>` (R+1+2N), call functype (R+2+2N), exported func N.
#[allow(dead_code)]
pub(super) fn resource_inner_component_multi_closure(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    resource_inner_component_multi_closure_borrow(makes, arg_bytes, result_byte, false)
}

/// [`resource_inner_component_multi_closure`] with a `call_borrow` switch. The shared `call`'s self handle
/// (the imported type `1+2N` and the re-exported type `R+1+2N`) is `borrow<t>` when TRUE, `own<t>` when
/// FALSE — matching the outer lift in [`assemble_mixed_closure_resource_borrow`]. `make`s stay `own<t>`.
pub(super) fn resource_inner_component_multi_closure_borrow(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
) -> Vec<u8> {
    resource_inner_component_multi_closure_borrow_tuple(
        makes,
        arg_bytes,
        result_byte,
        call_borrow,
        None,
        &[],
        &[],
        None,
        None,
    )
}

/// [`resource_inner_component_multi_closure_borrow`] with an optional FIXED-SHAPE SCALAR tuple ARGUMENT for
/// the shared `call`. When `tuple_arg_bytes` is `Some(field_bytes)`, a `tuple<field_bytes…>` DEFINED type is
/// minted just before the shared `call` functype on BOTH the import and export sides (the call functype
/// references it by index), so each side's `call` type block adds one extra type — which shifts the exported
/// resource type index `R` up by 1 (an import-side tuple type sits between). `None` reproduces the scalar
/// path byte-for-byte. Matches the outer lift in [`assemble_mixed_closure_resource_borrow_tuple`].
///
/// `call_arg_slots` is the N-COMPOUND-ARGS override: `Some(slots)` mints one `tuple<…>` group per tuple slot
/// (in arg order) before the shared `call` functype on each side, subsuming the single-tuple inputs.
#[allow(clippy::too_many_arguments)]
pub(super) fn resource_inner_component_multi_closure_borrow_tuple(
    makes: &[ClosureMakeAbi],
    arg_bytes: &[u8],
    result_byte: u8,
    call_borrow: bool,
    tuple_arg_bytes: Option<&[u8]>,
    tuple_prefix_bytes: &[u8],
    tuple_suffix_bytes: &[u8],
    tuple_shape: Option<&[TupleFieldShape]>,
    call_arg_slots: Option<&[ArgSlot]>,
) -> Vec<u8> {
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    // Emit the shared `call`'s type block: the self handle (an `own`/`borrow<resource_ty>`), then — for a
    // tuple arg — the `tuple<…>` defined type(s), then the `call` functype (referencing the self-handle type +,
    // for a tuple, the tuple type, by index). `resource_ty` is the resource the handle wraps (0 import-side,
    // R export-side); `block_base` is the type index the self-handle item lands at. Returns the emitted type
    // items + the index the CALL FUNCTYPE lands at + how many types the block added (2 scalar / 3 flat-tuple /
    // 2 + nested-count for a NESTED tuple).
    let call_type_block = |resource_ty: u32, block_base: u32| -> (Vec<u8>, u32, u32) {
        let handle_ty = block_base; // the self-handle defined type's own index
        let mut items = call_handle(resource_ty);
        if let Some(slots) = call_arg_slots {
            // N-COMPOUND-ARGS: mint each tuple slot's type(s) after the handle (in arg order), then the
            // slot-model scalar-result functype. Added = handle + all tuple types + functype.
            let mut next_type = block_base + 1;
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                handle_ty,
                slots,
                &tup_idxs,
                result_byte,
            ));
            let added = 1 + call_arg_tuple_type_count(slots) + 1;
            (items, next_type, added)
        } else if let Some(shape) = tuple_shape {
            let mut next_type = block_base + 1;
            let outer_tup = mint_tuple_type_nested(shape, &mut next_type, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                outer_tup,
                tuple_suffix_bytes,
                result_byte,
            ));
            let added = 1 + nested_tuple_type_count(shape) + 1; // handle + tuple types + functype
            (items, next_type, added)
        } else if let Some(fields) = tuple_arg_bytes {
            let tup_ty = block_base + 1; // handle at block_base, tuple next
            items.extend_from_slice(&tuple_defined_type(fields));
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                handle_ty,
                tuple_prefix_bytes,
                tup_ty,
                tuple_suffix_bytes,
                result_byte,
            ));
            (items, tup_ty + 1, 3)
        } else {
            items.extend_from_slice(&closure_call_functype(handle_ty, arg_bytes, result_byte));
            (items, block_base + 1, 2)
        }
    };
    let n = makes.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // Per make i: `own<0>` (type 1+2i) + make functype (type 2+2i); then import the func → func i.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = (1 + 2 * i) as u32;
        let ft_ty = (2 + 2 * i) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(0);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            // PRIVATE wiring name — indexed, not the user name (a user name may be non-kebab, e.g. `mkA`,
            // which wasmtime rejects as an extern name). The instantiate item pairs by this same `f<i>`.
            &wasm_vec(1, &import_func_item(&import_wire_name(i), ft_ty)),
        ));
    }
    // Shared call (import side): self handle `own<0>`/`borrow<0>` (type 1+2N) + [tuple type] + call functype;
    // import the func → N. The handle wraps the IMPORTED resource (type 0).
    let call_own_ty = (1 + 2 * n) as u32;
    let (call_import_items, call_import_ft, import_call_types) = call_type_block(0, call_own_ty);
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(import_call_types as usize, &call_import_items),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item(&import_wire_name(n), call_import_ft)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY as `t` → exported type R = the next type index
    // after all import-side types (resource + 2 per make + the call block). R = 2N+3 (scalar, call block = 2)
    // or 2N+4 (tuple, call block = 3).
    let r = 2 * n as u32 + 1 + import_call_types;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // Per make i: `own<R>` (R+1+2i) + make functype re-typed (R+2+2i); export func i under its name.
    for (i, mk) in makes.iter().enumerate() {
        let own_ty = r + (1 + 2 * i) as u32;
        let ft_ty = r + (2 + 2 * i) as u32;
        out.extend_from_slice(&{
            let mut items = own_item(r);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_func_ascribed_item(&mk.name, i as u32, ft_ty)),
        ));
    }
    // Shared call (export side): self handle `own<R>`/`borrow<R>` (R+1+2N) + [tuple type] + call functype
    // re-typed; export `call` (func N). The handle wraps the RE-EXPORTED resource (type R).
    let call_exp_own = r + (1 + 2 * n) as u32;
    let (call_export_items, call_export_ft, export_call_types) = call_type_block(r, call_exp_own);
    out.extend_from_slice(&section(
        sec::COMPONENT_TYPE,
        &wasm_vec(export_call_types as usize, &call_export_items),
    ));
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(CALL_BOUNDARY_NAME, n as u32, call_export_ft),
        ),
    ));
    out
}

/// The DISTINCT-SIGNATURE inner re-export component: imports G abstract resources (`import-type-t<g>`) +
/// each group's `make-<name>` (→ `own<t_g>`) and `call-<g>` (`(self: own<t_g>, args…) -> R`), then
/// re-exports all G resources (`t0`,`t1`,…) + every fn ascribed against its group's exported resource.
/// The only way to export G resources-with-methods together. Import-phase type layout: resources → types
/// 0..g; then per fn (flat, group order — each group's makes then its call): a make/scalar-call adds
/// `own<t_g>` + functype (2 types); a BYTE-ROPE call adds `own<t_g>` + `list<u8>` + `(…) -> list<u8>`
/// functype (3 types). Uses a running type counter (byte-rope calls break the fixed `g + 2f` formula).
/// Export phase re-exports the G resources then re-ascribes every fn against its group's exported resource.
#[allow(dead_code)]
pub(super) fn resource_inner_component_distinct_sig(groups: &[SigGroupAbi]) -> Vec<u8> {
    resource_inner_component_distinct_sig_borrow(groups, false)
}

/// [`resource_inner_component_distinct_sig`] with a `call_borrow` switch (C-HOST-6, distinct-sig per-group
/// `call-g<n>`). When TRUE each group's `call-g<n>` self handle is `borrow<t_g>` (repeatable) on both the
/// import side and the re-exported side; `make`s stay `own<t_g>`. Matches the outer lift's per-group `call`
/// typing in `assemble_distinct_sig_resource_mixed`.
pub(super) fn resource_inner_component_distinct_sig_borrow(
    groups: &[SigGroupAbi],
    call_borrow: bool,
) -> Vec<u8> {
    let call_handle = |idx: u32| -> Vec<u8> {
        if call_borrow {
            borrow_item(idx)
        } else {
            own_item(idx)
        }
    };
    let g = groups.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import G abstract resources → types 0..g.
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_subresource_item(&format!("import-type-t{gi}"))),
        ));
    }
    // IMPORT each fn (flat, group order); `ty` runs past `g` as types are minted. `f` is the func index.
    let mut ty = g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        for mk in &gr.makes {
            let own_ty = ty;
            let ft_ty = ty + 1;
            out.extend_from_slice(&{
                let mut items = own_item(gi as u32);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(own_ty),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 2;
            f += 1;
        }
        if gr.ret_is_bytes {
            // call-<gi> returns list<u8>. With a TUPLE arg: handle + tuple type(s) + list<u8> + functype;
            // without: handle + list<u8> + functype (3 types).
            let own_ty = ty;
            if let Some(slots) = &gr.call_arg_slots {
                let mut next = ty + 1;
                let mut items = call_handle(gi as u32);
                let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
                let list_ty = next;
                items.extend_from_slice(&list_u8_defined_type());
                next += 1;
                items.extend_from_slice(&closure_call_list_functype_slots(
                    own_ty, slots, &tup_idxs, list_ty,
                ));
                let ft_ty = next;
                let n_types = 1 + call_arg_tuple_type_count(slots) as usize + 2;
                out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty = ft_ty + 1;
            } else if let Some(shape) = &gr.tuple_shape {
                let mut next = ty + 1;
                let mut items = call_handle(gi as u32);
                let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
                let list_ty = next;
                items.extend_from_slice(&list_u8_defined_type());
                next += 1;
                items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                    own_ty,
                    &gr.tuple_prefix_bytes,
                    outer_tup,
                    &gr.tuple_suffix_bytes,
                    list_ty,
                ));
                let ft_ty = next;
                let n_types = 1 + nested_tuple_type_count(shape) as usize + 2; // handle + tuple + list + ft
                out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty = ft_ty + 1;
            } else if let Some(fields) = &gr.tuple_arg_bytes {
                let tup_ty = ty + 1;
                let list_ty = ty + 2;
                let ft_ty = ty + 3;
                out.extend_from_slice(&{
                    let mut items = call_handle(gi as u32);
                    items.extend_from_slice(&tuple_defined_type(fields));
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                        own_ty,
                        &gr.tuple_prefix_bytes,
                        tup_ty,
                        &gr.tuple_suffix_bytes,
                        list_ty,
                    ));
                    section(sec::COMPONENT_TYPE, &wasm_vec(4, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty += 4;
            } else {
                let list_ty = ty + 1;
                let ft_ty = ty + 2;
                out.extend_from_slice(&{
                    let mut items = call_handle(gi as u32);
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_functype(
                        own_ty,
                        &gr.arg_bytes,
                        list_ty,
                    ));
                    section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty += 3;
            }
        } else if let Some(slots) = &gr.call_arg_slots {
            // call-<gi> : (self: handle<t_gi>, N tuple/scalar slots) -> R  → handle + N tuple type(s) + ft.
            let own_ty = ty;
            let mut next = ty + 1;
            let mut items = call_handle(gi as u32);
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                own_ty,
                slots,
                &tup_idxs,
                gr.result_byte,
            ));
            let ft_ty = next;
            let n_types = 1 + call_arg_tuple_type_count(slots) as usize + 1;
            out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty = ft_ty + 1;
        } else if let Some(shape) = &gr.tuple_shape {
            // call-<gi> : (self: handle<t_gi>, p: nested tuple) -> R  → handle + nested tuple type(s) + ft.
            let own_ty = ty;
            let mut next = ty + 1;
            let mut items = call_handle(gi as u32);
            let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                own_ty,
                &gr.tuple_prefix_bytes,
                outer_tup,
                &gr.tuple_suffix_bytes,
                gr.result_byte,
            ));
            let ft_ty = next;
            let n_types = 1 + nested_tuple_type_count(shape) as usize + 1; // handle + tuple + ft
            out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty = ft_ty + 1;
        } else if let Some(fields) = &gr.tuple_arg_bytes {
            // call-<gi> : (self: handle<t_gi>, <prefix…>, p: tuple<…>, <suffix…>) -> R  → handle + tuple + ft.
            let own_ty = ty;
            let tup_ty = ty + 1;
            let ft_ty = ty + 2;
            out.extend_from_slice(&{
                let mut items = call_handle(gi as u32);
                items.extend_from_slice(&tuple_defined_type(fields));
                items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                    own_ty,
                    &gr.tuple_prefix_bytes,
                    tup_ty,
                    &gr.tuple_suffix_bytes,
                    gr.result_byte,
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 3;
        } else {
            // call-<gi> : (self: own/borrow<t_gi>, args…) -> R  → handle<t_gi> + functype.
            let own_ty = ty;
            let ft_ty = ty + 1;
            out.extend_from_slice(&{
                let mut items = call_handle(gi as u32);
                items.extend_from_slice(&closure_call_functype(
                    own_ty,
                    &gr.arg_bytes,
                    gr.result_byte,
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 2;
        }
        f += 1;
    }
    // sec 11: RE-EXPORT each resource DIRECTLY as `t<g>` → exported types E..E+g (E = the running `ty`).
    let e = ty;
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_type_direct_item(&format!("t{gi}"), gi as u32)),
        ));
    }
    // EXPORT each fn ascribed against its group's EXPORTED resource (exp type E + gi). Types after the
    // re-exports continue at E + g; a make/scalar-call adds own + functype, a byte-rope call own + list +
    // functype.
    let mut ti = e + g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        let exp_rty = e + gi as u32;
        for mk in &gr.makes {
            out.extend_from_slice(&{
                let mut items = own_item(exp_rty);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(ti),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ti + 1)),
            ));
            ti += 2;
            f += 1;
        }
        if gr.ret_is_bytes {
            // list<u8> result. With a TUPLE arg: handle + tuple type(s) + list<u8> + functype; without:
            // handle + list<u8> + functype (3 types).
            if let Some(slots) = &gr.call_arg_slots {
                let mut next = ti + 1;
                let mut items = call_handle(exp_rty);
                let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
                let list_ty = next;
                items.extend_from_slice(&list_u8_defined_type());
                next += 1;
                items.extend_from_slice(&closure_call_list_functype_slots(
                    ti, slots, &tup_idxs, list_ty,
                ));
                let ft_ty = next;
                let n_types = 1 + call_arg_tuple_type_count(slots) as usize + 2;
                out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(
                        1,
                        &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ft_ty),
                    ),
                ));
                ti = ft_ty + 1;
            } else if let Some(shape) = &gr.tuple_shape {
                let mut next = ti + 1;
                let mut items = call_handle(exp_rty);
                let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
                let list_ty = next;
                items.extend_from_slice(&list_u8_defined_type());
                next += 1;
                items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                    ti,
                    &gr.tuple_prefix_bytes,
                    outer_tup,
                    &gr.tuple_suffix_bytes,
                    list_ty,
                ));
                let ft_ty = next;
                let n_types = 1 + nested_tuple_type_count(shape) as usize + 2;
                out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(
                        1,
                        &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ft_ty),
                    ),
                ));
                ti = ft_ty + 1;
            } else if let Some(fields) = &gr.tuple_arg_bytes {
                out.extend_from_slice(&{
                    let mut items = call_handle(exp_rty);
                    items.extend_from_slice(&tuple_defined_type(fields));
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_tuple_arg_functype_interleaved(
                        ti,
                        &gr.tuple_prefix_bytes,
                        ti + 1,
                        &gr.tuple_suffix_bytes,
                        ti + 2,
                    ));
                    section(sec::COMPONENT_TYPE, &wasm_vec(4, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(
                        1,
                        &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ti + 3),
                    ),
                ));
                ti += 4;
            } else {
                out.extend_from_slice(&{
                    let mut items = call_handle(exp_rty);
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&closure_call_list_functype(ti, &gr.arg_bytes, ti + 1));
                    section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(
                        1,
                        &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ti + 2),
                    ),
                ));
                ti += 3;
            }
        } else if let Some(slots) = &gr.call_arg_slots {
            let mut next = ti + 1;
            let mut items = call_handle(exp_rty);
            let tup_idxs = mint_call_arg_tuple_types(slots, &mut next, &mut items);
            items.extend_from_slice(&closure_call_functype_slots(
                ti,
                slots,
                &tup_idxs,
                gr.result_byte,
            ));
            let ft_ty = next;
            let n_types = 1 + call_arg_tuple_type_count(slots) as usize + 1;
            out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(
                    1,
                    &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ft_ty),
                ),
            ));
            ti = ft_ty + 1;
        } else if let Some(shape) = &gr.tuple_shape {
            let mut next = ti + 1;
            let mut items = call_handle(exp_rty);
            let outer_tup = mint_tuple_type_nested(shape, &mut next, &mut items);
            items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                ti,
                &gr.tuple_prefix_bytes,
                outer_tup,
                &gr.tuple_suffix_bytes,
                gr.result_byte,
            ));
            let ft_ty = next;
            let n_types = 1 + nested_tuple_type_count(shape) as usize + 1;
            out.extend_from_slice(&section(sec::COMPONENT_TYPE, &wasm_vec(n_types, &items)));
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(
                    1,
                    &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ft_ty),
                ),
            ));
            ti = ft_ty + 1;
        } else if let Some(fields) = &gr.tuple_arg_bytes {
            out.extend_from_slice(&{
                let mut items = call_handle(exp_rty);
                items.extend_from_slice(&tuple_defined_type(fields));
                items.extend_from_slice(&closure_call_tuple_arg_functype_interleaved(
                    ti,
                    &gr.tuple_prefix_bytes,
                    ti + 1,
                    &gr.tuple_suffix_bytes,
                    gr.result_byte,
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(
                    1,
                    &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ti + 2),
                ),
            ));
            ti += 3;
        } else {
            out.extend_from_slice(&{
                let mut items = call_handle(exp_rty);
                items.extend_from_slice(&closure_call_functype(ti, &gr.arg_bytes, gr.result_byte));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(
                    1,
                    &export_func_ascribed_item(&format!("call-g{gi}"), f as u32, ti + 1),
                ),
            ));
            ti += 2;
        }
        f += 1;
    }
    out
}

/// The distinct-signature instantiate item: supply each imported resource (`import-type-t<g>` → its outer
/// resource type) + each fn (`import-func-<make>`/`import-func-call-<g>` → its lifted comp func, flat group
/// order starting at `first_fn`).
pub(super) fn component_instantiate_distinct_sig_item(
    res_type_idx: &[u32],
    first_fn: u32,
    groups: &[SigGroupAbi],
) -> Vec<u8> {
    let mut item = vec![0x00];
    uleb128(0, &mut item);
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    let mut n_args = 0usize;
    for (gi, &rty) in res_type_idx.iter().enumerate() {
        push(&format!("import-type-t{gi}"), 0x03, rty, &mut arg_items);
        n_args += 1;
    }
    // `f` is the comp-func INDEX (the arg value); `wire` is the 0-based wire NAME index (`import-func-f<n>`,
    // matching the inner component). They advance together but name/value are distinct.
    let mut f = first_fn;
    let mut wire = 0usize;
    for gr in groups.iter() {
        for _ in &gr.makes {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
        push(&import_wire_name(wire), 0x01, f, &mut arg_items);
        f += 1;
        wire += 1;
        n_args += 1;
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
    item
}

/// The DISTINCT-SIGNATURE ROUND-TRIP inner re-export component: like `resource_inner_component_distinct_sig`
/// but each group's functions are its makes (`(params…)->own<t_g>`) THEN its consumers (source-ordered
/// params via `consumer_functype`), rather than makes + one `call-g`. Imports G resources + all funcs typed
/// against their group's imported resource, then re-exports the G resources + all funcs ascribed against the
/// exported identity. Type-index layout identical to the distinct-sig one (own<t> + functype per fn, flat).
pub(super) fn resource_inner_component_distinct_sig_rt(groups: &[RtSigGroupAbi]) -> Vec<u8> {
    let g = groups.len();
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_subresource_item(&format!("import-type-t{gi}"))),
        ));
    }
    // IMPORT each fn (flat, group order: makes then consumers). A make/scalar-consumer pins own<t_g> +
    // functype (2 types); a BYTE-ROPE consumer pins own<t_g> + list<u8> + `(…)->list<u8>` (3 types). `ty`
    // runs past `g` (the G imported resources) as types are minted; `f` is the func index.
    let mut ty = g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        for mk in &gr.makes {
            let own_ty = ty;
            let ft_ty = ty + 1;
            out.extend_from_slice(&{
                let mut items = own_item(gi as u32);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(own_ty),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 2;
            f += 1;
        }
        for c in &gr.consumers {
            if c.ret_is_bytes {
                let own_ty = ty;
                let list_ty = ty + 1;
                let ft_ty = ty + 2;
                out.extend_from_slice(&{
                    let mut items = own_item(gi as u32);
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&consumer_list_functype(own_ty, &c.params, list_ty));
                    section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty += 3;
            } else {
                let own_ty = ty;
                let ft_ty = ty + 1;
                out.extend_from_slice(&{
                    let mut items = own_item(gi as u32);
                    items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
                    section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_IMPORT,
                    &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
                ));
                ty += 2;
            }
            f += 1;
        }
    }
    // RE-EXPORT G resources → exported types E..E+g (E = the running `ty`); then per fn re-ascribe against its
    // group's exported rty. Types continue at E+g; byte-rope consumers add 3, others 2.
    let e = ty;
    for gi in 0..g {
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_type_direct_item(&format!("t{gi}"), gi as u32)),
        ));
    }
    let mut ti = e + g as u32;
    let mut f = 0usize;
    for (gi, gr) in groups.iter().enumerate() {
        let exp_rty = e + gi as u32;
        for mk in &gr.makes {
            out.extend_from_slice(&{
                let mut items = own_item(exp_rty);
                items.extend_from_slice(&params_result_functype(
                    &mk.make_param_bytes,
                    &owned_valtype(ti),
                ));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ti + 1)),
            ));
            ti += 2;
            f += 1;
        }
        for c in &gr.consumers {
            if c.ret_is_bytes {
                out.extend_from_slice(&{
                    let mut items = own_item(exp_rty);
                    items.extend_from_slice(&list_u8_defined_type());
                    items.extend_from_slice(&consumer_list_functype(ti, &c.params, ti + 1));
                    section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ti + 2)),
                ));
                ti += 3;
            } else {
                out.extend_from_slice(&{
                    let mut items = own_item(exp_rty);
                    items.extend_from_slice(&consumer_functype(ti, &c.params, c.result_byte));
                    section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
                });
                out.extend_from_slice(&section(
                    sec::COMPONENT_EXPORT,
                    &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ti + 1)),
                ));
                ti += 2;
            }
            f += 1;
        }
    }
    out
}

/// The distinct-signature round-trip instantiate item: each resource + each fn (makes then consumers per
/// group), matching `resource_inner_component_distinct_sig_rt`'s import names + order.
pub(super) fn component_instantiate_distinct_sig_rt_item(
    res_type_idx: &[u32],
    first_fn: u32,
    groups: &[RtSigGroupAbi],
) -> Vec<u8> {
    let mut item = vec![0x00];
    uleb128(0, &mut item);
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    let mut n_args = 0usize;
    for (gi, &rty) in res_type_idx.iter().enumerate() {
        push(&format!("import-type-t{gi}"), 0x03, rty, &mut arg_items);
        n_args += 1;
    }
    // `f` is the comp-func INDEX (arg value); `wire` is the 0-based wire NAME index matching the inner component.
    let mut f = first_fn;
    let mut wire = 0usize;
    for gr in groups.iter() {
        for _ in &gr.makes {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
        for _ in &gr.consumers {
            push(&import_wire_name(wire), 0x01, f, &mut arg_items);
            f += 1;
            wire += 1;
            n_args += 1;
        }
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
    item
}

/// The MULTI-EXPORT-plus-CONSUMER (round-trip) inner re-export component: imports the abstract resource +
/// N `import-func-<make>` (each `(params…) -> own<t>`) + M `import-func-<consumer>` (each `(g: own<t>,
/// args…) -> R`), then re-exports the resource + all N+M funcs ascribed. A make and a consumer are both
/// "own<t>-shaped" component funcs, so they interleave uniformly here (makes first, then consumers), each
/// contributing an `own<0>` + functype pair on import and an `own<R>` + functype pair on export. Type-index
/// layout mirrors `resource_inner_component_multi_closure` with M extra funcs appended.
pub(super) fn resource_inner_component_roundtrip(
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // IMPORT each fn: make[i] `(params…) -> own<0>`, then consumer[j] `(g: own<0>, args…) -> R`. A
    // make/scalar-consumer pins `own<0>` + functype (2 types); a BYTE-ROPE consumer pins `own<0>` +
    // `list<u8>` + `(…)->list<u8>` functype (3 types). `ty` runs past 0 (the imported resource) as types are
    // minted; `f` is the func index.
    let mut ty = 1u32;
    let mut f = 0usize;
    for mk in makes {
        let own_ty = ty;
        let ft_ty = ty + 1;
        out.extend_from_slice(&{
            let mut items = own_item(0);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
        ));
        ty += 2;
        f += 1;
    }
    for c in consumers {
        if c.ret_is_bytes {
            let own_ty = ty;
            let list_ty = ty + 1;
            let ft_ty = ty + 2;
            out.extend_from_slice(&{
                let mut items = own_item(0);
                items.extend_from_slice(&list_u8_defined_type());
                items.extend_from_slice(&consumer_list_functype(own_ty, &c.params, list_ty));
                section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 3;
        } else {
            let own_ty = ty;
            let ft_ty = ty + 1;
            out.extend_from_slice(&{
                let mut items = own_item(0);
                items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_IMPORT,
                &wasm_vec(1, &import_func_item(&import_wire_name(f), ft_ty)),
            ));
            ty += 2;
        }
        f += 1;
    }
    // sec 11: RE-EXPORT the resource type 0 DIRECTLY as `t` → exported type R = the running `ty`.
    let r = ty;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // EXPORT each fn ascribed against the exported resource identity, in the same order. Types continue at
    // R+1; a make/scalar-consumer adds own + functype, a byte-rope consumer own + list + functype.
    let mut ti = r + 1;
    let mut f = 0usize;
    for mk in makes {
        let own_ty = ti;
        let ft_ty = ti + 1;
        out.extend_from_slice(&{
            let mut items = own_item(r);
            items.extend_from_slice(&params_result_functype(
                &mk.make_param_bytes,
                &owned_valtype(own_ty),
            ));
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(1, &export_func_ascribed_item(&mk.name, f as u32, ft_ty)),
        ));
        ti += 2;
        f += 1;
    }
    for c in consumers {
        if c.ret_is_bytes {
            let own_ty = ti;
            let list_ty = ti + 1;
            let ft_ty = ti + 2;
            out.extend_from_slice(&{
                let mut items = own_item(r);
                items.extend_from_slice(&list_u8_defined_type());
                items.extend_from_slice(&consumer_list_functype(own_ty, &c.params, list_ty));
                section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ft_ty)),
            ));
            ti += 3;
        } else {
            let own_ty = ti;
            let ft_ty = ti + 1;
            out.extend_from_slice(&{
                let mut items = own_item(r);
                items.extend_from_slice(&consumer_functype(own_ty, &c.params, c.result_byte));
                section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
            });
            out.extend_from_slice(&section(
                sec::COMPONENT_EXPORT,
                &wasm_vec(1, &export_func_ascribed_item(&c.name, f as u32, ft_ty)),
            ));
            ti += 2;
        }
        f += 1;
    }
    out
}

/// The round-trip instantiate item: supply the resource type + each make (`import-func-<make>` → comp func
/// `first_fn + i`) + each consumer (`import-func-<consumer>` → the following comp funcs). The inner
/// component imports under these same names, makes first then consumers.
pub(super) fn component_instantiate_roundtrip_item(
    res_ty: u32,
    first_fn: u32,
    makes: &[ClosureMakeAbi],
    consumers: &[ClosureConsumeAbi],
) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let mut arg_items = Vec::new();
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    let mut f = 0u32; // 0-based wire index; comp func = first_fn + f, name = import-func-f<f>
    for _ in makes {
        push(
            &import_wire_name(f as usize),
            0x01,
            first_fn + f,
            &mut arg_items,
        );
        f += 1;
    }
    for _ in consumers {
        push(
            &import_wire_name(f as usize),
            0x01,
            first_fn + f,
            &mut arg_items,
        );
        f += 1;
    }
    item.extend_from_slice(&wasm_vec(1 + makes.len() + consumers.len(), &arg_items));
    item
}

/// The nested RE-EXPORT component for the RUNTIME escape (R2) — like [`resource_inner_component`] but
/// `encode` takes `self: borrow<t>` (reads without consuming) instead of `own<t>`. The extra `borrow`
/// defined type shifts every type index after it by one vs the own variant. Inner index spaces:
/// imported resource → type 0; `own<0>` → 1; make-ft → 2; `borrow<0>` → 3; `list u8` → 4; encode-ft
/// `(self:borrow<0>)->list u8` → 5; imported `make` → func 0; imported `encode` → func 1; RE-EXPORTED
/// resource → type 6; `own<6>` → 7; make-exp-ft → 8; `borrow<6>` → 9, `list u8` → 10, encode-exp-ft → 11.
///
/// WARNING: NOT yet wired in: the `borrow<t>` encode is the correct R2-dtor fix (so the host keeps ownership
/// and drops → dtor fires), but it currently regresses the composed walk under wasmtime 37 with an
/// un-root-caused host-side trap in encode. Kept here (byte-layout worked out, byte-identity verified
/// against the ComponentBuilder borrow oracle) as scaffolding for the follow-up; the live path still
/// uses `own` ([[rcdzc-r1-resource-encode-linking-findings]]).
#[allow(dead_code)]
pub(super) fn resource_inner_component_borrow(make_slots: &[ArgSlot]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // Each COMPOUND make param mints a `tuple<…>` type just before EACH `make` functype (the imported one
    // AND the re-export ascription), so every later type index shifts by `s` = the compound-param count.
    // A scalar/nullary param mints nothing (`s`=0), byte-identical to before. `make_functype_items` emits,
    // for a given `own<resource_ty>` at `own_type_idx` and a running `tuple_base` type index, the minted
    // tuple types (one per compound slot, at `tuple_base..tuple_base+s`) + `own<resource_ty>` + the make
    // functype referencing each param (scalar byte or its tuple type index). `resource_ty` is the RESOURCE
    // type index (0 import side, the re-exported `E` export side).
    let s: u32 = call_arg_tuple_type_count(make_slots); // total tuple types (nesting mints >1 per compound param)
    let make_functype_items = |resource_ty: u32, own_type_idx: u32, tuple_base: u32| -> Vec<u8> {
        let mut items = Vec::new();
        let mut next = tuple_base;
        let tup_idxs = mint_call_arg_tuple_types(make_slots, &mut next, &mut items);
        items.extend_from_slice(&own_item(resource_ty));
        items.extend_from_slice(&make_functype_slots(
            make_slots,
            &tup_idxs,
            &owned_valtype(own_type_idx),
        ));
        items
    };
    // sec 10: import the abstract resource `import-type-t` (Type, SubResource bound) → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: [s tuple types at 1..1+s] `own<0>` then the imported `make` functype `(params) -> own<0>`.
    // With s compounds: tuples 1..1+s, own<0>=1+s, make-ft=2+s. Without: own<0>=1, make-ft=2.
    let own0 = 1 + s;
    let make_import_ft = 2 + s;
    let make_import_types = {
        let items = make_functype_items(0, own0, 1); // resource = imported type 0; tuples at 1..1+s
        section(sec::COMPONENT_TYPE, &wasm_vec((2 + s) as usize, &items))
    };
    out.extend_from_slice(&make_import_types);
    // sec 10: import `import-func-make` as a func of the make functype → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", make_import_ft)),
    ));
    // sec 7: `borrow<0>`, `list u8`, then the imported `encode` functype — shifted +s by the tuple type.
    let borrow0 = make_import_ft + 1; // 3 (+s)
    let list0 = borrow0 + 1; // 4 (+s)
    let enc_import_ft = list0 + 1; // 5 (+s)
    let encode_import_types = {
        let mut items = borrow_item(0);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow0, list0));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_import_types);
    // sec 10: import `import-func-encode` as a func of the encode functype → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", enc_import_ft)),
    ));
    // sec 11: RE-EXPORT the imported resource type 0 DIRECTLY under `t` → exported type E.
    let exp_rty = enc_import_ft + 1; // 6 (+s)
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: [s tuple types at E+1..E+1+s] `own<E>` then the `make` functype re-typed against the exported
    // resource — a SECOND set of tuple mints, shifting the export-side indices by another `s`.
    let exp_tuple_base = exp_rty + 1; // the export-side tuple types start here (s of them)
    let own_e = exp_rty + 1 + s; // own<E>, after the s export-side tuple types
    let make_export_ft = exp_rty + 2 + s;
    let make_export_types = {
        let items = make_functype_items(exp_rty, own_e, exp_tuple_base);
        section(sec::COMPONENT_TYPE, &wasm_vec((2 + s) as usize, &items))
    };
    out.extend_from_slice(&make_export_types);
    // sec 11: export `make` (func 0) ascribed to the exported make functype.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, make_export_ft),
        ),
    ));
    // sec 7: `borrow<E>`, `list u8`, then the `encode` functype re-typed against the exported resource.
    let borrow_e = make_export_ft + 1;
    let list_e = borrow_e + 1;
    let enc_export_ft = list_e + 1;
    let encode_export_types = {
        let mut items = borrow_item(exp_rty);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(borrow_e, list_e));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    };
    out.extend_from_slice(&encode_export_types);
    // sec 11: export `encode` (func 1) ascribed to the exported encode functype.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, enc_export_ft),
        ),
    ));
    out
}

/// The inner re-export component for make + encode + N extra SCALAR methods (VM-1/VM-2). Generalizes
/// `resource_inner_component_borrow` (N=0) and the former `_borrow_len` (N=1). Imports the abstract
/// resource + make + encode + each scalar method, then RE-EXPORTS the resource directly and re-declares
/// every method against the exported identity. BYTE-IDENTICAL to `tests::…::inner_reexport_component_*`
/// (the ComponentBuilder reference). Component defined-type progression, in emission order:
///   IMPORTS: own<0> 1, make-ft 2, borrow<0> 3, list 4, encode-ft 5, then per method i: borrow<0>
///            (6+2i), method-ft (7+2i). So after M methods, next type = 6 + 2M.
///   RE-EXPORT `t` → type `E` = 6 + 2M.
///   EXPORT re-decls: own<E> (E+1), make-ft (E+2), borrow<E> (E+3), list (E+4), encode-ft (E+5), then per
///            method i: borrow<E> (E+6+2i), method-ft (E+7+2i).
/// Funcs: make 0, encode 1, method i = 2+i.
pub(super) fn resource_inner_component_scalar_methods(
    make_param_bytes: &[u8],
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let m = methods.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(COMPONENT_MAGIC);
    // sec 10: import the abstract resource → type 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_subresource_item("import-type-t")),
    ));
    // sec 7: own<0> (type 1) + make functype `(make-params…) -> own<0>` (type 2).
    out.extend_from_slice(&{
        let mut items = own_item(0);
        items.extend_from_slice(&params_result_functype(make_param_bytes, &owned_valtype(1)));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 10: import `import-func-make` : type 2 → func 0.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-make", 2)),
    ));
    // sec 7: borrow<0> (type 3), list u8 (type 4), encode functype (type 5).
    out.extend_from_slice(&{
        let mut items = borrow_item(0);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(3, 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    // sec 10: import `import-func-encode` : type 5 → func 1.
    out.extend_from_slice(&section(
        sec::COMPONENT_IMPORT,
        &wasm_vec(1, &import_func_item("import-func-encode", 5)),
    ));
    // Per method i (IMPORT side): borrow<0> (type 6+2i) + method functype (type 7+2i), then import
    // `import-func-<name>` : that functype → func 2+i. A `list<u8>` result reuses the encode `list u8`
    // type 4 (identity-free); a scalar uses its primitive byte.
    for (i, meth) in methods.iter().enumerate() {
        let bt = 6 + 2 * i as u32;
        let ft = match meth.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(bt, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(bt, 4),
        };
        out.extend_from_slice(&{
            let mut items = borrow_item(0);
            items.extend_from_slice(&ft);
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_IMPORT,
            &wasm_vec(
                1,
                &import_func_item(&format!("import-func-{}", meth.boundary_name), bt + 1),
            ),
        ));
    }
    // sec 11: RE-EXPORT the resource directly as `t` → exported type E = 6 + 2M.
    let e = 6 + 2 * m;
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_type_direct_item(RESOURCE_TYPE_NAME, 0)),
    ));
    // sec 7: own<E> (type E+1) + make functype re-typed (type E+2), same forwarded params.
    out.extend_from_slice(&{
        let mut items = own_item(e);
        items.extend_from_slice(&params_result_functype(
            make_param_bytes,
            &owned_valtype(e + 1),
        ));
        section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
    });
    // sec 11: export `make` (func 0) ascribed to functype E+2.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(1, &export_func_ascribed_item(MAKE_BOUNDARY_NAME, 0, e + 2)),
    ));
    // sec 7: borrow<E> (type E+3), list u8 (type E+4), encode functype re-typed (type E+5).
    out.extend_from_slice(&{
        let mut items = borrow_item(e);
        items.extend_from_slice(&list_u8_defined_type());
        items.extend_from_slice(&self_borrow_to_list_functype(e + 3, e + 4));
        section(sec::COMPONENT_TYPE, &wasm_vec(3, &items))
    });
    // sec 11: export `encode` (func 1) ascribed to functype E+5.
    out.extend_from_slice(&section(
        sec::COMPONENT_EXPORT,
        &wasm_vec(
            1,
            &export_func_ascribed_item(ENCODE_BOUNDARY_NAME, 1, e + 5),
        ),
    ));
    // Per method i (EXPORT side): borrow<E> (type E+6+2i) + method functype re-typed (type E+7+2i), then
    // export `<name>` (func 2+i) ascribed to that functype. A `list<u8>` result reuses the export-side
    // `list u8` type E+4.
    for (i, meth) in methods.iter().enumerate() {
        let bt = e + 6 + 2 * i as u32;
        let ft = match meth.result {
            MethodResult::Scalar(prim) => self_borrow_to_scalar_functype(bt, prim),
            MethodResult::ListU8 => self_borrow_to_list_functype(bt, e + 4),
        };
        out.extend_from_slice(&{
            let mut items = borrow_item(e);
            items.extend_from_slice(&ft);
            section(sec::COMPONENT_TYPE, &wasm_vec(2, &items))
        });
        out.extend_from_slice(&section(
            sec::COMPONENT_EXPORT,
            &wasm_vec(
                1,
                &export_func_ascribed_item(meth.boundary_name, 2 + i as u32, bt + 1),
            ),
        ));
    }
    out
}
