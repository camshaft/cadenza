use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};

/// A minimal core module returning a constant `list<u8>` by the canonical ABI: exports `memory`, a
/// stub `cabi_realloc`, and `main : () -> i32` returning a pointer to an 8-byte return area holding
/// `[data-ptr:i32, data-len:i32]`. The payload bytes and the return area are preloaded via a data
/// segment (so `main` is just `i32.const <retarea>`), which is enough to RUN — the host reads the
/// result out of guest memory and never calls realloc for a nullary result. Payload = `[1,2,3]`.
/// The exact core the R0 serializer will emit (a real renderer writing bytes + the return area) is
/// R1/R2; this fixture isolates the envelope's list-lift byte layer.
pub(super) fn working_list_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    // Types: main `()->i32`, cabi_realloc `(i32×4)->i32`.
    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![ValType::I32]); // 0: main
    types.ty().function(
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        vec![ValType::I32],
    ); // 1: cabi_realloc
    m.section(&types);
    let mut funcs = FunctionSection::new();
    funcs.function(0); // main
    funcs.function(1); // cabi_realloc
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
    exports.export("main", ExportKind::Func, 0);
    exports.export("cabi_realloc", ExportKind::Func, 1);
    m.section(&exports);
    // Data: payload [1,2,3] at offset 0; return area [ptr=0, len=3] at offset 8.
    let mut data = DataSection::new();
    let bytes = [
        1u8, 2, 3, 0, 0, 0, 0, 0, /* retarea@8 */ 0, 0, 0, 0, 3, 0, 0, 0,
    ];
    data.active(0, &ConstExpr::i32_const(0), bytes.iter().copied());
    // Code.
    let mut code = CodeSection::new();
    let mut main = Function::new(vec![]);
    main.instruction(&Instruction::I32Const(8)); // return the retarea pointer
    main.instruction(&Instruction::End);
    code.function(&main);
    let mut realloc = Function::new(vec![]);
    realloc.instruction(&Instruction::I32Const(0)); // stub (never called for a nullary result)
    realloc.instruction(&Instruction::End);
    code.function(&realloc);
    m.section(&code);
    m.section(&data);
    m.finish()
}

/// Wrap `core` in a `() -> list<u8>` component with `ComponentBuilder` — the authoritative reference
/// the hand-emitted list-lift envelope is diffed against (memory + realloc aliases + the `list u8`
/// defined type + the Memory/Realloc canon-lift options). No runtime import (the bare shape).
fn oracle_list_component(core: &[u8]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    let module_idx = c.core_module_raw(core); // core module 0
    let prog_inst = c.core_instantiate(module_idx, std::iter::empty::<(&str, ModuleArg)>());
    let main_core = c.core_alias_export(prog_inst, "main", ExportKind::Func);
    let mem = c.core_alias_export(prog_inst, "memory", ExportKind::Memory);
    let realloc = c.core_alias_export(prog_inst, "cabi_realloc", ExportKind::Func);
    let (list_u8, ldef) = c.type_defined();
    ldef.list(ComponentValType::Primitive(PrimitiveValType::U8));
    let (main_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Type(list_u8)));
    let main_comp = c.lift_func(
        main_core,
        main_ty,
        [
            CanonicalOption::Memory(mem),
            CanonicalOption::Realloc(realloc),
        ],
    );
    c.export("main", ComponentExportKind::Func, main_comp, None);
    c.finish()
}

/// The hand-emitted `() -> list<u8>` bare-escape envelope is BYTE-IDENTICAL to the `ComponentBuilder`
/// oracle — the anchor that licenses hand-emitting the memory/realloc-aliasing + list-lift plumbing
/// with no external encoder in the compile path (`reference-compiler.md` §Emission Is Validated
/// Byte-Identical To An Independent Encoder). This is R0's byte gate.
#[test]
fn list_u8_envelope_matches_component_builder_oracle() {
    let core = working_list_core();
    let exports = vec![BoundaryExport {
        name: "main".to_string(),
        params: Vec::new(),
        result: BoundaryResult::Bytes,
    }];
    let ours = assemble(&core, &exports, &[], "");
    let oracle = oracle_list_component(&core);
    assert_eq!(
        ours, oracle,
        "list<u8> envelope mismatch vs ComponentBuilder"
    );
}
