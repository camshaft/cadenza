use crate::backend::wasm::runtime_abi::{AbiValType, OPS, RtOp};
use crate::lower::{LeafFill, ValueFormTemplate, runtime_value_form_template};
use crate::ty::Ty;

/// The runtime ops the escape shape uses, in the SORTED order the compiler's used-op set
/// (`collect_used_ops` → a `BTreeSet`) produces — so the oracle's op order matches what
/// `assemble_runtime_resource` receives. `arr-alloc`/`arr-set` build, `arr-get`/`get-int`/`get-bool`
/// walk, `box-int` boxes a leaf, and `drop` releases the compound's rc handle in the resource DTOR
/// (on host-drop / when `encode` consumes the `own<t>`). (`resource-new`/`resource-rep` are resource
/// intrinsics, not runtime ops, so they are threaded separately — not in this set.)
fn walker_ops() -> [&'static RtOp; 7] {
    // Sorted by name: arr-alloc, arr-get, arr-set, box-int, drop, get-bool, get-int.
    [
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_bool,
        OPS.get_int,
    ]
}

/// The component valtype of an ABI type — the boundary form the import instance-type declares.
fn abi_comp(p: AbiValType) -> wasm_encoder::ComponentValType {
    use wasm_encoder::{ComponentValType, PrimitiveValType};
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

/// The core functype `(params)->(result?)` of a runtime op — its CORE valtypes (a u32 handle is i32,
/// s64 is i64), the shape the program core module imports it under.
fn op_core_functype(op: &RtOp) -> (Vec<wasm_encoder::ValType>, Vec<wasm_encoder::ValType>) {
    use wasm_encoder::ValType;
    let core = |p: AbiValType| match p {
        // Every aliased int ≤32 (+ bool/char) lowers to core i32; a 64-bit int to i64.
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

/// The DTOR core module — `t-dtor : (i32 rep) -> ()`, which RELEASES the resource's rep by calling
/// the runtime `drop` (imported as `heap-dtor.drop`). On host-drop (or when `encode` consumes the
/// `own<t>`), the component invokes this to release the compound's rc handle — which cascades to its
/// boxed children. It imports `drop` from a SEPARATE small core instance (`heap-dtor`) built from the
/// LOWERED `drop` op (a core func available BEFORE the resource type), NOT from the full `heap`
/// instance — so the dtor module can still instantiate before the resource type, keeping the
/// resource↔dtor↔resource.new cycle dissolved ([[rcdzc-r1-resource-encode-linking-findings]]).
/// Byte-identical to `serialize::resource_dtor_module`.
fn dtor_module() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![]); // 0: (i32) -> () for both drop-import and t-dtor
    m.section(&types);
    // Import `drop : (i32) -> ()` from the `heap-dtor` module → core func 0.
    let mut imports = ImportSection::new();
    imports.import("heap-dtor", "drop", EntityType::Function(0));
    m.section(&imports);
    // t-dtor is defined func 1 (the import is func 0).
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("t-dtor", ExportKind::Func, 1);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut dtor = Function::new(vec![]);
    dtor.instruction(&Instruction::LocalGet(0)); // the rep
    dtor.instruction(&Instruction::Call(0)); // call the imported drop
    dtor.instruction(&Instruction::End);
    code.function(&dtor);
    m.section(&code);
    m.finish()
}

/// The PROGRAM core module for a runtime compound `(tuple 3 1)` escape. Imports the walker runtime
/// ops + `resource-new` from `"heap"`; exports `memory`, `make`, `t-encode`, `cabi_realloc`.
///
///  * `make()` BUILDS the tuple on the value heap — `arr-alloc(2)`, then each element boxed
///    (`box-int`) and `arr-set` — then `resource.new(handle)` registers the runtime handle as the
///    resource's rep and returns the resource handle.
///  * `t-encode(rep)` receives the runtime handle directly (the guest resource's canonical ABI
///    passes the rep). The value-form TEMPLATE is a data segment at offset 0 doubling as the output
///    buffer; `encode` walks each hole's `arr-get` path from `rep`, reads the leaf, and writes its
///    bytes at the hole's offset, then returns the `(ptr=0, len)` return area pointer.
fn walker_core(tpl: &ValueFormTemplate, elems: &[i64]) -> Vec<u8> {
    use wasm_encoder::*;
    // Import order fixes the core func indices the bodies call. Walker ops first (0..6), then
    // `resource-new` (6). The import NAMES are what the `heap` instance resolves by.
    let ops = walker_ops();
    let mut m = Module::new();

    // Type section: one functype per import (deduped-by-shape is unnecessary — just list them), then
    // the three defined-func types (make `()->i32`, encode `(i32)->i32`, cabi_realloc `(i32×4)->i32`).
    let mut types = TypeSection::new();
    let mut import_type_idx = Vec::new();
    for op in ops {
        let (p, r) = op_core_functype(op);
        types.ty().function(p, r);
        import_type_idx.push(import_type_idx.len() as u32);
    }
    // resource-new / resource-rep : both (i32)->i32.
    let rnew_ty = import_type_idx.len() as u32;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]);
    let make_ty = rnew_ty + 1;
    types.ty().function(vec![], vec![ValType::I32]); // make ()->i32
    let encode_ty = make_ty + 1;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // encode (i32)->i32
    let realloc_ty = encode_ty + 1;
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    );
    m.section(&types);

    // Import section: the walker ops + resource-new + resource-rep, all from module "heap". Core
    // func indices 0..=7 in this order; the defined funcs follow. `resource.new` registers the heap
    // rep → a resource handle in `make`; `resource.rep` recovers the heap rep from the handle the
    // canonical ABI hands `encode` (the handle is a guest resource-table index, NOT the rep).
    let mut imports = ImportSection::new();
    for (i, op) in ops.iter().enumerate() {
        imports.import("heap", op.name, EntityType::Function(import_type_idx[i]));
    }
    imports.import("heap", "resource-new", EntityType::Function(rnew_ty));
    imports.import("heap", "resource-rep", EntityType::Function(rnew_ty));
    m.section(&imports);
    // Import func indices — resolve each op by NAME against the (sorted) import order, so the body
    // is robust to the op ordering (the sorted used-set the compiler produces). `resource-new`/
    // `resource-rep` follow the k ops.
    let idx_of = |name: &str| ops.iter().position(|o| o.name == name).unwrap() as u32;
    let f_arr_alloc = idx_of("arr-alloc");
    let f_arr_set = idx_of("arr-set");
    let f_arr_get = idx_of("arr-get");
    let f_box_int = idx_of("box-int");
    let f_get_int = idx_of("get-int");
    let f_get_bool = idx_of("get-bool");
    let f_rnew = ops.len() as u32;
    let f_rrep = ops.len() as u32 + 1;

    // Defined funcs follow the `k` ops + resource-new + resource-rep (= k+2 imports): make=k+2,
    // encode=k+3, cabi_realloc=k+4.
    let make_fn = ops.len() as u32 + 2;
    let encode_fn = make_fn + 1;
    let realloc_fn = encode_fn + 1;
    let mut funcs = FunctionSection::new();
    funcs.function(make_ty);
    funcs.function(encode_ty);
    funcs.function(realloc_ty);
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("make", ExportKind::Func, make_fn);
    exports.export("t-encode", ExportKind::Func, encode_fn);
    exports.export("cabi_realloc", ExportKind::Func, realloc_fn);
    m.section(&exports);

    // Data: the value-form template at offset 0 (doubles as the output buffer), then the (ptr,len)
    // return area 4-aligned after it: ptr=0, len=template length.
    let tpl_len = tpl.bytes.len();
    let ret_off = (tpl_len + 3) & !3;
    let mut data_bytes = tpl.bytes.clone();
    data_bytes.resize(ret_off, 0);
    data_bytes.extend_from_slice(&0u32.to_le_bytes()); // ptr = 0 (template @ 0)
    data_bytes.extend_from_slice(&(tpl_len as u32).to_le_bytes()); // len
    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(0), data_bytes.iter().copied());

    // make: build (tuple <elems>) on the heap, then resource.new(handle).
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(elems.len() as i32));
    make.instruction(&Instruction::Call(f_arr_alloc)); // [arr]
    for (i, &v) in elems.iter().enumerate() {
        make.instruction(&Instruction::I32Const(i as i32)); // [arr, i]
        make.instruction(&Instruction::I64Const(v));
        make.instruction(&Instruction::Call(f_box_int)); // [arr, i, h]
        make.instruction(&Instruction::Call(f_arr_set)); // [arr]
    }
    make.instruction(&Instruction::Call(f_rnew)); // [resource-handle]
    make.instruction(&Instruction::End);

    // encode(handle): the canonical ABI passes the resource-table HANDLE, not the heap rep — so
    // FIRST recover the heap rep via `resource.rep(handle)` into local `rep`, then walk each hole
    // and write its bytes into the template-as-output buffer. Locals: 0 = handle param, 1 = the
    // recovered heap rep (i32), 2 = i64 scratch (leaf value / magnitude).
    let mut encode = Function::new(vec![(1, ValType::I32), (1, ValType::I64)]);
    let handle = 0u32;
    let rep = 1u32;
    let scratch = 2u32;
    encode.instruction(&Instruction::LocalGet(handle));
    encode.instruction(&Instruction::Call(f_rrep)); // handle → heap rep
    encode.instruction(&Instruction::LocalSet(rep));
    for hole in &tpl.leaves {
        let out_off = hole.offset as u64; // template is at memory offset 0
        match hole.kind {
            LeafFill::Int => {
                // Walk to the leaf handle, then get-int → i64 value in scratch.
                encode.instruction(&Instruction::LocalGet(rep));
                for &idx in &hole.path {
                    encode.instruction(&Instruction::I32Const(idx as i32));
                    encode.instruction(&Instruction::Call(f_arr_get));
                }
                encode.instruction(&Instruction::Call(f_get_int));
                encode.instruction(&Instruction::LocalSet(scratch));
                // Negative? flip the kind byte (offset-2: kind, then a 1-byte len=8) to NEG_DEC (3)
                // and negate the magnitude.
                encode.instruction(&Instruction::LocalGet(scratch));
                encode.instruction(&Instruction::I64Const(0));
                encode.instruction(&Instruction::I64LtS);
                encode.instruction(&Instruction::If(BlockType::Empty));
                encode.instruction(&Instruction::I32Const((out_off - 2) as i32));
                encode.instruction(&Instruction::I32Const(3)); // KIND_INT_NEG_DEC
                encode.instruction(&Instruction::I32Store8(MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                encode.instruction(&Instruction::I64Const(0));
                encode.instruction(&Instruction::LocalGet(scratch));
                encode.instruction(&Instruction::I64Sub);
                encode.instruction(&Instruction::LocalSet(scratch));
                encode.instruction(&Instruction::End);
                // Write 8 big-endian magnitude bytes at out_off.
                for k in 0..8u64 {
                    encode.instruction(&Instruction::I32Const((out_off + k) as i32));
                    encode.instruction(&Instruction::LocalGet(scratch));
                    encode.instruction(&Instruction::I64Const((8 * (7 - k)) as i64));
                    encode.instruction(&Instruction::I64ShrU);
                    encode.instruction(&Instruction::I32WrapI64);
                    encode.instruction(&Instruction::I32Store8(MemArg {
                        offset: 0,
                        align: 0,
                        memory_index: 0,
                    }));
                }
            }
            LeafFill::Bool => {
                // Write the kind byte 8+bool (8 false / 9 true) at out_off.
                encode.instruction(&Instruction::I32Const(out_off as i32));
                encode.instruction(&Instruction::LocalGet(rep));
                for &idx in &hole.path {
                    encode.instruction(&Instruction::I32Const(idx as i32));
                    encode.instruction(&Instruction::Call(f_arr_get));
                }
                encode.instruction(&Instruction::Call(f_get_bool));
                encode.instruction(&Instruction::I32Const(8));
                encode.instruction(&Instruction::I32Add);
                encode.instruction(&Instruction::I32Store8(MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
            }
        }
    }
    encode.instruction(&Instruction::I32Const(ret_off as i32)); // return the (ptr,len) area
    encode.instruction(&Instruction::End);

    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0)); // stub (never called for a nullary-input list result)
    realloc.instruction(&Instruction::End);

    let mut code = CodeSection::new();
    code.function(&make);
    code.function(&encode);
    code.function(&realloc);
    m.section(&code);
    m.section(&data);
    m.finish()
}

/// The INNER re-export component (identical to R1's — imports the abstract resource + funcs, re-
/// exports the resource directly + the funcs against it). The runtime resource path now uses the
/// BORROW inner (`inner_reexport_component_borrow`); this OWN-encode variant is kept as the reference
/// for the own shape.
#[allow(dead_code)]
fn inner_reexport_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    let (list1, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_ty, mut ef) = c.type_function();
    ef.params([("self", ComponentValType::Type(own_imp))])
        .result(Some(ComponentValType::Type(list1)));
    let enc_fn = c.import("import-func-encode", ComponentTypeRef::Func(enc_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od2) = c.type_defined();
    od2.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (own_exp2, od3) = c.type_defined();
    od3.own(exp_t);
    let (list2, ld2) = c.type_defined();
    ld2.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_exp_ty, mut ef2) = c.type_function();
    ef2.params([("self", ComponentValType::Type(own_exp2))])
        .result(Some(ComponentValType::Type(list2)));
    c.export(
        "encode",
        ComponentExportKind::Func,
        enc_fn,
        Some(ComponentTypeRef::Func(enc_exp_ty)),
    );
    c
}

/// Build the COMBINED runtime-import + resource component with `ComponentBuilder` — the authoritative
/// reference for the R2 shape. Imports the runtime `heap` interface (the walker ops), lowers them,
/// then builds the resource shape whose `heap` core-instance threads BOTH the lowered ops AND
/// `resource.new`. The program core (`make`/`t-encode`) uses all of them. Published as `cadenza:run/run`.
fn oracle_runtime_resource_component(core: &[u8], import_name: &str) -> Vec<u8> {
    use wasm_encoder::*;
    let ops = walker_ops();
    let mut c = ComponentBuilder::default();

    // (1) import instance-type declaring the walker ops.
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

    // (2) alias ALL ops out of the instance first (component funcs 0..k), then lower ALL (core funcs
    // 0..k) — the batched two-section style `assemble_with_imports` uses, so the combined hand-emit
    // stays uniform with the proven import path (one alias section, one lower section).
    let comp_fns: Vec<u32> = ops
        .iter()
        .map(|op| c.alias_export(inst, op.name, ComponentExportKind::Func))
        .collect();
    let lowered: Vec<(&str, u32)> = ops
        .iter()
        .zip(comp_fns)
        .map(|(op, f)| (op.name, c.lower_func(f, [])))
        .collect();

    // (3) The dtor module now CALLS `drop` to release the rep, so it imports `heap-dtor.drop`. Build
    // a small `heap-dtor` core instance exporting the lowered `drop` op (a core func from step 2,
    // available BEFORE the resource type), thread it into the dtor module's instantiation, THEN alias
    // `t-dtor`. This keeps the dtor instantiable before the resource type (the cycle stays dissolved:
    // `drop` is a plain lowered op, independent of resource.new/rep). Then resource type →
    // resource.new + resource.rep.
    let drop_core = lowered
        .iter()
        .find(|(n, _)| *n == "drop")
        .map(|(_, f)| *f)
        .expect("drop is in the op set");
    let heap_dtor_inst = c.core_instantiate_exports([("drop", ExportKind::Func, drop_core)]);
    let dtor_idx = c.core_module_raw(&dtor_module());
    let dtor_inst = c.core_instantiate(
        dtor_idx,
        [("heap-dtor", ModuleArg::Instance(heap_dtor_inst))],
    );
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);

    // (4) heap core-instance exporting the lowered ops + resource-new + resource-rep; instantiate
    // the program. `resource.rep` is what `encode` calls to turn the handle the canonical ABI hands
    // it back into the heap rep it walks (the handle is a guest resource-table index, not the rep).
    let mut heap_exports: Vec<(&str, ExportKind, u32)> = lowered
        .iter()
        .map(|(n, f)| (*n, ExportKind::Func, *f))
        .collect();
    heap_exports.push(("resource-new", ExportKind::Func, rnew_core));
    heap_exports.push(("resource-rep", ExportKind::Func, rrep_core));
    let heap_inst = c.core_instantiate_exports(heap_exports);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let encode_core = c.core_alias_export(prog_inst, "t-encode", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);

    // (5) lift make against `own<t>` (make PRODUCES the owned handle) and encode against `borrow<t>`
    // (encode READS self without consuming — the host keeps ownership, drops afterward → the dtor
    // reclaims the rep, and the resource is repeatable). The core `t-encode` uses the borrow's rep
    // DIRECTLY (no resource.rep), matching `RepSource::Borrow` in the production serializer.
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (borrow_t, bdef) = c.type_defined();
    bdef.borrow(res_ty);
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (encode_ty, mut enc2) = c.type_function();
    enc2.params([("self", ComponentValType::Type(borrow_t))])
        .result(Some(ComponentValType::Type(list_u8)));
    let encode_comp = c.lift_func(
        encode_core,
        encode_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );

    // (6) inner re-export component (BORROW variant — re-types encode against borrow<t>) → the
    // `cadenza:run/run` instance.
    let inner_idx = c.component(inner_reexport_component_borrow());
    let inst2 = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-encode", ComponentExportKind::Func, encode_comp),
        ],
    );
    c.export(
        "cadenza:run/run",
        ComponentExportKind::Instance,
        inst2,
        None,
    );
    c.finish()
}

/// STRUCTURALLY validate the combined runtime-resource component built from a hand-built walker `core`
/// (localizes any byte/index error). Formerly it also RAN the component via `cdz_run::run` to decode the
/// escaped `(: value type)` text; that run is dropped — the escaped value forms (a runtime tuple renders
/// `(: (tuple …) (Tuple …))`) are corpus-covered by the tuple-escape cases in 05-compound-types, and a
/// hand-built walker core cannot be a corpus (Cadenza-source) program, so it stays a compile+validate pin.
fn validate_composed(core: &[u8]) {
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    let comp = oracle_runtime_resource_component(core, &import_name);
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("combined runtime-resource component validates");
}

#[test]
fn a_flat_runtime_tuple_walks_and_crosses() {
    // Build `(tuple 3 1)` on the value heap, escape it as a resource, walk it in encode(), decode →
    // the exact corpus value form. The FIRST genuine heap-alloc→escape→walk round-trip. `run_composed`
    // wraps the BORROW envelope now, so the core must be the borrow walker (rep = param, no drop).
    let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    let core = walker_core_borrow(&tpl, &[3, 1]);
    // The escaped value form — `(: (tuple 3 1) (Tuple Int64 Int64))` — is corpus-covered by the runtime
    // tuple-escape cases in 05-compound-types; here the hand-built borrow-walker core need only compose
    // into a VALID runtime-resource component.
    validate_composed(&core);
}

#[test]
fn a_runtime_tuple_with_a_negative_element_walks() {
    // A negative element exercises the NEG kind-byte flip + absolute-magnitude write in the walker.
    let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    let core = walker_core_borrow(&tpl, &[-5, 7]);
    // The escaped value form — `(: (tuple -5 7) (Tuple Int64 Int64))`, exercising the NEG kind-byte flip
    // — is corpus-covered by the runtime tuple-escape cases (with negative elements) in
    // 05-compound-types; here the hand-built walker core need only compose into a VALID component.
    validate_composed(&core);
}

#[test]
fn bake_constant_leaves_pre_encodes_a_fully_constant_tuple_to_zero_holes() {
    // §2d PRE-ENCODE (Axis 2): a fully-constant compound RETURN has EVERY value-form leaf baked into the
    // template at compile time (the SAME bytes the runtime hole-fill walker would write), so ZERO runtime
    // holes remain — the escape becomes a hole-free copy with no per-event value-encode leaf walk.
    // Independent check: the baked bytes still DECODE (separate codec code) to the constant Ints 3 and 1.
    use crate::lower::bake_constant_leaves;
    use crate::testkit::parse;
    let src = "(module m (def (main) (tuple 3 1)) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("main has a body");
    let ty = crate::infer::type_of(&mut db, body);
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    assert_eq!(tpl.leaves.len(), 2, "two runtime holes before baking");
    let baked = bake_constant_leaves(&mut db, body, &tpl);
    assert!(
        baked.leaves.is_empty(),
        "a fully-constant tuple bakes to ZERO holes (fully static)"
    );
    let arenas = crate::codec::decode(&baked.bytes).expect("baked template bytes decode");
    let ints: std::collections::BTreeSet<i64> = arenas
        .leaves
        .iter()
        .filter_map(|l| match l {
            crate::ast::Leaf::Int { value, .. } => value.to_i64(),
            _ => None,
        })
        .collect();
    assert!(
        ints.contains(&3) && ints.contains(&1),
        "the baked bytes decode to the tuple's constants 3 and 1, got {ints:?}"
    );
}

#[test]
fn bake_constant_leaves_keeps_runtime_leaves_and_bakes_only_constants() {
    // A PARTIALLY-constant return: `(tuple x 42)` — the constant `42` (index 1) bakes out, the runtime
    // element `x` (index 0) STAYS a hole the walker fills per event. So one hole survives, at path [0] —
    // the per-event work drops from two leaf writes to one, byte-identical output.
    use crate::lower::bake_constant_leaves;
    use crate::testkit::parse;
    let src = "(module m (def (main (: x Int64)) (tuple x 42)) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("main has a body");
    let ty = crate::infer::type_of(&mut db, body);
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    assert_eq!(tpl.leaves.len(), 2, "two holes before baking");
    let baked = bake_constant_leaves(&mut db, body, &tpl);
    assert_eq!(
        baked.leaves.len(),
        1,
        "the constant leaf baked out; only the runtime leaf stays a hole"
    );
    assert_eq!(
        baked.leaves[0].path,
        vec![0u32],
        "the surviving hole is the runtime element `x` at index 0"
    );
}

#[test]
fn constant_value_form_bare_is_the_framed_value_without_the_type_frame() {
    // The bare compile-time encoder (Axis-2 provider-path foundation) must produce EXACTLY the runtime
    // `value-encode` op's bare output for a constant. Verified at the AST level (codec is canonical, so
    // AST equivalence ⇒ byte equivalence): the bare doc is the framed `constant_value_form`'s value node
    // WITHOUT the `(: value Type)` frame, carrying the same constant leaves. Both derive the value from
    // the SAME `const_value_ast` the corpus-verified framed path uses, and the reducer boundary is bare
    // (v-ah+v-runtime ruling 2026-08-12), so these bytes are what the per-event value-encode would emit.
    use crate::lower::{constant_value_form, constant_value_form_bare};
    use crate::testkit::parse;
    let src = "(module m (def (main) (tuple 3 1)) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("body");
    let bare = constant_value_form_bare(&mut db, body).expect("bare value form");
    let framed = constant_value_form(&mut db, body).expect("framed value form");
    let bare_a = crate::codec::decode(&bare).expect("bare decodes");
    let framed_a = crate::codec::decode(&framed).expect("framed decodes");
    let names = |a: &crate::ast::Arenas| -> std::collections::BTreeSet<String> {
        a.leaves
            .iter()
            .filter_map(|l| match l {
                crate::ast::Leaf::Name(n) => Some(n.to_string()),
                _ => None,
            })
            .collect()
    };
    let ints = |a: &crate::ast::Arenas| -> std::collections::BTreeSet<i64> {
        a.leaves
            .iter()
            .filter_map(|l| match l {
                crate::ast::Leaf::Int { value, .. } => value.to_i64(),
                _ => None,
            })
            .collect()
    };
    let want_ints: std::collections::BTreeSet<i64> = [1, 3].into_iter().collect();
    // M2 native compound heads: a tuple value is headed by the payloadless `Ctor(Tuple)` leaf
    // kind (not a `Name("tuple")` string head anymore), so recognize the native ctor-leaf.
    let has_tuple_ctor = |a: &crate::ast::Arenas| -> bool {
        a.leaves
            .iter()
            .any(|l| matches!(l, crate::ast::Leaf::Ctor(crate::ast::CompoundCtor::Tuple)))
    };
    // BARE: the tuple value + its constants, with NO `(: value Type)` frame.
    assert!(has_tuple_ctor(&bare_a), "bare doc is the tuple value");
    assert!(
        !names(&bare_a).contains(":"),
        "the bare doc carries NO `(: value type)` frame"
    );
    assert_eq!(
        ints(&bare_a),
        want_ints,
        "bare carries the constants 3 and 1"
    );
    // FRAMED: the SAME value, WITH the `:` frame — the contrast that shows bare = framed minus the frame.
    assert!(
        names(&framed_a).contains(":"),
        "the framed doc has the `(: value type)` frame"
    );
    assert!(has_tuple_ctor(&framed_a));
    assert_eq!(ints(&framed_a), want_ints);
}

#[test]
fn a_bytes_provider_member_with_a_constant_result_pre_encodes_the_static_bytes() {
    // §2d PRE-ENCODE (Axis 2, PROVIDER path): a reducer whose result is a compile-time constant (ignores
    // its event) emits an apply body that writes the precomputed bare value-form bytes and returns — no
    // per-event value-decode / body / value-encode. WHITE-BOX EMIT PIN (corpus-inexpressible — it inspects
    // the EMITTED code, not a value; the reducer value-encode boundary is wire-level, not a corpus value):
    // assert the emitted component writes EXACTLY `constant_value_form_bare(result).len()` bytes as static
    // `i32.store8`s (the pre-encode path — a per-event reducer would value-encode via a runtime call and
    // emit no such per-byte store run). The bytes' CORRECTNESS is pinned by
    // `constant_value_form_bare_is_the_framed_value_without_the_type_frame`; this pins the EMIT takes the
    // pre-encode path. (Formerly RAN the reducer via `cdz_run::run_reducer_bytes` to compare the output
    // list<u8> to those bytes — dropped with the cdz-run dep migration; the run half has no value-level
    // corpus home because the list<u8> reducer boundary output is the wire bytes themselves.)
    use crate::testkit::parse;
    let src = "(module m \
                     (world reducer (export fold (member apply \
                       (func (param input (\"list\" (u8))) (result (\"list\" (u8))))))) \
                     (def (apply (: e (Record (n Int64)))) (record (= a 1) (= b 2))) \
                     (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "constant-result reducer compiles: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let wasm = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("emits a bytes-roundtrip provider component")
        .to_vec();
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(&wasm)
        .expect("constant-result provider validates");
    // Expected: the bare value form of the constant result `(record (a 1) (b 2))`.
    let mut db = crate::db::Db::load(parse(
        "(module m (def (apply (: e (Record (n Int64)))) (record (= a 1) (= b 2))) (export apply))",
    ));
    let d = db.def_by_name("apply").expect("apply");
    let body = db.defs[d].body.expect("body");
    let cbytes = crate::lower::constant_value_form_bare(&mut db, body).expect("constant result");
    // The pre-encode apply writes each of the `cbytes.len()` constant bytes with one `i32.store8`
    // (address + value + store8); the retarea ptr/len writes use `i32.store` (4-byte), not store8. So the
    // component's store8 count equals the pre-encoded byte length exactly IFF the apply took the pre-encode
    // path. A per-event reducer would call the runtime value-encode op and emit no such per-byte store run.
    let store8s = crate::tests::count_opcode(&wasm, |op| {
        matches!(op, wasmparser::Operator::I32Store8 { .. })
    });
    assert_eq!(
        store8s,
        cbytes.len(),
        "the constant-result apply must pre-encode its {} result bytes as static i32.store8 writes \
             (the pre-encode path), got {} store8s",
        cbytes.len(),
        store8s
    );
}

#[test]
fn a_runtime_qty_over_int_at_reference_unit_templates_with_the_unit_baked() {
    // A RUNTIME Qty over an Int64 at the REFERENCE unit (scale 1/1) gets a value-form template whose
    // unit label is BAKED as a compile-time constant and whose ONE leaf hole is the erased inner scalar,
    // reached at an EMPTY path (the boxed scalar IS the root rep — the `make` body boxes it via
    // `box-int`, then the walker `get-int`s it directly). This is the operator-ruled compile-time-only
    // unit render: the Qty erases to its scalar at runtime; the unit lives only in the baked bytes.
    use crate::ty::Unit;
    let ty = Ty::Qty {
        inner: Box::new(Ty::int64()),
        unit: Unit::base("meter"),
    };
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[]))
        .expect("Int-inner reference-unit Qty has a template");
    // Exactly one runtime hole: the inner magnitude, an Int at the ROOT (empty path, not via a sum
    // payload) — the scalar the `make` body boxes into the root cell.
    assert_eq!(tpl.leaves.len(), 1, "one hole (the erased inner scalar)");
    assert_eq!(tpl.leaves[0].kind, LeafFill::Int);
    assert!(tpl.leaves[0].path.is_empty(), "the scalar is the root rep");
    assert!(!tpl.leaves[0].via_sum_payload);
    // The baked bytes carry the unit label + the `Qty`/`of` construction names as leaf strings — decode
    // and confirm the value form is the `Qty.of` construction with the unit, not a bare scalar.
    let arenas = crate::codec::decode(&tpl.bytes).expect("template bytes decode");
    let names: std::collections::HashSet<&str> = arenas
        .leaves
        .iter()
        .filter_map(|l| match l {
            crate::ast::Leaf::Name(n) => Some(&**n),
            crate::ast::Leaf::Sym(s) => Some(&**s),
            _ => None,
        })
        .collect();
    // The `Qty.of` head is now a single bare dotted-NAME leaf (seq-283 member-render sugar), not separate
    // `Qty` + `of` member-part leaves — so the baked leaf pool carries the one string `"Qty.of"`.
    assert!(
        names.contains("Qty.of"),
        "the Qty.of construction is baked as one dotted name: {names:?}"
    );
    assert!(
        names.contains("meter"),
        "the unit label is baked: {names:?}"
    );
}

#[test]
fn a_runtime_qty_over_a_narrow_int_at_reference_unit_templates_too() {
    // Slice 2: a NARROW int inner (8/16/32, signed OR unsigned) at a reference unit ALSO templates — its
    // magnitude hole is width-agnostic (8-byte, like any Int leaf), the width lives in the baked type
    // annotation, and the i32→i64 extend the narrow scalar needs before `box-int` is emitted in the
    // `make` body (`EscapeForm::FlatScalar { extend }`). So the template itself is produced for every
    // int width; only the make-side extend differs. (Slice 1 had gated this to width 64; the gate is
    // lifted now that the extend is wired.)
    use crate::ty::{IntTy, Unit};
    for signed in [true, false] {
        for w in [8u32, 16, 32, 64] {
            let ty = Ty::Qty {
                inner: Box::new(Ty::Int(IntTy::fixed(signed, w))),
                unit: Unit::base("meter"),
            };
            let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[]))
                .unwrap_or_else(|| panic!("Int width={w} signed={signed} Qty has a template"));
            assert_eq!(tpl.leaves.len(), 1, "one hole (width={w})");
            assert_eq!(tpl.leaves[0].kind, LeafFill::Int);
            assert!(
                tpl.leaves[0].path.is_empty(),
                "scalar is the root (width={w})"
            );
        }
    }
}

#[test]
fn a_runtime_qty_declines_the_template_for_a_scaled_unit_or_a_float_inner() {
    // A scaled (non-reference) unit needs a compile-time scale multiply the flat leaf-hole template can't
    // carry (slice 3); a Float inner needs `LeafFill::Float` (slice 4) — both DECLINE (return None) so
    // the export falls back to today's valid bare-scalar cross rather than emitting a wrong/invalid form.
    use crate::ty::Unit;
    let scaled = Ty::Qty {
        inner: Box::new(Ty::int64()),
        unit: Unit::base("meter").scaled(1000, 1).expect("scaled unit"),
    };
    assert!(
        runtime_value_form_template(&scaled, &crate::ty::NameCtx::new(&[])).is_none(),
        "a scaled-unit Qty declines the runtime template (falls back)"
    );
    let float_inner = Ty::Qty {
        inner: Box::new(Ty::float64()),
        unit: Unit::base("meter"),
    };
    assert!(
        runtime_value_form_template(&float_inner, &crate::ty::NameCtx::new(&[])).is_none(),
        "a Float-inner Qty declines the runtime template (falls back)"
    );
}

/// The PROGRAM core for the BORROW design: `encode` takes `borrow<t>` self and uses its i32 param
/// DIRECTLY as the heap rep — NO `resource.rep` (which traps on a borrow: the canonical ABI's
/// `lift_borrow` returns the rep itself, not a table index — verified in wasmtime 37's
/// `resource_lift_borrow`) and NO `drop` (a borrow does not own the value; the caller keeps it, so
/// the resource stays live for repeated calls). Identical to `walker_core` EXCEPT the encode prologue
/// (`local.get handle` used as the rep, no `resource.rep`) and no trailing drop. `resource.rep` is
/// therefore not imported; only `resource-new` (for `make`) is threaded from `heap`.
fn walker_core_borrow(tpl: &ValueFormTemplate, elems: &[i64]) -> Vec<u8> {
    use wasm_encoder::*;
    let ops = walker_ops();
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let mut import_type_idx = Vec::new();
    for op in ops {
        let (p, r) = op_core_functype(op);
        types.ty().function(p, r);
        import_type_idx.push(import_type_idx.len() as u32);
    }
    let rnew_ty = import_type_idx.len() as u32;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // resource-new (i32)->i32
    let make_ty = rnew_ty + 1;
    types.ty().function(vec![], vec![ValType::I32]);
    let encode_ty = make_ty + 1;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]);
    let realloc_ty = encode_ty + 1;
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    );
    m.section(&types);

    // Imports: the walker ops + resource-new ONLY (no resource-rep — the borrow rep is the param).
    let mut imports = ImportSection::new();
    for (i, op) in ops.iter().enumerate() {
        imports.import("heap", op.name, EntityType::Function(import_type_idx[i]));
    }
    imports.import("heap", "resource-new", EntityType::Function(rnew_ty));
    m.section(&imports);
    let idx_of = |name: &str| ops.iter().position(|o| o.name == name).unwrap() as u32;
    let f_arr_alloc = idx_of("arr-alloc");
    let f_arr_set = idx_of("arr-set");
    let f_arr_get = idx_of("arr-get");
    let f_box_int = idx_of("box-int");
    let f_get_int = idx_of("get-int");
    let f_rnew = ops.len() as u32; // resource-new is the last import

    let make_fn = ops.len() as u32 + 1; // k ops + resource-new = k+1 imports
    let encode_fn = make_fn + 1;
    let realloc_fn = encode_fn + 1;
    let mut funcs = FunctionSection::new();
    funcs.function(make_ty);
    funcs.function(encode_ty);
    funcs.function(realloc_ty);
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("make", ExportKind::Func, make_fn);
    exports.export("t-encode", ExportKind::Func, encode_fn);
    exports.export("cabi_realloc", ExportKind::Func, realloc_fn);
    m.section(&exports);

    let tpl_len = tpl.bytes.len();
    let ret_off = (tpl_len + 3) & !3;
    let mut data_bytes = tpl.bytes.clone();
    data_bytes.resize(ret_off, 0);
    data_bytes.extend_from_slice(&0u32.to_le_bytes());
    data_bytes.extend_from_slice(&(tpl_len as u32).to_le_bytes());

    // make: build the tuple, then resource.new(handle) — UNCHANGED from the own version.
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(elems.len() as i32));
    make.instruction(&Instruction::Call(f_arr_alloc));
    for (i, &v) in elems.iter().enumerate() {
        make.instruction(&Instruction::I32Const(i as i32));
        make.instruction(&Instruction::I64Const(v));
        make.instruction(&Instruction::Call(f_box_int));
        make.instruction(&Instruction::Call(f_arr_set));
    }
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);

    // encode(borrow self): the param IS the rep (no resource.rep). Walk each hole; write its bytes.
    // Do NOT drop (a borrow does not own). Locals: 0 = rep (the borrow param), 1 = i64 scratch.
    let mut encode = Function::new(vec![(1, ValType::I64)]);
    let rep = 0u32;
    let scratch = 1u32;
    for hole in &tpl.leaves {
        let out_off = hole.offset as u64;
        match hole.kind {
            LeafFill::Int => {
                encode.instruction(&Instruction::LocalGet(rep));
                for &idx in &hole.path {
                    encode.instruction(&Instruction::I32Const(idx as i32));
                    encode.instruction(&Instruction::Call(f_arr_get));
                }
                encode.instruction(&Instruction::Call(f_get_int));
                encode.instruction(&Instruction::LocalSet(scratch));
                encode.instruction(&Instruction::LocalGet(scratch));
                encode.instruction(&Instruction::I64Const(0));
                encode.instruction(&Instruction::I64LtS);
                encode.instruction(&Instruction::If(BlockType::Empty));
                encode.instruction(&Instruction::I32Const((out_off - 2) as i32));
                encode.instruction(&Instruction::I32Const(3));
                encode.instruction(&Instruction::I32Store8(MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
                encode.instruction(&Instruction::I64Const(0));
                encode.instruction(&Instruction::LocalGet(scratch));
                encode.instruction(&Instruction::I64Sub);
                encode.instruction(&Instruction::LocalSet(scratch));
                encode.instruction(&Instruction::End);
                // Write the 8 big-endian magnitude bytes at the hole offset.
                for b in 0..8u64 {
                    encode.instruction(&Instruction::I32Const((out_off + b) as i32));
                    encode.instruction(&Instruction::LocalGet(scratch));
                    let shift = (7 - b) * 8;
                    if shift > 0 {
                        encode.instruction(&Instruction::I64Const(shift as i64));
                        encode.instruction(&Instruction::I64ShrU);
                    }
                    encode.instruction(&Instruction::I32WrapI64);
                    encode.instruction(&Instruction::I32Store8(MemArg {
                        offset: 0,
                        align: 0,
                        memory_index: 0,
                    }));
                }
            }
            LeafFill::Bool => {
                encode.instruction(&Instruction::I32Const(out_off as i32));
                encode.instruction(&Instruction::LocalGet(rep));
                for &idx in &hole.path {
                    encode.instruction(&Instruction::I32Const(idx as i32));
                    encode.instruction(&Instruction::Call(f_arr_get));
                }
                encode.instruction(&Instruction::Call(idx_of("get-bool")));
                encode.instruction(&Instruction::I32Const(8));
                encode.instruction(&Instruction::I32Add);
                encode.instruction(&Instruction::I32Store8(MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }));
            }
        }
    }
    encode.instruction(&Instruction::I32Const(ret_off as i32));
    encode.instruction(&Instruction::End);

    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);

    let mut code = CodeSection::new();
    code.function(&make);
    code.function(&encode);
    code.function(&realloc);
    m.section(&code);
    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(0), data_bytes.iter().copied());
    m.section(&data);
    m.finish()
}

/// The BORROW oracle: identical envelope EXCEPT `encode` is lifted against `borrow<t>` (not `own<t>`),
/// and the inner re-export component re-types `encode` against a borrowed self. `resource.rep` is NOT
/// threaded (the borrow core never calls it). This is the target production shape.
fn oracle_runtime_resource_component_borrow(core: &[u8], import_name: &str) -> Vec<u8> {
    use wasm_encoder::*;
    let ops = walker_ops();
    let mut c = ComponentBuilder::default();
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
    let drop_core = lowered
        .iter()
        .find(|(n, _)| *n == "drop")
        .map(|(_, f)| *f)
        .expect("drop op");
    let heap_dtor_inst = c.core_instantiate_exports([("drop", ExportKind::Func, drop_core)]);
    let dtor_idx = c.core_module_raw(&dtor_module());
    let dtor_inst = c.core_instantiate(
        dtor_idx,
        [("heap-dtor", ModuleArg::Instance(heap_dtor_inst))],
    );
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    // Only resource-new is threaded (no resource-rep — the borrow rep is the param).
    let mut heap_exports: Vec<(&str, ExportKind, u32)> = lowered
        .iter()
        .map(|(n, f)| (*n, ExportKind::Func, *f))
        .collect();
    heap_exports.push(("resource-new", ExportKind::Func, rnew_core));
    let heap_inst = c.core_instantiate_exports(heap_exports);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let encode_core = c.core_alias_export(prog_inst, "t-encode", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (borrow_t, bdef) = c.type_defined();
    bdef.borrow(res_ty);
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (encode_ty, mut enc2) = c.type_function();
    enc2.params([("self", ComponentValType::Type(borrow_t))])
        .result(Some(ComponentValType::Type(list_u8)));
    let encode_comp = c.lift_func(
        encode_core,
        encode_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    let inner_idx = c.component(inner_reexport_component_borrow());
    let inst2 = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-encode", ComponentExportKind::Func, encode_comp),
        ],
    );
    c.export(
        "cadenza:run/run",
        ComponentExportKind::Instance,
        inst2,
        None,
    );
    c.finish()
}

/// The borrow inner re-export component: `encode` re-typed against `borrow<t>` (both against the
/// imported abstract resource and re-declared against the exported one). Mirrors the proven
/// `r1_reference::inner_reexport_component_borrow` shape.
fn inner_reexport_component_borrow() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    let (borrow_imp, bd) = c.type_defined();
    bd.borrow(imp_t);
    let (list1, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_ty, mut ef) = c.type_function();
    ef.params([("self", ComponentValType::Type(borrow_imp))])
        .result(Some(ComponentValType::Type(list1)));
    let enc_fn = c.import("import-func-encode", ComponentTypeRef::Func(enc_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od2) = c.type_defined();
    od2.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (borrow_exp, bd2) = c.type_defined();
    bd2.borrow(exp_t);
    let (list2, ld2) = c.type_defined();
    ld2.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_exp_ty, mut ef2) = c.type_function();
    ef2.params([("self", ComponentValType::Type(borrow_exp))])
        .result(Some(ComponentValType::Type(list2)));
    c.export(
        "encode",
        ComponentExportKind::Func,
        enc_fn,
        Some(ComponentTypeRef::Func(enc_exp_ty)),
    );
    c
}

#[test]
fn a_borrow_self_encode_walks_and_crosses() {
    // THE DECISIVE PROBE for the resource-with-methods redesign: `encode` takes `borrow<t>` and uses
    // its param DIRECTLY as the heap rep (NO `resource.rep` — which traps on a borrow; the canonical
    // ABI's `lift_borrow` returns the rep itself, confirmed in wasmtime 37's `resource_lift_borrow`)
    // and does NOT drop (a borrow does not own — the value survives, so the resource is repeatable).
    // If this composes + runs + decodes to the exact value form, the clean borrow design works
    // end-to-end and the own-consume-and-drop hack can be retired. Build `(tuple 3 1)`, escape it as a
    // borrow-self resource, walk it in encode(borrow), decode.
    let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    let core = walker_core_borrow(&tpl, &[3, 1]);
    use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    let comp = oracle_runtime_resource_component_borrow(&core, &import_name);
    // STRUCTURAL pin: the hand-built borrow-self resource envelope (encode takes `borrow<t>`, uses the
    // param directly as the heap rep — no `resource.rep`, no drop) composes into a VALID component — the
    // ABI-shape guard for the borrow-self redesign. The RUN — the walked tuple decodes to
    // `(: (tuple 3 1) (Tuple Int64 Int64))` — is corpus-covered by the runtime tuple-escape cases in
    // 05-compound-types; a hand-built walker core cannot be a corpus (Cadenza-source) program, so it
    // stays a compile+validate pin (the R2-walker / x3-x4a family).
    let mut validator = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    validator
        .validate_all(&comp)
        .expect("borrow runtime-resource component validates");
}

/// The hand-emitted combined runtime-import + resource envelope
/// ([`crate::backend::wasm::envelope::assemble_runtime_resource`]) is BYTE-IDENTICAL to the
/// `ComponentBuilder` combined oracle — the R2 byte gate, licensing the hand-emit of the fused
/// import-prologue + resource shape (now with the DROP-calling dtor + its `heap-dtor` instance) with
/// no external encoder in the compile path. Diffs against the SAME walker core + dtor module the
/// oracle wraps, so the diff isolates the ENVELOPE. The op set is the seven walker ops (incl. `drop`)
/// in the SAME sorted order the compiler's used-set would produce.
#[test]
fn combined_envelope_matches_component_builder_oracle() {
    use crate::backend::wasm::envelope::assemble_runtime_resource;
    let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    let core = walker_core(&tpl, &[3, 1]);
    let dtor = dtor_module();
    let import_name = "cadenza:runtime/heap@0.0.0+deadbeef";
    let ops: Vec<&RtOp> = walker_ops().to_vec();
    let ours = assemble_runtime_resource(&core, &dtor, &ops, import_name, &[]);
    let oracle = oracle_runtime_resource_component(&core, import_name);
    assert_eq!(
        ours, oracle,
        "combined envelope mismatch vs ComponentBuilder"
    );
}

/// VM-1 byte gate: the hand-emitted `assemble_runtime_resource_with_len` (make + encode + a scalar
/// `len : borrow<t> -> u32`) is BYTE-IDENTICAL to the `ComponentBuilder` `oracle_tuple_methods`
/// reference. Both wrap the SAME `tuple_methods_core` (which exports `t-len` = arr-len) + the SAME
/// dtor + the same `methods_ops` set, so the diff isolates the ENVELOPE — licensing the hand-emitted
/// three-method envelope (the added `len` lift + inner-component re-export) with no external encoder.
#[test]
fn len_method_envelope_matches_component_builder_oracle() {
    use crate::backend::wasm::envelope::assemble_runtime_resource_with_len;
    let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
    let tpl = runtime_value_form_template(&ty, &crate::ty::NameCtx::new(&[])).expect("template");
    let core = tuple_methods_core(&tpl, &[7, 9]);
    let dtor = dtor_module();
    let import_name = "cadenza:runtime/heap@0.0.0+deadbeef";
    let ops: Vec<&RtOp> = methods_ops().to_vec();
    let ours = assemble_runtime_resource_with_len(&core, &dtor, &ops, import_name, &[]);
    let oracle = oracle_tuple_methods(&core, import_name);
    assert_eq!(
        ours, oracle,
        "len-method envelope mismatch vs ComponentBuilder"
    );
}

/// The compiler's hand-emitted drop-dtor core module
/// ([`crate::backend::wasm::serialize::resource_dtor_module_with_drop`]) is byte-identical to the
/// oracle's `dtor_module` — so the envelope byte test above (which wraps the oracle's dtor bytes)
/// also pins the real dtor the compiler emits.
#[test]
fn drop_dtor_module_matches_the_oracle() {
    assert_eq!(
        crate::backend::wasm::serialize::resource_dtor_module_with_drop(),
        dtor_module(),
        "the compiler's drop-dtor module must match the oracle's"
    );
}

// ── #20 FEASIBILITY PROBE: a value resource with a REPEATABLE second method ──────────────────────
//
// The value resource already carries TWO methods (make + encode) and encode is now `borrow<t>`
// (repeatable). The genuinely-new risk for String/Bytes-as-resource-with-methods is whether ONE live
// handle survives a SEQUENCE of borrow-method calls — `make → len → len → encode → drop` — since a
// wasmtime borrow-lend scope is per-call and multiple lends of the same handle could interact. This
// oracle proves it does: a `(tuple 7 9)` resource whose `len : borrow<t> -> u32` reads `arr-len(rep)`
// (repeatable, no consume) coexists with `encode`, and every call uses the borrow's rep DIRECTLY.

/// The runtime ops this multi-method core imports (sorted by name — the order the compiler's used-set
/// would produce): arr-alloc/arr-get/arr-len/arr-set build + read the tuple, box-int boxes a leaf,
/// drop is the dtor's release op, get-int reads a leaf value in encode.
fn methods_ops() -> [&'static RtOp; 7] {
    [
        OPS.arr_alloc,
        OPS.arr_get,
        OPS.arr_len,
        OPS.arr_set,
        OPS.box_int,
        OPS.drop,
        OPS.get_int,
    ]
}

/// The program core for the multi-method probe: `make()` builds `(tuple 7 9)` + resource.new; the
/// BORROW methods `t-encode(rep)` (walk the value-form holes) and `t-len(rep)` (= `arr-len(rep)`) both
/// use the param DIRECTLY as the rep (no resource.rep) and do NOT drop. Imports the ops + resource-new
/// from "heap"; exports memory/make/t-encode/t-len/cabi_realloc.
fn tuple_methods_core(tpl: &ValueFormTemplate, elems: &[i64]) -> Vec<u8> {
    use wasm_encoder::*;
    let ops = methods_ops();
    let mut m = Module::new();
    let mut types = TypeSection::new();
    let mut import_type_idx = Vec::new();
    for op in ops {
        let (p, r) = op_core_functype(op);
        types.ty().function(p, r);
        import_type_idx.push(import_type_idx.len() as u32);
    }
    let rnew_ty = import_type_idx.len() as u32;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // resource-new (i32)->i32
    let rrep_ty = rnew_ty + 1;
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // resource-rep (i32)->i32
    let make_ty = rrep_ty + 1;
    types.ty().function(vec![], vec![ValType::I32]);
    let unary_ty = make_ty + 1; // (i32)->i32 for both t-encode and t-len
    types.ty().function(vec![ValType::I32], vec![ValType::I32]);
    let realloc_ty = unary_ty + 1;
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    );
    m.section(&types);

    // Imports: the ops + resource-new + resource-rep (BOTH, matching the production core; the borrow
    // methods never call resource-rep but it is imported for uniform index parity with value/sum cores).
    let mut imports = ImportSection::new();
    for (i, op) in ops.iter().enumerate() {
        imports.import("heap", op.name, EntityType::Function(import_type_idx[i]));
    }
    imports.import("heap", "resource-new", EntityType::Function(rnew_ty));
    imports.import("heap", "resource-rep", EntityType::Function(rrep_ty));
    m.section(&imports);
    let idx_of = |name: &str| ops.iter().position(|o| o.name == name).unwrap() as u32;
    let f_arr_alloc = idx_of("arr-alloc");
    let f_arr_get = idx_of("arr-get");
    let f_arr_len = idx_of("arr-len");
    let f_arr_set = idx_of("arr-set");
    let f_box_int = idx_of("box-int");
    let f_get_int = idx_of("get-int");
    let f_rnew = ops.len() as u32; // resource-new is import k

    let make_fn = ops.len() as u32 + 2; // k ops + resource-new + resource-rep
    let encode_fn = make_fn + 1;
    let len_fn = encode_fn + 1;
    let realloc_fn = len_fn + 1;
    let mut funcs = FunctionSection::new();
    funcs.function(make_ty);
    funcs.function(unary_ty); // t-encode
    funcs.function(unary_ty); // t-len
    funcs.function(realloc_ty);
    m.section(&funcs);

    let mut mems = MemorySection::new();
    mems.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    m.section(&mems);

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("make", ExportKind::Func, make_fn);
    exports.export("t-encode", ExportKind::Func, encode_fn);
    exports.export("t-len", ExportKind::Func, len_fn);
    exports.export("cabi_realloc", ExportKind::Func, realloc_fn);
    m.section(&exports);

    let tpl_len = tpl.bytes.len();
    let ret_off = (tpl_len + 3) & !3;
    let mut data_bytes = tpl.bytes.clone();
    data_bytes.resize(ret_off, 0);
    data_bytes.extend_from_slice(&0u32.to_le_bytes());
    data_bytes.extend_from_slice(&(tpl_len as u32).to_le_bytes());

    // make: build the tuple, then resource.new(handle).
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(elems.len() as i32));
    make.instruction(&Instruction::Call(f_arr_alloc));
    for (i, &v) in elems.iter().enumerate() {
        make.instruction(&Instruction::I32Const(i as i32));
        make.instruction(&Instruction::I64Const(v));
        make.instruction(&Instruction::Call(f_box_int));
        make.instruction(&Instruction::Call(f_arr_set));
    }
    make.instruction(&Instruction::Call(f_rnew));
    make.instruction(&Instruction::End);

    // t-encode(borrow rep): the param IS the rep; walk each hole (int leaves only here). No drop.
    let mut encode = Function::new(vec![(1, ValType::I64)]);
    let rep = 0u32;
    let scratch = 1u32;
    for hole in &tpl.leaves {
        let out_off = hole.offset as u64;
        // int-leaf write (the probe uses only non-negative ints, so skip the neg-flip for brevity —
        // 7 and 9 are positive; the negative path is already covered by walker_core_borrow's tests).
        encode.instruction(&Instruction::LocalGet(rep));
        for &idx in &hole.path {
            encode.instruction(&Instruction::I32Const(idx as i32));
            encode.instruction(&Instruction::Call(f_arr_get));
        }
        encode.instruction(&Instruction::Call(f_get_int));
        encode.instruction(&Instruction::LocalSet(scratch));
        for b in 0..8u64 {
            encode.instruction(&Instruction::I32Const((out_off + b) as i32));
            encode.instruction(&Instruction::LocalGet(scratch));
            let shift = (7 - b) * 8;
            if shift > 0 {
                encode.instruction(&Instruction::I64Const(shift as i64));
                encode.instruction(&Instruction::I64ShrU);
            }
            encode.instruction(&Instruction::I32WrapI64);
            encode.instruction(&Instruction::I32Store8(MemArg {
                offset: 0,
                align: 0,
                memory_index: 0,
            }));
        }
    }
    encode.instruction(&Instruction::I32Const(ret_off as i32));
    encode.instruction(&Instruction::End);

    // t-len(borrow rep) -> u32: the param IS the rep; `arr-len(rep)`. Repeatable — reads without
    // consuming, no drop. This is the shape a String/Bytes `len` method takes (bytes-len instead).
    let mut len = Function::new(vec![]);
    len.instruction(&Instruction::LocalGet(rep));
    len.instruction(&Instruction::Call(f_arr_len));
    len.instruction(&Instruction::End);

    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);

    let mut code = CodeSection::new();
    code.function(&make);
    code.function(&encode);
    code.function(&len);
    code.function(&realloc);
    m.section(&code);
    let mut data = DataSection::new();
    data.active(0, &ConstExpr::i32_const(0), data_bytes.iter().copied());
    m.section(&data);
    m.finish()
}

/// The multi-method oracle: the borrow envelope plus a THIRD lifted method `len : borrow<t> -> u32`
/// alongside make + encode, all published in `cadenza:run/run`. Mirrors
/// `oracle_runtime_resource_component_borrow` with the extra len functype/lift + inner re-export entry.
fn oracle_tuple_methods(core: &[u8], import_name: &str) -> Vec<u8> {
    use wasm_encoder::*;
    let ops = methods_ops();
    let mut c = ComponentBuilder::default();
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
    let drop_core = lowered
        .iter()
        .find(|(n, _)| *n == "drop")
        .map(|(_, f)| *f)
        .expect("drop op");
    let heap_dtor_inst = c.core_instantiate_exports([("drop", ExportKind::Func, drop_core)]);
    let dtor_idx = c.core_module_raw(&dtor_module());
    let dtor_inst = c.core_instantiate(
        dtor_idx,
        [("heap-dtor", ModuleArg::Instance(heap_dtor_inst))],
    );
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    let rrep_core = c.resource_rep(res_ty);
    let mut heap_exports: Vec<(&str, ExportKind, u32)> = lowered
        .iter()
        .map(|(n, f)| (*n, ExportKind::Func, *f))
        .collect();
    // Thread BOTH resource intrinsics — the production core module imports resource-new (for make) AND
    // resource-rep (unused by the borrow methods, but present for index-parity with the value/sum
    // resource cores, which the compiler emits uniformly). Matches `assemble_runtime_resource_with_len`.
    heap_exports.push(("resource-new", ExportKind::Func, rnew_core));
    heap_exports.push(("resource-rep", ExportKind::Func, rrep_core));
    let heap_inst = c.core_instantiate_exports(heap_exports);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    // Alias ORDER fixes the component-level core-func indices — match the hand-emit
    // `assemble_runtime_resource_with_len`: make (k+3), t-encode (k+4), memory (not a func),
    // cabi_realloc (k+5), then t-len LAST (k+6). (memory is a Memory alias, not a func index.)
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let encode_core = c.core_alias_export(prog_inst, "t-encode", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    let len_core = c.core_alias_export(prog_inst, "t-len", ExportKind::Func);
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (borrow_t, bdef) = c.type_defined();
    bdef.borrow(res_ty);
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (encode_ty, mut enc2) = c.type_function();
    enc2.params([("self", ComponentValType::Type(borrow_t))])
        .result(Some(ComponentValType::Type(list_u8)));
    let encode_comp = c.lift_func(
        encode_core,
        encode_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    // len : (self: borrow<t>) -> u32. Reuse the borrow<t> defined type; no memory/realloc (scalar).
    let (len_ty, mut lf) = c.type_function();
    lf.params([("self", ComponentValType::Type(borrow_t))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    let len_comp = c.lift_func(len_core, len_ty, []);
    let inner_idx = c.component(inner_reexport_component_methods());
    let inst2 = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-encode", ComponentExportKind::Func, encode_comp),
            ("import-func-len", ComponentExportKind::Func, len_comp),
        ],
    );
    c.export(
        "cadenza:run/run",
        ComponentExportKind::Instance,
        inst2,
        None,
    );
    c.finish()
}

/// The inner re-export component for the multi-method oracle: imports the abstract resource + make +
/// encode + len, re-exports the resource directly and re-declares all three against it. Extends
/// `inner_reexport_component_borrow` with the `len` method.
fn inner_reexport_component_methods() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    );
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty));
    let (borrow_imp, bd) = c.type_defined();
    bd.borrow(imp_t);
    let (list1, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_ty, mut ef) = c.type_function();
    ef.params([("self", ComponentValType::Type(borrow_imp))])
        .result(Some(ComponentValType::Type(list1)));
    let enc_fn = c.import("import-func-encode", ComponentTypeRef::Func(enc_ty));
    let (borrow_imp2, bd1b) = c.type_defined();
    bd1b.borrow(imp_t);
    let (len_ty, mut lf) = c.type_function();
    lf.params([("self", ComponentValType::Type(borrow_imp2))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    let len_fn = c.import("import-func-len", ComponentTypeRef::Func(len_ty));
    let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
    let (own_exp, od2) = c.type_defined();
    od2.own(exp_t);
    let (make_exp_ty, mut mf2) = c.type_function();
    mf2.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_exp)));
    c.export(
        "make",
        ComponentExportKind::Func,
        make_fn,
        Some(ComponentTypeRef::Func(make_exp_ty)),
    );
    let (borrow_exp, bd2) = c.type_defined();
    bd2.borrow(exp_t);
    let (list2, ld2) = c.type_defined();
    ld2.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_exp_ty, mut ef2) = c.type_function();
    ef2.params([("self", ComponentValType::Type(borrow_exp))])
        .result(Some(ComponentValType::Type(list2)));
    c.export(
        "encode",
        ComponentExportKind::Func,
        enc_fn,
        Some(ComponentTypeRef::Func(enc_exp_ty)),
    );
    let (borrow_exp2, bd3) = c.type_defined();
    bd3.borrow(exp_t);
    let (len_exp_ty, mut lf2) = c.type_function();
    lf2.params([("self", ComponentValType::Type(borrow_exp2))])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    c.export(
        "len",
        ComponentExportKind::Func,
        len_fn,
        Some(ComponentTypeRef::Func(len_exp_ty)),
    );
    c
}
