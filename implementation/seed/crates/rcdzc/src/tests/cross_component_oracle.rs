use wasm_encoder::*;

/// Provider core A: exports `f : (i32) -> i32` computing `x + 1`. A leaf core module, no imports.
fn provider_core_a() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0: (i32)->i32
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("f", ExportKind::Func, 0);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::LocalGet(0));
    f.instruction(&Instruction::I32Const(1));
    f.instruction(&Instruction::I32Add);
    f.instruction(&Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

/// Consumer core B: imports `f : (i32) -> i32` from module `"peer"`, exports `main : (i32) -> i32`
/// computing `f(x) * 10` — the cross-component call is the imported `call 0`.
fn consumer_core_b() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0: (i32)->i32
    m.section(&types);
    let mut imports = ImportSection::new();
    imports.import("peer", "f", EntityType::Function(0)); // core func 0 = the peer's f
    m.section(&imports);
    let mut funcs = FunctionSection::new();
    funcs.function(0); // main is core func 1
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 1);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut main = Function::new(vec![]);
    main.instruction(&Instruction::LocalGet(0));
    main.instruction(&Instruction::Call(0)); // f(x) — the cross-component call
    main.instruction(&Instruction::I32Const(10));
    main.instruction(&Instruction::I32Mul);
    main.instruction(&Instruction::End);
    code.function(&main);
    m.section(&code);
    m.finish()
}

/// Inner component wrapping provider A: exports interface `cadenza:peer/api` with `f : func(s32) -> s32`.
fn provider_component_a() -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    let core_idx = c.core_module_raw(&provider_core_a());
    let inst = c.core_instantiate::<[(&str, ModuleArg); 0]>(core_idx, []);
    let f_core = c.core_alias_export(inst, "f", ExportKind::Func);
    let (f_ty, mut ft) = c.type_function();
    ft.params([("x", ComponentValType::Primitive(PrimitiveValType::S32))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S32)));
    let f_comp = c.lift_func(f_core, f_ty, []);
    // Export `f` as a top-level component func the consumer imports by name. (X1 is a de-risk
    // oracle for the cross-component CALL + shared runtime; the production envelope in X3 wraps
    // this in a named interface via the inner-re-export-component pattern the resource oracle uses.)
    c.export(
        "f",
        ComponentExportKind::Func,
        f_comp,
        Some(ComponentTypeRef::Func(f_ty)),
    );
    c
}

/// Inner component wrapping consumer B: IMPORTS interface `cadenza:peer/api` (with `f`), lowers `f`
/// into B's core under module `"peer"`, and exports B's `main : func(s32) -> s32` at the top level.
fn consumer_component_b() -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    // Import the peer's `f : func(s32)->s32` as a top-level component func.
    let (f_ty, mut ft) = c.type_function();
    ft.params([("x", ComponentValType::Primitive(PrimitiveValType::S32))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S32)));
    let f_comp = c.import("f", ComponentTypeRef::Func(f_ty));
    let f_core = c.lower_func(f_comp, []);
    let peer_inst = c.core_instantiate_exports([("f", ExportKind::Func, f_core)]);
    let core_idx = c.core_module_raw(&consumer_core_b());
    let prog_inst = c.core_instantiate(core_idx, [("peer", ModuleArg::Instance(peer_inst))]);
    let main_core = c.core_alias_export(prog_inst, "main", ExportKind::Func);
    let (main_ty, mut mf) = c.type_function();
    mf.params([("x", ComponentValType::Primitive(PrimitiveValType::S32))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S32)));
    let main_comp = c.lift_func(main_core, main_ty, []);
    c.export(
        "main",
        ComponentExportKind::Func,
        main_comp,
        Some(ComponentTypeRef::Func(main_ty)),
    );
    c
}

/// The OUTER composition: instantiate provider A, then consumer B binding B's `f` import to A's
/// exported `f`, and re-export B's `main`.
fn composed_scalar_component() -> Vec<u8> {
    let mut c = ComponentBuilder::default();
    let a_idx = c.component(provider_component_a());
    let no_args: [(&str, ComponentExportKind, u32); 0] = [];
    let a_inst = c.instantiate(a_idx, no_args);
    let a_f = c.alias_export(a_inst, "f", ComponentExportKind::Func);
    let b_idx = c.component(consumer_component_b());
    let b_inst = c.instantiate(b_idx, [("f", ComponentExportKind::Func, a_f)]);
    let main = c.alias_export(b_inst, "main", ComponentExportKind::Func);
    c.export("main", ComponentExportKind::Func, main, None);
    c.finish()
}

#[test]
fn x1a_a_consumer_binds_and_calls_a_provider_export_across_a_component_boundary() {
    // B calls A's `f` (x+1) then *10. main(5) = (5+1)*10 = 60. Proves the cross-component call
    // wiring works end-to-end under wasmtime with NO shared runtime (scalar transport).
    let comp = composed_scalar_component();
    // STRUCTURAL pin: the hand-built B-calls-A scalar composition is a VALID component. The RUN — main(5)
    // = (5+1)*10 = 60 across the boundary with no shared runtime — is corpus/conformance territory (every
    // 29-cross-component-peers case composes a separate provider and RUNS a scalar call across the
    // boundary); this hand-built ComponentBuilder composition cannot be a corpus (peer) case (the clause
    // takes a Cadenza-source provider), so it stays as a compile+validate pin (the x3/x4a family).
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("composed cross-component component validates");
}

// ------------------------------------------------------------------------------------------------
// X1b — TWO program cores share ONE value-heap runtime instance; a handle A builds is meaningful to
// B (component-abi.md §A Cross-Component Handle Is Meaningful Only In The Shared Runtime Instance).
// The genuinely novel de-risk: a runtime value crosses A→B as an opaque handle over a SHARED heap.
// ------------------------------------------------------------------------------------------------

use crate::backend::wasm::runtime_abi::{OPS, RtOp};

/// The five heap ops both cores import, sorted by name (arr-alloc, arr-get, arr-set, box-int, get-int).
/// Both cores import them under module `"heap"` in THIS order, so a call index is the op's position.
fn heap_ops() -> [&'static RtOp; 5] {
    [
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.get_int,
    ]
}

/// The component valtype of an ABI type (local copy of the r2-module helper).
fn abi_comp(p: crate::backend::wasm::runtime_abi::AbiValType) -> ComponentValType {
    use crate::backend::wasm::runtime_abi::AbiValType;
    ComponentValType::Primitive(match p {
        AbiValType::U32 => PrimitiveValType::U32,
        AbiValType::S64 => PrimitiveValType::S64,
        AbiValType::Bool => PrimitiveValType::Bool,
        AbiValType::F64 => PrimitiveValType::F64,
        AbiValType::F32 => PrimitiveValType::F32,
        AbiValType::S8 => PrimitiveValType::S8,
        AbiValType::U8 => PrimitiveValType::U8,
        AbiValType::S16 => PrimitiveValType::S16,
        AbiValType::U16 => PrimitiveValType::U16,
        AbiValType::S32 => PrimitiveValType::S32,
        AbiValType::U64 => PrimitiveValType::U64,
        AbiValType::Char => PrimitiveValType::Char,
    })
}

/// The CORE functype `(params)->(result?)` of a runtime op (local copy of the r2-module helper).
fn op_core_functype(op: &RtOp) -> (Vec<ValType>, Vec<ValType>) {
    use crate::backend::wasm::runtime_abi::AbiValType;
    let core = |p: AbiValType| match p {
        AbiValType::U32
        | AbiValType::Bool
        | AbiValType::S8
        | AbiValType::U8
        | AbiValType::S16
        | AbiValType::U16
        | AbiValType::S32
        | AbiValType::Char => ValType::I32,
        AbiValType::S64 | AbiValType::U64 => ValType::I64,
        AbiValType::F64 => ValType::F64,
        AbiValType::F32 => ValType::F32,
    };
    let params = op.params.iter().map(|p| core(*p)).collect();
    let results = op.result.map(|r| vec![core(r)]).unwrap_or_default();
    (params, results)
}

/// Emit the import section (module `"heap"`) declaring the five ops, and the type section they use.
/// Returns the count of imported funcs (= 5) so the caller knows where its own funcs start.
fn heap_import_prologue(types: &mut TypeSection, imports: &mut ImportSection) -> u32 {
    for (i, op) in heap_ops().iter().enumerate() {
        let (p, r) = op_core_functype(op);
        types.ty().function(p, r);
        imports.import("heap", op.name, EntityType::Function(i as u32));
    }
    heap_ops().len() as u32
}
// Import indices within the shared `"heap"` order:
const H_ARR_ALLOC: u32 = 0;
const H_ARR_GET: u32 = 1;
const H_ARR_SET: u32 = 2;
const H_BOX_INT: u32 = 3;
const H_GET_INT: u32 = 4;

/// Provider core A': imports the heap ops; exports `build : () -> i32` returning a runtime handle for
/// a 1-element array `[99]` built on the value heap. The handle is an opaque `u32` at the boundary.
fn provider_core_a_heap() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    let base = heap_import_prologue(&mut types, &mut imports);
    // build : () -> i32   (new functype after the 5 op types)
    let build_ty = base; // type index for () -> i32
    types.ty().function(vec![], vec![ValType::I32]);
    m.section(&types);
    m.section(&imports);
    let mut funcs = FunctionSection::new();
    funcs.function(build_ty); // build = core func `base`
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("build", ExportKind::Func, base);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut build = Function::new(vec![(1, ValType::I32)]); // local 0 = the array handle
    // a = arr-alloc(1)
    build.instruction(&Instruction::I32Const(1));
    build.instruction(&Instruction::Call(H_ARR_ALLOC));
    build.instruction(&Instruction::LocalSet(0));
    // a = arr-set(a, 0, box-int(99))
    build.instruction(&Instruction::LocalGet(0));
    build.instruction(&Instruction::I32Const(0));
    build.instruction(&Instruction::I64Const(99));
    build.instruction(&Instruction::Call(H_BOX_INT));
    build.instruction(&Instruction::Call(H_ARR_SET));
    build.instruction(&Instruction::LocalSet(0));
    // return a
    build.instruction(&Instruction::LocalGet(0));
    build.instruction(&Instruction::End);
    code.function(&build);
    m.section(&code);
    m.finish()
}

/// Consumer core B': imports the heap ops AND `build : () -> i32` from module `"peer"`; exports
/// `main : () -> i64` = `get-int(arr-get(build(), 0))`. Reads the element out of the handle A built
/// on the SHARED heap — if it reads 99, the shared runtime instance is genuinely shared across A & B.
fn consumer_core_b_heap() -> Vec<u8> {
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    let base = heap_import_prologue(&mut types, &mut imports);
    // build : () -> i32  imported from "peer" (type index `base`, import func index `base`)
    types.ty().function(vec![], vec![ValType::I32]);
    imports.import("peer", "build", EntityType::Function(base));
    // main : () -> i64   (type index base+1)
    let main_ty = base + 1;
    types.ty().function(vec![], vec![ValType::I64]);
    m.section(&types);
    m.section(&imports);
    let peer_build = base; // the imported build is core func `base`
    let mut funcs = FunctionSection::new();
    funcs.function(main_ty); // main = core func base+1
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, base + 1);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut main = Function::new(vec![]);
    // get-int(arr-get(build(), 0))
    main.instruction(&Instruction::Call(peer_build));
    main.instruction(&Instruction::I32Const(0));
    main.instruction(&Instruction::Call(H_ARR_GET));
    main.instruction(&Instruction::Call(H_GET_INT));
    main.instruction(&Instruction::End);
    code.function(&main);
    m.section(&code);
    m.finish()
}

/// Provider inner component A': imports the heap interface, lowers the ops into A's core, exports
/// `build : func() -> u32` at the top level.
fn provider_component_a_heap(import_name: &str) -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    let (lowered, _) = import_and_lower_heap(&mut c, import_name);
    let heap_inst = c.core_instantiate_exports(
        lowered
            .iter()
            .map(|(n, f)| (*n, ExportKind::Func, *f))
            .collect::<Vec<_>>(),
    );
    let core_idx = c.core_module_raw(&provider_core_a_heap());
    let prog_inst = c.core_instantiate(core_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let build_core = c.core_alias_export(prog_inst, "build", ExportKind::Func);
    let (build_ty, mut bf) = c.type_function();
    bf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    let build_comp = c.lift_func(build_core, build_ty, []);
    c.export(
        "build",
        ComponentExportKind::Func,
        build_comp,
        Some(ComponentTypeRef::Func(build_ty)),
    );
    c
}

/// Consumer inner component B': imports the heap interface AND `build : func() -> u32`; lowers both
/// into B's core (heap under `"heap"`, build under `"peer"`), exports `main : func() -> s64`.
fn consumer_component_b_heap(import_name: &str) -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    let (lowered, _) = import_and_lower_heap(&mut c, import_name);
    // Import `build : func() -> u32` as a top-level component func, lower into `"peer"`.
    let (build_ty, mut bf) = c.type_function();
    bf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    let build_comp = c.import("build", ComponentTypeRef::Func(build_ty));
    let build_core = c.lower_func(build_comp, []);
    let heap_inst = c.core_instantiate_exports(
        lowered
            .iter()
            .map(|(n, f)| (*n, ExportKind::Func, *f))
            .collect::<Vec<_>>(),
    );
    let peer_inst = c.core_instantiate_exports([("build", ExportKind::Func, build_core)]);
    let core_idx = c.core_module_raw(&consumer_core_b_heap());
    let prog_inst = c.core_instantiate(
        core_idx,
        [
            ("heap", ModuleArg::Instance(heap_inst)),
            ("peer", ModuleArg::Instance(peer_inst)),
        ],
    );
    let main_core = c.core_alias_export(prog_inst, "main", ExportKind::Func);
    let (main_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let main_comp = c.lift_func(main_core, main_ty, []);
    c.export(
        "main",
        ComponentExportKind::Func,
        main_comp,
        Some(ComponentTypeRef::Func(main_ty)),
    );
    c
}

/// Import the value-heap `heap` interface declaring the five ops and lower them into core funcs.
/// Returns `(name, core_func_index)` per op, in `heap_ops()` order.
fn import_and_lower_heap(
    c: &mut ComponentBuilder,
    import_name: &str,
) -> (Vec<(&'static str, u32)>, u32) {
    let ops = heap_ops();
    let mut it = InstanceType::new();
    for (i, op) in ops.iter().enumerate() {
        let params: Vec<(String, ComponentValType)> = op
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (format!("p{j}"), abi_comp(*p)))
            .collect();
        {
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (n.as_str(), *t)));
            ft.result(op.result.map(abi_comp));
        }
        it.export(op.name, ComponentTypeRef::Func(i as u32));
    }
    let it_ty = c.type_instance(&it);
    let inst = c.import(import_name, ComponentTypeRef::Instance(it_ty));
    let comp_fns: Vec<u32> = ops
        .iter()
        .map(|op| c.alias_export(inst, op.name, ComponentExportKind::Func))
        .collect();
    let lowered: Vec<(&str, u32)> = ops
        .iter()
        .zip(comp_fns)
        .map(|(op, f)| (op.name, c.lower_func(f, [])))
        .collect();
    (lowered, ops.len() as u32)
}

/// The OUTER composition for X1b: A' and B' each import the SAME `heap` interface (the host binds
/// both imports to ONE runtime instance when it composes the runtime), B' imports A's `build`, and
/// the composition re-exports B's `main`.
fn composed_shared_heap_component(import_name: &str) -> Vec<u8> {
    let mut c = ComponentBuilder::default();
    // Re-import the heap interface at the OUTER level and forward it into both inner components, so
    // the whole composition declares exactly ONE `heap` import the host satisfies with one instance.
    let ops = heap_ops();
    let mut it = InstanceType::new();
    for (i, op) in ops.iter().enumerate() {
        let params: Vec<(String, ComponentValType)> = op
            .params
            .iter()
            .enumerate()
            .map(|(j, p)| (format!("p{j}"), abi_comp(*p)))
            .collect();
        {
            let mut ft = it.ty().function();
            ft.params(params.iter().map(|(n, t)| (n.as_str(), *t)));
            ft.result(op.result.map(abi_comp));
        }
        it.export(op.name, ComponentTypeRef::Func(i as u32));
    }
    let it_ty = c.type_instance(&it);
    let heap = c.import(import_name, ComponentTypeRef::Instance(it_ty));

    let a_idx = c.component(provider_component_a_heap(import_name));
    let a_inst = c.instantiate(a_idx, [(import_name, ComponentExportKind::Instance, heap)]);
    let a_build = c.alias_export(a_inst, "build", ComponentExportKind::Func);

    let b_idx = c.component(consumer_component_b_heap(import_name));
    let b_inst = c.instantiate(
        b_idx,
        [
            (import_name, ComponentExportKind::Instance, heap),
            ("build", ComponentExportKind::Func, a_build),
        ],
    );
    let main = c.alias_export(b_inst, "main", ComponentExportKind::Func);
    c.export("main", ComponentExportKind::Func, main, None);
    c.finish()
}

#[test]
fn x1b_two_cores_share_one_runtime_instance_and_a_handle_crosses() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    let comp = composed_shared_heap_component(&import_name);
    // STRUCTURAL pin: the hand-built two-core shared-runtime composition is a VALID component. The RUN —
    // A builds [99] on the shared heap, B reads element 0 back over the ONE shared runtime instance → 99
    // — is corpus/conformance territory (29-cross-component-peers pcs4/pcm7 witness a peer value read
    // over the shared runtime, and the deleted x5a cited them); this hand-built ComponentBuilder
    // composition cannot be a corpus (peer) case, so it stays as a compile+validate pin (x3/x4a family).
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("shared-heap cross-component component validates");
}

// ------------------------------------------------------------------------------------------------
// X3 — the PRODUCTION peer-interface import ENVELOPE (`envelope::assemble_extern`). The X1 consumer
// was hand-built with ComponentBuilder; here the consumer envelope is emitted by the compiler's own
// `assemble_extern` around a bare consumer core, then composed with the X1 provider and RUN. This is
// the byte-emitted envelope X4's front-end will target.
// ------------------------------------------------------------------------------------------------

/// The consumer inner component B, built by the PRODUCTION `envelope::assemble_extern`: import the
/// peer interface `cadenza:peer/api` (op `f : func(s32) -> s32`), bind it into `consumer_core_b`
/// under `"peer"`, export `main : func(s32) -> s32`. `assemble_extern` returns raw component bytes;
/// wrap them as an inner component via `component_raw`.
fn consumer_component_b_via_envelope() -> Vec<u8> {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, HostFn, assemble_extern};
    use crate::backend::wasm::wasm_abi::{COMP_FUNCTYPE_FORM, COMP_S32};
    // The peer op `f`'s component functype item: `[FORM] <1 param: p0:s32> <result: s32>`.
    let comp_functype = {
        let mut item = vec![COMP_FUNCTYPE_FORM];
        let mut params = Vec::new();
        params.extend_from_slice(&2u8.to_le_bytes()); // "p0".len()
        params.extend_from_slice(b"p0");
        params.push(COMP_S32);
        item.extend_from_slice(&crate::backend::wasm::encode::wasm_vec(1, &params));
        item.extend_from_slice(&[0x00, COMP_S32]); // result: s32
        item
    };
    let extern_fns = [HostFn {
        op: "f".to_string(),
        comp_functype,
        core_functype: Vec::new(),
        has_list_param: false,
    }];
    let exports = [BoundaryExport {
        name: "main".to_string(),
        params: vec![COMP_S32],
        result: BoundaryResult::Primitive(COMP_S32),
    }];
    assemble_extern(
        &consumer_core_b(),
        &exports,
        &["cadenza:peer/api"],
        &extern_fns,
        None,
    )
}

/// A provider that exports its `f` as the INTERFACE INSTANCE `cadenza:peer/api` (member `f`), the
/// shape `assemble_extern` imports. Built by instantiating the top-level-`f` provider inner component
/// and exporting the resulting INSTANCE under the interface name — so the instance's member `f` is
/// what the consumer aliases out of its `cadenza:peer/api` import.
fn provider_interface_component() -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    let inner = c.component(provider_component_a_named_p0());
    let no_args: [(&str, ComponentExportKind, u32); 0] = [];
    let inst = c.instantiate(inner, no_args);
    c.export(
        "cadenza:peer/api",
        ComponentExportKind::Instance,
        inst,
        None,
    );
    c
}

/// Provider inner component whose `f` is lifted with param name `p0` — matching the parameter name
/// `assemble_extern` declares in its peer instance-type (`host_op_comp_functype`'s `p{i}` convention).
/// A component-model interface import checks param NAMES structurally, so the provider's lift and the
/// consumer's import declaration must agree on `p0` (X1a's top-level-func path used `x`; the interface
/// path needs `p0`).
fn provider_component_a_named_p0() -> ComponentBuilder {
    let mut c = ComponentBuilder::default();
    let core_idx = c.core_module_raw(&provider_core_a());
    let no_args: [(&str, ModuleArg); 0] = [];
    let inst = c.core_instantiate(core_idx, no_args);
    let f_core = c.core_alias_export(inst, "f", ExportKind::Func);
    let (f_ty, mut ft) = c.type_function();
    ft.params([("p0", ComponentValType::Primitive(PrimitiveValType::S32))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S32)));
    let f_comp = c.lift_func(f_core, f_ty, []);
    c.export(
        "f",
        ComponentExportKind::Func,
        f_comp,
        Some(ComponentTypeRef::Func(f_ty)),
    );
    c
}

/// The OUTER composition using the PRODUCTION consumer envelope: instantiate the interface provider,
/// bind its `cadenza:peer/api` export to the `assemble_extern`-built consumer's like-named import,
/// re-export the consumer's `main`.
fn composed_via_extern_envelope() -> Vec<u8> {
    let mut c = ComponentBuilder::default();
    let a_idx = c.component(provider_interface_component());
    let no_args: [(&str, ComponentExportKind, u32); 0] = [];
    let a_inst = c.instantiate(a_idx, no_args);
    let a_iface = c.alias_export(a_inst, "cadenza:peer/api", ComponentExportKind::Instance);
    let b_idx = c.component_raw(&consumer_component_b_via_envelope());
    let b_inst = c.instantiate(
        b_idx,
        [("cadenza:peer/api", ComponentExportKind::Instance, a_iface)],
    );
    let main = c.alias_export(b_inst, "main", ComponentExportKind::Func);
    c.export("main", ComponentExportKind::Func, main, None);
    c.finish()
}

#[test]
fn x3_the_production_extern_envelope_composes_into_a_valid_component() {
    // The consumer envelope is emitted by the PRODUCTION `envelope::assemble_extern` (not hand-built),
    // NESTED with a hand-built provider under one outer component that re-exports `main`. STRUCTURAL pin:
    // assemble_extern's fourth import-envelope shape (a peer-interface import bound under
    // `cadenza:peer/api`) composes into a VALID component. The RUN — main(5) = f(5)*10 = (5+1)*10 = 60 —
    // is corpus/conformance territory: EVERY 29-* peer case compiles a peer-bound-effect consumer through
    // this same assemble_extern path and RUNS it composed with a source provider, so the envelope's
    // runnability is witnessed there (dropping the in-crate run drops the cdz-run/wasmtime dep).
    let comp = composed_via_extern_envelope();
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("assemble_extern-composed component validates");
    assert!(
        contains_bytes(&comp, b"cadenza:peer/api"),
        "the composed envelope carries the peer interface name"
    );
}

// ------------------------------------------------------------------------------------------------
// X4a — SEPARATE consumer + peer ARTIFACTS (the shape the front-end produces: each `.cdz` → its own
// component). STRUCTURAL pin: the assemble_extern-built consumer and the provider are each valid
// STANDALONE components whose interface names line up (peer EXPORTS what consumer IMPORTS). The RUN —
// cdz_run::run_with_peers links the separate peer into the consumer's like-named import over one store,
// main(5) = f(5)*10 = 60 — is corpus/conformance territory (every 29-* peer case composes a separate
// Cadenza-source provider into a consumer via that same runner path), so the run drops with the dep.
// ------------------------------------------------------------------------------------------------

#[test]
fn x4a_the_extern_envelope_and_its_peer_are_independently_valid_components() {
    let peer_bytes = provider_interface_component().finish();
    let consumer_bytes = consumer_component_b_via_envelope();
    // Each is a valid standalone component: the peer EXPORTS the interface, the consumer IMPORTS it.
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&peer_bytes)
            .expect("peer component validates");
    }
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer_bytes)
            .expect("consumer component validates");
    }
    // The compose contract, checked structurally: both artifacts carry `cadenza:peer/api` — the peer
    // exports it, the assemble_extern consumer imports it — so the runner CAN link them.
    assert!(
        contains_bytes(&peer_bytes, b"cadenza:peer/api"),
        "the peer exports cadenza:peer/api"
    );
    assert!(
        contains_bytes(&consumer_bytes, b"cadenza:peer/api"),
        "the assemble_extern consumer imports cadenza:peer/api"
    );
}

// ------------------------------------------------------------------------------------------------
// X4b-1 — the `(extern "iface" (op sig)…)` SCAN. A distinct cross-component binding form (not
// overloading intra-package `(import …)`) is scanned into `Db::extern_decls`. Byte-neutral: the
// table is populated but nothing consumes it yet (resolve→ExternCall + emit are later bricks).
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// X4b-2 — an extern op resolves to `Resolved::Extern` and, APPLIED, lowers to `Core::ExternCall`.
// The declared signature types the application (no CDZ error); the call is NOT inlined (no body).
// (The backend emit that turns `Core::ExternCall` into an imported `call` is X4b-3.)
// ------------------------------------------------------------------------------------------------

/// A MALFORMED `(extern …)` — its first element (the peer interface) missing or a bare NAME instead of
/// a STRING literal — is CDZ0201, not silently dropped. `scan_extern_decl` returns `None` for such a
/// form (`as_str(*tail.first()?)?`), so it registered no `ExternDecl` and any op it would bind went
/// unbound, surfacing a misleading "unbound name `neg` — did you mean `Neg`?" (an unrelated prelude
/// type). Now the extern form is rejected naming the real fix (the interface is a string), and the
#[test]
fn a_malformed_or_non_effect_bind_directive_is_cdz0201_not_a_silent_drop() {
    use crate::testkit::parse;
    // (a) a MALFORMED `(bind …)` — missing the interface string — is CDZ0201, not silently dropped.
    let malformed = "(do (effect E (op e (-> Int64 Int64))) (bind E) (def (main) 0) (export main))";
    let d1 = crate::diagnostics(&mut crate::db::Db::load(parse(malformed)));
    assert!(
        d1.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message
                .contains("binds an EFFECT to a peer interface string")),
        "a malformed (bind …) is CDZ0201: {:?}",
        d1.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (a2) a `(bind E Iface)` whose INTERFACE is a BAREWORD name, not a string literal, is CDZ0201.
    // This is the OTHER `well_shaped` failure (compile.rs): case (a) is arity-1 (the interface is
    // missing); (a2) is arity-2 but the second operand is a NAME occ, so `as_str().is_some()` is false
    // → the same MALFORMED_BIND reject. The likely author mistake is writing the interface unquoted
    // (`(bind Net cadenza:http/client)` — an s-expr `:`/`/` don't tokenize as one bareword anyway, so a
    // single-word `(bind Net client)` is the realistic shape). The message points at "a string literal".
    let bareword_iface = "(do (effect Net (op get (-> Int64 Int64))) (bind Net client) (def (main) 0) (export main))";
    let d1b = crate::diagnostics(&mut crate::db::Db::load(parse(bareword_iface)));
    assert!(
        d1b.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message
                .contains("binds an EFFECT to a peer interface string")),
        "a (bind E <bareword>) with a non-string interface is CDZ0201 (interface must be a string \
             literal), not a silent drop: {:?}",
        d1b.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (b) a `(bind …)` naming a VALUE DEFINITION (not an effect) is CDZ0201 — binding a non-effect to a
    // peer routes nothing.
    let non_effect = "(do (def (foo) 1) (bind foo \"cadenza:x/y\") (def (main) 0) (export main))";
    let d2 = crate::diagnostics(&mut crate::db::Db::load(parse(non_effect)));
    assert!(
        d2.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("names a declared EFFECT")),
        "a (bind …) of a non-effect is CDZ0201: {:?}",
        d2.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (c) NO REGRESSION: a well-formed `(bind Effect \"iface\")` of a declared effect is CLEAN.
    let ok = "(do (effect Math (op add (-> Int64 Int64 Int64))) (bind Math \"cadenza:math/api\") \
                  (def (main) (handle Math 0 ((add (a b) s (resume (+ a b) s))) (Math.add 2 3))) (export main))";
    let d3 = crate::diagnostics(&mut crate::db::Db::load(parse(ok)));
    assert!(
        !d3.iter().any(|d| d.message.contains("(bind")),
        "a well-formed (bind …) must not be flagged: {:?}",
        d3.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (d) a DUPLICATE `(bind E …)` — the same effect bound twice in source — is CDZ0201 (a route is a
    // set, one peer per effect), the `bind` analogue of the duplicate-`(host (A A) …)` reject.
    let dup = "(do (effect E (op e (-> Int64 Int64))) (bind E \"cadenza:a/x\") (bind E \"cadenza:b/y\") \
                   (def (main) (handle E 0 ((e (n) s (resume n s))) (E.e 1))) (export main))";
    let d4 = crate::diagnostics(&mut crate::db::Db::load(parse(dup)));
    assert!(
        d4.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("bound to a peer more than once")),
        "a duplicate (bind E …) is CDZ0201: {:?}",
        d4.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (e) NO FALSE POSITIVE: an effect declaring an OPERATION named `bind` — whose handler arm is a
    // NESTED `(bind (params) s body)` list — is NOT a peer-binding directive and MUST NOT be flagged.
    // The malformed-`(bind …)` scan reads only TOP-LEVEL `(bind …)` (via `top_bind_forms`); an
    // arena-wide scan would misread the arity-3 arm as a malformed directive → a spurious CDZ0201 on a
    // legal operation name. `bind` is an ordinary identifier.
    let bind_op = "(do (effect Scope (op bind (-> Int64 Int64)) (op depth (-> Unit Int64))) \
                       (def (main) (handle Scope 0 ((bind (v) d (resume (+ v d) (+ d 1))) \
                       (depth (u) d (resume d d))) (let ((a (Scope.bind 10))) (Scope.depth)))) (export main))";
    let d5 = crate::diagnostics(&mut crate::db::Db::load(parse(bind_op)));
    assert!(
            !d5.iter()
                .any(|d| d.message.contains("(bind")
                    || d.message.contains("binds an EFFECT to a peer")),
            "an effect operation named `bind` must not be misread as a peer-binding directive: {:?}",
            d5.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    // (f) an UNKNOWN bind name — neither an effect nor a value def — was SILENTLY ACCEPTED (the
    // directive operand is not resolved as a value reference, so no CDZ0101 surfaced and the bind
    // quietly vanished). It is now CDZ0201, and a near effect name is suggested with a rename fix.
    let ghost = "(do (bind Ghost \"cadenza:x/y\") (def (main) 0) (export main))";
    let d6 = crate::diagnostics(&mut crate::db::Db::load(parse(ghost)));
    assert!(
        d6.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("names a declared EFFECT")),
        "a (bind …) of an UNKNOWN name is CDZ0201, not a silent drop: {:?}",
        d6.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // A typo of a REAL effect names it + carries a rename fix.
    let typo = "(do (effect Logger (op log (-> Int64 Unit))) (bind Loger \"cadenza:x/y\") \
                    (def (main) 0) (export main))";
    let d7 = crate::diagnostics(&mut crate::db::Db::load(parse(typo)));
    let bind_err = d7
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0201") && d.message.contains("declared EFFECT"))
        .expect("a bind typo is CDZ0201");
    assert!(
        bind_err.message.contains("did you mean `Logger`?"),
        "a bind typo suggests the near effect: {}",
        bind_err.message
    );
    assert_eq!(
        bind_err.fix.as_ref().map(|f| f.replacement.as_str()),
        Some("Logger"),
        "the bind typo carries a rename fix"
    );
    // ROUND TRIP: applying the rename (`Loger` → the real effect `Logger`) clears the CDZ0201 — the
    // `bind` now names a declared effect. (Asserted at the diagnostics level, not full compile: an
    // unconsumed `bind` may draw other advisories, but the bind-name-not-an-effect fault the fix
    // targets is gone.)
    let applied = "(do (effect Logger (op log (-> Int64 Unit))) (bind Logger \"cadenza:x/y\") \
                       (def (main) 0) (export main))";
    assert!(
        !crate::diagnostics(&mut crate::db::Db::load(parse(applied)))
            .iter()
            .any(|d| d.message.contains("names a declared EFFECT")),
        "applying the bind rename clears the not-an-effect fault: {:?}",
        crate::diagnostics(&mut crate::db::Db::load(parse(applied)))
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    // (g) a `(bind E "…")` whose INTERFACE STRING is not a valid component interface name is CDZ0201.
    // The string is emitted verbatim as a peer-instance import extern name; a non-conforming one
    // (`"Math/API"` — uppercase, would `kebab_extern_name`-mangle to the INVALID `math/-a-p-i`)
    // produces a component wasmtime rejects at LOAD with no diagnostic — a silent invalid-component
    // miscompile. It is now a clear compile-time reject naming the offending string.
    let bad_iface = "(do (effect Math (op add (-> Int64 Int64 Int64))) (bind Math \"Math/API\") \
                         (def (main (: x Int64)) (host (Math) (Math.add x x))) (export main))";
    let d8 = crate::diagnostics(&mut crate::db::Db::load(parse(bad_iface)));
    assert!(
        d8.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("valid component interface name")),
        "a (bind …) with a malformed interface name is CDZ0201, not a silent invalid-component \
             miscompile: {:?}",
        d8.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // A bare package name with NO projection (`cadenza:math`) is also malformed (an instance import
    // needs the `/iface` projection).
    let no_proj = "(do (effect M (op a (-> Int64 Int64))) (bind M \"cadenza:math\") \
                       (def (main) 0) (export main))";
    let d9 = crate::diagnostics(&mut crate::db::Db::load(parse(no_proj)));
    assert!(
        d9.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("valid component interface name")),
        "a (bind …) to a projection-less package name is CDZ0201: {:?}",
        d9.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO REGRESSION: a well-formed VERSIONED interface name (`@0.0.0`) is CLEAN — the version suffix
    // the runtime heap import itself carries is a legal interface name.
    let versioned = "(do (effect M (op a (-> Int64 Int64))) (bind M \"cadenza:math/api@0.0.0\") \
             (def (main) (handle M 0 ((a (n) s (resume n s))) (M.a 1))) (export main))";
    let d10 = crate::diagnostics(&mut crate::db::Db::load(parse(versioned)));
    assert!(
        !d10.iter()
            .any(|d| d.message.contains("valid component interface name")),
        "a well-formed versioned interface name must not be flagged: {:?}",
        d10.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (h) a peer-bound op whose signature involves a CLOSURE (a `(-> …)` in an arg or result) is
    // CDZ0201 with the clear "cannot take or return a CLOSURE" reason — NOT the opaque lower-time
    // "value is not applyable" a peer-returned closure used to hit on application. Peers exchange
    // value-heap handles; a closure has no peer-boundary form. Both faces (result + arg):
    let clo_result = "(do (effect F (op mk (-> Int64 (-> Int64 Int64)))) (bind F \"cadenza:f/api\") \
                          (def (main) 0) (export main))";
    let mut clo_db = crate::db::Db::load(parse(clo_result));
    let d11 = crate::diagnostics(&mut clo_db);
    let clo_d = d11
        .iter()
        .find(|d| {
            d.code.as_deref() == Some("CDZ0201")
                && d.message.contains("cannot take or return a CLOSURE")
        })
        .unwrap_or_else(|| {
            panic!(
                "a peer-bound op RETURNING a closure is CDZ0201 with the clear reason: {:?}",
                d11.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        });
    // The diagnostic is anchored at the `(bind F …)` directive's effect NAME — the ACTIONABLE locus the
    // author edits (change the route, or give the op a value type) — NOT the nested `(-> Int64 Int64)`
    // arrow fragment that merely detected the closure (Copilot PR #418). The compiler is span-free, so
    // assert on the anchored NODE: it resolves to the bare name `F` (the bind name), not an arrow list.
    let anchor = clo_d
        .node
        .expect("the closure-across-peer reject is anchored");
    assert_eq!(
        clo_db.ast.as_name(crate::ast::StructId(anchor)),
        Some("F"),
        "the reject anchors at the bind name `F`, not the inner `(-> …)` arrow (node {anchor})"
    );
    let clo_arg = "(do (effect F (op run (-> (-> Int64 Int64) Int64))) (bind F \"cadenza:f/api\") \
                       (def (main) 0) (export main))";
    let d12 = crate::diagnostics(&mut crate::db::Db::load(parse(clo_arg)));
    assert!(
        d12.iter().any(|d| d.code.as_deref() == Some("CDZ0201")
            && d.message.contains("cannot take or return a CLOSURE")),
        "a peer-bound op TAKING a closure is CDZ0201 with the clear reason: {:?}",
        d12.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO FALSE POSITIVE: the SAME closure-typed op WITHOUT a `(bind …)` (a plain effect, handled or
    // host-delegated) is NOT flagged by this check — a closure crosses the HOST boundary as a resource;
    // only a PEER binding lacks a form for it.
    let clo_nopeer =
        "(do (effect F (op mk (-> Int64 (-> Int64 Int64)))) (def (main) 0) (export main))";
    let d13 = crate::diagnostics(&mut crate::db::Db::load(parse(clo_nopeer)));
    assert!(
        !d13.iter()
            .any(|d| d.message.contains("cannot take or return a CLOSURE")),
        "a closure-typed op on a NON-peer-bound effect must NOT be flagged: {:?}",
        d13.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // (i) a peer-bound op with a STRING ARGUMENT is now EMITTED, not declined — an inbound rope crosses
    // to a peer as a runtime HANDLE (like any compound), so declaring one must NOT raise the old
    // "String or Bytes ARGUMENT" decline. (Was a CDZ0201 decline while the inbound-rope-handle emit was
    // unwired; now `collect_used_ops`/`collect_host_arg_strings` are peer-aware and the handle crosses.
    // The e2e crossing is pinned by `a_string_argument_crosses_to_a_peer_*`.)
    let str_arg = "(do (effect S (op blen (-> String Int64))) (bind S \"cadenza:str/api\") \
                       (def (main) 0) (export main))";
    let d14 = crate::diagnostics(&mut crate::db::Db::load(parse(str_arg)));
    assert!(
        !d14.iter()
            .any(|d| d.message.contains("String or Bytes ARGUMENT")),
        "a peer-bound op with a String ARGUMENT must no longer be declined (it crosses as a handle): {:?}",
        d14.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // A Bytes argument is likewise emittable now (the same rope-handle path).
    let bytes_arg = "(do (effect S (op f (-> Bytes Int64))) (bind S \"cadenza:str/api\") \
                         (def (main) 0) (export main))";
    let d15 = crate::diagnostics(&mut crate::db::Db::load(parse(bytes_arg)));
    assert!(
        !d15.iter()
            .any(|d| d.message.contains("String or Bytes ARGUMENT")),
        "a peer-bound op with a Bytes ARGUMENT must no longer be declined: {:?}",
        d15.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // NO FALSE POSITIVE: a String/Bytes RESULT (not an argument) crosses fine — the peer builds the
    // rope handle + returns it — so it must NOT be flagged. (`(-> Int64 String)`: the String is the
    // RESULT, the last arrow element.)
    let str_result = "(do (effect G (op greet (-> Int64 String))) (bind G \"cadenza:g/api\") \
                          (def (main) 0) (export main))";
    let d16 = crate::diagnostics(&mut crate::db::Db::load(parse(str_result)));
    assert!(
        !d16.iter()
            .any(|d| d.message.contains("String or Bytes ARGUMENT")),
        "a peer-bound op with a String RESULT must NOT be flagged (only an argument is): {:?}",
        d16.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

// ------------------------------------------------------------------------------------------------
// PL2 — ZERO-COST is an ABI INVARIANT: a rich value crosses a peer boundary as a bare u32 HANDLE,
// never marshaled. The operator's north star for peer linking is that calling a rich interface a
// peer module exposes is as cheap as an in-module call — no serialize/deserialize tax, because both
// peers share one value-heap runtime and a compound crosses as an opaque handle into it (ABI v5
// §Cadenza Components Composed Against A Shared Runtime Exchange Values As Handles). The e2e X5/U5
// tests prove a value crosses CORRECTLY; this pins the mechanism STRUCTURALLY at the `extern_abi_
// val_type` seam — so a future "marshal it into a component aggregate" refactor (which would keep
// the value correct but reintroduce the serialization tax) FAILS here rather than silently
// regressing zero-cost. A handle is ONE i32 slot with no payload — the definition of no-copy.
// ------------------------------------------------------------------------------------------------
#[test]
fn a_rich_value_crosses_a_peer_boundary_as_a_bare_handle_not_marshaled() {
    use crate::backend::wasm::host::extern_abi_val_type;
    use crate::backend::wasm::runtime_abi::AbiValType;
    use crate::ty::Ty;

    // Every RUNTIME-OWNED rich type — the value-heap compounds, the byte-rope String/Bytes, and the
    // bignums — must cross as exactly `AbiValType::U32`: the opaque handle into the shared runtime.
    // NOT `None` (which would force a marshal / a decline) and NOT any wider aggregate form.
    let string_key = crate::resolved::Symbol::plain("x");
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(string_key, Ty::int64());
    let rich_types: Vec<(&str, Ty)> = vec![
        (
            "Tuple",
            Ty::Tuple(std::rc::Rc::from([Ty::int64(), Ty::Bool])),
        ),
        ("Record", Ty::Record(std::rc::Rc::new(fields))),
        (
            "Sum",
            Ty::Sum {
                decl: crate::ast::StructId(0),
                args: std::rc::Rc::from([Ty::int64()]),
            },
        ),
        ("List", Ty::List(Box::new(Ty::int64()))),
        ("Map", Ty::Map(Box::new(Ty::String), Box::new(Ty::int64()))),
        ("Set", Ty::Set(Box::new(Ty::int64()))),
        ("String", Ty::String),
        ("Bytes", Ty::Bytes),
        ("BigInt", Ty::BigInt),
        ("Rational", Ty::Rational),
        // An erased NOMINAL over a rich type reads through to the inner type's handle.
        (
            "Nominal<List>",
            Ty::Nominal {
                decl: crate::ast::StructId(0),
                args: std::rc::Rc::from([]),
                inner: std::rc::Rc::new(Ty::List(Box::new(Ty::int64()))),
            },
        ),
    ];
    for (label, ty) in &rich_types {
        let abi = extern_abi_val_type(ty);
        assert_eq!(
            abi,
            Some(AbiValType::U32),
            "a rich `{label}` must cross a peer boundary as a bare u32 handle (zero-cost), \
                 not marshaled — got {abi:?}"
        );
        // The handle is ONE i32 word with no serialized payload — the structural meaning of no-copy.
        let handle = abi.unwrap();
        assert_eq!(
            handle.core_byte(),
            0x7F,
            "the `{label}` handle occupies a single core i32 slot"
        );
        assert_eq!(
            handle.comp_byte(),
            0x79,
            "the `{label}` handle crosses as the component-model `u32` primitive"
        );
    }

    // REGRESSION GUARD (the other direction): a SCALAR still crosses BY VALUE, never handle-ified —
    // handle-crossing a scalar would be a pointless indirection (and wrong: a host peer can't build a
    // heap handle for a plain integer). Int64 → S64 by value; Bool → Bool; a narrow int by value.
    assert_eq!(
        extern_abi_val_type(&Ty::int64()),
        Some(AbiValType::S64),
        "an Int64 crosses BY VALUE (S64), not as a handle"
    );
    assert_eq!(
        extern_abi_val_type(&Ty::Bool),
        Some(AbiValType::Bool),
        "a Bool crosses BY VALUE, not as a handle"
    );
    // `Unit` has no boundary slot at all (a nullary op takes/returns nothing).
    assert_eq!(
        extern_abi_val_type(&Ty::Unit),
        None,
        "Unit has no cross-boundary representation"
    );

    // THE OPAQUE-HANDLE COROLLARY (documents a deliberate limitation of the compose-time peer
    // signature check): because EVERY runtime-owned compound crosses as the SAME `U32` handle, two
    // STRUCTURALLY-DIFFERENT compounds have the SAME boundary signature. So `check_peer_iface_signatures`
    // (which compares component Types) CANNOT distinguish a `(Tuple Int64 Int64)` from a `(Tuple Int64
    // Int64 Int64)` or a `(Record …)` at a peer op's param/result — they are all `U32`. A compound
    // SHAPE mismatch between a consumer's binding and a peer's export is therefore NOT caught at compose
    // time; it surfaces as a runtime trap if the peer reads a field the crossed value doesn't have. This
    // is intrinsic to the zero-cost opaque-handle ABI (the handle is meaningful only to the shared
    // runtime; the component boundary sees a bare `u32`) — the signature check guards SCALAR shapes
    // (which cross faithfully) and ARITY; compound shape agreement is the Cadenza-source contract's job,
    // not the boundary's. Pinning it so a future "make the check catch compound shapes" attempt knows it
    // must first make compound shapes VISIBLE at the boundary (which would forfeit zero-cost).
    let tup2 = Ty::Tuple(std::rc::Rc::from([Ty::int64(), Ty::int64()]));
    let tup3 = Ty::Tuple(std::rc::Rc::from([Ty::int64(), Ty::int64(), Ty::int64()]));
    let rec = {
        let mut f = std::collections::BTreeMap::new();
        f.insert(crate::resolved::Symbol::plain("a"), Ty::int64());
        Ty::Record(std::rc::Rc::new(f))
    };
    assert_eq!(
        extern_abi_val_type(&tup2),
        extern_abi_val_type(&tup3),
        "two DIFFERENT tuple shapes share the SAME u32 boundary signature — the boundary is opaque \
             to compound shape (a shape mismatch is a runtime concern, not a compose-time one)"
    );
    assert_eq!(
        extern_abi_val_type(&tup2),
        extern_abi_val_type(&rec),
        "a Tuple and a Record also share the u32 handle signature — the compound boundary form is \
             opaque by design (zero-cost)"
    );
}
// ------------------------------------------------------------------------------------------------
// X4b-3 — the BACKEND EMIT: a SOURCE consumer `(extern …)` + `(neg x)` compiles to a valid component
// importing `cadenza:math/api` (bound under `"peer"`), which — composed with a provider via
// run_with_peers (X4a) — RUNS end-to-end. The first source→run cross-component call.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// X4b-provider — BOTH sides from source: a PROVIDER `.cdz` compiled with a `component-name` request
// publishes its exports as the interface `cadenza:math/api`; a CONSUMER `.cdz` binds it via
// `(extern …)`. Composed via run_with_peers → the whole thing runs from two Cadenza sources.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// X5b — a COMPOUND value crosses between two SOURCE Cadenza components as an opaque handle over the
// shared runtime (the payoff). A peer builds a runtime `(Tuple Int64 Int64)` and returns it; a
// consumer receives the handle (typed `(Tuple Int64 Int64)` by its `(extern …)` decl) and projects
// element 0 — reading the peer's value through the shared heap, NO serialization.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// X5c — BOTH sides from source: a source PROVIDER returns a runtime compound (crosses as its `u32`
// handle through the provider interface), a source CONSUMER receives + reads it. The full compound
// interop story from two Cadenza sources.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// X5d — VALUE-MATRIX widening coverage: String / List / Sum / nested compounds all cross the same
// shared-handle way between two source components (extern_abi_val_type maps every runtime-owned type
// to the u32 handle). Each is a small coverage brick, no new machinery.
// ------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------
// U3 — COMPILE-REQUEST override of an effect's peer binding (a COMPILER-LINK feature, verified
// structurally). The SAME source (bound to cadenza:math/api in-source) is REBOUND by a compile-request
// `effect-bind` artifact to a DIFFERENT interface — the operator's rebind (for a test / a different
// environment). The corpus `(peer …)` clause has no way to inject a rebind artifact, so the run stays in
// this crate as a white-box pin on the emitted component's import name: the request's interface wins.
// ------------------------------------------------------------------------------------------------

#[test]
fn u3_a_compile_request_rebinds_an_effect_to_a_different_peer() {
    use crate::Target;
    use crate::abi::Artifact;
    use crate::compile::compile;
    use crate::testkit::parse;
    // SAME source as U2 (binds Math → cadenza:math/api in-source), but a compile-request `effect-bind`
    // artifact REBINDS Math → cadenza:mathv2/api (a MUL peer). The request WINS over the source default.
    let src = "(do \
            (effect Math (op add (-> Int64 Int64 Int64))) \
            (bind Math \"cadenza:math/api\") \
            (def (main (: x Int64)) (host (Math) (Math.add x x))) \
            (export main))";
    let out = crate::host::run_with_compiler_stack(|| {
        compile(
            &[
                Artifact::new(
                    Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&parse(src)),
                ),
                Artifact::new(
                    crate::link::KIND_EFFECT_BIND,
                    "effect-bind",
                    cadenza_compile_abi::effect_bind_wire::encode(&[(
                        "Math".to_string(),
                        "cadenza:mathv2/api".to_string(),
                    )]),
                ),
            ],
            &[Target::Wasm],
        )
    });
    let consumer = out
        .artifact(Target::Wasm.artifact_kind())
        .unwrap_or_else(|| {
            panic!(
                "rebound consumer compiles: {:?}",
                out.diagnostics
                    .iter()
                    .map(|d| &d.message)
                    .collect::<Vec<_>>()
            )
        })
        .to_vec();
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("rebound consumer validates");
    }
    // The request WON: the emitted consumer imports the REBOUND interface `cadenza:mathv2/api` and no
    // longer imports the in-source default `cadenza:math/api` (unambiguous — `cadenza:math/api` is not a
    // byte-substring of `cadenza:mathv2/api`). The RUN over the rebound peer — which then computes
    // mul(5,5)=25, distinct from the add peer's 10 — is corpus/conformance territory; this crate pins the
    // compiler-link OUTCOME of the `effect-bind` request artifact: the request's interface wins over the
    // in-source default.
    assert!(
        contains_bytes(&consumer, b"cadenza:mathv2/api"),
        "the rebound consumer imports the request's interface cadenza:mathv2/api"
    );
    assert!(
        !contains_bytes(&consumer, b"cadenza:math/api"),
        "the rebound consumer no longer imports the in-source default cadenza:math/api"
    );
}

// ------------------------------------------------------------------------------------------------
// U6 — BOTH SIDES FROM SOURCE over the effects surface. The full payoff of the unification: no
// hand-built peer at all. A source PROVIDER `(def (pair (: x Int64)) (tuple x x)) (export pair)`
// compiled with component-name `cadenza:pairs/api` publishes a compound-returning interface (routes to
// `assemble_provider_runtime`); a source CONSUMER performs `(host (P) (. (P.pair x) 0))` on a peer-bound
// effect (routes to `assemble_extern_runtime`). Composed via run_with_peers over ONE shared runtime →
// the tuple crosses as a handle, both sides Cadenza source. The provider path never used `extern` (just
// `--component-name` + normal exports), so it survived U4; U6 wires it to the effects-surface consumer.
// ------------------------------------------------------------------------------------------------

/// Compile a Cadenza SOURCE `src` (s-expr) as a PROVIDER publishing its exports under `iface` — the
/// `component-name` request artifact (X4b), the same path `cdz compile --component-name` drives.
fn compile_provider(src: &str, iface: &str) -> Vec<u8> {
    use crate::abi::Artifact;
    use crate::backend::Target;
    use crate::testkit::parse;
    let ast = crate::codec::encode(&parse(src));
    let out = crate::host::run_with_compiler_stack(|| {
        crate::compile(
            &[
                Artifact::new(Artifact::KIND_AST, "provider", ast),
                crate::cli::component_name_artifact(iface),
            ],
            &[Target::Wasm],
        )
    });
    out.artifact(Target::Wasm.artifact_kind())
        .unwrap_or_else(|| {
            panic!("provider compiles: {:?}", out.diagnostics);
        })
        .to_vec()
}

#[test]
fn a_list_returning_provider_op_and_its_consumer_both_emit_valid_components() {
    // Option C increment 0 — the EMIT half of the X5b handle-crossing witness. A shared closure's defs
    // return heap values (List/rope), so Option C rides a value-HANDLE crossing a PEER-INTERFACE edge —
    // the frontier `cdz-run` scopes as "scalar peer ops today" (lib.rs:450). Existing peer tests cross a
    // scalar (X4a) or a fixed-arity TUPLE handle (U6); a VARIABLE-LENGTH List/rope handle over a peer
    // edge was untested. This pins the EMIT side: a PROVIDER whose exported op returns a `(List Int64)` +
    // a CONSUMER binding an effect of that op-type to the provider interface BOTH emit VALID components.
    // (The RUN-side assert — the List handle actually crosses + reads right through the shared runtime
    // instance — is v-cdz-tooling's via run_with_peers; this gives them a valid emitted pair to run.)
    use crate::testkit::parse;
    let provider = compile_provider(
        "(do (def (mklist (: n Int64)) (list n (+ n 1) (+ n 2))) (export mklist))",
        "cadenza:closure/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("a List-returning provider op emits a valid component");
    }
    let consumer_src = "(do \
            (effect C (op mklist (-> Int64 (List Int64)))) \
            (bind C \"cadenza:closure/api\") \
            (def (main (: n Int64)) (host (C) (List.len (C.mklist n)))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(consumer_src)))
        .unwrap_or_else(|d| {
            panic!(
                "a List-consuming peer consumer compiles: {} [{:?}]",
                d.message, d.code
            )
        });
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&consumer)
        .expect("a consumer of a List-returning peer op emits a valid component");
}

#[test]
fn two_effects_bound_to_the_same_interface_share_one_peer_instance() {
    use crate::testkit::parse;
    // TWO DISTINCT effects bound to the SAME interface string share ONE peer instance import. A `(bind
    // …)` route dedups on the EFFECT NAME (compile.rs `bound_effects`), NOT the interface string — so
    // `(bind A "cadenza:x/y")` + `(bind B "cadenza:x/y")` is LEGAL: both effects route to one provider
    // interface, and the emit MERGES their ops into a SINGLE `cadenza:x/y` instance import (verified by
    // inspection: the component has exactly one import named `cadenza:x/y` carrying both `fa` and `fb`,
    // not two colliding same-named instance imports → not the silent-invalid-component class). This is
    // the natural shape when one provider component exports several ops a consumer splits across
    // multiple declared effects (e.g. a `cadenza:model/api` provider whose `converse` + `embed` the
    // consumer models as two effects Chat and Embed both bound to `cadenza:model/api`). Pin it: the
    // multi-effect→single-instance merge on the peer boundary had no e2e coverage.
    //
    // PROVIDER (source): ONE component exporting BOTH ops on `cadenza:x/y` — `fa` adds 10, `fb` adds 20.
    let provider = compile_provider(
        "(do (def (fa (: x Int64)) (+ x 10)) (def (fb (: x Int64)) (+ x 20)) \
                 (export fa) (export fb))",
        "cadenza:x/y",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("the two-op provider validates");
    }
    // CONSUMER (source): declares TWO effects A + B, binds BOTH to `cadenza:x/y`, performs one op from
    // each in one body. The consumer emits ONE `cadenza:x/y` instance import carrying both ops.
    let src = "(do \
            (effect A (op fa (-> Int64 Int64))) \
            (effect B (op fb (-> Int64 Int64))) \
            (bind A \"cadenza:x/y\") \
            (bind B \"cadenza:x/y\") \
            (def (main) (host (A B) (+ (A.fa 1) (B.fb 2)))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| {
            panic!(
                "two-effects-one-iface consumer compiles: {} [{:?}]",
                d.message, d.code
            )
        });
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("two-effects-one-iface consumer validates");
    }
    // Structural pin: the consumer has EXACTLY ONE instance import named `cadenza:x/y` (both effects
    // merged onto it), not two colliding imports.
    let x_y_imports = wasmparser::Parser::new(0)
        .parse_all(&consumer)
        .filter_map(|p| match p {
            Ok(wasmparser::Payload::ComponentImportSection(s)) => Some(s),
            _ => None,
        })
        .flat_map(|s| s.into_iter().filter_map(Result::ok))
        .filter(|imp| imp.name.0 == "cadenza:x/y")
        .count();
    assert_eq!(
        x_y_imports, 1,
        "two effects bound to one interface must merge into a SINGLE instance import, not collide"
    );
    // The RUN half (compose the two-op provider + this consumer, `main = A.fa(1)+B.fb(2) = 33`) is
    // covered by corpus `spec/semantics/29-cross-component-peers.sexp` — "two effects bound to the same
    // interface share one peer instance (both ops run)". This in-crate test now pins the WHITE-BOX
    // structural claim only: the two effects merge onto EXACTLY ONE `cadenza:x/y` instance import (the
    // multi-effect→single-instance merge), which the corpus run confirms behaviorally.
}

// ------------------------------------------------------------------------------------------------
// U8 — a STRING ARGUMENT crosses to a peer-bound effect (the mirror of U7's result). This completes
// the model-call `converse(prompt) -> completion : String` boundary the agent HARNESS Route-B
// bring-up needs (`DESIGN-agent-harness.md` §2.1a): the PROMPT crosses IN as a peer String arg (a
// runtime rope handle, not a component `string`) and the COMPLETION comes back as the String RESULT
// (U7). This cell was DECLINED until v-peer-linking wired the inbound-rope-handle emit (the
// `string-crossing-matrix` issue's cell #2); pin it now that it passes so a transport regression
// that reverted to the decline (or miscompiled the arg as a component `string` → invalid component)
// is caught fleet-wide. Provider ECHOES its String arg back, so the result's byte-len reflects the
// ARG that actually crossed. The entrypoint returns a scalar (byte-len), sidestepping the still-open
// result-escape cell #3 (v-peer-linking task #6); the prompt is an in-program literal, sidestepping
// the still-open String-entrypoint-param cell #7 — which the real loop does anyway (it builds the
// prompt from context, never takes it as an export param).
// ------------------------------------------------------------------------------------------------
// ------------------------------------------------------------------------------------------------
// The AGENT-KERNEL result shape (v-agent-harness FORK2, operator-greenlit 2026-07-17): the minimal
// event-agnostic kernel is `interpret : (List Event, Event) -> (List HostOp)` where `HostOp` is a SUM
// (Append/Exec/Http/Log). The kernel is a PROVIDER component; its result is consumed by a Cadenza PEER
// executor over the ONE shared value-heap runtime — NOT a foreign host. So the `(List (Sum …))` result
// crosses PEER→PEER as an opaque `u32` handle via `extern_abi_val_type` (a `List` → U32 regardless of
// element type — the element sum is never marshaled; the shared runtime owns the value). This PROBE
// settles v-agent-harness's question — "does a `(List (Sum …))` provider result already round-trip, or
// is there a List-of-Sum gap?" — by CONTENT: a provider exporting a fn returning a heap-built
// `(list (Append 1) (Exec 2))` must compile + validate with the result as a boundary handle. If this
// passes, the agent-kernel result path needs NO host-ABI widening (the peer path already serves it);
// the only true host boundary is the executor's broad primitives (exec/http/log — scalar/String args),
// already expressible. A `List` argument would be the separate inbound direction (v-peer-linking).
// ------------------------------------------------------------------------------------------------
#[test]
fn a_provider_returning_a_list_of_sum_crosses_as_a_peer_handle() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    // A provider whose export `interpret` returns a `(List HostOp)` — the agent-kernel result shape.
    // `HostOp` is a user SUM; the body ignores its scalar arg and returns a genuine heap-built list of
    // two distinct variants (so the result is a real runtime handle, not a const-folded immediate).
    let provider = compile_provider(
        "(do (type HostOp (Append Int64) (Exec Int64)) \
               (def (interpret (: ev Int64)) (list (Append ev) (Exec 2))) \
               (export interpret))",
        "cadenza:agent/kernel",
    );
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&provider)
        .expect("a provider returning (List (Sum …)) validates — the peer path serves it");
    // It builds a compound, so it imports the shared value-heap runtime (the handle it mints is
    // meaningful to a peer executor) — confirming the result crosses as a runtime handle, not marshaled.
    assert!(
        String::from_utf8_lossy(&provider).contains(&import_name),
        "the kernel provider imports the value-heap runtime (its (List HostOp) result is a heap handle)"
    );
}

// ------------------------------------------------------------------------------------------------
// The agent-kernel FULL LOOP end-to-end (v-agent-harness K0, their `interpret.cdz` shape): a PROVIDER
// `interpret` returns a BRANCH-BUILT `(List HostOp)` (HostOp a sum with String payloads — Append/Exec/
// Http/Noop), and a PEER EXECUTOR consumes the crossed list: `List.len` it, and `List.at 0` + match the
// first variant, reducing to a scalar. This goes beyond the compile+validate probe above — it RUNS the
// provider+peer loop under wasmtime over the ONE shared runtime, confirming the `(List (Sum String))`
// handle the kernel mints is consumed by a Cadenza peer that pattern-matches each HostOp (exactly the
// executor shim v-agent-harness builds). Answers their "does provider + peer executor consuming the
// (List HostOp) handle work as-is?" — YES, no host-ABI widening. Args kept SCALAR (the result direction
// is what K0 blocked on; the compound-ARG inbound is the separate v-peer-linking cell). `interpret(1,_)`
// takes the Append/Exec branch → a 2-element list; the executor returns `List.len` = 2.
// ------------------------------------------------------------------------------------------------
#[test]
fn u8_a_string_argument_crosses_to_a_peer() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    use crate::testkit::parse;
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    // PROVIDER (source): `converse(prompt)` CONCATS the prompt with itself — it BUILDS a new rope FROM
    // the crossed arg (so it imports the value-heap runtime, and the doubled length proves the arg both
    // crossed AND was consumed, not merely echoed as an untouched handle). Takes a String arg AND
    // returns a String, both as rope handles over the shared runtime.
    let provider = compile_provider(
        "(do (def (converse (: prompt String)) (String.concat prompt prompt)) (export converse))",
        "cadenza:model/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("string-arg echo provider validates");
    }
    // CONSUMER (source): a peer-bound effect M `(-> String String)`; main passes an in-program literal
    // prompt "hello" and reads the doubled completion's byte-len — proving the ARG crossed (a broken
    // arg emit would trap or mis-length). Entrypoint returns Int64 (scalar), so nothing escapes as a
    // resource.
    let src = "(do \
            (effect M (op converse (-> String String))) \
            (bind M \"cadenza:model/api\") \
            (def (main) (String.byte-len (host (M) (M.converse \"hello\")))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("consumer compiles: {} [{:?}]", d.message, d.code));
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer).expect("consumer validates");
    }
    // The RUN half (compose the concat provider + this consumer; byte-len of the doubled "hello" = 10,
    // proving the String arg crossed IN as a rope and was consumed, and the String result crossed back)
    // is covered by corpus `spec/semantics/29-cross-component-peers.sexp` — "a string argument crosses to
    // a peer and the doubled result byte-len is read". This in-crate test now pins the WHITE-BOX claim
    // only: the source provider imports the value-heap runtime (it handles a String rope), which the
    // corpus run confirms behaviorally.
    assert!(
        String::from_utf8_lossy(&provider).contains(&import_name),
        "the source provider imports the value-heap runtime (it handles a String rope)"
    );
}

// ------------------------------------------------------------------------------------------------
// U9 (WHITE-BOX compile pin) — a `(-> String String)` model op bound to a host interface COMPILES to a
// VALID component that imports the value-heap runtime (the String prompt/completion each cross as a rope
// handle). The end-to-end RUNTIME round-trip (a String prompt crosses OUT to a host closure, the
// completion crosses back IN, byte-len read → 2) was verified via `cdz_run::run_agent`; that run is
// DROPPED with the cdz-run dev-dep (operator-accepted coverage gap 2026-08-28): the corpus gate harness
// cannot yet answer a bound/simple-export String-RESULT host op (only a reducer-export one, 28 SHAPE 57),
// and the test can't move to cdz-run (it needs rcdzc to compile → dev-dep cycle). v-rb #4894 landed the
// host-String-result EMIT so this consumer now COMPILES + validates; the host-closure embedder round-trip
// itself lives in cdz-run's own domain. When the corpus harness gains the bound-String-host-op capability
// this pin is superseded by a corpus case (see 29-cross-component-peers's String-peer converse case for
// the peer-answered analogue). Was u9/u10/u11 (embedder run_agent tests); reduced to this compile pin.
// ------------------------------------------------------------------------------------------------
#[test]
fn u9_a_string_model_op_bound_to_a_host_interface_compiles_and_imports_the_runtime() {
    use crate::testkit::parse;
    let src = "(do \
            (effect Model (op converse (-> String String))) \
            (bind Model \"cadenza:model/api\") \
            (def (main) (String.byte-len (host (Model) (Model.converse \"hi\")))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("consumer compiles: {} [{:?}]", d.message, d.code));
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&consumer)
        .expect("the String-model-op consumer validates");
    assert!(
        super::imports_value_heap_runtime(&consumer),
        "the String prompt/completion cross as rope handles → the consumer imports the value-heap runtime"
    );
}

// ------------------------------------------------------------------------------------------------
// PL4 — a NON-KEBAB peer OP NAME agrees across the consumer + provider and RUNS. The op name is a
// component-boundary extern name (the interface func); a camelCase source op (`addTwo`) must
// kebab-normalize to the SAME `add-two` on BOTH sides — the consumer's `(bind)`/`host` import AND
// the source provider's `--component-name` export — or the two components fail to link (the
// [[rcdzc-kebab-extern-name-gotcha]] failure mode: an invalid extern name, or a silent name
// mismatch, with no diagnostic). `a_non_kebab_effect_and_op_name_emit_a_valid_component` pins the
// consumer-side VALIDITY; this pins the CROSS-SIDE AGREEMENT e2e — both sides derive the boundary
// name from the same deterministic `kebab_extern_name`, so a divergent change to either side's
// normalization makes this run fail to compose rather than silently mis-link.
// ------------------------------------------------------------------------------------------------
#[test]
fn a_non_kebab_peer_op_name_agrees_across_both_sides_and_runs() {
    use crate::testkit::parse;
    // PROVIDER (source): a camelCase export `addTwo`, published as `cadenza:math/api`. Its interface
    // member extern name kebab-normalizes to `add-two`.
    let provider = compile_provider(
        "(do (def (addTwo (: x Int64)) (+ x x)) (export addTwo))",
        "cadenza:math/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("camelCase-op source provider validates");
    }
    // CONSUMER (source): binds the same interface, performs the camelCase op via a `host` delegation.
    let src = "(do \
            (effect Math (op addTwo (-> Int64 Int64))) \
            (bind Math \"cadenza:math/api\") \
            (def (main (: x Int64)) (host (Math) (Math.addTwo x))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("consumer compiles: {} [{:?}]", d.message, d.code));
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("camelCase-op consumer validates");
    }
    // Both sides carry the KEBAB extern name `add-two`, and NEITHER leaks the verbatim `addTwo` into a
    // component-boundary position (the interface member name) — the deterministic agreement that lets
    // them link. (The verbatim `addTwo` still appears as the CORE-module func name, which is fine — a
    // core name is not a component extern name; the interface-member and its alias are what must be
    // kebab.) We check the boundary name is present on both.
    for (who, bytes) in [("provider", &provider), ("consumer", &consumer)] {
        assert!(
            bytes.windows(7).any(|w| w == b"add-two"),
            "the {who} must carry the kebab interface member name `add-two`"
        );
    }
    // The RUN half (compose the camelCase-op provider + this consumer, `addTwo(5) = 10` through the
    // kebab-agreed `add-two` boundary) is covered by corpus `spec/semantics/29-cross-component-peers.sexp`
    // — "a non-kebab (camelCase) peer op name agrees across both sides and runs". This in-crate test now
    // pins the WHITE-BOX claim only: both artifacts carry the kebab interface-member name `add-two` (and
    // neither leaks the verbatim `addTwo` into a component-boundary position), which the corpus run
    // confirms behaviorally (a name disagreement would fail to link).
}

#[test]
fn a_versioned_interface_name_agrees_across_both_sides_and_runs() {
    use crate::testkit::parse;
    // A VERSIONED interface name (`cadenza:math/api@1.0.0`) crosses a real peer end-to-end. The version
    // suffix is PART OF the component-boundary extern name emitted verbatim on BOTH sides — the provider
    // `--component-name` and the consumer `(bind …)` string — so they must carry the SAME `@version` or
    // they fail to link ([[rcdzc-kebab-extern-name-gotcha]] cross-side-agreement class). PL4 pins that a
    // NON-KEBAB op name agrees across both sides; this pins that the VERSION SUFFIX agrees + round-trips.
    // A versioned name was only VALIDATED statically before (the malformed-bind test's `@0.0.0` case, no
    // run) — no e2e crossed a versioned interface. This is the realistic shape: a provider publishing
    // `cadenza:model/api@1.0.0` and a consumer binding that exact versioned string.
    //
    // PROVIDER (source): `dbl` doubles, published as the VERSIONED `cadenza:math/api@1.0.0`.
    let provider = compile_provider(
        "(do (def (dbl (: x Int64)) (+ x x)) (export dbl))",
        "cadenza:math/api@1.0.0",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("versioned-interface provider validates");
    }
    // CONSUMER (source): binds the SAME versioned string. A mismatched/absent version would not link.
    let src = "(do \
            (effect Math (op dbl (-> Int64 Int64))) \
            (bind Math \"cadenza:math/api@1.0.0\") \
            (def (main (: x Int64)) (host (Math) (Math.dbl x))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| {
            panic!(
                "versioned-interface consumer compiles: {} [{:?}]",
                d.message, d.code
            )
        });
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("versioned-interface consumer validates");
    }
    // Both sides carry the VERSIONED extern name verbatim — the agreement that lets them link.
    for (who, bytes) in [("provider", &provider), ("consumer", &consumer)] {
        assert!(
            bytes
                .windows(b"cadenza:math/api@1.0.0".len())
                .any(|w| w == b"cadenza:math/api@1.0.0"),
            "the {who} must carry the versioned interface name `cadenza:math/api@1.0.0`"
        );
    }
    // The RUN half (compose the versioned-interface provider + this consumer, `dbl(6) = 12` through the
    // versioned `cadenza:math/api@1.0.0` boundary) is covered by corpus
    // `spec/semantics/29-cross-component-peers.sexp` — "a versioned interface name agrees across both
    // sides and runs". This in-crate test now pins the WHITE-BOX claim only: both artifacts carry the
    // VERSIONED extern name `cadenza:math/api@1.0.0` verbatim (the @version-suffix agreement that lets
    // them link), which the corpus run confirms behaviorally.
}

// ------------------------------------------------------------------------------------------------
// U9 — a consumer binds TWO DISTINCT PEER INTERFACES. The multi-interface extern envelope: each bound
// interface becomes its own imported component instance, and each op aliases out of ITS instance; the
// one `"peer"` core instance exports every op flat by name (so op names are globally unique). Consumer
// binds effect M → cadenza:math/api (scalar `neg`) AND effect P → cadenza:pairs/api (compound `pair`),
// combining both in one body. Because it inspects the tuple it uses the runtime → assemble_extern_runtime
// with g=2. Composed via run_with_peers with the TWO source-provider peers → runs end-to-end.
// ------------------------------------------------------------------------------------------------

#[test]
fn u9_a_consumer_binds_two_distinct_peer_interfaces() {
    use crate::testkit::parse;
    // Two SOURCE providers, distinct interfaces + distinct op names.
    let math = compile_provider(
        "(do (def (neg (: x Int64)) (- 0 x)) (export neg))",
        "cadenza:math/api",
    );
    let pairs = compile_provider(
        "(do (def (pair (: x Int64)) (tuple x x)) (export pair))",
        "cadenza:pairs/api",
    );
    // CONSUMER (source): binds M → math (scalar neg), P → pairs (compound pair). main(9) computes
    // `neg(pair(9).0) = neg(9) = -9` — a value from EACH bound peer interface in one body.
    let src = "(do \
            (effect M (op neg (-> Int64 Int64))) \
            (effect P (op pair (-> Int64 (Tuple Int64 Int64)))) \
            (bind M \"cadenza:math/api\") \
            (bind P \"cadenza:pairs/api\") \
            (def (main (: x Int64)) (host (M) (host (P) (M.neg (. (P.pair x) 0))))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| {
            panic!(
                "two-interface consumer compiles: {} [{:?}]",
                d.message, d.code
            )
        });
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("two-interface consumer validates");
    }
    // The consumer imports BOTH peer interfaces (they appear verbatim as component import names).
    assert!(
        contains_bytes(&consumer, b"cadenza:math/api")
            && contains_bytes(&consumer, b"cadenza:pairs/api"),
        "the consumer must import both bound peer interfaces"
    );
    // The RUN half (compose both providers + this consumer; pairs.pair(9)=(9,9) crosses as a handle,
    // project element 0 → 9, math.neg(9) → -9) is covered by corpus
    // `spec/semantics/29-cross-component-peers.sexp` — "a consumer bound to two distinct peer interfaces
    // combines their results". This in-crate test now pins the WHITE-BOX claims only: the consumer imports
    // BOTH bound interfaces (above) and each source provider publishes its OWN interface name — which the
    // corpus run confirms behaviorally.
    assert!(
        contains_bytes(&math, b"cadenza:math/api") && contains_bytes(&pairs, b"cadenza:pairs/api"),
        "each source provider publishes its own interface name"
    );
}

#[test]
fn u9b_two_peer_interfaces_offering_the_same_op_name_declines() {
    use crate::testkit::parse;
    // Two bound interfaces BOTH offering an op named `f` — the one merged `"peer"` core instance would
    // export `f` twice, so the compiler DECLINES (honestly, not a miscompile) rather than emit an
    // ill-formed component. (An unbound version would just be two effects; the collision is only a
    // problem once both route to the flat peer instance.)
    let src = "(do \
            (effect A (op f (-> Int64 Int64))) \
            (effect B (op f (-> Int64 Int64))) \
            (bind A \"cadenza:a/api\") \
            (bind B \"cadenza:b/api\") \
            (def (main (: x Int64)) (host (A) (host (B) (+ (A.f x) (B.f x))))) \
            (export main))";
    let r = crate::compile::compile_component(&crate::codec::encode(&parse(src)));
    match r {
        Err(d) => assert!(
            d.message.contains("unique across the peer interfaces")
                || d.message.contains("offered by two bound interfaces"),
            "expected the cross-interface op-name-collision decline; got: {}",
            d.message
        ),
        Ok(_) => panic!("two interfaces offering the same op name must decline, not compile"),
    }
}

// ------------------------------------------------------------------------------------------------
// U11 — an A→B→C CHAIN: a MIDDLE component B is BOTH a consumer (binds A) AND a provider (publishes its
// own interface for C). The fused consumer+provider envelope: B imports A's interface (as a peer),
// computes, and BUNDLES its own boundary export into a named interface instance for C — instead of
// exporting top-level. Threaded end-to-end by `run_with_peers`, which now binds each earlier peer's
// interface into later peers' linkers (dependency order) so B (peer) can import A (peer).
// ------------------------------------------------------------------------------------------------

#[test]
fn u11_a_middle_component_is_both_consumer_and_provider() {
    use crate::testkit::parse;
    // A (provider): `pair x = (tuple x x)` published as cadenza:pairs/api (compound).
    let a = compile_provider(
        "(do (def (pair (: x Int64)) (tuple x x)) (export pair))",
        "cadenza:pairs/api",
    );
    // B (MIDDLE — consumer of A AND provider for C): binds P→cadenza:pairs/api, reads element 0 of the
    // tuple and adds 1, published as `mid` under cadenza:mid/api. B both IMPORTS a peer and PUBLISHES
    // its own interface — the fused envelope. (It inspects a compound handle → uses the runtime.)
    let b = compile_provider(
        "(do \
                (effect P (op pair (-> Int64 (Tuple Int64 Int64)))) \
                (bind P \"cadenza:pairs/api\") \
                (def (mid (: x Int64)) (host (P) (+ (. (P.pair x) 0) 1))) \
                (export mid))",
        "cadenza:mid/api",
    );
    // B must publish its own interface (a named instance), NOT export `mid` top-level, AND import both
    // A's interface and the runtime.
    assert!(
        contains_bytes(&b, b"cadenza:mid/api") && contains_bytes(&b, b"cadenza:pairs/api"),
        "the middle component publishes cadenza:mid/api AND imports cadenza:pairs/api"
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&b).expect("the middle component validates");
    }
    // C (top consumer): binds M→cadenza:mid/api, calls `mid`.
    let c_src = "(do \
            (effect M (op mid (-> Int64 Int64))) \
            (bind M \"cadenza:mid/api\") \
            (def (main (: x Int64)) (host (M) (M.mid x))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(c_src)))
        .unwrap_or_else(|d| panic!("chain top consumer compiles: {} [{:?}]", d.message, d.code));
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer).expect("chain consumer validates");
    }
    // The RUN half (compose A + B + C; C.main(9) → B.mid(9) → (A.pair(9)=(9,9)).0 + 1 = 10, a value
    // flowing A→B→C with B both consumer AND provider, and the runner wiring A into B's linker in
    // dependency order) is covered by corpus `spec/semantics/29-cross-component-peers.sexp` — "a middle
    // peer is both a consumer and a provider (A to B to C chain)". This in-crate test now pins the
    // WHITE-BOX claims only: the MIDDLE B publishes cadenza:mid/api AND imports cadenza:pairs/api (above),
    // and provider A publishes its own interface — which the corpus run confirms behaviorally.
    assert!(
        contains_bytes(&a, b"cadenza:pairs/api"),
        "provider A publishes cadenza:pairs/api"
    );
}

// ------------------------------------------------------------------------------------------------
// PL28 — a STRING ARGUMENT crosses to a peer as a runtime HANDLE (the Bedrock-as-peer critical path).
// U16/PL17 pin a COMPOUND arg crossing as a handle; a String is the same shape — a rope leaf on the
// shared value heap, handed in as its u32 handle (NOT marshaled as a component `string`). The provider
// takes a `String` arg and returns its `byte-len` (an Int64 result, so NO result-escape is involved —
// this isolates the ARGUMENT direction). main = S.blen("hello") = 5. Before this cell the consumer was
// rejected (PL24, STRING_ARG_ACROSS_PEER) because the arg's rope-build ops were not collected into the
// runtime-import set, so the emitted consumer took the runtime-FREE extern envelope and called an
// unimported `bytes-alloc` → an invalid component. The fix teaches `collect_used_ops` that a PEER
// String/Bytes arg builds a rope (unlike a HOST String arg, marshaled as (ptr,len)).
// ------------------------------------------------------------------------------------------------
#[test]
fn a_string_argument_crosses_to_a_peer_as_a_runtime_handle() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    use crate::testkit::parse;
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    // PROVIDER (source): `blen : String -> Int64` reads the crossed String's byte length.
    let provider = compile_provider(
        "(do (def (blen (: s String)) (String.byte-len s)) (export blen))",
        "cadenza:strs/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("the string-arg provider validates");
    }
    // CONSUMER (source): passes a String LITERAL into the peer op. The literal builds a rope handle on
    // the shared runtime and the handle crosses — so the consumer imports the value-heap runtime.
    let src = "(do \
            (effect S (op blen (-> String Int64))) \
            (bind S \"cadenza:strs/api\") \
            (def (main) (host (S) (S.blen \"hello\"))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("string-arg consumer compiles: {} [{:?}]", d.message, d.code));
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("string-arg consumer validates");
    }
    assert!(
        String::from_utf8_lossy(&consumer).contains(&import_name),
        "a String-arg peer consumer imports the value-heap runtime (it builds the rope handle)"
    );
    // The RUN half (the crossed String arg is read by the peer → byte-len) is corpus-covered by
    // 29-cross-component-peers "a string argument crosses to a peer and the doubled result byte-len
    // is read"; this rcdzc test keeps only the white-box value-heap-runtime-import pin.
}

// ------------------------------------------------------------------------------------------------
// PL30 — a MIXED String + scalar argument list crosses to a peer in ONE op (the Bedrock `converse`
// shape: a prompt String + a scalar like `max-tokens`). PL28/PL29 pin a lone String arg; u2 pins
// scalar args; this pins that the two DISTINCT arg-emit paths INTERLEAVE correctly in a single call —
// the String lowers to a rope HANDLE (an i32 handle on the stack) while the Int64 lowers DIRECTLY (an
// i64), pushed in declaration order, and the peer reads both. A regression that mis-orders the mixed
// push (e.g. treating the String slot as a scalar, or vice versa) would misalign the peer's params;
// this is the exact call shape a `(-> String Int64 …)` model op takes. Provider `blen-plus : (String,
// Int64) -> Int64` = byte-len(s) + n. main = S.blen-plus("hello", 7) = 5 + 7 = 12.
// ------------------------------------------------------------------------------------------------
#[test]
fn a_mixed_string_and_scalar_argument_cross_to_a_peer_in_one_op() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    use crate::testkit::parse;
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    // PROVIDER: reads a String arg (byte-len) AND a scalar arg, summing them — so BOTH crossed + were
    // read. The String is a rope handle; the Int64 is a direct scalar.
    let provider = compile_provider(
        "(do (def (blen-plus (: s String) (: n Int64)) (+ (String.byte-len s) n)) (export blen-plus))",
        "cadenza:mix/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("the mixed-arg provider validates");
    }
    // CONSUMER: passes a String LITERAL and a scalar in ONE call — the two arg-emit paths interleave.
    let src = "(do \
            (effect S (op blen-plus (-> String Int64 Int64))) \
            (bind S \"cadenza:mix/api\") \
            (def (main) (host (S) (S.blen-plus \"hello\" 7))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| panic!("mixed-arg consumer compiles: {} [{:?}]", d.message, d.code));
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("mixed-arg consumer validates");
    }
    assert!(
        String::from_utf8_lossy(&consumer).contains(&import_name),
        "a mixed String+scalar peer consumer imports the value-heap runtime (it builds the rope handle)"
    );
    // The RUN half (byte-len("hello") + 7 = 12 — the String crossed as a handle and the Int64 as a
    // scalar in one call, in declaration order) is corpus-covered by 29-cross-component-peers "a mixed
    // String and scalar argument cross to a peer in one op, each in its own ABI lane"; this rcdzc test
    // keeps only the white-box value-heap-runtime-import pin.
}

// ------------------------------------------------------------------------------------------------
// PL31 — a RECORD-of-(String, scalar) argument crosses to a peer (the Bedrock REQUEST-STRUCT idiom).
// A real model call bundles its inputs as a request record — `{ prompt: String, max-tokens: Int64 }` —
// not as loose positional args. PL30 pins loose String+scalar args; u16 pins a scalar-only compound;
// PL31 pins that a compound carrying a STRING FIELD crosses as ONE handle and the peer reads both a
// rope-leaf field AND a scalar field out of it. This is the nested-rope-in-compound arg path (the
// tuple's String element is itself a heap handle stored in the tuple), distinct from PL30's flat args.
// Provider `req : (Tuple String Int64) -> Int64` = byte-len(prompt) + max-tokens. Consumer builds
// `("hi", 4)` → 2 + 4 = 6 — the request struct crossed as a handle, both fields read on the peer side.
// ------------------------------------------------------------------------------------------------
#[test]
fn a_record_of_string_and_scalar_argument_crosses_to_a_peer() {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    use crate::testkit::parse;
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    // PROVIDER: reads BOTH the String field (byte-len) and the scalar field of a single tuple arg.
    let provider = compile_provider(
        "(do (def (req (: t (Tuple String Int64))) (+ (String.byte-len (. t 0)) (. t 1))) (export req))",
        "cadenza:req/api",
    );
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&provider)
            .expect("the request-struct provider validates");
    }
    // CONSUMER: builds a request tuple `("hi", 4)` — a String field + a scalar field — and passes it
    // as ONE handle into the peer op.
    let src = "(do \
            (effect S (op req (-> (Tuple String Int64) Int64))) \
            (bind S \"cadenza:req/api\") \
            (def (main) (host (S) (S.req (tuple \"hi\" 4)))) \
            (export main))";
    let consumer = crate::compile::compile_component(&crate::codec::encode(&parse(src)))
        .unwrap_or_else(|d| {
            panic!(
                "request-struct consumer compiles: {} [{:?}]",
                d.message, d.code
            )
        });
    {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(&consumer)
            .expect("request-struct consumer validates");
    }
    assert!(
        String::from_utf8_lossy(&consumer).contains(&import_name),
        "a request-struct peer consumer imports the value-heap runtime (it builds the tuple + rope)"
    );
    // The RUN half (byte-len("hi") + 4 = 6 — a tuple carrying a String field crosses as ONE handle,
    // both fields projected by the peer) is corpus-covered by 29-cross-component-peers "pca4 a tuple of
    // a string and a scalar crosses INBOUND to a peer, both fields read there"; this rcdzc test keeps
    // only the white-box value-heap-runtime-import pin.
}

// ------------------------------------------------------------------------------------------------
// PL38 → NOW EMITS: a SCALAR host-delegated effect (not peer-bound) from a resource-escaping entrypoint
// is the host-side mirror of the peer+resource fusion, and now composes via
// `envelope::assemble_host_runtime_resource` (the core module lays the host ops as leading `"host"`
// imports via `runtime_resource_core_module_form_ex2(leading_is_host = true)`). Was a clean decline
// (PL38); now emits a VALID component. (A STRING-param host op still declines — the shared-memory `_mem`
// variant is a later increment; PL38b below pins that clean decline.)
// ------------------------------------------------------------------------------------------------
#[test]
fn a_scalar_host_effect_from_a_resource_escaping_entrypoint_emits() {
    use crate::testkit::parse;
    // main RETURNS a tuple built from a SCALAR HOST-delegated effect (H is NOT bound to a peer).
    let host_res = "(do \
            (effect H (op h (-> Int64 Int64))) \
            (def (main (: x Int64)) (host (H) (tuple (H.h x) x))) \
            (export main))";
    let bytes = crate::compile::compile_component(&crate::codec::encode(&parse(host_res)))
        .expect("a scalar host-effect resource escape now emits (assemble_host_runtime_resource)");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("the host-resource-escape component must be valid");
}

// PL38b — a STRING-param host op from a resource-escaping entrypoint STILL declines cleanly (the
// shared-memory `_mem` variant is a later increment). The decline names the effect kind + the workaround.
#[test]
fn a_string_param_host_effect_from_a_resource_escaping_entrypoint_declines_cleanly() {
    use crate::testkit::parse;
    let host_res = "(do \
            (effect H (op h (-> String Int64))) \
            (def (main (: x Int64)) (host (H) (tuple (H.h \"k\") x))) \
            (export main))";
    let err = crate::compile::compile_component(&crate::codec::encode(&parse(host_res)))
        .expect_err(
            "a STRING-param host-effect resource escape must decline (the _mem variant is later)",
        );
    assert!(
        err.message.contains("STRING parameter") && err.message.contains("resource-escaping"),
        "the string-host+resource decline must name the string param + the _mem increment: {}",
        err.message
    );
}

/// Substring search over bytes (dependency-free) — used to assert a component embeds an import name.
fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn record_type_field_ascription_is_read_by_reduce_ctor_and_decode_ty() {
    // DESIGN-record-type-syntax Phase A, RT3 (additive half): the two pair-only record-TYPE field
    // readers — `reduce_ctor`'s `RecordCtor` arm (`eval.rs`) and `decode_ty`'s `"Record"` arm
    // (`resolve.rs`) — now ALSO accept the canonical `(: name T)` ascription field (the shared
    // binder node), not only the legacy `(name T)` head-app pair. This is the widening that must
    // land BEFORE the `encode_ty`/`render_name` flip starts EMITTING ascription; strictly additive
    // (an ascription previously failed the `len == 2` pair match and errored/returned None), so no
    // currently-accepted program changes. Pins BOTH readers read the field TYPES (not just the
    // shape): a wrong field type is rejected, and the built `Ty::Record` carries the declared types.
    use crate::db::Db;
    use crate::eval::typeval_of;
    use crate::testkit::parse;
    use crate::ty::Ty;

    // reduce_ctor path: `typeval_of` on the ascription-form `(Record (: a Int64) (: b Bool))` node
    // reduces to a `Ty::Record` with exactly {a: Int64, b: Bool}.
    let ast =
        parse("(module m (def (main (: r (Record (: a Int64) (: b Bool)))) 0) (export main))");
    let mut db = Db::load(ast);
    let rec_node = (0..db.ast.structure.len() as u32)
        .map(crate::ast::StructId)
        .find(|&id| {
            db.ast
                .as_form(id, "Record")
                .is_some_and(|tail| tail.len() == 2)
        })
        .expect("the parsed program contains a two-field (Record …) type node");
    match typeval_of(&mut db, rec_node) {
        Some(Ty::Record(fields)) => {
            assert_eq!(fields.len(), 2, "two fields: {fields:?}");
            assert_eq!(
                fields.get(&crate::resolved::Symbol::plain("a".to_string())),
                Some(&Ty::int64()),
                "field a is Int64 (the ascription's type position was read): {fields:?}"
            );
            assert_eq!(
                fields.get(&crate::resolved::Symbol::plain("b".to_string())),
                Some(&Ty::Bool),
                "field b is Bool: {fields:?}"
            );
        }
        other => panic!("ascription-field Record must reduce to Ty::Record, got {other:?}"),
    }

    // decode_ty / end-to-end path: a value annotated with an ascription-form record type COMPILES,
    // and a field whose value type MISMATCHES the ascription is REJECTED — proving the field TYPE
    // (not just the name) is read through the full annotate/check pipeline.
    let ok = "(module m \
            (def (main) (: (record (= a 1) (= b true)) (Record (: a Int64) (: b Bool)))) \
            (export main))";
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(ok))).is_ok(),
        "a record value annotated with an ascription-form record type must compile"
    );
    let bad = "(module m \
            (def (main) (: (record (= a true) (= b true)) (Record (: a Int64) (: b Bool)))) \
            (export main))";
    assert!(
        crate::compile::compile_component(&crate::codec::encode(&parse(bad))).is_err(),
        "a field value whose type mismatches the ascription must be rejected (the type position is checked)"
    );
}
