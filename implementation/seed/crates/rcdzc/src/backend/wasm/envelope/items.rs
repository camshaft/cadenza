//! `envelope::items` — the per-item and functype byte-grammar builders for the component-model
//! envelope. Split out of `envelope/mod.rs` to keep each source file under the 512 KiB limit; these
//! are the small `-> Vec<u8>` helpers (component/core section wrappers, alias/canon-lift/lower items,
//! defined-type and functype encoders) that the `assemble*` entry points in the parent module compose.
//! Behavior is unchanged by the move — the byte grammars are identical and remain pinned by the
//! parent module's byte-identity oracle tests.

use super::*;

/// The sec-4 nested-component bytes: `<id> <byte-length> <component>` — like [`core_module_section`] but
/// for a whole embedded component (its own magic + sections travel as a raw blob).
pub(super) fn component_section(component: &[u8]) -> Vec<u8> {
    let mut out = vec![sec::COMPONENT];
    out.extend_from_slice(&uleb_bytes(component.len() as u64));
    out.extend_from_slice(component);
    out
}

/// A core-instance INSTANTIATE item: `00 <module-idx> <args-vec>`, each arg `<name> 0x12 <core-instance>`
/// (`0x12` = the ModuleArg::Instance / core-instance sort). Instantiate-arg names are BARE (no `0x00`
/// extern-name prefix), unlike component imports/exports.
pub(super) fn core_instantiate_item(module_idx: u32, args: &[(&str, u32)]) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(module_idx as u64, &mut item);
    let mut arg_items = Vec::new();
    for (name, core_instance) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(0x12); // ModuleArg::Instance (core-instance sort)
        uleb128(*core_instance as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// A core-instance EXPORT-ITEMS item: `01 <exports-vec>`, each export `<name> 0x00 <core-func>` (`0x00`
/// = ExportKind::Func). Forms an inline core instance from already-defined core funcs (here the lowered
/// `resource.new`, bound as the `heap` instance the program module imports).
pub(super) fn core_export_instance_item(exports: &[(&str, u32)]) -> Vec<u8> {
    let mut item = vec![0x01]; // export-items form
    let mut export_items = Vec::new();
    for (name, core_func) in exports {
        export_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        export_items.extend_from_slice(name.as_bytes());
        export_items.push(wasm_abi::EXPORT_KIND_FUNC);
        uleb128(*core_func as u64, &mut export_items);
    }
    item.extend_from_slice(&wasm_vec(exports.len(), &export_items));
    item
}

/// A sec-7 RESOURCE-type item: `3f 7f 01 <dtor-core-func>` — resource-def tag `0x3f`, rep `i32`
/// (`CORE_I32`), has-dtor flag `0x01`, then the dtor core-func index. The `0x3f`/`0x01` are
/// component-model structural bytes `wasm-encoder` does not expose as constants; pinned by the R1
/// byte-identity oracle.
pub(super) fn resource_type_item(dtor_core_func: u32) -> Vec<u8> {
    let mut item = vec![0x3f, wasm_abi::CORE_I32, 0x01];
    uleb128(dtor_core_func as u64, &mut item);
    item
}

/// A sec-8 canon `resource.new` item: `02 <resource-type>` — canon tag `0x02` (resource.new) then the
/// resource type index; lowers the constructor intrinsic to a core func (the guest calls it in `make` to
/// register a rep → an export-table handle).
pub(super) fn resource_new_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x02];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A sec-8 canon `resource.rep` item: `04 <resource-type>` — canon tag `0x04` (resource.rep) then the
/// resource type index; lowers the rep-recovery intrinsic to a core func. `encode` calls it to turn the
/// resource-table HANDLE the canonical ABI hands it (an `own<t>` param crosses as a table index, NOT the
/// heap rep) back into the i32 heap rep the guest registered via `resource.new` — then walks that rep
/// ([[rcdzc-r1-resource-encode-linking-findings]] R2: without `resource.rep`, `arr-get` traps on the
/// small handle index, which `is_immediate` misreads as an inline value).
pub(super) fn resource_rep_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x04];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A sec-7 `own<resource>` defined-type item: `69 <resource-type>` — the own-handle tag `0x69` then the
/// resource type index. A functype references a resource ONLY through an `own`/`borrow` handle, never the
/// resource type directly.
pub(super) fn own_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x69];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A component VALTYPE referencing a defined type (an `own<…>` handle here) by index — encoded as the
/// bare type-index uleb (distinct from a primitive, which is its own negative-space byte).
pub(super) fn owned_valtype(type_idx: u32) -> Vec<u8> {
    uleb_bytes(type_idx as u64)
}

/// A component functype with NO params and ONE result: `40 00 00 <result-valtype>` — functype form,
/// empty param vec, result-form `0x00` (one result), then the result valtype bytes. Used for `make : ()
/// -> own<t>`.
pub(super) fn nullary_result_functype(result_valtype: &[u8]) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x00, 0x00];
    item.extend_from_slice(result_valtype);
    item
}

/// A component functype `(p0: <vt>, …) -> <result>` — form, a param vec of the given scalar valtype bytes
/// (named `p0`, `p1`, …), result-form `0x00` (one result), then the result valtype bytes. Used for a
/// PARAMETERIZED closure export's `make(export-params…) -> own<t>` (C-HOST-2); an empty `param_bytes`
/// reduces to the nullary shape. The result may be a DEFINED type (an `own<t>` handle) referenced by index
/// — pass its `owned_valtype(idx)` bytes.
pub(super) fn params_result_functype(param_bytes: &[u8], result_valtype: &[u8]) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut params = Vec::new();
    for (i, &vt) in param_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        params.extend_from_slice(&uleb_bytes(pname.len() as u64));
        params.extend_from_slice(pname.as_bytes());
        params.push(vt);
    }
    item.extend_from_slice(&wasm_vec(param_bytes.len(), &params));
    item.push(0x00); // one result
    item.extend_from_slice(result_valtype);
    item
}

/// A resource-`make` component functype `(params…) -> result` over a per-parameter [`ArgSlot`] list: a
/// SCALAR slot is an inline primitive byte, a TUPLE slot references its minted `tuple<…>` type index (from
/// `tuple_type_idxs`, positionally). No `self` receiver (unlike the closure `call` slot functype). An
/// empty slot list is the nullary `() -> result`, byte-identical to [`params_result_functype`] over `&[]`.
pub(super) fn make_functype_slots(
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    result_valtype: &[u8],
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            // A String mem-leaf param is the inline `string` primitive (mints no defined type).
            (ArgSlot::MemLeaf { is_string: true }, _) => param_items.push(wasm_abi::COMP_STRING),
            // A Bytes/list/value-form mem-leaf param references its minted `list<u8>` defined type by index.
            (ArgSlot::MemLeaf { is_string: false }, Some(idx)) => {
                param_items.extend_from_slice(&owned_valtype(*idx))
            }
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _)
                | ArgSlot::MemLeaf { is_string: false },
                None,
            ) => {
                unreachable!(
                    "a Tuple/Option/list-mem-leaf make param must carry a minted defined-type index"
                )
            }
        }
    }
    item.extend_from_slice(&wasm_vec(slots.len(), &param_items));
    item.push(0x00); // one result
    item.extend_from_slice(result_valtype);
    item
}

/// A component functype `(self: own<t>) -> list<u8>` — form, one param named `self` of type
/// `own<own_type_idx>`, one `list<u8>` result. Used by the CONSTANT escape (R1), whose resource carries
/// no live heap handle, so consuming it in `encode` leaks nothing.
pub(super) fn self_own_to_list_functype(own_type_idx: u32, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(own_type_idx));
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A sec-7 `borrow<resource>` defined-type item: `68 <resource-type>` — the borrow-handle tag `0x68`
/// then the resource type index. `encode` takes a BORROW (reads self without consuming), so the caller
/// keeps ownership and drops the handle afterward — which fires the dtor. (An `own` self would move the
/// handle into `encode`, which then leaks it.)
#[allow(dead_code)]
pub(super) fn borrow_item(resource_type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x68];
    uleb128(resource_type_idx as u64, &mut item);
    item
}

/// A component functype `(self: borrow<t>) -> list<u8>`: `40 01 <"self"> <borrow-valtype> 00 <list-type>`
/// — form, one param named `self` of type `borrow<borrow_type_idx>` (a DEFINED type by index),
/// result-form `0x00` (one result), the list defined-type index. Used for `encode`. `borrow_type_idx` is
/// the component-type index of the `borrow<t>` defined type (laid just before the functype).
#[allow(dead_code)]
pub(super) fn self_borrow_to_list_functype(borrow_type_idx: u32, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(borrow_type_idx));
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A component functype `(self: borrow<t>) -> <scalar prim>`: `40 01 <"self"> <borrow-valtype> 00
/// <prim-byte>` — like [`self_borrow_to_list_functype`] but the result is a PRIMITIVE valtype (its
/// negative-space byte, e.g. `COMP_U32`), not a defined-type index. Used for a scalar-result value-resource
/// method such as `len : borrow<t> -> u32` (`bytes-len`/`vec-len` over the borrow rep) — a method that
/// needs NO Memory/Realloc canon options (nothing crosses through linear memory). `borrow_type_idx` is the
/// component-type index of the `borrow<t>` defined type laid just before the functype.
/// (`#[allow(dead_code)]`: wired into the value-resource envelope in the `len`-method increment; the
/// byte-shape test below already exercises it, mirroring how `borrow_item` was staged before its use.)
#[allow(dead_code)]
pub(super) fn self_borrow_to_scalar_functype(borrow_type_idx: u32, result_prim: u8) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM, 0x01];
    item.extend_from_slice(&uleb_bytes("self".len() as u64));
    item.extend_from_slice(b"self");
    item.extend_from_slice(&owned_valtype(borrow_type_idx));
    item.push(0x00); // result form: one result
    item.push(result_prim); // a primitive valtype byte (not a type index)
    item
}

/// A component functype for a CLOSURE-RESOURCE `call` method: `(self: <handle<t>>, p0: <vt>, …) -> <vt>`
/// — form `0x40`, then the param vec `[self : own/borrow<self_type_idx>, p0.., …]`, then the result form.
/// `self` is the receiver (an `own`/`borrow` handle to the closure resource — a DEFINED type by index);
/// the remaining params are the closure's argument valtypes and the result is its return valtype, both
/// scalar `AbiValType::comp_byte`s (the aliased boundary widths). This is the closure analog of
/// `self_borrow_to_list_functype` — a method whose body does `resource.rep(self)` then a `call_indirect`
/// on the recovered cell (C-HOST-1). `arg_bytes`/`result_byte` are `AbiValType::comp_byte()` values.
/// (C-HOST-0: emitted + oracle-checked; not yet wired into an assembled component.)
#[allow(dead_code)]
pub(super) fn closure_call_functype(
    self_handle_type_idx: u32,
    arg_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    // The closure's arguments, named `p0`, `p1`, … (positional at a boundary call; the names are cosmetic).
    for (i, &vt) in arg_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(1 + arg_bytes.len(), &param_items));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// The `call` functype for a `Unit` (zero-result) closure result: `(self: own/borrow<t>, args…)` with NO
/// result (task_968). Identical to [`closure_call_functype`] but the component-model result list is the
/// EMPTY named-results form `0x01 0x00` (zero results) instead of the single-unnamed-result `0x00 <byte>`,
/// matching the core `call`'s empty result vector. The host calls it for its (absent) side effect.
pub(super) fn closure_call_zero_result_functype(
    self_handle_type_idx: u32,
    arg_bytes: &[u8],
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (i, &vt) in arg_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(1 + arg_bytes.len(), &param_items));
    // Zero results — the named-results form with an empty vec (component-model `resultlist` case `0x01`).
    item.extend_from_slice(&[0x01, 0x00]);
    item
}

/// The `call` functype for a COMPOUND-RESULT closure: `(self: own<t>, args…) -> list<u8>` — like
/// [`closure_call_functype`] but the result references the `list<u8>` DEFINED type by index (not an inline
/// scalar byte). `self_handle_type_idx` is the `own<t>` defined type; `list_type_idx` the `list<u8>` type
/// laid just before this functype. Its lift carries Memory/Realloc (the caller uses `canon_lift_list_item`).
pub(super) fn closure_call_list_functype(
    self_handle_type_idx: u32,
    arg_bytes: &[u8],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (i, &vt) in arg_bytes.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(1 + arg_bytes.len(), &param_items));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A round-trip CONSUMER's component functype: its params in SOURCE ORDER (each an `own<t>` closure handle
/// or a scalar byte) → `result_byte`. Unlike [`closure_call_functype`] (which hardcodes `own<t>` FIRST +
/// scalar args), this follows the actual param order — so a closure param may sit anywhere, and there may
/// be several (all `own<t>` of the same resource). `own_ty` is the `own<t>` defined-type index every
/// closure param references. Params named `p0`,`p1`,… (positional; names cosmetic).
pub(super) fn consumer_functype(
    own_ty: u32,
    params: &[ConsumeParamAbi],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        match p {
            ConsumeParamAbi::Closure => param_items.extend_from_slice(&owned_valtype(own_ty)),
            ConsumeParamAbi::Scalar(vt) => param_items.push(*vt),
        }
    }
    item.extend_from_slice(&wasm_vec(params.len(), &param_items));
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// A round-trip CONSUMER's component functype whose RESULT is a byte-rope `list<u8>` (the compound-result
/// consumer). Identical param handling to [`consumer_functype`] — params in SOURCE ORDER, each an `own<t>`
/// closure handle or a scalar byte — but the single result is the `list<u8>` defined type at `list_type_idx`
/// (referenced by index) rather than an inline scalar primitive byte.
pub(super) fn consumer_list_functype(
    own_ty: u32,
    params: &[ConsumeParamAbi],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, p) in params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        match p {
            ConsumeParamAbi::Closure => param_items.extend_from_slice(&owned_valtype(own_ty)),
            ConsumeParamAbi::Scalar(vt) => param_items.push(*vt),
        }
    }
    item.extend_from_slice(&wasm_vec(params.len(), &param_items));
    item.push(0x00); // result form: one result
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A sec-10 component-import item for an abstract RESOURCE: `<extern-name> 03 01` — `0x03` =
/// ComponentTypeRef::Type, `0x01` = TypeBounds::SubResource (mint a fresh abstract resource the importer
/// binds).
pub(super) fn import_subresource_item(name: &str) -> Vec<u8> {
    let mut item = extern_name(name);
    item.push(0x03); // ComponentTypeRef::Type
    item.push(0x01); // TypeBounds::SubResource
    item
}

/// The PRIVATE wiring name for the `f`-th function an inner re-export component imports from the outer
/// envelope. Indexed (`import-func-f0`, `f1`, …) rather than the user export name, because a user name may
/// not be valid kebab-case (e.g. `mkA` — wasmtime rejects a non-kebab extern name at parse time), whereas
/// this internal name is always kebab. The instantiate item pairs its args by this same `f<i>` sequence,
/// so the wiring stays consistent. The HOST-facing EXPORT names still use the user names (component export
/// extern names are unrestricted); only these internal imports are indexed.
pub(super) fn import_wire_name(f: usize) -> String {
    format!("import-func-f{f}")
}

/// A sec-10 component-import item for a FUNC: `<extern-name> 01 <type-idx>` — `0x01` =
/// ComponentTypeRef::Func, then the functype index.
pub(super) fn import_func_item(name: &str, type_idx: u32) -> Vec<u8> {
    let mut item = extern_name(name);
    item.push(0x01); // ComponentTypeRef::Func
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-11 export item RE-EXPORTING a TYPE directly: `00 <name> 03 <type-idx> 00` — extern-name, sort
/// type `0x03`, the type index, no outer-type ascription (`0x00`). A direct re-export publishes the
/// imported resource's identity unchanged; an ascription would mint a fresh, incompatible identity.
pub(super) fn export_type_direct_item(name: &str, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x03); // sort: type
    uleb128(type_idx as u64, &mut item);
    item.push(0x00); // no outer-type ascription
    item
}

/// A sec-11 export item for a FUNC WITH an outer-type ascription: `00 <name> 01 <func-idx> 01 01
/// <type-idx>` — extern-name, sort func `0x01`, the func index, ascription-present `0x01`, then a
/// ComponentTypeRef::Func (`0x01`) + the functype index. The ascription re-types the imported func
/// against the EXPORTED resource identity.
pub(super) fn export_func_ascribed_item(name: &str, func_idx: u32, type_idx: u32) -> Vec<u8> {
    // This is a PUBLIC component-boundary export name — it MUST be kebab-case (wasmtime rejects a
    // non-kebab extern name). A closure export named from source (`make-<src>`, a consumer's own name)
    // may carry uppercase/underscore (`mkA`, `my_func`); normalize it the same way `comp_export_item`
    // does for a bare scalar export. Already-kebab names (`make`, `call`, `call-g0`, `make-adder`) are
    // the identity, so the byte layout of every existing corpus case is unchanged. The runner resolves
    // a source-derived name through the SAME `kebab_extern_name` rule, so both sides agree.
    let name = crate::backend::common::export_name::kebab_extern_name(name);
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x01); // sort: component func
    uleb128(func_idx as u64, &mut item);
    item.push(0x01); // outer-type ascription present
    item.push(0x01); // ComponentTypeRef::Func
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-5 component-INSTANTIATE item wiring the resource re-export component's imports: `00 <component>
/// <args-vec>` with the three args — `import-type-t` = the internal resource (comp type `res_ty`),
/// `import-func-make` = the lifted `make` (comp func `make_fn`), `import-func-encode` = the lifted
/// `encode` (comp func `encode_fn`). The constant escape wires `(0, 0, 1)` (no ops precede); the runtime
/// escape wires `(1, k, k+1)` (the import-instance-type is comp type 0 + the `k` aliased ops precede the
/// lifts). Instantiate-arg names are BARE (no `0x00` prefix); the sort byte is `0x03` (type) / `0x01`
/// (func).
pub(super) fn component_instantiate_item(res_ty: u32, make_fn: u32, encode_fn: u32) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (always component 0 in both shapes)
    let args: [(&str, u8, u32); 3] = [
        ("import-type-t", 0x03, res_ty), // Type → internal resource comp type
        ("import-func-make", 0x01, make_fn), // Func → lifted make comp func
        ("import-func-encode", 0x01, encode_fn), // Func → lifted encode comp func
    ];
    let mut arg_items = Vec::new();
    for (name, sort, idx) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(sort);
        uleb128(idx as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// Like [`component_instantiate_item`] but for the value-resource-with-scalar-methods inner component
/// (VM-1/VM-2): the three fixed args (`import-type-t`=`res_ty`, `import-func-make`=`first_fn`,
/// `import-func-encode`=`first_fn+1`) plus one `import-func-<name>` per scalar method (comp func
/// `first_fn+2+i`, in method order). The inner component ([`resource_inner_component_scalar_methods`])
/// imports under these same names.
pub(super) fn component_instantiate_scalar_methods_item(
    res_ty: u32,
    first_fn: u32,
    methods: &[ScalarMethod],
) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let mut arg_items = Vec::new();
    let mut n_args = 0usize;
    let push = |name: &str, sort: u8, idx: u32, out: &mut Vec<u8>| {
        out.extend_from_slice(&uleb_bytes(name.len() as u64));
        out.extend_from_slice(name.as_bytes());
        out.push(sort);
        uleb128(idx as u64, out);
    };
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    push("import-func-make", 0x01, first_fn, &mut arg_items);
    push("import-func-encode", 0x01, first_fn + 1, &mut arg_items);
    n_args += 3;
    for (i, meth) in methods.iter().enumerate() {
        push(
            &format!("import-func-{}", meth.boundary_name),
            0x01,
            first_fn + 2 + i as u32,
            &mut arg_items,
        );
        n_args += 1;
    }
    item.extend_from_slice(&wasm_vec(n_args, &arg_items));
    item
}

/// Like [`component_instantiate_item`] but for the CLOSURE inner component: the second imported func is
/// `import-func-call` (the `call` method), not `import-func-encode`.
pub(super) fn component_instantiate_call_item(res_ty: u32, make_fn: u32, call_fn: u32) -> Vec<u8> {
    let mut item = vec![0x00]; // instantiate form
    uleb128(0, &mut item); // inner component index (component 0)
    let args: [(&str, u8, u32); 3] = [
        ("import-type-t", 0x03, res_ty),
        ("import-func-make", 0x01, make_fn),
        ("import-func-call", 0x01, call_fn),
    ];
    let mut arg_items = Vec::new();
    for (name, sort, idx) in args {
        arg_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        arg_items.extend_from_slice(name.as_bytes());
        arg_items.push(sort);
        uleb128(idx as u64, &mut arg_items);
    }
    item.extend_from_slice(&wasm_vec(args.len(), &arg_items));
    item
}

/// The MULTI-EXPORT instantiate item: supply the resource type + N make funcs (`import-func-make-<i>` →
/// comp func `first_make_fn + i`) + the shared `call` (`import-func-call` → comp func `first_make_fn +
/// N`). The inner component ([`resource_inner_component_multi_closure`]) imports under these same names.
pub(super) fn component_instantiate_multi_call_item(
    res_ty: u32,
    first_make_fn: u32,
    nmk: usize,
    makes: &[ClosureMakeAbi],
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
    let _ = makes;
    push("import-type-t", 0x03, res_ty, &mut arg_items);
    for i in 0..nmk {
        push(
            &import_wire_name(i),
            0x01,
            first_make_fn + i as u32,
            &mut arg_items,
        );
    }
    push(
        &import_wire_name(nmk),
        0x01,
        first_make_fn + nmk as u32,
        &mut arg_items,
    );
    item.extend_from_slice(&wasm_vec(1 + nmk + 1, &arg_items));
    item
}

/// A sec-11 export item for an INSTANCE: `00 <name> 05 <instance-idx> 00` — extern-name, sort
/// component-instance `0x05`, the instance index, no type ascription. Publishes the instantiated
/// re-export component as the well-known `cadenza:run/run` interface.
pub(super) fn export_instance_item(name: &str, instance_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item.push(0x05); // sort: component instance
    uleb128(instance_idx as u64, &mut item);
    item.push(0x00); // no type ascription
    item
}

/// The sec-1 embedded-core-module bytes: `<id> <byte-length> <core>` (the module is a raw blob, not a
/// wasm_vec of items).
pub(super) fn core_module_section(core: &[u8]) -> Vec<u8> {
    let mut out = vec![sec::CORE_MODULE];
    out.extend_from_slice(&uleb_bytes(core.len() as u64));
    out.extend_from_slice(core);
    out
}

/// A component-model extern name (import name / instance-type export name): a `0x00` prefix, then the
/// length-prefixed UTF-8 bytes.
pub(super) fn extern_name(name: &str) -> Vec<u8> {
    let mut out = vec![0x00];
    out.extend_from_slice(&uleb_bytes(name.len() as u64));
    out.extend_from_slice(name.as_bytes());
    out
}

/// A component functype `0x40 <params-vec> <result-form>` for a runtime OP, using its COMPONENT
/// valtype bytes (`AbiValType::comp_byte` — a `u32` handle is `0x79`, distinct from its core i32).
/// Params are NAMED at the boundary (a positional call ignores the name → synthesized `p0`, `p1`, …).
pub(super) fn op_comp_functype(op: &RtOp) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    for (i, ty) in op.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(ty.comp_byte());
    }
    item.extend_from_slice(&wasm_vec(op.params.len(), &param_items));
    match op.result {
        Some(ty) => item.extend_from_slice(&[0x00, ty.comp_byte()]),
        None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// The runtime-import INSTANCE TYPE (component type form `0x42`) for a set of runtime ops: per op, a `ty`
/// decl (`0x01` + the op's component functype) then an `export` decl (`0x04` + the op's extern name +
/// sort `0x01` + the op's component-func index `i`), all `2*k` decls wrapped as one instance type. This is
/// the shape every `assemble_*` variant open-codes to build the imported runtime interface's component
/// type 0 — factored here (the runtime-op twin of [`host_effect_instance_type`]) so the ~20 assemblers
/// share one source of truth for the import shape. Byte-identical to the inlined loop; callers that
/// COMPOSE it with prepended defined types (a host-fused list/option type) build their decls directly
/// rather than calling this.
pub(super) fn runtime_op_instance_type(imports: &[&RtOp]) -> Vec<u8> {
    // bytes-new/bytes-read carry a `list<u8>` (arg / result) the scalar op model cannot express, so when
    // either is imported PREPEND a shared `(list u8)` defined type at instance-type index 0 (mirrors
    // `host_effect_instance_type`'s `needs_list` prepend); their comp_functypes reference it by index. The
    // per-op func types then occupy indices `base..`, so each export decl references `base + i`. A pure
    // scalar import set prepends nothing (`base = 0`) → BYTE-IDENTICAL to the pre-bulk-bytes emit.
    let needs_list = imports
        .iter()
        .any(|o| o.name == "bytes-new" || o.name == "bytes-read");
    let mut decls = Vec::new();
    let mut prepended: u64 = 0;
    if needs_list {
        decls.push(0x01); // ty decl
        decls.extend_from_slice(&list_u8_defined_type()); // (list u8) → instance-type index 0
        prepended = 1;
    }
    for (i, op) in imports.iter().enumerate() {
        decls.push(0x01); // ty decl
        decls.extend_from_slice(&op_comp_functype_maybe_list(op, 0));
        decls.push(0x04); // export decl
        decls.extend_from_slice(&extern_name(op.name));
        decls.push(0x01); // sort: component func
        uleb128(prepended + i as u64, &mut decls);
    }
    let decl_count = prepended as usize + 2 * imports.len();
    let mut it = vec![0x42]; // instance type form
    it.extend_from_slice(&wasm_vec(decl_count, &decls));
    it
}

/// A runtime op's component functype, list-aware for the two bulk-bytes ops. `bytes-new`/`bytes-read` carry
/// a `list<u8>` the scalar [`op_comp_functype`] cannot express (their `RtOp` shape drops it); here the
/// `list<u8>` param/result references the shared `(list u8)` defined type at `list_idx`:
/// `bytes-new: (data: list<u8>) -> u32`, `bytes-read: (buf: u32) -> list<u8>`. Every other op falls through
/// to the plain scalar `op_comp_functype`. Param names are synthesized (`p0`) — a positional import ignores
/// them, matching `op_comp_functype`.
pub(super) fn op_comp_functype_maybe_list(op: &RtOp, list_idx: u64) -> Vec<u8> {
    use crate::backend::wasm::runtime_abi::AbiValType;
    match op.name {
        "bytes-new" => {
            // (data: list<u8>) -> u32 (handle)
            let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
            let mut params = Vec::new();
            params.extend_from_slice(&uleb_bytes(2));
            params.extend_from_slice(b"p0");
            uleb128(list_idx, &mut params); // param valtype = (list u8) defined-type index
            item.extend_from_slice(&wasm_vec(1, &params));
            item.push(0x00); // one result
            item.push(AbiValType::U32.comp_byte());
            item
        }
        "bytes-read" => {
            // (buf: u32) -> list<u8>
            let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
            let mut params = Vec::new();
            params.extend_from_slice(&uleb_bytes(2));
            params.extend_from_slice(b"p0");
            params.push(AbiValType::U32.comp_byte()); // param valtype = u32 (buf handle)
            item.extend_from_slice(&wasm_vec(1, &params));
            item.push(0x00); // one result
            uleb128(list_idx, &mut item); // -> (list u8) defined-type index
            item
        }
        _ => op_comp_functype(op),
    }
}

/// A sec-7 component functype item for a BOUNDARY export: `<func:0x40> <params-vec> <result-form>`. The
/// params vec is `<count> (<name> <valtype>)*` — each parameter NAMED (synthesized `p0`, `p1`, …). The
/// result form is `00 <valtype>` for one result (a primitive's own byte, or a DEFINED type by index for
/// a `list<u8>`), `01 00` for none. `list_type_idx` is the component-type index of the shared `list u8`
/// defined type, referenced when the result is [`BoundaryResult::Bytes`].
///
/// This is a PLAIN `params → result` function type: nothing beyond the input and output — no resume
/// parameter, no suspension/trap arm on the result. A trap is the wasm-level out-of-band halt the
/// embedder observes, not a variant the result declares; how a host suspends/resumes a host call is its
/// own policy the ABI does not represent. The params and result each carry a boundary valtype fixed by
/// this contract (`BoundaryExport` built from `export_result`/param selection), lowered/lifted by the
/// same canonical-ABI convention any boundary value uses.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The entry's exported signature MUST be a plain function from the program's input type to its result type, carrying no additional outcome arm — no suspension outcome and no injected trap outcome — so that a run either returns its result value or halts out-of-band, and the interface declares nothing beyond `input -> output`.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The entry MUST NOT carry a resume parameter and its result MUST NOT encode a pending host call or a position in the program's execution, so that how a host call suspends and resumes is host runtime policy the ABI does not represent (capabilities-and-effects.md §A Host Call Returns A Response) and the same emitted bytes serve a host that answers inline, one that suspends a fiber and resumes in place, and one that tears down and replays from a log.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# A trap MUST be an out-of-band halt the embedder observes when it invokes the entry — the wasm-level failure a partial operation or an aborting host function raises — rather than a variant the entry's result type declares, so that the internal trap mechanism (core-semantics.md §A Trap Halts Execution At A Defined Point) stays a run's terminal behavior and is not duplicated as a redundant arm of the interface.
//= spec/contracts/component-abi.md#the-entry-is-a-plain-function
//# The host MUST NOT require the component to encode any resume state, so that whichever resumption strategy a host chooses is invisible to the emitted component and constrained only by the run's determinism (capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).
//= spec/capabilities/capabilities-and-effects.md#how-a-host-resumes-is-host-policy-not-language
//# The mechanism by which a host resolves a call it cannot answer immediately — suspending an in-memory fiber and resuming in place, or discarding the run and re-deriving it from the ordered responses it has recorded — MUST be host runtime policy the language neither prescribes nor represents, so that portable re-derivation and local fiber suspension are both admissible and the emitted component is identical under either.
//= spec/capabilities/capabilities-and-effects.md#how-a-host-resumes-is-host-policy-not-language
//# Because a host MAY choose to re-derive a run from its input and recorded responses, a program that is a deterministic function of those (the requirement above) MUST remain resumable under that strategy without carrying any resume state itself; but a host that instead suspends the run in place MAY hold the run's live state, so the language requires determinism rather than statelessness and leaves the choice to the host.
// The plain functype — no resume parameter, no suspension arm — is also how the seed mandates NOTHING
// about the host's resume mechanism: determinism is the only boundary requirement, and every faithful
// resolution strategy (inline, suspend-in-place, tear-down-and-replay) sees these same emitted bytes.
//= spec/capabilities/capabilities-and-effects.md#a-run-is-a-deterministic-function-of-its-input-and-responses
//# This determinism MUST be the language's only requirement on the host boundary: the language MUST NOT mandate how a host suspends, resumes, or resolves a call, so that a host is free to answer synchronously, to suspend and resume a run in place, or to tear a run down and re-derive it, and every faithful strategy produces identical observable behavior.
//= spec/contracts/component-abi.md#the-entry-signature-crosses-the-boundary-by-the-same-rules
//# The entry's parameter and result types MUST each have a boundary representation fixed by this contract.
//= spec/contracts/component-abi.md#the-entry-signature-crosses-the-boundary-by-the-same-rules
//# The entry's input and output MUST lower and lift across the boundary by the same calling convention as any other boundary value.
pub(super) fn comp_functype(e: &BoundaryExport, list_type_idx: u32) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM]; // function type form
    let mut param_items = Vec::new();
    for (i, &vt) in e.params.iter().enumerate() {
        let pname = format!("p{i}");
        param_items.extend_from_slice(&uleb_bytes(pname.len() as u64));
        param_items.extend_from_slice(pname.as_bytes());
        param_items.push(vt);
    }
    item.extend_from_slice(&wasm_vec(e.params.len(), &param_items));
    match e.result {
        // A primitive result is its own valtype byte, inline; a `list<u8>` result references the shared
        // defined type by index (both under result-form `0x00` = "one result").
        BoundaryResult::Primitive(vt) => item.extend_from_slice(&[0x00, vt]),
        BoundaryResult::Bytes => {
            item.push(0x00);
            uleb128(list_type_idx as u64, &mut item);
        }
        BoundaryResult::None => item.extend_from_slice(&[0x01, 0x00]),
    }
    item
}

/// A sec-6 CORE-func alias item (alias a core-instance export): `00 00 01 <instance> <namelen> <name>`
/// — core-sort `0x00`, core-func-kind `0x00`, alias-target core-instance-export `0x01`, the instance
/// index, then the export name.
pub(super) fn core_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x00, 0x01];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// A sec-6 COMPONENT-func alias item (alias a component-instance export): `01 00 <instance> <namelen>
/// <name>` — component-func sort `0x01`, alias-target component-instance-export `0x00`, the instance
/// index, then the export name.
pub(super) fn comp_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// A sec-8 canon-lower item: `01 00 <comp-func> 00` — `01 00` canon lower a component func, the
/// component func index, then `00` empty canon-options.
pub(super) fn canon_lower_item(comp_func: u32) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x00); // canon options: none
    item
}

/// A sec-8 canon-lower item WITH a MEMORY option: `01 00 <comp-func> 01 03 <mem-idx>` — lower a component
/// func, then a one-option canon-options vec carrying `Memory(mem-idx)` (tag `0x03`). A host op with a
/// STRING parameter needs the memory the `(ptr,len)` lowering reads the string from, so its lower binds
/// the shared memory (the `0x03` Memory tag is the same component-model canon-opt encoding
/// `canon_lift_list_item` uses, pinned by the byte-identity oracle).
pub(super) fn canon_lower_item_mem(comp_func: u32, mem_idx: u32) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x01); // canon options: count 1
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item
}

/// A sec-8 canon-LOWER item WITH both Memory + Realloc options — needed to lower a host import that RETURNS
/// a heap value (`option<list<u8>>`, kv.get): the adapter uses `realloc` to allocate the returned list in
/// guest memory `mem_idx`. A host op with no list result carries the realloc option unused (harmless). Both
/// the memory and the realloc core func are aliased BEFORE `lower_sec` (the shared mem module; realloc =
/// core func 0 in the bytes-provider), breaking the lower↔realloc circularity.
pub(super) fn canon_lower_item_mem_realloc(
    comp_func: u32,
    mem_idx: u32,
    realloc_func: u32,
) -> Vec<u8> {
    let mut item = vec![0x01, 0x00];
    uleb128(comp_func as u64, &mut item);
    item.push(0x02); // canon options: count 2
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item.push(0x04); // CanonicalOption::Realloc
    uleb128(realloc_func as u64, &mut item);
    item
}

/// A sec-8 canon-lift item: `00 00 <core-func> 00 <type>` — `00 00` canon lift core func, `00` empty
/// canon-options, then the component type index.
pub(super) fn canon_lift_item(core_func: u32, type_idx: u32) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    item.push(0x00); // canon options: none
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-8 canon-lift item for a `list<u8>`-returning boundary func: `00 00 <core-func> <opts> <type>`,
/// where the canon-options vec carries the MEMORY and REALLOC the canonical ABI needs to read the
/// `(ptr, len)` return area out of the core module's linear memory (`00 00 <core-func> 02 03 <mem-idx>
/// 04 <realloc-func-idx> <type>`). Option tags: `0x03 <mem-idx>` = Memory, `0x04 <core-func-idx>` =
/// Realloc — the exact byte shape the `ComponentBuilder` oracle emits for `CanonicalOption::Memory` +
/// `::Realloc` (pinned by the R0 byte-identity test; the tags are component-model canon-opt encodings
/// `wasm-encoder` does not expose as public constants). Options are ordered Memory-then-Realloc.
pub(super) fn canon_lift_list_item(
    core_func: u32,
    mem_idx: u32,
    realloc_func: u32,
    type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![0x00, 0x00];
    uleb128(core_func as u64, &mut item);
    // canon options vec: count 2, then Memory then Realloc.
    item.push(0x02);
    item.push(0x03); // CanonicalOption::Memory
    uleb128(mem_idx as u64, &mut item);
    item.push(0x04); // CanonicalOption::Realloc
    uleb128(realloc_func as u64, &mut item);
    uleb128(type_idx as u64, &mut item);
    item
}

/// A sec-6 MEMORY alias item (alias a core-instance's exported memory): `00 02 01 <instance> <namelen>
/// <name>` — core-sort `0x00`, core-MEMORY-kind `0x02`, alias-target core-instance-export `0x01`, the
/// instance index, then the export name. The `0x02` (memory) kind is the only difference from
/// [`core_alias_item`] (a func alias, kind `0x00`); pinned by the R0 byte-identity oracle.
pub(super) fn memory_alias_item(instance: u32, name: &str) -> Vec<u8> {
    let mut item = vec![0x00, 0x02, 0x01];
    uleb128(instance as u64, &mut item);
    item.extend_from_slice(&uleb_bytes(name.len() as u64));
    item.extend_from_slice(name.as_bytes());
    item
}

/// The sec-7 defined-type item for `list<u8>`: `70 7d` — the component-model `list` defined-type tag
/// `0x70` followed by the element valtype (`u8` = `wasm_abi::COMP_U8`). It is the canonical binary value
/// form's boundary type (the resource `encode()` return); shared by every `list<u8>`-returning export,
/// laid at a fixed component-type index. The `0x70` list tag is a component-model structural encoding
/// `wasm-encoder` does not expose as a constant, pinned by the R0 byte-identity oracle.
pub(super) fn list_u8_defined_type() -> Vec<u8> {
    vec![0x70, wasm_abi::COMP_U8]
}

/// Build the HOST-effect import instance-type (`0x42 <decls>`) from `host_fns`. Each op is a `ty` decl
/// (`01 <comp_functype>`) + an `export` decl (`04 <name> 01 <func-type-index>`). When ANY op has a
/// `list<u8>` (Bytes) parameter (`has_list_param`), PREPEND a `(list u8)` defined type as instance-type
/// type index 0 — a Bytes param's `comp_functype` references it by index 0 — and the per-op func types
/// then occupy indices `1..=h`, so each export decl references `base + i` where `base = 1`. A pure
/// scalar/string set (no Bytes param) takes `base = 0`, no prepend, `2*h` decls — byte-identical to the
/// pre-Bytes shape. Shared by every host-import assembly variant so the prepend/shift is defined once.
pub(super) fn host_effect_instance_type(
    host_fns: &[HostFn],
    needs_list: bool,
    // Each spilled-RESULT defined type + whether it is NOMINAL (a `variant`/`enum`/`record` an import func's
    // result references, which MUST be exported — like a record param — else "instance not valid as import").
    result_defs: &[(Vec<u8>, bool)],
    record_defs: &[Vec<u8>],
) -> Vec<u8> {
    let h = host_fns.len();
    let mut decls = Vec::new();
    let mut prepended: u64 = 0;
    // idx 0: the shared `(list u8)` defined type — referenced by every `list<u8>` PARAM and every `list<u8>`
    // leaf of a spilled result. `needs_list` is computed by the caller (`build_host_result_types`) over both
    // params and results (every admitted spilled result bottoms out at `list<u8>`, so it forces this type).
    if needs_list {
        decls.push(0x01);
        decls.extend_from_slice(&list_u8_defined_type()); // type index 0
        prepended += 1;
    }
    // The spilled-RESULT defined types, built GENERALLY (Ty → WitType → CDef via `wit_ctype`) by the caller
    // and emitted here at instance-type indices 1.. (right after `(list u8)`), children-first + deduped. Each
    // op's `comp_functype` references its own result type by the `CRef` index the caller computed. This ONE
    // list REPLACES the former per-shape option / tuple / list<tuple> blocks; `list<list<u8>>`
    // (graph.neighbors) is just another entry here, no new branch.
    for (i, (rd, is_nominal)) in result_defs.iter().enumerate() {
        let defined_idx = prepended;
        decls.push(0x01);
        decls.extend_from_slice(rd);
        prepended += 1;
        // A NOMINAL result-def (`variant`/`enum`/`record` — e.g. run.run's `result<list<u8>, enum>` err arm)
        // must be EXPORTED (component-model rule, like a record param); a structural `list`/`option`/`result`/
        // `tuple` stays an anonymous bare define. The caller (`build_host_result_types`) already remapped every
        // reference to the EXPORT index (`defined_idx + 1`) via its export-aware indexing.
        if *is_nominal {
            decls.push(0x04); // export decl
            decls.extend_from_slice(&extern_name(&format!("host-result-t{i}")));
            decls.push(0x03); // externdesc: type
            decls.push(0x00); // typebound: eq
            uleb128(defined_idx, &mut decls);
            prepended += 1;
        }
    }
    // RECORD-param types (shape d): a NOMINAL type (record) that a func in an IMPORT instance-type uses
    // must be EXPORTED from the instance (a component-model rule — a structural `list<u8>` may be anonymous,
    // a record may NOT; verified against `wasm-tools component wit`'s own encoding of a record-param import).
    // So per record param lay TWO decls: (1) DEFINE the record (`0x01 <record-bytes>`, type index = current)
    // then (2) EXPORT it as a named type (`0x04 <name> 0x03 0x00 <defined-idx>` — export decl, a `type`
    // externdesc with an `eq` bound to the defined index), which introduces the EXPORTED type at the NEXT
    // index. The op's `comp_functype` references that EXPORTED index (defined+1), NOT the raw defined index.
    // In the supported shape (a single record param + NO Bytes/option/pairs op — enforced by the caller) no
    // shared type is prepended, so the record is DEFINED at index 0 and EXPORTED at index 1 (the index the
    // Record arm of `host_op_comp_functype` references), and the func types follow at index 2+.
    for (i, rd) in record_defs.iter().enumerate() {
        let defined_idx = prepended;
        decls.push(0x01);
        decls.extend_from_slice(rd);
        prepended += 1;
        decls.push(0x04); // export decl
        decls.extend_from_slice(&extern_name(&format!("host-record-p{i}")));
        decls.push(0x03); // externdesc: type
        decls.push(0x00); // typebound: eq
        uleb128(defined_idx, &mut decls);
        prepended += 1;
    }
    let base: u64 = prepended;
    for (i, f) in host_fns.iter().enumerate() {
        decls.push(0x01);
        decls.extend_from_slice(&f.comp_functype);
        decls.push(0x04);
        decls.extend_from_slice(&extern_name(
            &crate::backend::common::export_name::kebab_extern_name(&f.op),
        ));
        decls.push(0x01); // sort: component func
        uleb128(base + i as u64, &mut decls);
    }
    let decl_count = prepended as usize + 2 * h;
    let mut it = vec![0x42]; // instance type form
    it.extend_from_slice(&wasm_vec(decl_count, &decls));
    it
}

/// The sec-7 defined-type item for a component `tuple<vt0, vt1, …>`: `6f <count> <vt>*` — the component-
/// model `tuple` defined-type tag `0x6f`, then the field-count vec of primitive valtype bytes. A FIXED-SHAPE
/// SCALAR tuple closure argument crosses the DIRECT-CALL boundary as this native type; the canonical ABI
/// FLATTENS it (≤16 scalar fields) into scalar core params, which the guest `call` rebuilds into a cell
/// (`serialize::TupleArgRebuild`). The `0x6f` tuple tag is a component-model structural encoding
/// `wasm-encoder` writes via `ComponentDefinedType::tuple`; the `a_fixed_shape_tuple_closure_arg_crosses_by_
/// native_flattening` oracle pins that a `tuple<s64,s64>` param lifts + runs (matching `type_defined().tuple`).
pub(super) fn tuple_defined_type(field_bytes: &[u8]) -> Vec<u8> {
    let mut item = vec![0x6f];
    item.extend_from_slice(&wasm_vec(field_bytes.len(), field_bytes));
    item
}

/// A component `option<T>` DEFINED TYPE: `0x6b <T-valtype>` — the `option` former tag then the payload's
/// primitive valtype byte. The direct-call SUM-arg path: an `(Option scalar)` closure argument crosses as
/// this native type, which the canonical ABI FLATTENS into `(disc: i32, payload: <T>)` core params — the
/// guest `call` rebuilds the sum cell from them (`serialize::SumArgRebuild`). Pinned runnable by the
/// `an_option_scalar_closure_arg_crosses_by_native_flattening` oracle (`wasm_encoder`'s `.option(...)`).
pub(super) fn option_defined_type(payload_byte: u8) -> Vec<u8> {
    vec![0x6b, payload_byte]
}

/// A component `result<ok, err>` DEFINED TYPE: `0x6a <ok-valtype-opt> <err-valtype-opt>` — the `result`
/// former tag then each side's OPTIONAL valtype (`0x01 <byte>` for a present scalar payload, `0x00` for a
/// nullary side). A general `variant` must be NAMED, but `result`/`option` are anonymous-allowed, so a
/// `(Result scalar scalar)` arg crosses as this native type. Flattened by the canonical ABI to `(disc: i32,
/// payload)` (Ok=0, Err=1). Pinned runnable by the `a_result_scalar_closure_arg_crosses_by_native_flattening`
/// oracle (`wasm_encoder`'s `.result(Some, Some)`).
pub(super) fn result_defined_type(ok_byte: u8, err_byte: u8) -> Vec<u8> {
    // `0x01 <byte>` is the `Some(primitive)` valtype encoding (an inline primitive, not a type-index ref).
    vec![0x6a, 0x01, ok_byte, 0x01, err_byte]
}

/// The boundary component-TYPE shape of a fixed-shape compound closure argument, recursively: each field is
/// either a PRIMITIVE valtype byte (an aliased-width scalar leaf) or a NESTED tuple (its own field shapes).
/// A `Scalar` field is one flattened core param; a `Nested` field is its own `tuple<…>` DEFINED type the
/// canonical ABI flattens recursively. `TupleFieldShape::Scalar`-only tuples reduce to the flat
/// `tuple_defined_type(field_bytes)` case; a `Nested` field forces the recursive minting below.
#[derive(Clone)]
pub enum TupleFieldShape {
    /// A scalar leaf field carrying its component primitive valtype byte (`COMP_S64`, …).
    Scalar(u8),
    /// A nested fixed-shape tuple/record field, its own fields in cell order.
    Nested(Vec<TupleFieldShape>),
}

/// Mint the `tuple<…>` DEFINED TYPE for a (possibly NESTED) fixed-shape compound argument into `items`,
/// emitting every INNER nested tuple type FIRST (bottom-up) so the outer tuple can reference them by index.
/// `next_type` is the component-type index the FIRST minted type will occupy; it is advanced past every type
/// this mints. Returns the type index of the OUTERMOST tuple (the one a `call` functype references as the
/// argument). A field shape of all `Scalar`s mints exactly ONE type (byte-identical to `tuple_defined_type`);
/// each `Nested` field mints its own sub-tuple first (recursively). Used by the single-export scalar-result
/// `call` path; other paths (flat `tuple_defined_type`) still assume all-scalar fields.
pub(super) fn mint_tuple_type_nested(
    fields: &[TupleFieldShape],
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> u32 {
    // Mint each nested field's sub-tuple first, recording the valtype byte(s) each field contributes to the
    // outer tuple (a scalar → its primitive byte; a nested → an sleb128 type-index reference).
    let mut field_valtypes: Vec<Vec<u8>> = Vec::with_capacity(fields.len());
    for f in fields {
        match f {
            TupleFieldShape::Scalar(b) => field_valtypes.push(vec![*b]),
            TupleFieldShape::Nested(sub) => {
                let sub_idx = mint_tuple_type_nested(sub, next_type, items);
                // A defined-type reference in a `tuple` field is the type index as a SIGNED LEB128 (matching
                // `wasm_encoder`'s `ComponentValType::Type(i) => (i as i64).encode`); a small positive index
                // is one byte ≥ 0 and < 0x64, distinct from the primitive-byte range (0x64..=0x7f).
                let mut enc = Vec::new();
                crate::backend::wasm::encode::sleb128(sub_idx as i64, &mut enc);
                field_valtypes.push(enc);
            }
        }
    }
    // Now emit the OUTER tuple type: `0x6f <count> <field-valtype-encoding>*`.
    let mut tup = vec![0x6f];
    let mut body = Vec::new();
    for fv in &field_valtypes {
        body.extend_from_slice(fv);
    }
    tup.extend_from_slice(&wasm_vec(field_valtypes.len(), &body));
    items.extend_from_slice(&tup);
    let outer_idx = *next_type;
    *next_type += 1;
    outer_idx
}

/// The number of component TYPES [`mint_tuple_type_nested`] emits for `fields`: 1 for the outer tuple + the
/// recursive count for every nested field. A flat all-scalar shape is 1 (byte-identical to the flat path).
pub(super) fn nested_tuple_type_count(fields: &[TupleFieldShape]) -> u32 {
    1 + fields
        .iter()
        .map(|f| match f {
            TupleFieldShape::Scalar(_) => 0,
            TupleFieldShape::Nested(sub) => nested_tuple_type_count(sub),
        })
        .sum::<u32>()
}

/// ONE `call`-argument slot in the closure's original arg order: either an aliased-width SCALAR (crossing as
/// its component primitive valtype byte) or a fixed-shape TUPLE/record (crossing as a native `tuple<…>` the
/// canonical ABI flattens — possibly nested, so its own `TupleFieldShape` tree). This is the N-arg
/// generalization of the single-tuple `(prefix_bytes, tuple_shape, suffix_bytes)` interleave: a slot list with
/// exactly ONE `Tuple` and the rest `Scalar` reproduces that shape byte-for-byte, and TWO+ `Tuple` slots are
/// the N-compound-args case (each tuple mints its own `tuple<…>` defined type, referenced by index in order).
#[derive(Clone)]
pub enum ArgSlot {
    /// A scalar leaf arg carrying its component primitive valtype byte.
    Scalar(u8),
    /// A fixed-shape tuple/record arg, its (possibly nested) field shape.
    Tuple(Vec<TupleFieldShape>),
    /// An `(Option scalar)` arg carrying its payload's component primitive valtype byte — crosses as a native
    /// `option<payload>` DEFINED type (minted by [`mint_call_arg_tuple_types`], flattened by the canonical ABI
    /// to `(disc: i32, payload)`; the guest rebuilds the sum cell via `serialize::SumArgRebuild`).
    OptionScalar(u8),
    /// A `(Result ok-scalar err-scalar)` arg carrying its ok + err payload component primitive valtype bytes —
    /// crosses as a native `result<ok, err>` DEFINED type (the `0x6a` former; anonymous-allowed, unlike a
    /// general `variant`). Flattened by the canonical ABI to `(disc: i32, payload)`; the guest rebuilds the
    /// sum cell via `serialize::SumArgRebuild`.
    Result(u8, u8),
    /// An `(Option compound)` arg whose payload is a fixed-shape TUPLE/record — crosses as a native
    /// `option<tuple<…>>` DEFINED type (the inner `tuple<…>` minted first — possibly nested — then `option`
    /// referencing it; both formers anonymous-allowed, unlike a general `variant`). Flattened by the canonical
    /// ABI to `(disc: i32, <payload tuple's leaves…>)`; the guest rebuilds the payload cell + `sum-new`s the
    /// Some over it via `serialize::SumArgRebuild` (a `SumArmPayload::Compound` arm). Carries the payload's
    /// (possibly nested) `TupleFieldShape` tree, exactly like [`ArgSlot::Tuple`].
    OptionCompound(Vec<TupleFieldShape>),
    /// A `(Result ok err)` arg where AT LEAST ONE side's payload is a fixed-shape TUPLE/record (a compound) —
    /// crosses as a native `result<ok, err>` DEFINED type whose ok/err valtypes are each a primitive byte
    /// (scalar side) OR a minted `tuple<…>` (compound side). The canonical ABI flattens it to `(disc: i32,
    /// <joined payload leaves…>)`, the two arms' payloads joined position-by-position (the wider arm sets each
    /// slot's width). The guest rebuilds the selected arm's cell over a PREFIX of the joined slots via
    /// `serialize::SumArgRebuild`. Each side carries its [`ResultSide`] (scalar byte or tuple shape).
    ResultCompound(ResultSide, ResultSide),
    /// A MEMORY-BEARING / VALUE-FORM leaf param crossing as its natural WIT: `string` when `is_string`, else
    /// `list<u8>` (a `Bytes`/`list<scalar>` or a `BigInt`/`Rational`/`Symbol` value-form). The canonical ABI
    /// lowers it to a `(ptr: i32, len: i32)` pair the make body lifts out of linear memory. A `string` is an
    /// inline primitive component type; a `list<u8>` is a minted defined type the functype references by index.
    MemLeaf { is_string: bool },
}

/// One side (ok or err) of a [`ArgSlot::ResultCompound`]: a scalar leaf (its component primitive byte) OR a
/// fixed-shape TUPLE/record payload (its own, possibly nested, `TupleFieldShape` tree — minted as a `tuple<…>`
/// the `result` former references by index). A nullary side (`Result … ()`) is not modeled here (both sides of
/// a Cadenza `(Result a b)` carry a payload).
#[derive(Clone)]
pub enum ResultSide {
    Scalar(u8),
    Compound(Vec<TupleFieldShape>),
}

/// The number of component TYPES [`mint_call_arg_tuple_types`] emits for `slots`: the sum of
/// [`nested_tuple_type_count`] over every `Tuple` slot, plus ONE `option<…>` type per `OptionScalar` slot (a
/// `Scalar` slot mints none). Zero when every slot is a plain scalar (byte-identical to the all-scalar path).
pub(super) fn call_arg_tuple_type_count(slots: &[ArgSlot]) -> u32 {
    slots
        .iter()
        .map(|s| match s {
            ArgSlot::Scalar(_) => 0,
            ArgSlot::Tuple(shape) => nested_tuple_type_count(shape),
            ArgSlot::OptionScalar(_) | ArgSlot::Result(_, _) => 1,
            // the payload tuple's types (possibly nested) + the outer `option<…>` referencing it.
            ArgSlot::OptionCompound(shape) => nested_tuple_type_count(shape) + 1,
            // each compound side's tuple types + the outer `result<…>` referencing them (a scalar side = 0).
            ArgSlot::ResultCompound(ok, err) => {
                result_side_type_count(ok) + result_side_type_count(err) + 1
            }
            // A String mem-leaf is the inline `string` primitive (0 types); a Bytes/list/value-form mem-leaf
            // mints ONE `list<u8>` defined type.
            ArgSlot::MemLeaf { is_string } => u32::from(!*is_string),
        })
        .sum()
}

/// The number of component types a [`ResultSide`] mints: 0 for a scalar (an inline primitive byte), the nested
/// tuple type count for a compound.
pub(super) fn result_side_type_count(side: &ResultSide) -> u32 {
    match side {
        ResultSide::Scalar(_) => 0,
        ResultSide::Compound(shape) => nested_tuple_type_count(shape),
    }
}

/// Mint the aggregate DEFINED TYPES for every non-scalar slot into `items`, in arg order, advancing
/// `next_type` past each. Returns, per slot, `Some(defined_type_idx)` for a `Tuple`/`OptionScalar` slot (the
/// `tuple<…>`/`option<…>` type the `call` functype references by index) and `None` for a `Scalar` slot. A
/// single `Tuple` slot mints byte-identically to `mint_tuple_type_nested`; an `OptionScalar` mints one
/// `option<payload>`.
pub(super) fn mint_call_arg_tuple_types(
    slots: &[ArgSlot],
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> Vec<Option<u32>> {
    slots
        .iter()
        .map(|s| match s {
            ArgSlot::Scalar(_) => None,
            ArgSlot::Tuple(shape) => Some(mint_tuple_type_nested(shape, next_type, items)),
            ArgSlot::OptionScalar(payload_byte) => {
                items.extend_from_slice(&option_defined_type(*payload_byte));
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::Result(ok_byte, err_byte) => {
                items.extend_from_slice(&result_defined_type(*ok_byte, *err_byte));
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::OptionCompound(shape) => {
                // Mint the payload `tuple<…>` (possibly nested) FIRST, then the `option<…>` referencing it by
                // index. The `option` former is `0x6b <valtype>`; a defined-type reference is the type index as
                // a SIGNED LEB128 (matching `ComponentValType::Type(i) => (i as i64).encode`).
                let tup_idx = mint_tuple_type_nested(shape, next_type, items);
                let mut opt = vec![0x6b];
                crate::backend::wasm::encode::sleb128(tup_idx as i64, &mut opt);
                items.extend_from_slice(&opt);
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            ArgSlot::ResultCompound(ok, err) => {
                // Mint each COMPOUND side's `tuple<…>` FIRST (in order ok, err), then the `result<…>` (former
                // `0x6a <ok-valtype-opt> <err-valtype-opt>`, each `0x01 <valtype>`) referencing them. A scalar
                // side's valtype is its inline primitive byte; a compound side's is its minted tuple index
                // (SIGNED LEB128). `mint_result_side_valtype` mints the tuple (if any) + returns the encoding.
                let ok_vt = mint_result_side_valtype(ok, next_type, items);
                let err_vt = mint_result_side_valtype(err, next_type, items);
                let mut res = vec![0x6a, 0x01];
                res.extend_from_slice(&ok_vt);
                res.push(0x01);
                res.extend_from_slice(&err_vt);
                items.extend_from_slice(&res);
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
            // A String mem-leaf is the inline `string` primitive — no defined type. A Bytes/list/value-form
            // mem-leaf mints ONE `list<u8>` defined type the make functype references by index.
            ArgSlot::MemLeaf { is_string: true } => None,
            ArgSlot::MemLeaf { is_string: false } => {
                items.extend_from_slice(&list_u8_defined_type());
                let idx = *next_type;
                *next_type += 1;
                Some(idx)
            }
        })
        .collect()
}

/// Mint a [`ResultSide`]'s tuple type (if compound) into `items`, advancing `next_type`, and return the side's
/// component valtype ENCODING for the enclosing `result<…>` former: an inline primitive byte for a scalar, or
/// the minted tuple type index as a SIGNED LEB128 for a compound.
pub(super) fn mint_result_side_valtype(
    side: &ResultSide,
    next_type: &mut u32,
    items: &mut Vec<u8>,
) -> Vec<u8> {
    match side {
        ResultSide::Scalar(byte) => vec![*byte],
        ResultSide::Compound(shape) => {
            let tup_idx = mint_tuple_type_nested(shape, next_type, items);
            let mut enc = Vec::new();
            crate::backend::wasm::encode::sleb128(tup_idx as i64, &mut enc);
            enc
        }
    }
}

/// A `call` functype for a closure whose args are the given ordered `slots` (scalars + fixed-shape tuples
/// interleaved): `(self: <handle<t>>, p0: <slot0>, …) -> R`. `tuple_type_idxs[i]` is `Some(idx)` for a `Tuple`
/// slot (the `tuple<…>` defined type minted by [`mint_call_arg_tuple_types`], referenced by index) and `None`
/// for a `Scalar` slot (its primitive byte is taken from the slot). This is the N-tuple generalization of
/// [`closure_call_tuple_arg_functype_interleaved`]: a slot list of `[Scalar…, Tuple, Scalar…]` produces the
/// exact same bytes (one tuple among scalars); TWO+ `Tuple` slots interleave their type-index references.
pub(super) fn closure_call_functype_slots(
    self_handle_type_idx: u32,
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                None,
            ) => {
                unreachable!("a Tuple/Option slot must carry a minted defined-type index")
            }
            // A mem-leaf/value-form param crosses only on the resource-`make` path, never a closure call.
            (ArgSlot::MemLeaf { .. }, _) => {
                unreachable!("a mem-leaf make param does not occur on the closure-call path")
            }
        }
    }
    item.extend_from_slice(&wasm_vec(1 + slots.len(), &param_items));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// The `list<u8>`-result counterpart of [`closure_call_functype_slots`]: `(self: <handle<t>>, p0: <slot0>, …)
/// -> list<u8>`. The param list is identical (scalars + fixed-shape tuples interleaved by the `ArgSlot`
/// model); only the result references the `list<u8>` DEFINED type by index instead of an inline scalar byte.
/// Its lift carries Memory/Realloc (the caller uses `canon_lift_list_item`). The N-tuple generalization of
/// [`closure_call_list_tuple_arg_functype_interleaved`].
pub(super) fn closure_call_list_functype_slots(
    self_handle_type_idx: u32,
    slots: &[ArgSlot],
    tuple_type_idxs: &[Option<u32>],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    for (pn, (slot, tup_idx)) in slots.iter().zip(tuple_type_idxs).enumerate() {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        match (slot, tup_idx) {
            (ArgSlot::Scalar(vt), _) => param_items.push(*vt),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                Some(idx),
            ) => param_items.extend_from_slice(&owned_valtype(*idx)),
            (
                ArgSlot::Tuple(_)
                | ArgSlot::OptionScalar(_)
                | ArgSlot::Result(_, _)
                | ArgSlot::OptionCompound(_)
                | ArgSlot::ResultCompound(_, _),
                None,
            ) => {
                unreachable!("a Tuple/Option slot must carry a minted defined-type index")
            }
            // A mem-leaf/value-form param crosses only on the resource-`make` path, never a closure call.
            (ArgSlot::MemLeaf { .. }, _) => {
                unreachable!("a mem-leaf make param does not occur on the closure-call path")
            }
        }
    }
    item.extend_from_slice(&wasm_vec(1 + slots.len(), &param_items));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00);
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A `call` functype for a closure taking ONE fixed-shape scalar tuple arg AMONG scalar args: `(self:
/// <handle<t>>, <prefix scalars…>, p: tuple<…>, <suffix scalars…>) -> R`. `prefix_bytes`/`suffix_bytes` are
/// the scalar boundary bytes BEFORE/AFTER the tuple (in the closure's original arg order); `tuple_type_idx`
/// the `tuple<…>` defined type. The scalar-result compound-arg path with the tuple at any position.
pub(super) fn closure_call_tuple_arg_functype_interleaved(
    self_handle_type_idx: u32,
    prefix_bytes: &[u8],
    tuple_type_idx: u32,
    suffix_bytes: &[u8],
    result_byte: u8,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    // `self` — the receiver handle (own/borrow<t>), a defined type referenced by index.
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    let mut pn = 0usize; // positional param name counter (cosmetic)
    for &vt in prefix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    // the tuple argument, a defined type referenced by index.
    {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.extend_from_slice(&owned_valtype(tuple_type_idx));
        pn += 1;
    }
    for &vt in suffix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    item.extend_from_slice(&wasm_vec(
        1 + prefix_bytes.len() + 1 + suffix_bytes.len(),
        &param_items,
    ));
    // One result — the closure's return valtype (a scalar boundary byte).
    item.extend_from_slice(&[0x00, result_byte]);
    item
}

/// A `call` functype for a closure taking ONE fixed-shape scalar tuple arg AMONG scalar args AND returning a
/// `list<u8>` (byte-rope / compound / collection result): `(self: <handle<t>>, <prefix scalars…>, p:
/// tuple<…>, <suffix scalars…>) -> list<u8>`. Combines the interleaved-arg shape of
/// [`closure_call_tuple_arg_functype_interleaved`] with the `list<u8>` result. Its lift carries Memory/Realloc.
pub(super) fn closure_call_list_tuple_arg_functype_interleaved(
    self_handle_type_idx: u32,
    prefix_bytes: &[u8],
    tuple_type_idx: u32,
    suffix_bytes: &[u8],
    list_type_idx: u32,
) -> Vec<u8> {
    let mut item = vec![wasm_abi::COMP_FUNCTYPE_FORM];
    let mut param_items = Vec::new();
    param_items.extend_from_slice(&uleb_bytes("self".len() as u64));
    param_items.extend_from_slice(b"self");
    param_items.extend_from_slice(&owned_valtype(self_handle_type_idx));
    let mut pn = 0usize;
    for &vt in prefix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.extend_from_slice(&owned_valtype(tuple_type_idx));
        pn += 1;
    }
    for &vt in suffix_bytes {
        let name = format!("p{pn}");
        param_items.extend_from_slice(&uleb_bytes(name.len() as u64));
        param_items.extend_from_slice(name.as_bytes());
        param_items.push(vt);
        pn += 1;
    }
    item.extend_from_slice(&wasm_vec(
        1 + prefix_bytes.len() + 1 + suffix_bytes.len(),
        &param_items,
    ));
    // One result — the `list<u8>` defined type, referenced by index.
    item.push(0x00);
    uleb128(list_type_idx as u64, &mut item);
    item
}

/// A sec-11 component-export item: `00 <namelen><name> 01 <func-idx> 00` — name, sort component
/// func:0x01, the func index, no declared type ascription.
///
/// The extern name is NORMALIZED to kebab-case (`kebab_extern_name`): a source export name may be a
/// valid Cadenza identifier that is NOT a valid component extern name (an uppercase letter or underscore
/// — `fA`, `my_func`), which would make the component fail to validate. An already-kebab name (the
/// common case — every corpus export) normalizes to itself, so this is byte-identical for existing
/// programs. A collision (two source names → one extern name) is rejected at export planning, before
/// emit, so this site never silently merges two exports. The CORE module export + its alias keep the
/// verbatim source name (a valid core wasm name); only this component-boundary extern is kebab.
pub(super) fn comp_export_item(name: &str, func_idx: u32) -> Vec<u8> {
    let extern_name = crate::backend::common::export_name::kebab_extern_name(name);
    let mut item = vec![0x00];
    item.extend_from_slice(&uleb_bytes(extern_name.len() as u64));
    item.extend_from_slice(extern_name.as_bytes());
    item.push(0x01); // sort: component func
    uleb128(func_idx as u64, &mut item);
    item.push(0x00); // no declared type ascription
    item
}
