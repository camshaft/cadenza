/// The DTOR core module — a standalone unit exporting `t-dtor : (i32 rep) -> ()`, the destructor
/// the component invokes on host-drop. The resource wraps an rc handle to the runtime heap, so the
/// dtor must release it (call the runtime's `drop`); for R1's constant-bytes proof the BODY is a
/// stub, but the SLOT is wired end-to-end (R2 fills it with the real drop call). KEY: keeping the
/// dtor in its OWN module (which imports nothing) is what lets us avoid the wit-bindgen shim/fixup
/// — this module can be instantiated FIRST, so the resource type has a real dtor core-func before
/// `resource.new` (and hence the main module) needs the resource type. No circular dependency.
fn dtor_module() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![]); // 0: (i32)->()
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("t-dtor", ExportKind::Func, 0);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut dtor = Function::new(vec![]);
    dtor.instruction(&Instruction::End); // stub: release the rep (real `drop` is R2)
    code.function(&dtor);
    m.section(&code);
    m.finish()
}

/// The MAIN core module for the resource oracle. IMPORTS `heap.resource-new : (i32 rep) -> i32
/// handle` (the `resource.new` intrinsic the component threads in — a raw rep is NOT auto-wrapped
/// by the lift; `make` MUST register it, else "unknown handle index"). Exports `memory`,
/// `cabi_realloc`, `make : () -> i32 handle` (dummy rep `7` → resource-new → a handle), and
/// `t-encode : (i32 handle-rep) -> i32 retptr` (retptr to constant `list<u8>` `[1,2,3]`). The dtor
/// lives in [`dtor_module`] (separate → no cycle).
fn resource_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0: (i32)->i32 (resource-new / encode)
    types.ty().function(vec![], vec![ValType::I32]); // 1: make ()->i32
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    ); // 2: cabi_realloc
    m.section(&types);
    // Import resource-new : (rep)->handle (type 0), from module "heap".
    let mut imports = ImportSection::new();
    imports.import("heap", "resource-new", EntityType::Function(0));
    m.section(&imports);
    // Defined funcs start at index 1 (import is func 0): make=1, encode=2, realloc=3.
    let mut funcs = FunctionSection::new();
    funcs.function(1); // make ()->i32
    funcs.function(0); // encode (i32)->i32 (reuse type 0 shape)
    funcs.function(2); // cabi_realloc
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
    exports.export("make", ExportKind::Func, 1);
    exports.export("t-encode", ExportKind::Func, 2);
    exports.export("cabi_realloc", ExportKind::Func, 3);
    m.section(&exports);
    let mut data = DataSection::new();
    // payload [1,2,3] @0; return area [ptr=0, len=3] @8.
    let bytes = [1u8, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0];
    data.active(0, &ConstExpr::i32_const(0), bytes.iter().copied());
    let mut code = CodeSection::new();
    // make: rep 7 → resource-new(7) → handle.
    let mut make = Function::new(vec![]);
    make.instruction(&Instruction::I32Const(7));
    make.instruction(&Instruction::Call(0)); // call the imported resource-new
    make.instruction(&Instruction::End);
    code.function(&make);
    // encode: ignore the rep, return the retptr.
    let mut encode = Function::new(vec![]);
    encode.instruction(&Instruction::I32Const(8));
    encode.instruction(&Instruction::End);
    code.function(&encode);
    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0));
    realloc.instruction(&Instruction::End);
    code.function(&realloc);
    m.section(&code);
    m.section(&data);
    m.finish()
}

/// The INNER re-export component (a real nested component, not a type): it IMPORTS an abstract
/// resource + the two funcs typed against it, and RE-EXPORTS the resource (making its identity
/// public) + the funcs typed against the EXPORTED resource. This is the wit-bindgen mechanism that
/// converts a rep-carrying internal resource identity into an exported abstract one — the ONLY way
/// to export a resource-with-methods (a flat top-level func typed against the rep-carrying type "is
/// not valid to be used as an export"; one typed against the abstract exported type treats a
/// returned rep as an existing handle → "unknown handle index"). Its body is pure imports + exports
/// (no core content); the outer component instantiates it with the real (rep-carrying) resource +
/// lifted funcs.
fn inner_reexport_component() -> wasm_encoder::ComponentBuilder {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // import the abstract resource → type 0, func 0/1 references.
    let imp_t = c.import(
        "import-type-t",
        ComponentTypeRef::Type(TypeBounds::SubResource),
    ); // type 0
    // make : () -> own<0>.
    let (own_imp, od) = c.type_defined();
    od.own(imp_t);
    let (make_ty, mut mf) = c.type_function();
    mf.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_imp)));
    let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty)); // func 0
    // encode : (self: own<0>) -> list<u8>.
    let (list1, ld) = c.type_defined();
    ld.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (enc_ty, mut ef) = c.type_function();
    ef.params([("self", ComponentValType::Type(own_imp))])
        .result(Some(ComponentValType::Type(list1)));
    let enc_fn = c.import("import-func-encode", ComponentTypeRef::Func(enc_ty)); // func 1
    // RE-EXPORT the resource type (publishing its identity), then the funcs against it. The
    //  re-declared func types (against the EXPORTED resource) are built BEFORE the export call —
    //  `type_defined`/`type_function` borrow `c`, so they cannot run inside a `c.export(...)` arg.
    // Re-export the imported resource type DIRECTLY (no `SubResource` ascription — that would mint
    // a fresh resource identity distinct from `imp_t`, and the re-typed func exports would then
    // reference a different resource → "resource types are not the same"). Re-publishes `imp_t`'s
    // identity under the name `t`, returning its new export-index.
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

/// Build the resource-exporting component with `ComponentBuilder` — the authoritative reference.
/// A LEANER shape than wit-bindgen's: because our DTOR lives in its own core module (importing
/// nothing — [`dtor_module`]), we instantiate it FIRST, so the resource type has a real dtor
/// core-func before `resource.new` (and the main module) need the resource type. This dissolves the
/// circular dependency WITHOUT the shim/fixup trampoline+table+elem dance wit-bindgen needs (it
/// only needs the shim because it puts the dtor in the same module that imports `resource.new`).
/// Order: dtor module → resource type (with real dtor) → lower `resource.new` → main module
/// (threading `resource.new` in) → lift make/encode → inner re-export component → export instance.
fn oracle_resource_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // (1) dtor module FIRST — a self-contained unit, so its `t-dtor` core func exists before the
    //     resource type needs it. No cycle → no shim/fixup.
    let dtor_idx = c.core_module_raw(&dtor_module());
    let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
    let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
    // (2) resource `t` with the REAL dtor; lower resource.new(t) → a core func.
    let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
    let rnew_core = c.resource_new(res_ty);
    // (3) thread resource.new into the main core module (as `heap.resource-new`) + instantiate.
    let heap_inst = c.core_instantiate_exports([("resource-new", ExportKind::Func, rnew_core)]);
    let module_idx = c.core_module_raw(core);
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
    let encode_core = c.core_alias_export(prog_inst, "t-encode", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    // (4) lift make/encode against the INTERNAL res_ty (rep-carrying).
    let (own_t, odef) = c.type_defined();
    odef.own(res_ty);
    let (make_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(own_t)));
    let make_comp = c.lift_func(make_core, make_ty, []);
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (encode_ty, mut enc2) = c.type_function();
    enc2.params([("self", ComponentValType::Type(own_t))])
        .result(Some(ComponentValType::Type(list_u8)));
    let encode_comp = c.lift_func(
        encode_core,
        encode_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    // (6) inner re-export component → the `cadenza:run/run` instance.
    let inner_idx = c.component(inner_reexport_component());
    let inst = c.instantiate(
        inner_idx,
        [
            ("import-type-t", ComponentExportKind::Type, res_ty),
            ("import-func-make", ComponentExportKind::Func, make_comp),
            ("import-func-encode", ComponentExportKind::Func, encode_comp),
        ],
    );
    c.export("cadenza:run/run", ComponentExportKind::Instance, inst, None);
    c.finish()
}

/// The `ComponentBuilder` reference for the R1 resource-export envelope: a monomorphized resource
/// `t` (rep i32, with a dtor) exported inside the `cadenza:run/run` instance, alongside `make : ()
/// -> own<t>` (calls `resource.new`) and `encode : (own<t>) -> list<u8>` (the canonical binary
/// value form). Built via the leaner resource-linking pattern (separate dtor module + threaded
/// `resource.new` + inner re-export component, no shim/fixup). VALIDATES under wasmparser + wasmtime and RUNS the
/// round-trip: `make()` → a strongly-typed resource handle (NOT a bare u32), `encode(handle)` →
/// `[1,2,3]`. This is the authoritative shape the hand-emitted `envelope.rs` R1 path must mirror.
/// The hand-emitted resource-escape envelope ([`envelope::assemble_resource`]) is BYTE-IDENTICAL to
/// the `ComponentBuilder` oracle — the anchor that licenses hand-emitting the resource + dtor +
/// `resource.new` + nested-re-export plumbing with no external encoder in the compile path
/// (`reference-compiler.md` §Emission Is Validated Byte-Identical To An Independent Encoder). This is
/// R1's byte gate; it takes the SAME core modules the oracle wraps (`resource_core` + `dtor_module`),
/// so the diff isolates the ENVELOPE encoding (the core-module emission is a separate concern, R2).
#[test]
fn resource_envelope_matches_component_builder_oracle() {
    use crate::backend::wasm::envelope::assemble_resource;
    let main_core = resource_core();
    let dtor_core = dtor_module();
    let ours = assemble_resource(&main_core, &dtor_core);
    let oracle = oracle_resource_component(&main_core);
    assert_eq!(
        ours, oracle,
        "resource envelope mismatch vs ComponentBuilder"
    );
}
