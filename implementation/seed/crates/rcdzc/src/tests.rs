//! Stage-0 end-to-end tests: the thin vertical slice compiles, its bytes are byte-identical to an
//! independent encoder, and the emitted component RUNS to the recorded value.
//!
//! Two independent checks, both dev-only (neither `wasm-encoder` nor `wasmtime` is in the compile
//! path — the byte path is hand-emitted so it ports to the self-host):
//!  - the byte ORACLE — the hand-emitted component is byte-identical to what the authoritative
//!    component-model encoder (`wasm-encoder`) produces for the same core module, at N=1 and N=2
//!    exports (`reference-compiler.md` §Emission Is Validated Byte-Identical To An Independent
//!    Encoder);
//!  - the BEHAVIOR run — the component instantiates under `wasmtime` and its export returns the value
//!    the corpus records (`42`; the `if` case `2`), observed by EXECUTION, not by inspecting shape
//!    (`reference-compiler.md` §The Gate Runs The Artifact).

#![cfg(test)]

use crate::ast::{Builder, IntValue, Leaf, Radix};
use crate::compile::compile_component;

// ── program builders (whole programs, distinct from the per-node fixtures in `testkit`) ──────────

/// `(module m (def (main) 42) (export main))` — the scalar slice. Returns its AST bytes.
fn prog_scalar() -> Vec<u8> {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    let def = b.name("def");
    let sig = {
        let main = b.name("main");
        b.list(vec![main])
    };
    let body = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(42),
        radix: Radix::Dec,
    });
    let def_form = b.list(vec![def, sig, body]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    crate::codec::encode(&b.finish(root))
}

/// `(module m (def (main) (if false 1 2)) (export main))` — the two-way-branch slice.
fn prog_if() -> Vec<u8> {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    let def = b.name("def");
    let sig = {
        let main = b.name("main");
        b.list(vec![main])
    };
    let if_head = b.name("if");
    let cond = b.atom_leaf(Leaf::Bool(false));
    let then_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(1),
        radix: Radix::Dec,
    });
    let else_ = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(2),
        radix: Radix::Dec,
    });
    let if_form = b.list(vec![if_head, cond, then_, else_]);
    let def_form = b.list(vec![def, sig, if_form]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    crate::codec::encode(&b.finish(root))
}

/// A malformed program: `(module m (def (main) (frobnicate 1)) (export main))` — an unsupported form
/// reaches the entry unconditionally, so it must decline (no component + a diagnostic).
fn prog_unsupported() -> Vec<u8> {
    let mut b = Builder::new();
    let module = b.name("module");
    let m = b.name("m");
    let def = b.name("def");
    let sig = {
        let main = b.name("main");
        b.list(vec![main])
    };
    let frob = b.name("frobnicate");
    let one = b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(1),
        radix: Radix::Dec,
    });
    let body = b.list(vec![frob, one]);
    let def_form = b.list(vec![def, sig, body]);
    let export = b.name("export");
    let main_ref = b.name("main");
    let export_form = b.list(vec![export, main_ref]);
    let root = b.list(vec![module, m, def_form, export_form]);
    crate::codec::encode(&b.finish(root))
}

// ── the byte oracle ──────────────────────────────────────────────────────────────────────────

/// Build, with the authoritative `wasm-encoder`, the component we expect for a set of nullary
/// `() -> s64` exports over a given core module — the independent reference the hand-emitted bytes are
/// diffed against. Section order matches our envelope (component-type before core-alias).
fn oracle_component(core: &[u8], names: &[&str]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = Component::new();
    c.section(&RawSection {
        id: ComponentSectionId::CoreModule as u8,
        data: core,
    });
    let mut inst = InstanceSection::new();
    inst.instantiate(0, std::iter::empty::<(&str, ModuleArg)>());
    c.section(&inst);
    let mut ts = ComponentTypeSection::new();
    for _ in names {
        ts.function()
            .params(std::iter::empty::<(&str, ComponentValType)>())
            .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    }
    c.section(&ts);
    let mut al = ComponentAliasSection::new();
    for nm in names {
        al.alias(Alias::CoreInstanceExport {
            instance: 0,
            kind: ExportKind::Func,
            name: nm,
        });
    }
    c.section(&al);
    let mut canon = CanonicalFunctionSection::new();
    for i in 0..names.len() {
        canon.lift(i as u32, i as u32, []);
    }
    c.section(&canon);
    let mut ex = ComponentExportSection::new();
    for (i, nm) in names.iter().enumerate() {
        ex.export(nm, ComponentExportKind::Func, i as u32, None);
    }
    c.section(&ex);
    c.finish()
}

/// Build a core module of `names`, each a nullary `() -> i64` returning a distinct constant, with the
/// SAME `wasm-encoder`, so the oracle test isolates the component ENVELOPE from the core.
fn oracle_core(names: &[&str]) -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![], vec![ValType::I64]);
    m.section(&types);
    let mut funcs = FunctionSection::new();
    for _ in names {
        funcs.function(0);
    }
    m.section(&funcs);
    let mut exports = ExportSection::new();
    for (i, nm) in names.iter().enumerate() {
        exports.export(nm, ExportKind::Func, i as u32);
    }
    m.section(&exports);
    let mut code = CodeSection::new();
    for (i, _) in names.iter().enumerate() {
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::I64Const(42 + i as i64));
        f.instruction(&Instruction::End);
        code.function(&f);
    }
    m.section(&code);
    m.finish()
}

/// The hand-emitted envelope is byte-identical to the `wasm-encoder` oracle for a set of nullary-s64
/// exports over one core, at N=1 and N=2 — what licenses hand-encoding the envelope (no external
/// encoder in the byte path).
#[test]
fn envelope_matches_wasm_encoder_oracle() {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};
    for names in [&["run"][..], &["run", "double"][..]] {
        let core = oracle_core(names);
        let exports: Vec<BoundaryExport> = names
            .iter()
            .map(|n| BoundaryExport {
                name: n.to_string(),
                params: Vec::new(),
                result: BoundaryResult::Primitive(0x78),
            })
            .collect();
        let ours = assemble(&core, &exports, &[], "");
        assert_eq!(
            ours,
            oracle_component(&core, names),
            "envelope mismatch for {names:?}"
        );
    }
}

/// The hand-emitted envelope for a PARAMETERIZED export `(p0: s64, p1: s64) -> s64` is byte-identical
/// to the `wasm-encoder` oracle — licenses the named-parameter component functype the exported
/// runtime function needs.
#[test]
fn parameterized_envelope_matches_wasm_encoder_oracle() {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};
    // A core module with one `(i64, i64) -> i64` function returning param 0 + param 1.
    let core = {
        use wasm_encoder::*;
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types
            .ty()
            .function(vec![ValType::I64, ValType::I64], vec![ValType::I64]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut exports = ExportSection::new();
        exports.export("add", ExportKind::Func, 0);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::LocalGet(0));
        f.instruction(&Instruction::LocalGet(1));
        f.instruction(&Instruction::I64Add);
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        m.finish()
    };
    // The oracle component: one export `add : (p0: s64, p1: s64) -> s64`.
    let oracle = {
        use wasm_encoder::*;
        let mut c = Component::new();
        c.section(&RawSection {
            id: ComponentSectionId::CoreModule as u8,
            data: &core,
        });
        let mut inst = InstanceSection::new();
        inst.instantiate(0, std::iter::empty::<(&str, ModuleArg)>());
        c.section(&inst);
        let mut ts = ComponentTypeSection::new();
        ts.function()
            .params([
                ("p0", ComponentValType::Primitive(PrimitiveValType::S64)),
                ("p1", ComponentValType::Primitive(PrimitiveValType::S64)),
            ])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        c.section(&ts);
        let mut al = ComponentAliasSection::new();
        al.alias(Alias::CoreInstanceExport {
            instance: 0,
            kind: ExportKind::Func,
            name: "add",
        });
        c.section(&al);
        let mut canon = CanonicalFunctionSection::new();
        canon.lift(0, 0, []);
        c.section(&canon);
        let mut ex = ComponentExportSection::new();
        ex.export("add", ComponentExportKind::Func, 0, None);
        c.section(&ex);
        c.finish()
    };
    let ours = assemble(
        &core,
        &[BoundaryExport {
            name: "add".to_string(),
            params: vec![0x78, 0x78], // two s64 params
            result: BoundaryResult::Primitive(0x78),
        }],
        &[],
        "",
    );
    assert_eq!(ours, oracle, "parameterized envelope mismatch");
}

/// The NARROW component primitive valtype bytes (`u8`/`s8`/`u16`/`s16`) our `comp_valtype_of` emits are
/// byte-identical to the `wasm-encoder` oracle — the byte-identity discipline for the R2 faithful
/// boundary mapping (a wrong byte would misreport a value's type at the component edge). One export
/// `(p0: u8, p1: s16) -> s8` over a `(i32, i32) -> i32` core covers three of the four narrow primitives.
#[test]
fn narrow_primitive_envelope_matches_wasm_encoder_oracle() {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};
    // A minimal `(i32, i32) -> i32` core — the machine signature a narrow-width export lowers to (a
    // ≤32-bit value occupies an i32 slot; the ABI lifts it to the narrow primitive at the edge).
    let core = {
        use wasm_encoder::*;
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut exports = ExportSection::new();
        exports.export("f", ExportKind::Func, 0);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut fbody = Function::new(vec![]);
        fbody.instruction(&Instruction::LocalGet(0));
        fbody.instruction(&Instruction::End);
        code.function(&fbody);
        m.section(&code);
        m.finish()
    };
    // Oracle: `f : (p0: u8, p1: s16) -> s8` — the narrow primitives our comp_valtype_of maps to.
    let oracle = {
        use wasm_encoder::*;
        let mut c = Component::new();
        c.section(&RawSection {
            id: ComponentSectionId::CoreModule as u8,
            data: &core,
        });
        let mut inst = InstanceSection::new();
        inst.instantiate(0, std::iter::empty::<(&str, ModuleArg)>());
        c.section(&inst);
        let mut ts = ComponentTypeSection::new();
        ts.function()
            .params([
                ("p0", ComponentValType::Primitive(PrimitiveValType::U8)),
                ("p1", ComponentValType::Primitive(PrimitiveValType::S16)),
            ])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::S8)));
        c.section(&ts);
        let mut al = ComponentAliasSection::new();
        al.alias(Alias::CoreInstanceExport {
            instance: 0,
            kind: ExportKind::Func,
            name: "f",
        });
        c.section(&al);
        let mut canon = CanonicalFunctionSection::new();
        canon.lift(0, 0, []);
        c.section(&canon);
        let mut ex = ComponentExportSection::new();
        ex.export("f", ComponentExportKind::Func, 0, None);
        c.section(&ex);
        c.finish()
    };
    let ours = assemble(
        &core,
        &[BoundaryExport {
            name: "f".to_string(),
            params: vec![0x7D, 0x7C],                // u8, s16
            result: BoundaryResult::Primitive(0x7E), // s8
        }],
        &[],
        "",
    );
    assert_eq!(ours, oracle, "narrow-primitive envelope mismatch");
}

/// Value-heap H1a: a core module that IMPORTS a runtime op (`heap.arr-alloc`) is byte-identical to the
/// `wasm-encoder` oracle — the per-program import section + the function-index SHIFT (the imported func
/// is index 0, so the program's own exported func is index 1). One import, one nullary `() -> i64`
/// export. This is the byte anchor for the import machinery before any `Core` compound op drives it.
#[test]
fn core_module_with_a_runtime_import_matches_wasm_encoder_oracle() {
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::core_module;
    use crate::layout::{ExportPlan, Layout};
    use crate::ty::Ty;

    // One exported nullary `() -> i64` function returning `42`. With one import, it is DEFINED func
    // index 1 (import `arr-alloc` is index 0) — `import_base = 1` in the layout makes `abs` report 1.
    let func = SelectedFunc {
        params: vec![],
        ret: Ty::int64(),
        code: vec![Lir::ConstI64(42)],
        declared: vec![],
        src_body: None,
        locals: vec![],
        scopes: vec![],
        stmt_lines: vec![],
    };
    let layout = Layout::new(
        vec![ExportPlan {
            name: "main".to_string(),
            def: 0,
            body: crate::ast::StructId(0),
            params: vec![],
            result: Ty::int64(),
        }],
        vec![0],
        1,
    );
    let ours = core_module(&[func], &[OPS.arr_alloc], &layout).expect("core module");

    // The oracle: a core module importing `heap."arr-alloc" : (i32) -> i32`, then one defined
    // `() -> i64` func (function index 1) exported as `main`, returning 42.
    let oracle = {
        use wasm_encoder::*;
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I32], vec![ValType::I32]); // type 0: arr-alloc
        types.ty().function(vec![], vec![ValType::I64]); // type 1: main
        m.section(&types);
        let mut imports = ImportSection::new();
        imports.import("heap", "arr-alloc", EntityType::Function(0));
        m.section(&imports);
        let mut funcs = FunctionSection::new();
        funcs.function(1); // defined func (index 1) uses type 1
        m.section(&funcs);
        let mut exports = ExportSection::new();
        exports.export("main", ExportKind::Func, 1); // export the DEFINED func at index 1
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::I64Const(42));
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        m.finish()
    };
    assert_eq!(ours, oracle, "core module with a runtime import mismatch");
}

/// The program core module for the import-envelope oracle tests: imports `heap."arr-alloc" :
/// (i32)->i32` (core func 0) and defines one nullary `() -> i64` `main` (core func 1) returning 42 —
/// the same shape H1a's core oracle emits. Shared by our envelope and the `ComponentBuilder` oracle so
/// the diff isolates the ENVELOPE.
fn import_oracle_core() -> Vec<u8> {
    use wasm_encoder::*;
    let mut m = Module::new();
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![ValType::I32]); // type 0: arr-alloc
    types.ty().function(vec![], vec![ValType::I64]); // type 1: main
    m.section(&types);
    let mut imports = ImportSection::new();
    imports.import("heap", "arr-alloc", EntityType::Function(0));
    m.section(&imports);
    let mut funcs = FunctionSection::new();
    funcs.function(1);
    m.section(&funcs);
    let mut exports = ExportSection::new();
    exports.export("main", ExportKind::Func, 1);
    m.section(&exports);
    let mut code = CodeSection::new();
    let mut f = Function::new(vec![]);
    f.instruction(&Instruction::I64Const(42));
    f.instruction(&Instruction::End);
    code.function(&f);
    m.section(&code);
    m.finish()
}

/// The 1-op import-envelope built by `wasm-encoder`'s `ComponentBuilder` — the authoritative encoder,
/// which tracks the alias/lower/instantiate index bookkeeping. The reference our hand-emitted import
/// path is diffed against (`import arr-alloc:(u32)->u32`, one nullary `main:()->s64` export). This is
/// exactly the 7-step shape the old `wit_envelope.rs::build_heap_envelope` produced.
fn import_oracle_component(core: &[u8], import_name: &str) -> Vec<u8> {
    use wasm_encoder::*;
    let mut c = ComponentBuilder::default();
    // (1) import instance-type: one func-type + export per used op (`arr-alloc:(u32)->u32`).
    let mut it = InstanceType::new();
    {
        let mut ft = it.ty().function();
        ft.params([("p0", ComponentValType::Primitive(PrimitiveValType::U32))]);
        ft.result(Some(ComponentValType::Primitive(PrimitiveValType::U32)));
    }
    it.export("arr-alloc", ComponentTypeRef::Func(0));
    let it_ty = c.type_instance(&it); // component type 0
    // (2) import the runtime interface as an instance of that type.
    let inst = c.import(import_name, ComponentTypeRef::Instance(it_ty)); // instance 0
    // (3) alias-export each op out of the imported instance + canon-lower → core funcs 0..k.
    let comp_fn = c.alias_export(inst, "arr-alloc", ComponentExportKind::Func);
    c.lower_func(comp_fn, []); // core func 0
    // (4) embed the program core module, (5) build a core-instance of the lowered funcs named "heap",
    // then instantiate the program threading it in.
    let module_idx = c.core_module_raw(core); // core module 0
    let heap_inst = c.core_instantiate_exports([("arr-alloc", ExportKind::Func, 0u32)]); // core inst 0
    let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
    // (6) alias `main` out of the instantiated program, (7) lift it as `() -> s64` and export it.
    let run_core = c.core_alias_export(prog_inst, "main", ExportKind::Func);
    let (run_ty, mut enc) = c.type_function();
    enc.params::<[(&str, ComponentValType); 0], _>([])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
    let run_comp = c.lift_func(run_core, run_ty, []);
    c.export("main", ComponentExportKind::Func, run_comp, None);
    c.finish()
}

/// Value-heap H1b: the hand-emitted IMPORT envelope (a program importing one runtime op) is
/// byte-identical to the `ComponentBuilder` oracle — the byte anchor that licenses hand-emitting the
/// full component-model import plumbing (import-instance-type → import → alias → lower → core module →
/// two core-instances → boundary alias/lift/export) with no external encoder in the compile path. Uses
/// the SAME versioned import name and the GENERATED `OPS.arr_alloc` signature on our side.
#[test]
fn import_envelope_matches_component_builder_oracle() {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};
    use crate::backend::wasm::runtime_abi::OPS;
    let core = import_oracle_core();
    let import_name = "cadenza:runtime/heap@0.0.0+deadbeef";
    let exports = vec![BoundaryExport {
        name: "main".to_string(),
        params: Vec::new(),
        result: BoundaryResult::Primitive(0x78), // s64
    }];
    let ours = assemble(&core, &exports, &[OPS.arr_alloc], import_name);
    let oracle = import_oracle_component(&core, import_name);
    assert_eq!(ours, oracle, "import envelope mismatch vs ComponentBuilder");
}

/// Value-heap H1b: the hand-emitted import envelope PARSES and TYPE-CHECKS under wasmtime
/// (`Component::new`) — structural + component-type validity, the semantic floor that catches every
/// index/layout error the byte diff might not localize. It also carries the versioned, hashed import
/// name `cdz-run` composes the runtime by. (Instantiation needs the runtime linked, so this checks
/// validity/type only — the end-to-end run lands in H2 with a real compound op + the composed runtime.)
#[test]
fn import_envelope_is_a_valid_component_with_the_versioned_import() {
    use crate::backend::wasm::envelope::{BoundaryExport, BoundaryResult, assemble};
    use crate::backend::wasm::runtime_abi::{OPS, REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
    let core = import_oracle_core();
    let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
    let exports = vec![BoundaryExport {
        name: "main".to_string(),
        params: Vec::new(),
        result: BoundaryResult::Primitive(0x78),
    }];
    let bytes = assemble(&core, &exports, &[OPS.arr_alloc], &import_name);

    // Structural + type validity: a bad section order, wrong index, or malformed item fails here.
    let engine = wasmtime::Engine::default();
    let component = wasmtime::component::Component::from_binary(&engine, &bytes);
    assert!(
        component.is_ok(),
        "import envelope failed to load under wasmtime: {:?}",
        component.err()
    );

    // The emitted bytes carry the exact versioned import name (interface@semver+hash) verbatim — the
    // name the linker matches the composed runtime against. Assert its literal bytes are present.
    let needle = import_name.as_bytes();
    assert!(
        bytes.windows(needle.len()).any(|w| w == needle),
        "versioned import name `{import_name}` not found in the emitted component"
    );
}

// ── the behavior run (wasmtime) ────────────────────────────────────────────────────────────────

/// A component-boundary result type a behavior test can read back — one method decoding a wasmtime
/// `Val` into the Rust value the test asserts on. Adding a new boundary return type (a `u32`, a
/// string, later a compound) is one more `impl FromVal`, no new run helper.
trait FromVal: Sized {
    /// Decode a boundary result value, panicking with a type-named message on a mismatch.
    fn from_val(v: &wasmtime::component::Val) -> Self;
}

impl FromVal for i64 {
    fn from_val(v: &wasmtime::component::Val) -> i64 {
        match v {
            wasmtime::component::Val::S64(n) => *n,
            other => panic!("expected S64 result, got {other:?}"),
        }
    }
}

impl FromVal for bool {
    fn from_val(v: &wasmtime::component::Val) -> bool {
        match v {
            wasmtime::component::Val::Bool(b) => *b,
            other => panic!("expected Bool result, got {other:?}"),
        }
    }
}

impl FromVal for u64 {
    fn from_val(v: &wasmtime::component::Val) -> u64 {
        match v {
            wasmtime::component::Val::U64(n) => *n,
            other => panic!("expected U64 result, got {other:?}"),
        }
    }
}

impl FromVal for u32 {
    fn from_val(v: &wasmtime::component::Val) -> u32 {
        match v {
            wasmtime::component::Val::U32(n) => *n,
            other => panic!("expected U32 result, got {other:?}"),
        }
    }
}

impl FromVal for i32 {
    fn from_val(v: &wasmtime::component::Val) -> i32 {
        match v {
            wasmtime::component::Val::S32(n) => *n,
            other => panic!("expected S32 result, got {other:?}"),
        }
    }
}

// The narrow component primitives — an aliased ≤16-bit width crosses as its faithful `s8`/`u8`/`s16`/
// `u16` (not the machine-slot `s32`/`u32`), so a behavior test reads it back as the matching Rust type.
impl FromVal for i8 {
    fn from_val(v: &wasmtime::component::Val) -> i8 {
        match v {
            wasmtime::component::Val::S8(n) => *n,
            other => panic!("expected S8 result, got {other:?}"),
        }
    }
}

impl FromVal for u8 {
    fn from_val(v: &wasmtime::component::Val) -> u8 {
        match v {
            wasmtime::component::Val::U8(n) => *n,
            other => panic!("expected U8 result, got {other:?}"),
        }
    }
}

impl FromVal for i16 {
    fn from_val(v: &wasmtime::component::Val) -> i16 {
        match v {
            wasmtime::component::Val::S16(n) => *n,
            other => panic!("expected S16 result, got {other:?}"),
        }
    }
}

impl FromVal for u16 {
    fn from_val(v: &wasmtime::component::Val) -> u16 {
        match v {
            wasmtime::component::Val::U16(n) => *n,
            other => panic!("expected U16 result, got {other:?}"),
        }
    }
}

// A Float64 crosses the boundary as the component `f64` primitive — read back as an `f64`. Compared by
// BITS in the test (so `-0.0` ≠ `0.0` and a NaN is exact), the canonical-value contract.
impl FromVal for f64 {
    fn from_val(v: &wasmtime::component::Val) -> f64 {
        match v {
            wasmtime::component::Val::Float64(n) => *n,
            other => panic!("expected Float64 result, got {other:?}"),
        }
    }
}

// A Float32 crosses as the component `f32` primitive — read back as an `f32` (bits-compared in tests).
impl FromVal for f32 {
    fn from_val(v: &wasmtime::component::Val) -> f32 {
        match v {
            wasmtime::component::Val::Float32(n) => *n,
            other => panic!("expected Float32 result, got {other:?}"),
        }
    }
}

/// Instantiate `component_bytes` under wasmtime, call its nullary export `name`, and return the single
/// result decoded to `T` — the "run the artifact" behavior check, generic over the boundary type.
fn run_returns<T: FromVal>(component_bytes: &[u8], name: &str) -> T {
    run_returns_with(component_bytes, name, &[])
}

/// Instantiate `component_bytes` and call export `name` WITH the given argument values, decoding the
/// single result to `T` — the behavior check for a parameterized exported function.
fn run_returns_with<T: FromVal>(
    component_bytes: &[u8],
    name: &str,
    args: &[wasmtime::component::Val],
) -> T {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    let component = Component::from_binary(&engine, component_bytes).expect("valid component");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker
        .instantiate(&mut store, &component)
        .expect("instantiate");
    let func = instance.get_func(&mut store, name).expect("export present");
    // A one-slot result buffer; the initial value is overwritten by the call (its variant is
    // irrelevant — `call` writes the actual result), then decoded to `T`.
    let mut results = [Val::Bool(false)];
    func.call(&mut store, args, &mut results).expect("call");
    func.post_return(&mut store).expect("post_return");
    T::from_val(&results[0])
}

/// Whether calling export `name` with `args` TRAPS (an `unreachable` from an overflow guard). Returns
/// `true` if the call errored (a wasm trap), `false` if it returned normally.
fn call_traps(component_bytes: &[u8], name: &str, args: &[wasmtime::component::Val]) -> bool {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    let component = Component::from_binary(&engine, component_bytes).expect("valid component");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker
        .instantiate(&mut store, &component)
        .expect("instantiate");
    let func = instance.get_func(&mut store, name).expect("export present");
    let mut results = [Val::S64(0)];
    func.call(&mut store, args, &mut results).is_err()
}

// ── value-heap H2b: a program that IMPORTS the runtime, run COMPOSED with it ─────────────────────
//
// A tuple built and projected at run time lowers to the heap ops (`arr-alloc`/`box-*`/`arr-set`/
// `arr-get`/`get-*`), so the emitted component IMPORTS `cadenza:runtime/heap`. Running it needs the
// runtime composed in — `cdz-run` does exactly that (the canonical runner), so the behavior test
// drives the composed round-trip through it rather than re-implementing composition.

/// Locate the built value-heap runtime component `.wasm` whose content hash matches the compiler's
/// `REQUIRED_RUNTIME_HASH`, or `None` if it is not present (so the test SKIPS rather than fails when the
/// runtime has not been built — `cargo xtask build`/`codegen` produces it). Searches the content-
/// addressed store and the `cargo component` build output, relative to the repo root.
fn find_runtime_wasm() -> Option<Vec<u8>> {
    use crate::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH;
    // `CARGO_MANIFEST_DIR` is `.../implementation/seed/crates/rcdzc`; the seed workspace root is three
    // levels up, and the repo root one more.
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let seed = manifest.join("../..").canonicalize().ok()?; // .../implementation/seed
    let repo = seed.join("../..").canonicalize().ok()?; // repo root
    let candidates = [
        // The content-addressed store (populated by `xtask build`).
        repo.join(format!("target/cadenza-store/{REQUIRED_RUNTIME_HASH}.wasm")),
        // The `cargo component build` output (populated by `xtask codegen`/`build`).
        seed.join("crates/cdz-runtime/target/wasm32-unknown-unknown/release/cdz_runtime.wasm"),
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            // Only accept a runtime whose content hash matches what the compiler compiled against —
            // a stale runtime would compose wrong ops (the /loop gotcha). Verify before use.
            let hash = sha256_hex(&bytes);
            if hash == REQUIRED_RUNTIME_HASH {
                return Some(bytes);
            }
        }
    }
    None
}

/// The SHA-256 of `bytes` as lowercase hex — the same content-address the store keys on and
/// `REQUIRED_RUNTIME_HASH` records. (Uses `cdz-run`'s dep chain transitively; computed here to compare
/// a found runtime against the pinned hash.)
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A `let`-bound tuple built from runtime params but ONLY PROJECTED (never used as a whole value)
/// FOLDS AWAY: each `(. t i)` reduces straight to its element (the param), so the body is just
/// `(+ a b)` — no `arr-alloc`, no heap round-trip, no runtime import at all. Building the tuple only to
/// read it back is pure waste when it never has to exist as a value. (The GENUINE heap-alloc → escape →
/// walk round-trip is exercised where a compound is RETURNED whole and must be built — see
/// `a_recursive_runtime_tuple_escapes_to_the_host`, which passes a runtime value in via the harness
/// `call` and escapes the built tuple as a resource.)
#[test]
fn a_projection_only_runtime_tuple_folds_without_the_heap() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // Runtime params, but `t` is only projected → folds; the program touches the value heap not at all.
    let src = "(module m (def (pair-sum (: a Int64) (: b Int64)) \
                 (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (export pair-sum))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_none(),
        "a projection-only tuple must fold, importing no runtime op (no per-call arr-alloc)"
    );
    // It runs as a plain scalar function (no runtime composed in) — 20 + 22 = 42.
    let got: i64 = run_returns_with(&bytes, "pair-sum", &[Val::S64(20), Val::S64(22)]);
    assert_eq!(got, 42, "folds to (+ a b)");
}

/// A SINGLE projection of a tuple built through an `if` FOLDS the tuple away by pushing the projection
/// INTO the branches: `(. (if c (tuple a b) (tuple b a)) 0)` → `(if c a b)`. The `if`-selected tuple was
/// previously OPAQUE to the projection fold (`reduce_to_tuple_elems` stops at an `if`), forcing a
/// per-call `arr-alloc` + `arr-get`; now the projection reduces to one `if` over the two selected element
/// occurrences — no heap, no runtime import. The transform reuses the EXISTING element occurrences
/// (each keeps the scope it was resolved in — no ast synthesis), so `.0` selects `a`/`b` by the
/// condition and `.1` selects `b`/`a`. A trap in the condition is preserved (it still gates each branch),
/// and the un-projected sibling drops exactly as it does for a plain visible-tuple projection. Contrast
/// `a_narrow_runtime_tuple_element_crosses_the_heap_boundary`, where the tuple is read TWICE (a kept
/// multi-use binding) so the fold correctly DECLINES and the heap round-trip stands.
#[test]
fn a_projection_of_an_if_selected_tuple_pushes_into_the_branches() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // `.0` of `(if p (tuple a b) (tuple b a))` → `(if p a b)`: p true selects a, p false selects b.
    let src = "(module m (def (pick (: p Bool) (: a Int64) (: b Int64)) \
                 (. (if p (tuple a b) (tuple b a)) 0)) (export pick))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_none(),
        "a single projection of an if-of-tuples must fold (no per-call arr-alloc, no runtime import)"
    );
    let t: i64 = run_returns_with(
        &bytes,
        "pick",
        &[Val::Bool(true), Val::S64(10), Val::S64(20)],
    );
    assert_eq!(t, 10, "p true → .0 = a");
    let f: i64 = run_returns_with(
        &bytes,
        "pick",
        &[Val::Bool(false), Val::S64(10), Val::S64(20)],
    );
    assert_eq!(f, 20, "p false → .0 = b (the else branch's element 0)");
}

/// The RECORD analogue of the tuple `PROJECTION-INTO-IF` fold: a SINGLE field read of a record built
/// through an `if` pushes the member read INTO the branches — `(. (if c (record (x a)(y b))
/// (record (x b)(y a))) x)` → `(if c a b)`. The `if`-selected record was OPAQUE to `member_value`
/// (`reduce_to_record_id` stops at an `if`), forcing a per-call heap build of BOTH branch records
/// (`arr-alloc` + per-field box/set) then an `arr-get`/unbox to read one field back; now the field is
/// projected from each branch (by NAME — order-independent) and the two records fold away, leaving one
/// `if` over the selected field values — no heap, no runtime import. Reuses the EXISTING field-value
/// occurrences (no ast synthesis). Fields are written out of sorted order to confirm the fold is by key,
/// not slot. Contrast `a_field_of_a_runtime_record_reads_the_heap_at_its_sorted_index`, where the record
/// is built behind a RECURSIVE call so it stays opaque and the heap read stands.
#[test]
fn a_member_read_of_an_if_selected_record_pushes_into_the_branches() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // `.x` of `(if p (record (y b)(x a)) (record (y a)(x b)))` → `(if p a b)`.
    let src = "(module m (def (pick (: p Bool) (: a Int64) (: b Int64)) \
                 (. (if p (record (y b) (x a)) (record (y a) (x b))) x)) (export pick))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_none(),
        "a single member read of an if-of-records must fold (no per-call arr-alloc, no runtime import)"
    );
    let t: i64 = run_returns_with(
        &bytes,
        "pick",
        &[Val::Bool(true), Val::S64(10), Val::S64(20)],
    );
    assert_eq!(t, 10, "p true → .x = a");
    let f: i64 = run_returns_with(
        &bytes,
        "pick",
        &[Val::Bool(false), Val::S64(10), Val::S64(20)],
    );
    assert_eq!(f, 20, "p false → .x = b (the else branch's x field)");
}

/// A NARROW-width element crosses the heap boundary with an explicit slot conversion. The heap stores
/// an integer as one i64 cell, but a narrow width (UInt8) lives in an i32 slot — so a narrow element is
/// extended i32→i64 into the heap and narrowed i64→i32 out of it. To exercise a GENUINE heap round-trip
/// the tuple must be OPAQUE to the fold: a projection-only tuple LITERAL folds away (no heap), and a
/// call-returned one now folds through too (the callee β-reduces — cycle 16). So the tuple comes from
/// an `if` whose two branches DIFFER (`(tuple a b)` vs `(tuple b a)`) — `reduce_to_tuple_elems` does
/// not see through an `if`, and the differing branches block the identical-branch collapse — so `t`
/// stays a runtime handle and both projections are `arr-get`s. `(+ (. t 0) (. t 1))` must build,
/// validate, and compute 100+50 = 150 through the heap (the sum is order-independent, so either branch
/// gives 150).
#[test]
fn a_narrow_runtime_tuple_element_crosses_the_heap_boundary() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (pair-sum (: a UInt8) (: b UInt8)) \
                   (let ((t (if (< a b) (tuple a b) (tuple b a)))) (+ (. t 0) (. t 1)))) \
                 (export pair-sum))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "an if-selected narrow tuple, projected, must build on the heap (import the runtime)"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: Some("pair-sum".to_string()),
        args: vec!["100".to_string(), "50".to_string()],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => assert_eq!(s, "150", "narrow heap round-trip"),
        cdz_run::Outcome::Trap(t) => panic!("narrow heap run trapped (miscompile?): {t}"),
    }
}

/// A FIELD read on a RUNTIME record (one whose value is not a compile-time-visible literal) projects
/// like a tuple element: a record at run time IS a positional heap array in SORTED-KEY order, so
/// `(. rec field)` is an `arr-get` at the field's sorted index. The record is written OUT of sorted
/// order — `(record (z 9) (a n))` — so `a` is heap slot 0 and `z` is slot 1; projecting `a` must read
/// the runtime value (slot 0), not the literal 9. Earlier the member-access path folded only through a
/// compile-time record and REJECTED a call-produced one with a self-contradictory CDZ0201 "requires a
/// record, found (record …)"; this pins the runtime field read AND that it indexes by sorted position.
/// To keep the record OPAQUE to the fold, it is built behind a RECURSIVE call: `is_recursive` declines
/// the compile-time β-reduction, so `reduce_to_record_id` (and thus the member fold, incl. the cycle-33
/// member-into-if fold) cannot see through `mk` — `main` reads `a` off the heap at run time. (Earlier
/// this test used an `if` with branches differing in `a`, but cycle 33's member-into-if fold now sees
/// through such an `if`, so an `if` no longer keeps a record opaque — a recursive call does.)
#[test]
fn a_field_of_a_runtime_record_reads_the_heap_at_its_sorted_index() {
    use crate::testkit::parse;
    // The record's `a` field is the parameter; fields written z-before-a so the sorted layout (a=0, z=1)
    // differs from the write order. `mk` recurses to a base case that builds the record, so the compiler
    // cannot fold through it — `main` selects the runtime handle and reads `a` off the heap. For v = 41
    // the recursion bottoms out with `a` = 41, and projecting `a` reads slot 0.
    let src = "(module m \
                 (def (mk (: n Int64)) (if (< n 0) (record (z 9) (a 0)) \
                   (if (= n 41) (record (z 9) (a n)) (mk (- n 1))))) \
                 (def (main (: v Int64)) (. (mk v) a)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "a runtime-record field read must import the value-heap runtime (genuine heap, not a fold)"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: Some("main".to_string()),
        args: vec!["41".to_string()],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => {
            assert_eq!(
                s, "41",
                "runtime-record field `a` reads sorted slot 0 = the argument"
            )
        }
        cdz_run::Outcome::Trap(t) => panic!("runtime-record field read trapped (miscompile?): {t}"),
    }
}

/// A bare (unannotated) parameter PROJECTED in a helper's body is UNCONSTRAINED there (typed `Any`
/// until the def inlines) — so the projection is checked at the CALL SITE, where the argument's
/// compound type flows in, not standalone. `(def (get-x r) (. r x))` is well-formed even though `r`'s
/// type is unknown in the body, exactly as `(+ r 1)` on an `Any` parameter is; applied to a record
/// value, `r` is that record and `(. r x)` is its `x`. Earlier the seed rejected the body standalone
/// with a self-contradictory CDZ0201 "requires a record, found Any" (an `Any` operand is unconstrained,
/// not a proven non-record) — projection was the outlier vs arithmetic, which never faulted on `Any`.
/// The argument is a record built behind a RECURSIVE call so it stays OPAQUE to the fold (a plain
/// `(mk v)` would fold through — cycle 16; and an `if`-selected record now folds through too — cycle
/// 33's member-into-if): `is_recursive` declines the β-reduction, so `get-x` reads `x` off the heap at
/// run time. For v = 41 the recursion bottoms out with `x` = v; composed run: 41.
#[test]
fn a_projected_bare_parameter_is_constrained_at_the_call_site() {
    use crate::testkit::parse;
    let src = "(module m (def (get-x r) (. r x)) \
                 (def (mk n) (if (= n 41) (record (x n) (y 2)) (mk (+ n 1)))) \
                 (def (main (: v Int64)) (get-x (mk v))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "projecting a record parameter built at run time must import the value-heap runtime"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: Some("main".to_string()),
        args: vec!["41".to_string()],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => {
            assert_eq!(s, "41", "get-x (mk 41) projects field x = the argument")
        }
        cdz_run::Outcome::Trap(t) => {
            panic!("projected-parameter helper trapped (miscompile?): {t}")
        }
    }
}

/// The deferral above is NOT a drop: applying a projecting helper to a NON-compound argument is still
/// rejected — at the call site, where the concrete type is known. `(get-x v)` with `v : Int64` reduces
/// the body to a field read of an integer, which faults CDZ0201. Pins that a bad structural use is
/// caught (just deferred from the polymorphic body to the concrete call), so accepting the well-typed
/// helper above did not weaken the check.
#[test]
fn projecting_a_non_compound_argument_is_still_rejected() {
    use crate::testkit::parse;
    let src = "(module m (def (get-x r) (. r x)) \
                 (def (main (: v Int64)) (get-x v)) (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("projecting a field of an Int64 argument must be rejected");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0201"),
        "expected a CDZ0201 member-access rejection, got: {}",
        err.message
    );
}

/// R2 e2e: a RUNTIME compound built behind a RECURSIVE call ESCAPES to the host as a resource, and its
/// `encode()` walks the live handle to the canonical value form. `f` is genuinely recursive (calls
/// itself), so `is_recursive` DECLINES the compile-time fold — `f` becomes a real `Core::Call`, `(f 3)`
/// runs it at run time, and the returned `(tuple n 7)` is built on the value heap (NOT constant-folded).
/// It must IMPORT the runtime (the genuine-heap signal, distinct from a folded constant tuple which
/// imports nothing) and, composed, decode to the exact value form. This is the first COMPILER-EMITTED
/// runtime-compound host escape (R2) — the walker/envelope proven independently in `r2_runtime_resource`,
/// here driven by the real compile pipeline.
#[test]
fn a_recursive_runtime_tuple_escapes_to_the_host() {
    use crate::testkit::parse;
    // `f` recurses to its base case, which builds `(tuple n 7)` from the runtime-threaded `n`. Recursive
    // → not folded → the tuple is a genuine heap value that escapes as a resource.
    let src = "(module m (def (f n) (if (= n 0) (tuple n 7) (f (- n 1)))) \
                 (def (main) (f 3)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");

    // The genuine-heap signal: it imports the value-heap runtime (a folded constant tuple would import
    // nothing — this is what distinguishes R2 from the R1 constant escape).
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "a recursive runtime-tuple escape must import the value-heap runtime (genuine heap, not a fold)"
    );

    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("[R2] runtime wasm not found; skipping composed escape run");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: None, // a resource-escape program exports no bare func — the host takes the escape path
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => {
            assert_eq!(
                s, "(: (tuple 0 7) (Tuple Int64 Int64))",
                "R2 escape value form"
            )
        }
        cdz_run::Outcome::Trap(t) => panic!("R2 escape run trapped: {t}"),
    }
}

/// R2 e2e for a RECORD: a runtime record built behind a recursive call escapes as a resource and its
/// `encode()` walks the heap array (a record IS the same positional array as a tuple — its fields in
/// canonical sorted order — the distinction is only the static type the renderer bakes). Companion of
/// `a_recursive_runtime_tuple_escapes_to_the_host`, on the record surface. Pins that a runtime record
/// crosses with its field NAMES rendered (from the type) though the runtime holds a nameless array.
#[test]
fn a_recursive_runtime_record_escapes_to_the_host() {
    use crate::testkit::parse;
    let src = "(module m (def (f n) (if (= n 0) (record (a n) (b 7)) (f (- n 1)))) \
                 (def (main) (f 3)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "a recursive runtime-record escape must import the value-heap runtime (genuine heap)"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("[R2] runtime wasm not found; skipping composed record escape");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: None,
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => assert_eq!(
            s, "(: (record (a 0) (b 7)) (Record (a Int64) (b Int64)))",
            "R2 record escape value form"
        ),
        cdz_run::Outcome::Trap(t) => panic!("R2 record escape run trapped: {t}"),
    }
}

/// R2 NESTED: a runtime tuple whose element is ITSELF a runtime tuple escapes — the inner compound is
/// built on the heap as its own array, and the outer `arr-set`s the inner HANDLE directly (no box), so
/// the outer array holds a handle to the inner array. `encode()` walks the nested `arr-get` path and
/// renders both levels. Recursive → unfoldable → genuine nested-heap construction. Pins that a nested
/// compound element is stored/read as a handle (not boxed/unboxed) — the `box_op`/`get_op` `None` path.
#[test]
fn a_nested_runtime_tuple_escapes_to_the_host() {
    use crate::testkit::parse;
    let src = "(module m (def (f n) (if (= n 0) (tuple n (tuple n n)) (f (- n 1)))) \
                 (def (main) (f 2)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "a nested runtime-tuple escape must import the value-heap runtime (genuine nested heap)"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("[R2] runtime wasm not found; skipping composed nested escape");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: None,
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => assert_eq!(
            s, "(: (tuple 0 (tuple 0 0)) (Tuple Int64 (Tuple Int64 Int64)))",
            "R2 nested tuple escape value form"
        ),
        cdz_run::Outcome::Trap(t) => panic!("R2 nested tuple escape run trapped: {t}"),
    }
}

/// A NESTED CONSTANT tuple whose inner and outer levels SHARE an element occurrence must fold and
/// escape as the right value — the regression guard for the value-constructor build-once cache. A
/// non-recursive `(f 2)` inlines by β-reduction, which SHARES a constant-atom occurrence
/// (`copy_structural` returns the original id for a literal), so in `(tuple n (tuple n n))` the outer
/// and inner tuples reduce over the SAME `2` occurrences. An earlier build-once cache keyed on an
/// ARGUMENT id collided the inner build onto the outer and recursed without end (a stack overflow);
/// keying on the construction-site occurrence (`origin`) is collision-free. Pins the fold-and-escape
/// of a shared-occurrence nested compound.
#[test]
fn a_nested_constant_tuple_with_shared_element_occurrences_escapes() {
    use crate::testkit::parse;
    // `(f 2)` is NON-recursive → inlines; β-reduction shares the `2` occurrence across both tuple
    // levels — exactly the shape the build-once cache must not collide.
    let src = "(module m (def (f n) (tuple n (tuple n n))) \
                 (def (main) (f 2)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("[R2] runtime wasm not found; skipping shared-occurrence nested escape");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: None,
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => assert_eq!(
            s, "(: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))",
            "a shared-occurrence nested constant tuple must escape as its true value form"
        ),
        cdz_run::Outcome::Trap(t) => panic!("shared-occurrence nested tuple escape trapped: {t}"),
    }
}

/// R2 NESTED (heterogeneous): a runtime RECORD whose field is a runtime TUPLE — the type-directed
/// walker recurses record→tuple, each sub-shape rendered by its own head. Pins the nested handle path
/// across a record boundary.
#[test]
fn a_record_with_a_runtime_tuple_field_escapes_to_the_host() {
    use crate::testkit::parse;
    let src = "(module m (def (f n) (if (= n 0) (record (x n) (y (tuple n 1))) (f (- n 1)))) \
                 (def (main) (f 2)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_some(),
        "a record-with-tuple-field escape must import the value-heap runtime"
    );
    let Some(runtime) = find_runtime_wasm() else {
        eprintln!("[R2] runtime wasm not found; skipping composed nested-record escape");
        return;
    };
    let opts = cdz_run::RunOpts {
        export: None,
        args: vec![],
        runtime: Some(runtime),
        runtime_cache_dir: None,
        host_responses: Vec::new(),
    };
    match cdz_run::run(&bytes, &opts).expect("run") {
        cdz_run::Outcome::Value(s) => assert_eq!(
            s, "(: (record (x 0) (y (tuple 0 1))) (Record (x Int64) (y (Tuple Int64 Int64))))",
            "R2 nested record escape value form"
        ),
        cdz_run::Outcome::Trap(t) => panic!("R2 nested record escape run trapped: {t}"),
    }
}

/// A `let`-bound value INSIDE a function that takes a PARAMETER compiles and runs — the β-reduction
/// atom-copy fix. `(g 10)` inlines g's body `(let ((x (+ n 1))) (+ x x))`; the reduction copies the
/// body, substituting the param `n`→`10` in the binding's init. The BUG was that the body's `x`
/// references (shared name atoms) kept their stale resolution to the ORIGINAL (unsubstituted) init, so
/// a `Core::Param{n}` surfaced at select (no slot) → decline. The fix COPIES name atoms in `beta_reduce`
/// so a `let`-local re-resolves against the copied (substituted) init. Runs to `(+ 11 11)` = 22.
#[test]
fn a_let_bound_value_under_a_fn_param_compiles() {
    use crate::testkit::parse;
    let src =
        "(module m (def (g n) (let ((x (+ n 1))) (+ x x))) (def (main) (g 10)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 22);
}

/// A record field KEY (and a member-access field name) that coincides with a PARAMETER name survives the
/// call — the β-reduction key-immunity fix. `beta_reduce` substitutes the argument for VALUE references to
/// the parameter, but a field key `(record (x 5))` and a projection key `(. r x)` are LABELS, not value
/// reads; substituting the argument for them (turning `(record (x 5))` into `(record (7 5))`) corrupted a
/// valid record and rejected the program CDZ0201. `is_binder_occurrence` (record key) +
/// `is_member_key_occurrence` (projection key) now copy such a name structurally. The `let`-bound analogue
/// always worked (a `let`-local is not β-substituted), which proved the param case was a bug.
#[test]
fn a_record_field_key_coinciding_with_a_parameter_survives_the_call() {
    use crate::testkit::parse;
    // Field keyed `x`, projected `x`, under param `x` → 5 (the key is a label, immune to substitution).
    let proj =
        "(module m (def (f (: x Int64)) (. (record (x 5)) x)) (def (main) (f 7)) (export main))";
    assert_eq!(
        run_returns::<i64>(
            &compile_component(&crate::codec::encode(&parse(proj))).expect("compile"),
            "main"
        ),
        5
    );
    // Multi-field: the colliding key `x` does not corrupt the record; project the non-colliding `y` → 2.
    let multi = "(module m (def (f (: x Int64)) (. (record (x 1) (y 2)) y)) (def (main) (f 7)) (export main))";
    assert_eq!(
        run_returns::<i64>(
            &compile_component(&crate::codec::encode(&parse(multi))).expect("compile"),
            "main"
        ),
        2
    );
    // The COMPLEMENT — a field VALUE that references the param STILL substitutes (immunity is key-only):
    // `(record (y x))` with x=7, projected → 7. Guards against over-immunizing the whole field pair.
    let value =
        "(module m (def (f (: x Int64)) (. (record (y x)) y)) (def (main) (f 7)) (export main))";
    assert_eq!(
        run_returns::<i64>(
            &compile_component(&crate::codec::encode(&parse(value))).expect("compile"),
            "main"
        ),
        7
    );
}

/// A reference to EACH of a wide signature's parameters resolves to the right one — the correctness
/// guard for the O(1) per-scope binder index (`Db::scope_binders`) that replaced `binder_in`'s
/// O(params)-per-reference signature scan. `f` takes six params and its body references three of them
/// out of order; the wrong index (or an off-by-one on the def NAME) would bind a reference to the
/// wrong parameter and compute a wrong result. `(f 1 2 4 8 16 32)` → `p1 + p3 + p5` = 2+8+32 = 42.
#[test]
fn each_parameter_of_a_wide_signature_resolves_to_its_own_binder() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (f (: p0 Int64) (: p1 Int64) (: p2 Int64) (: p3 Int64) (: p4 Int64) (: p5 Int64)) \
                    (+ p1 (+ p3 p5))) \
                 (def (main) (f 1 2 4 8 16 32)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 42);
}

/// A synthesized (β-reduced) scope form's parameters still resolve — the regression guard for the
/// per-scope binder index. The index is built at load over the ORIGINAL arena, but β-reduction copies
/// a lambda/def body into FRESH `fn`/`def` nodes; a parameter reference inside such a copy must find
/// its binder too (the incremental `push_list` re-index). `(Int 8)` builds its module by reducing the
/// `Int` type-lambda `(fn (w) …)`, and `.wrap` reads the built width — this exact path regressed to "a
/// conversion target is not a definite integer type" when the synthesized `fn`'s `w` found no binder.
#[test]
fn a_parameter_inside_a_synthesized_scope_form_resolves() {
    use crate::testkit::parse;
    // `(Int 8).wrap 200` = -56: builds the width-8 module through the copied `Int` type-lambda, then
    // wraps. A miss on the synthesized lambda's width param declines the conversion.
    let src = "(module m (def (main) ((Int 8).wrap 200)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i8>(&bytes, "main"), -56);
}

/// A `wrap` whose SOURCE value already fits the TARGET (same signedness, source width <= target width)
/// is the IDENTITY — the truncation mask/sign-extend is elided — and it must still compute the same
/// value. A `UInt8.wrap` of a runtime `UInt8` is the identity (no mask); a same-sign WIDENING
/// `UInt16.wrap` of a `UInt8` is too. A NARROWING (`UInt8.wrap` of a `UInt64`) still masks, and a SIGN
/// CHANGE (`Int8.wrap` of a `UInt8`) still sign-extends — those genuinely reshape the value. The point
/// is that eliding the redundant truncation does not change any observed result.
#[test]
fn a_wrap_whose_source_fits_the_target_is_the_identity() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // UInt8 → UInt8: identity, value preserved (incl. the top-bit value 200 and the max 255).
    let same = "(module m (def (f (: a UInt8)) (UInt8.wrap a)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(same))).expect("compile");
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U8(200)]), 200);
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U8(255)]), 255);
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U8(0)]), 0);
    // UInt8 → UInt16: same-sign widening is also the identity (a UInt8 already fits UInt16).
    let widen = "(module m (def (f (: a UInt8)) (UInt16.wrap a)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(widen))).expect("compile");
    assert_eq!(run_returns_with::<u16>(&b, "f", &[Val::U8(200)]), 200);
    // UInt64 → UInt8: a NARROWING still masks (300 mod 256 = 44).
    let narrow = "(module m (def (f (: a UInt64)) (UInt8.wrap a)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(narrow))).expect("compile");
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U64(300)]), 44);
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U64(255)]), 255);
    // UInt8 → Int8: a SIGN CHANGE still sign-extends (200 reinterpreted as -56).
    let sign = "(module m (def (f (: a UInt8)) (Int8.wrap a)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(sign))).expect("compile");
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::U8(200)]), -56);
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::U8(100)]), 100);
}

/// `T.wrap(U.wrap x)` where the outer width N ≤ the inner width M — the inner wrap is redundant (the
/// outer keeps only the low N ≤ M bits, unchanged by the inner). The inner Convert is elided; VALUE
/// parity is preserved (the composed wrap gives the same low bits as the single outer wrap).
#[test]
fn a_narrowing_wrap_of_a_wider_wrap_elides_the_inner_wrap() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // Int32 → Int16 → Int8 (8 ≤ 16): folds to Int8.wrap x. 70000 & 0xFF = 112 (both paths agree).
    let src = "(module m (def (f (: x Int32)) ((. Int8 wrap) ((. Int16 wrap) x))) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::S32(70000)]), 112);
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::S32(300)]), 44); // 300 → low 8 bits = 44
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::S32(-1)]), -1);
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::S32(127)]), 127);
    assert_eq!(run_returns_with::<i8>(&b, "f", &[Val::S32(128)]), -128);

    // Lir: exactly ONE sign-extend sequence (a single pair of shl/shr_s) — the inner wrap is gone.
    {
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::select::select_function;
        let full = "(module m (def (f (: x Int32)) ((. Int8 wrap) ((. Int16 wrap) x))) (def (main) 0) (export main))";
        let mut db = crate::db::Db::load(parse(full));
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("f");
        let ps: Vec<_> = db.defs[d]
            .params
            .clone()
            .into_iter()
            .map(|p| {
                let bb = db
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (bb, crate::infer::type_of(&mut db, bb))
            })
            .collect();
        let body = db.defs[d].body.expect("body");
        let code = select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code;
        // Two shifts (shl + shr_s) = ONE wrap; four = two wraps. The fold leaves exactly one wrap.
        let shifts = code
            .iter()
            .filter(|i| matches!(i, Lir::I32Shl | Lir::I32ShrS))
            .count();
        assert_eq!(
            shifts, 2,
            "one sign-extend pair, inner wrap elided; got {code:?}"
        );
    }
}

/// A reference buried under many NON-binding forms still resolves to its outer binder — the
/// correctness guard for the lexical-scope SKIP index (`Db::scope_skip`) that hops over the non-binding
/// spine (record/`if`/application) instead of visiting every enclosing form. A wrong skip pointer (or
/// skipping a form that DOES bind) would mis-resolve `k` or lose it. Here `k` sits under a chain of
/// `if`s and a record, all non-binding, above the `let` that binds it: `(if …)` selects the true branch
/// each time so the innermost `k` (= 5) is what runs. Pins that the skip walk lands on the `let`.
#[test]
fn a_reference_under_non_binding_nesting_resolves_to_its_outer_binder() {
    use crate::testkit::parse;
    // k is bound by the let; the body nests it under three non-binding `if`s and a record projection,
    // none of which bind `k`. `(< k N)` is true for k=5 < 10/20/30, so each `if` takes its `then`
    // (which carries `k` further in), and `(. (record (a k)) a)` projects back to k = 5.
    let src = "(module m (def (main) \
                 (let ((k 5)) \
                   (if (< k 30) (if (< k 20) (if (< k 10) (. (record (a k)) a) 1) 2) 3))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 5);
}

/// Shadowing still resolves nearest-wins THROUGH the skip index: an inner `let` rebinding a name that
/// an outer `let` also binds must reach the INNER one (the skip pointer lands on the nearest binding
/// candidate first). `(let ((x 1)) (let ((x 2)) x))` = 2, not 1 — a skip that jumped past the inner
/// `let` to the outer would return 1.
#[test]
fn shadowing_resolves_nearest_through_the_skip_index() {
    use crate::testkit::parse;
    let src = "(module m (def (main) (let ((x 1)) (let ((x 2)) x))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 2);
}

/// The value-heap companion: a `let`-bound runtime TUPLE inside a parameterized function, projected —
/// `(g 10)` inlines `(let ((t (tuple (+ n 1) (+ n 2)))) (. t 0))`, `t`'s init substitutes `n`, and `(. t
/// 0)` reads element 0 = `n+1` = 11. Exercises the atom-copy fix on a heap-compound let-local (the case
/// that motivated finding the bug — an internal runtime compound under a live param). Folds to the
/// scalar 11, so it needs no runtime.
#[test]
fn a_let_bound_runtime_tuple_under_a_fn_param_projects() {
    use crate::testkit::parse;
    let src = "(module m (def (g n) (let ((t (tuple (+ n 1) (+ n 2)))) (. t 0))) \
                 (def (main) (g 10)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 11);
}

/// The atom-copy fix must NOT substitute a BINDER occurrence that shadows the parameter. `(def (f x)
/// (let ((x true)) x))` binds the inner `x = true` shadowing the Int64 param `x`; `(f 99)` must return
/// `true`. The atom-copy pass resolved the let's BINDER-name occurrence `x` (a `Ref` up to the param,
/// because it shares the name) and SUBSTITUTED the argument into it — turning the binding into `(99
/// true)`, so the body's `x` found no binding and reported a spurious unbound name. A differently-typed
/// shadow additionally MISCOMPILED (the parameter's i64 slot reused for the Bool). A binder occurrence
/// NAMES a binding — β-reduction must copy it, never substitute — so both the same-type and Bool
/// shadows now run. (The companion `a_let_bound_value_under_a_fn_param_compiles` pins that a genuine
/// let-local REFERENCE is still substituted/re-resolved — this pins that its BINDER is not.)
#[test]
fn a_let_binder_shadowing_a_parameter_is_not_substituted() {
    use crate::testkit::parse;
    // Bool shadow: the inner binding's type differs from the parameter — the worst case (was an invalid
    // component). `(f 99)` returns the inner `x = true`.
    let bool_shadow = "(module m (def (f x) (let ((x true)) x)) (def (main) (f 99)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(bool_shadow))).expect("compile");
    assert!(
        run_returns::<bool>(&bytes, "main"),
        "Bool shadow of an Int64 param returns true"
    );

    // Same-type shadow: `(let ((x 7)) x)` over Int64 param `x` — `(f 99)` returns the inner `x = 7`.
    let int_shadow = "(module m (def (f x) (let ((x 7)) x)) (def (main) (f 99)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(int_shadow))).expect("compile");
    assert_eq!(
        run_returns::<i64>(&bytes, "main"),
        7,
        "same-type shadow returns the inner binding"
    );

    // A match-arm pattern binder that shadows the param is likewise a binder, not a reference:
    // `(match 5 (x x))` binds `x` to the scrutinee 5 for the arm, shadowing the param `x`.
    let arm_shadow = "(module m (def (f x) (match 5 (x x))) (def (main) (f 99)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(arm_shadow))).expect("compile");
    assert_eq!(
        run_returns::<i64>(&bytes, "main"),
        5,
        "match-arm binder shadow binds the scrutinee"
    );
}

// ── value-heap H2c: a STATIC (fully-constant) tuple pays NO per-call heap cost (§2d) ─────────────
//
// The operator directive: get the static heap-value path right FIRST — a constant compound must never
// emit per-call `arr-alloc`. A fully-constant tuple is NOT a runtime computation, so a multi-use
// `let`-bound constant tuple is NOT kept; each projection then folds straight through to the constant
// element (`reduce_to_tuple_elems` follows an unkept binding). The result: the program imports NO
// runtime op and builds no heap value — better than build-once. (The build-once GLOBAL for a constant
// tuple that ESCAPES as a value activates with the first escape path — the type-directed renderer.)

/// A multi-use `let`-bound CONSTANT tuple FOLDS AWAY: its projections reduce to the constant elements,
/// so the program uses the value heap not at all — it imports NO runtime op and runs as a plain scalar.
/// This is the H2c static-correctness win: zero per-call construction for a value that never varies.
#[test]
fn a_multi_use_constant_tuple_folds_away_with_no_heap() {
    use crate::testkit::parse;
    // `(let ((t (tuple 10 20))) (+ (. t 0) (. t 1)))` — `t` is multi-use, but its value is a CONSTANT
    // tuple, so it is not kept; both projections fold to 10 and 20, the body folds to `(+ 10 20)` → 30.
    let src = "(module m (def (main) (let ((t (tuple 10 20))) (+ (. t 0) (. t 1)))) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The H2c property: NO runtime import — the static tuple left no heap trace at all.
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_none(),
        "a constant tuple must fold away, importing no runtime op (no per-call arr-alloc)"
    );
    // And it runs to the folded scalar without any runtime composed in.
    assert_eq!(run_returns::<i64>(&bytes, "main"), 30);
}

/// The routing pin: a projection-only tuple LITERAL folds whether or not its elements are runtime —
/// what a projection reads is the element's own computation, not a heap cell. `(let ((t (tuple 10 a)))
/// (+ (. t 0) (. t 1)))` folds to `(+ 10 a)`: no heap, no runtime import, even though `a` is a runtime
/// param. A tuple reaches the heap only when it is used as a WHOLE value (returned/passed) and so must
/// be built — see `a_narrow_runtime_tuple_element_crosses_the_heap_boundary` (a call-returned tuple)
/// and `a_recursive_runtime_tuple_escapes_to_the_host` (a returned tuple). Shape alone never forces the
/// heap; escaping-as-a-value does.
#[test]
fn a_projection_only_tuple_folds_even_with_a_runtime_element() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (with-param (: a Int64)) \
                 (let ((t (tuple 10 a))) (+ (. t 0) (. t 1)))) (export with-param))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        cdz_run::required_runtime(&bytes).expect("valid").is_none(),
        "a projection-only tuple folds — no heap — even with a runtime element"
    );
    // Runs as a plain scalar function: 10 + 32 = 42, folded, no runtime composed in.
    let got: i64 = run_returns_with(&bytes, "with-param", &[Val::S64(32)]);
    assert_eq!(got, 42, "folds to (+ 10 a)");
}

// ── value-heap H2d: the Perceus BALANCE probe (no leak) ──────────────────────────────────────────
//
// The composed round-trip already rules out use-after-free (a drop of a still-live child would corrupt
// the projection and change the result / trap). The remaining risk is a LEAK — a drop that never fires.
// The runtime's `live-objects` (WIT 54) reports the live heap-cell count, but ONLY in a build compiled
// with `--features debug-counters` (the shipped build returns 0 unconditionally). So this ACCEPTANCE
// probe LOCATES that runtime by its generated `DEBUG_RUNTIME_HASH` (from the content-addressed store —
// `xtask build` puts it there; NEVER built by the test — shelling out to cargo would be slow and couple
// the test to the toolchain), composes it in one store, runs the heap round-trip, and asserts
// `live-objects == 0`. `#[ignore]` because it needs `xtask build` to have populated the store — run
// with `cargo test -p rcdzc perceus_balance -- --ignored` after a build.

/// The `debug-counters` runtime bytes, located by the generated `DEBUG_RUNTIME_HASH` in the content-
/// addressed store, or `None` if absent (so the probe skips rather than fails when `xtask build` has
/// not populated the store). A plain file read keyed by content address — NO build, NO cargo call.
fn find_debug_runtime_wasm() -> Option<Vec<u8>> {
    use crate::backend::wasm::runtime_abi::DEBUG_RUNTIME_HASH;
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let seed = manifest.join("../..").canonicalize().ok()?;
    let repo = seed.join("../..").canonicalize().ok()?;
    let path = repo.join(format!("target/cadenza-store/{DEBUG_RUNTIME_HASH}.wasm"));
    let bytes = std::fs::read(&path).ok()?;
    // Guard against a stale store entry: only accept bytes whose hash matches the pinned debug hash.
    (sha256_hex(&bytes) == DEBUG_RUNTIME_HASH).then_some(bytes)
}

/// A program COMPOSED with the value-heap runtime in ONE wasmtime store — the reusable harness for the
/// heap behavior tests that need to (a) run a program export AND (b) call the runtime's OWN ops (read a
/// returned handle's elements, or read `live-objects`) against the SAME store/allocator the program
/// used. Instantiates the runtime once, forwards its heap funcs into the program's import (under the
/// program's exact hashed name), and instantiates the program. `program` bytes must import the runtime.
struct ComposedRuntime {
    store: wasmtime::Store<()>,
    program: wasmtime::component::Instance,
    rt_instance: wasmtime::component::Instance,
    heap_idx: wasmtime::component::ComponentExportIndex,
}

impl ComposedRuntime {
    /// Compose `program_bytes` with `runtime_bytes`. Panics on any wiring failure (a test-only harness).
    fn new(program_bytes: &[u8], runtime_bytes: &[u8]) -> ComposedRuntime {
        use wasmtime::component::{Component, Linker};
        use wasmtime::{Engine, Store};
        let engine = Engine::default();
        let program_component = Component::new(&engine, program_bytes).expect("program component");
        let runtime_component = Component::new(&engine, runtime_bytes).expect("runtime component");
        let mut store = Store::new(&engine, ());

        let rt_linker: Linker<()> = Linker::new(&engine);
        let rt_instance = rt_linker
            .instantiate(&mut store, &runtime_component)
            .expect("instantiate runtime");
        let heap_idx = rt_instance
            .get_export_index(&mut store, None, "cadenza:runtime/heap")
            .expect("runtime exports heap");

        let req = cdz_run::required_runtime(program_bytes)
            .expect("valid")
            .expect("program imports the runtime");
        let mut linker: Linker<()> = Linker::new(&engine);
        let heap_func_names = runtime_heap_func_names(&engine, &runtime_component);
        let mut iface = linker.instance(&req.import_name).expect("linker instance");
        for fname in &heap_func_names {
            let fidx = rt_instance
                .get_export_index(&mut store, Some(&heap_idx), fname)
                .expect("heap func");
            let f = rt_instance.get_func(&mut store, fidx).expect("func");
            let fname = fname.clone();
            iface
                .func_new(&fname, move |mut ctx, params, results| {
                    f.call(&mut ctx, params, results)?;
                    f.post_return(&mut ctx)?;
                    Ok(())
                })
                .expect("forward heap func");
        }
        let program = linker
            .instantiate(&mut store, &program_component)
            .expect("instantiate program");
        ComposedRuntime {
            store,
            program,
            rt_instance,
            heap_idx,
        }
    }

    /// Call the program export `name` with `args`, returning its single result value.
    fn call(&mut self, name: &str, args: &[wasmtime::component::Val]) -> wasmtime::component::Val {
        let func = self
            .program
            .get_func(&mut self.store, name)
            .expect("export present");
        let mut results = [wasmtime::component::Val::Bool(false)];
        func.call(&mut self.store, args, &mut results)
            .expect("call");
        func.post_return(&mut self.store).expect("post_return");
        results[0].clone()
    }

    /// Drive a CLOSURE-RESOURCE export (C-HOST-1): call `make()` on the `cadenza:closure/exports`
    /// instance to get the closure resource handle, then `call(handle, args…)` to invoke it — returning
    /// the `call` result. This is the host acting as the closure's custodian: it holds an opaque handle
    /// and invokes the guest's `call` method (which dispatches the closure via the guest's own
    /// `call_indirect`). `own<t>` consumes the handle per call, so each `closure_make_call` mints a fresh
    /// handle (C-HOST-1 own/no-drop; the `borrow<t>` repeated-call form is C-HOST-5).
    fn closure_make_call(
        &mut self,
        make_args: &[wasmtime::component::Val],
        call_args: &[wasmtime::component::Val],
    ) -> wasmtime::component::Val {
        use wasmtime::component::Val;
        let iface = self
            .program
            .get_export_index(&mut self.store, None, "cadenza:closure/exports")
            .expect("closure interface exported");
        let make_idx = self
            .program
            .get_export_index(&mut self.store, Some(&iface), "make")
            .expect("closure `make` exported");
        let call_idx = self
            .program
            .get_export_index(&mut self.store, Some(&iface), "call")
            .expect("closure `call` exported");
        let make = self.program.get_func(&mut self.store, make_idx).expect("make func");
        let call = self.program.get_func(&mut self.store, call_idx).expect("call func");
        // make(make_args…) → the closure handle. A PARAMETERIZED export (C-HOST-2) passes its params here
        // (e.g. `adder(10)`); a nullary export passes none.
        let mut handle = [Val::Bool(false)];
        make.call(&mut self.store, make_args, &mut handle).expect("make call");
        make.post_return(&mut self.store).expect("make post_return");
        let mut full_call_args = vec![handle[0].clone()];
        full_call_args.extend_from_slice(call_args);
        let mut out = [Val::Bool(false)];
        call.call(&mut self.store, &full_call_args, &mut out).expect("call");
        call.post_return(&mut self.store).expect("call post_return");
        out[0].clone()
    }

    /// Call a runtime HEAP op by name (e.g. `arr-get`, `get-int`, `arr-len`, `drop`) on the shared
    /// runtime instance — how a test reads back a handle the program returned (the "display function"),
    /// or releases it. Returns the op's single result, or `Val::Bool(false)` for a no-result op like
    /// `drop` (the buffer is sized to the func's actual result count).
    fn heap_call(
        &mut self,
        op: &str,
        args: &[wasmtime::component::Val],
    ) -> wasmtime::component::Val {
        let idx = self
            .rt_instance
            .get_export_index(&mut self.store, Some(&self.heap_idx), op)
            .unwrap_or_else(|| panic!("runtime exports `{op}`"));
        let f = self
            .rt_instance
            .get_func(&mut self.store, idx)
            .expect("heap func");
        let mut results = vec![wasmtime::component::Val::Bool(false); f.results(&self.store).len()];
        f.call(&mut self.store, args, &mut results)
            .unwrap_or_else(|e| panic!("heap call `{op}`: {e}"));
        let _ = f.post_return(&mut self.store);
        results
            .into_iter()
            .next()
            .unwrap_or(wasmtime::component::Val::Bool(false))
    }

    /// The runtime's live heap-cell count (`live-objects`) — 0 in the shipped build, real in the
    /// debug-counters build. The Perceus leak check reads this after a run.
    fn live_objects(&mut self) -> u32 {
        match self.heap_call("live-objects", &[]) {
            wasmtime::component::Val::U32(n) => n,
            other => panic!("live-objects returned {other:?}"),
        }
    }

    /// Drive a RESOURCE-ESCAPE program to completion: reach `make`/`encode` inside the `cadenza:run/run`
    /// instance, call `make()` → a resource handle, then `encode(handle)` → the value bytes. `encode`
    /// takes `self: own<t>`, so it CONSUMES the resource and OWNS the last reference to the compound's
    /// heap handle — so `encode` itself calls `heap.drop(rep)` after the walk (see
    /// `serialize::encode_walk_body`), reclaiming it and balancing `make`'s `arr-alloc`. So after the
    /// `make`/`encode` round-trip NO heap cell remains live — `live_objects()` reads 0. (We do NOT rely
    /// on the resource dtor / a host `resource_drop`: `resource.rep` on a BORROWED self traps in wasmtime
    /// 37, so `encode` keeps `own` + releases the rep itself — the sound release point for the nullary
    /// escape, which always calls encode. See [[rcdzc-r1-resource-encode-linking-findings]].) Returns the
    /// decoded value text.
    fn run_escape_and_drop(&mut self) -> String {
        use wasmtime::component::Val;
        let iface = self
            .program
            .get_export_index(&mut self.store, None, "cadenza:run/run")
            .expect("run interface");
        let make_idx = self
            .program
            .get_export_index(&mut self.store, Some(&iface), "make")
            .expect("make export");
        let encode_idx = self
            .program
            .get_export_index(&mut self.store, Some(&iface), "encode")
            .expect("encode export");
        let make = self
            .program
            .get_func(&mut self.store, make_idx)
            .expect("make func");
        let encode = self
            .program
            .get_func(&mut self.store, encode_idx)
            .expect("encode func");
        let mut handle = [Val::Bool(false)];
        make.call(&mut self.store, &[], &mut handle).expect("make");
        make.post_return(&mut self.store).expect("make post");
        let mut out = [Val::Bool(false)];
        encode
            .call(&mut self.store, &handle, &mut out)
            .expect("encode"); // consumes the own<t>
        encode.post_return(&mut self.store).expect("encode post");
        let bytes: Vec<u8> = match &out[0] {
            Val::List(items) => items
                .iter()
                .map(|v| match v {
                    Val::U8(b) => *b,
                    o => panic!("not u8: {o:?}"),
                })
                .collect(),
            o => panic!("expected list<u8>, got {o:?}"),
        };
        let arenas = cadenza_syntax::codec::decode(&bytes).expect("decode value form");
        cadenza_syntax::sexpr::print(&arenas).trim().to_string()
    }
}

/// H2d ACCEPTANCE: after a heap round-trip, the runtime's live-cell count is 0 — the dup/drop discipline
/// balanced (no leak). Composes a `debug-counters` runtime in ONE store, runs `pair-sum(20,22)` (builds
/// a 2-tuple, boxes 2 elements, projects both, drops the tuple — which cascades to the 2 boxes), then
/// reads `live-objects`.
#[test]
#[ignore]
fn perceus_balance_leaves_no_live_objects() {
    use crate::testkit::parse;
    use wasmtime::component::Val;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!(
            "[H2d] debug-counters runtime not in the store (run `cargo xtask build`); skipping balance probe"
        );
        return;
    };
    let src = "(module m (def (pair-sum (: a Int64) (: b Int64)) \
                 (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (export pair-sum))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    assert_eq!(
        rt.call("pair-sum", &[Val::S64(20), Val::S64(22)]),
        Val::S64(42),
        "the round-trip still computes 42"
    );
    assert_eq!(
        rt.live_objects(),
        0,
        "Perceus leak: heap cells still live after the round-trip (expected 0)"
    );
}

/// RUNTIME STRUCTURAL EQUALITY leak balance: a `=` on two RUNTIME sum values (`value-eq`) leaves NO
/// live heap cells. Both operands are OWNED temporaries — `(build 3)` allocates a cons-list each side —
/// and `value-eq` only BORROWS them, so the emit must `drop` each after the compare (else two whole
/// lists leak). `main` returns a scalar (`if (= …) 1 0`), so the ONLY heap traffic is the two built
/// operands; after the run `live-objects` must be 0. Guards the ownership/drop discipline of the
/// `Core::ValueEq` emit (the exact refcount hazard the feature introduces). `#[ignore]` — needs
/// `xtask build` to have populated the store (run with `-- --ignored`).
#[test]
#[ignore]
fn runtime_value_eq_leaves_no_live_objects() {
    use crate::testkit::parse;
    use wasmtime::component::Val;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!("[value-eq] debug-counters runtime not in the store; skipping balance probe");
        return;
    };
    // Two OWNED cons-list operands (recursion-built, so neither folds); `value-eq` borrows both, the
    // emit drops both. `main` returns a scalar so nothing else touches the heap.
    let src = "(module m \
                 (type IntList (Cons (Tuple Int64 IntList)) Nil) \
                 (def (build n) (if (< n 1) (IntList.Nil ()) \
                    (IntList.Cons (tuple n (build (- n 1)))))) \
                 (def (main) (if (= (build 3) (build 3)) 1 0)) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    assert_eq!(
        rt.call("main", &[]),
        Val::S64(1),
        "the two equal runtime lists compare equal → 1"
    );
    assert_eq!(
        rt.live_objects(),
        0,
        "value-eq leak: the two built operands' heap cells are still live after the compare (expected \
         0 — the borrowing value-eq must drop each owned operand)"
    );
}

/// RUNTIME STRUCTURAL EQUALITY, BORROWED operand: a `let`-bound list compared by `=` (a BORROW) leaves
/// no cell leaked or double-freed. `xs = (build 3)` is compared to a fresh `(build 3)` (an OWNED
/// temporary `value-eq` drops); the result is a scalar `1`, so `xs` is used ONLY as the borrowed
/// operand. `value-eq` must NOT drop `xs` (it only borrows) — the enclosing `let` drops it exactly once.
/// So `live-objects` is 0: the fresh operand reclaimed by `value-eq`, `xs` by the `let`, neither leaked
/// nor doubly-freed. The borrowed-vs-owned companion of `runtime_value_eq_leaves_no_live_objects`
/// (avoids a recursive fold, whose own reclamation is a separate concern). `#[ignore]` — needs the
/// store populated.
#[test]
#[ignore]
fn runtime_value_eq_borrowed_operand_survives_and_balances() {
    use crate::testkit::parse;
    use wasmtime::component::Val;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!("[value-eq] debug-counters runtime not in the store; skipping borrow probe");
        return;
    };
    let src = "(module m \
                 (type IntList (Cons (Tuple Int64 IntList)) Nil) \
                 (def (build n) (if (< n 1) (IntList.Nil ()) \
                    (IntList.Cons (tuple n (build (- n 1)))))) \
                 (def (main) (let ((xs (build 3))) (if (= xs (build 3)) 1 0))) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    assert_eq!(
        rt.call("main", &[]),
        Val::S64(1),
        "the two equal lists compare equal → 1"
    );
    assert_eq!(
        rt.live_objects(),
        0,
        "value-eq borrow leak/double-free: `xs` (borrowed by `=`, dropped by the `let`) plus the owned \
         `(build 3)` operand (dropped by value-eq) must net to 0 live cells"
    );
}

/// KNOWN LEAK (tracking probe): a RECURSIVE function with a HEAP-typed PARAMETER threaded through the
/// recursion and only BORROWED (never destructured/consumed) leaks the cell — a BOUNDED O(1) leak (one
/// cell per heap value created, NOT per recursion level), correctness-PRESERVING (the answer is right).
/// This is a CROSS-CUTTING Perceus-in-recursion gap, NOT closure-specific: a `(Tuple …)` param threaded
/// the same way leaks identically (verified — 0 `drop`s emitted). ROOT: a function that received an
/// OWNED heap param and does not consume it on a control path (the base case `n=0` returns `0` without
/// touching `g`) must DROP it there; today no such drop is inserted (`emit_call_args` also does not
/// `dup` a re-passed heap arg). The correct fix is whole-function last-use drop insertion for params
/// (the general Perceus pass), which is its own workstream — a partial fix risks a double-free (far worse
/// than a bounded leak), so it is tracked here rather than hacked in. This probe ASSERTS the current
/// (leaking) count so the leak cannot silently WORSEN (grow past O(1)); flip the expected value to 0 when
/// the Perceus pass lands. `#[ignore]` — needs the debug-counters store (`cargo xtask build`).
#[test]
#[ignore]
fn a_runtime_closure_leaks_exactly_one_cell_known_gap() {
    use crate::testkit::parse;
    use wasmtime::component::Val;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!(
            "[closure-leak] debug-counters runtime not in the store; skipping known-leak probe"
        );
        return;
    };
    let src = "(module m \
                 (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
                    (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
                 (def (main (: k Int64)) (apply-sum (fn ((: x Int64)) (+ x k)) 3)) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The answer is CORRECT despite the leak (the leak is a reclamation gap, not a miscompile).
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    assert_eq!(
        rt.call("main", &[Val::S64(10)]),
        Val::S64(36),
        "the closure still computes 36"
    );
    // ONE closure cell leaks, regardless of the recursion depth (the cell is built once and only
    // borrowed down the recursion). A larger `n` must NOT increase the leak — that would signal a
    // per-level allocation regression, which this pins.
    let mut rt_deep = ComposedRuntime::new(&program, &runtime_bytes);
    assert_eq!(rt_deep.call("main", &[Val::S64(50)]), Val::S64(6 + 3 * 50));
    assert_eq!(
        rt.live_objects(),
        1,
        "KNOWN GAP: a heap closure param threaded through recursion leaks exactly 1 cell (flip to 0 when \
         the general Perceus param-drop pass lands). A count > 1 is a REGRESSION (per-level allocation)."
    );
    assert_eq!(
        rt_deep.live_objects(),
        1,
        "the leak is O(1): n=50 leaks the same ONE cell as n=3 (no per-recursion-level growth)"
    );
}

/// KNOWN LEAK (tracking probe, the PERVASIVE case): a recursive fold over a HEAP LIST leaks EVERY node —
/// the recursion-heap-reclamation gap is NOT closure-specific and NOT O(1); it is O(N) in the data. A
/// `match` reading `sum-payload` does not drop the matched shell, so a list threaded through a recursive
/// fold and consumed by `match` at each level leaks all its cells. `len(build 3)` returns the CORRECT 3
/// but leaves 7 live cells (3 Cons + 3 tuples + 1 Nil). This pins the leak COUNT so it cannot silently
/// worsen and documents the real scope of the gap (contrast the O(1) closure sub-case above). The FIX is
/// the general Perceus drop-insertion pass (a sum-match drops the matched shell after dup-ing its payload;
/// a dead heap param drops on its branch) — its own workstream, high correctness stakes. Flip the expected
/// counts to 0 when it lands. `#[ignore]` — needs the debug-counters store (`cargo xtask build`).
#[test]
#[ignore]
fn a_recursive_list_fold_leaks_every_node_known_gap() {
    use crate::testkit::parse;
    use wasmtime::component::Val;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!("[list-leak] debug-counters runtime not in the store; skipping known-leak probe");
        return;
    };
    let src = "(module m (type L (Cons (Tuple Int64 L)) Nil) \
        (def (len (: xs L) (: acc Int64)) \
           (match xs ((L.Cons (tuple h t)) (len t (+ acc 1))) ((L.Nil _) acc))) \
        (def (build (: n Int64)) (if (< n 1) (L.Nil ()) (L.Cons (tuple n (build (- n 1)))))) \
        (def (main) (len (build 3) 0)) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    // The answer is CORRECT despite the leak (reclamation gap, not a miscompile).
    assert_eq!(rt.call("main", &[]), Val::S64(3), "len(build 3) computes 3");
    // A 3-element list = 3 Cons + 3 (Int,list) tuples + 1 Nil = 7 cells, none reclaimed (the recursion
    // heap-reclamation gap). Flip to 0 when the general Perceus drop-insertion pass lands. This documents
    // the PERVASIVE, O(N)-in-data scope of the gap (the closure sub-case above is only O(1)).
    assert_eq!(
        rt.live_objects(),
        7,
        "KNOWN GAP: a recursive list fold leaks every heap node (match does not reclaim the matched \
         shell); 7 cells for a 3-element list. Flip to 0 when the Perceus drop-insertion pass lands."
    );
}

/// R2 RECLAMATION ACCEPTANCE: a RUNTIME compound that ESCAPES to the host as a resource leaves NO live
/// heap cells after the `make`/`encode` round-trip — `encode` (which takes `own<t>`, consuming the
/// resource) calls `heap.drop(rep)` after the walk, reclaiming the compound's rc handle (cascading to its
/// boxed elements) and balancing `make`'s `arr-alloc`. Composes the debug-counters runtime, runs the
/// recursive-tuple escape, and reads `live-objects` → 0. `#[ignore]` — needs `xtask build` to have
/// populated the store (run with `-- --ignored`). (This was the R2 leak bug; the fix is encode-owns-and-
/// drops, NOT a resource dtor + borrow — `resource.rep` on a borrow traps in wasmtime 37, see
/// [[rcdzc-r1-resource-encode-linking-findings]].)
#[test]
#[ignore]
fn a_runtime_resource_escape_leaves_no_live_objects() {
    use crate::testkit::parse;

    let Some(runtime_bytes) = find_debug_runtime_wasm() else {
        eprintln!("[R2] debug-counters runtime not in the store; skipping reclamation probe");
        return;
    };
    // A recursive (unfoldable) tuple that escapes to the host as a resource.
    let src = "(module m (def (f n) (if (= n 0) (tuple n 7) (f (- n 1)))) \
                 (def (main) (f 3)) (export main))";
    let program = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut rt = ComposedRuntime::new(&program, &runtime_bytes);
    let value = rt.run_escape_and_drop();
    assert_eq!(
        value, "(: (tuple 0 7) (Tuple Int64 Int64))",
        "the escaped value still decodes correctly"
    );
    assert_eq!(
        rt.live_objects(),
        0,
        "R2 leak: the escaped compound's heap cells are still live after the round-trip (expected 0 — \
         encode must call `heap.drop(rep)` on its owned handle)"
    );
}

// ── value-heap H3a: a compound RETURNED transfers ownership; the HOST-facing return is a RESOURCE ──
//
// A compound leaving a function transfers ownership of its handle to the consumer (the callee does not
// drop it — `binding_escapes`). The HOST-facing return (a compound crossing the component boundary) is
// NOT a raw handle: a single nullary export returning a tuple/record crosses as a component-model
// RESOURCE whose `encode() -> list<u8>` walks the live handle into the canonical binary value form
// (the resource vertical, tests below). A compound on the MULTI-EXPORT boundary has no such shape and
// still DECLINES (reject-don't-miscompile — `export_result_valtype`). The internal ownership-transfer
// behavior is exercised by the H2 round-trips + the balance probe.

/// The heap interface's function names, read off the runtime component's type (the same discovery
/// `cdz-run` does) — so the balance probe forwards exactly what the runtime exports, nothing hard-coded.
fn runtime_heap_func_names(
    engine: &wasmtime::Engine,
    runtime: &wasmtime::component::Component,
) -> Vec<String> {
    use wasmtime::component::types::ComponentItem;
    for (name, item) in runtime.component_type().exports(engine) {
        if name != "cadenza:runtime/heap" {
            continue;
        }
        if let ComponentItem::ComponentInstance(inst) = item {
            return inst
                .exports(engine)
                .filter_map(|(fname, i)| {
                    matches!(i, ComponentItem::ComponentFunc(_)).then(|| fname.to_string())
                })
                .collect();
        }
    }
    Vec::new()
}

/// `(def (main) 42)` compiles to a component that runs to 42.
#[test]
fn scalar_runs_to_42() {
    let bytes = compile_component(&prog_scalar()).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 42);
}

/// `(def (main) (if false 1 2))` compiles to a component that runs to 2 (the else branch).
#[test]
fn if_runs_to_2() {
    let bytes = compile_component(&prog_if()).expect("compile");
    assert_eq!(run_returns::<i64>(&bytes, "main"), 2);
}

// ── exported parameterized functions: runtime operands, end-to-end (compile → run with args) ─────

/// An exported `(def (add (: a Int64) (: b Int64)) (+ a b))` compiles to a component whose `add`
/// export is a real wasm function taking two s64 params — run under wasmtime with (20, 22) it returns
/// 42. This is the end-to-end proof of runtime integer operands: the body did NOT fold (the params
/// are unknown), it emitted `local.get 0; local.get 1; i64.add`, and the boundary carries the params.
#[test]
fn an_exported_addition_runs_over_runtime_args() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let got: i64 = run_returns_with(&bytes, "add", &[Val::S64(20), Val::S64(22)]);
    assert_eq!(got, 42);
    // A different argument pair exercises the SAME function — the value is genuinely runtime.
    let got2: i64 = run_returns_with(&bytes, "add", &[Val::S64(100), Val::S64(-1)]);
    assert_eq!(got2, 99);
}

/// The float dual: an exported `(+. a b)` over two runtime Float64 params emits `local.get 0;
/// local.get 1; f64.add` (no fold — the params are unknown, no overflow guard — floats never trap) and
/// the boundary carries the params as component `f64`. Run under wasmtime it computes the IEEE sum.
/// Exercised with a NON-exact pair (0.1, 0.2 → 0.30000000000000004) so a wrong op or a fold-to-0.3
/// would be caught, plus `*.`/`/.` to pin each machine op. Compared BY BITS.
#[test]
fn an_exported_float_op_runs_over_runtime_args() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    for (op, a, b, want) in [
        ("+.", 0.1f64, 0.2f64, 0.1f64 + 0.2f64),
        ("*.", 6.0, 7.0, 42.0f64),
        ("-.", 5.5, 2.0, 3.5f64),
        ("/.", 1.0, 3.0, 1.0f64 / 3.0f64),
    ] {
        let src = format!("(module m (def (f (: a Float64) (: b Float64)) ({op} a b)) (export f))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        let got: f64 = run_returns_with(&bytes, "f", &[Val::Float64(a), Val::Float64(b)]);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "runtime float {op} over ({a}, {b}) must equal the IEEE result by bits"
        );
    }
}

/// The explicit INT→FLOAT conversion `Float64.of-int` over a RUNTIME integer emits
/// `f64.convert_i64_s` (a constant folds instead). `(def (f (: n Int64)) (Float64.of-int n))` run with
/// 42 returns 42.0; a large Int64 rounds to the nearest f64 — total, never trapping.
#[test]
fn float_of_int_converts_a_runtime_integer() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (f (: n Int64)) (Float64.of-int n)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let got: f64 = run_returns_with(&bytes, "f", &[Val::S64(42)]);
    assert_eq!(got.to_bits(), 42.0f64.to_bits(), "of-int 42 = 42.0");
    let big: f64 = run_returns_with(&bytes, "f", &[Val::S64(9223372036854775807)]);
    assert_eq!(
        big.to_bits(),
        (9223372036854775807i64 as f64).to_bits(),
        "of-int Int64.max rounds to the nearest f64"
    );
}

/// The explicit FLOAT-WIDTH conversion `Float N.of` over a RUNTIME float: `Float32.of` DEMOTES
/// (`f32.demote_f64`, rounds), `Float64.of` PROMOTES (`f64.promote_f32`, exact). A constant folds; a
/// runtime float emits the machine op. Run under wasmtime and compared BY BITS.
#[test]
fn float_of_converts_a_runtime_float_width() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // Demote: 0.1 : Float64 → Float32 rounds to the nearest binary32 (the f32 bits, lifted back to f64
    // at the boundary read as an f32 result).
    let demote = "(module m (def (f (: x Float64)) (Float32.of x)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(demote))).expect("compile");
    let got: f32 = run_returns_with(&bytes, "f", &[Val::Float64(0.1)]);
    assert_eq!(
        got.to_bits(),
        (0.1f64 as f32).to_bits(),
        "Float32.of 0.1 demotes to the nearest binary32"
    );
    // Promote: a Float32 → Float64 is exact.
    let promote = "(module m (def (f (: x Float32)) (Float64.of x)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(promote))).expect("compile");
    let got: f64 = run_returns_with(&bytes, "f", &[Val::Float32(1.5f32)]);
    assert_eq!(
        got.to_bits(),
        (1.5f32 as f64).to_bits(),
        "Float64.of 1.5f32 promotes exactly"
    );
}

/// `(= n 0)` selects to `eqz` (one instruction), and it must still compute the correct boolean under
/// wasmtime: true at zero, false otherwise, for both operand orders — a wrong `eqz` (say, ignoring the
/// operand) would return a constant. `is-zero`/`is-zero2` (commuted) both return 1 at 0, 0 elsewhere.
#[test]
fn equality_with_zero_runs_correctly() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // (if (= n 0) 1 0) so the boolean crosses as an Int64 the harness reads.
    let src = "(module m \
                 (def (is-zero (: n Int64)) (if (= n 0) 1 0)) \
                 (def (is-zero2 (: n Int64)) (if (= 0 n) 1 0)) \
                 (export is-zero) (export is-zero2))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(
        run_returns_with::<i64>(&bytes, "is-zero", &[Val::S64(0)]),
        1
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "is-zero", &[Val::S64(7)]),
        0
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "is-zero", &[Val::S64(-1)]),
        0
    );
    // Commuted form dispatches identically.
    assert_eq!(
        run_returns_with::<i64>(&bytes, "is-zero2", &[Val::S64(0)]),
        1
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "is-zero2", &[Val::S64(42)]),
        0
    );
}

/// A runtime `if` with two CHEAP LEAF branches emits wasm's BRANCHLESS `select` (no `if`/`else`/`end`),
/// and it must compute the SAME value the structured block would — `select` pops `[then, else, cond]`
/// and pushes `then` iff `cond` is nonzero. `min` `(if (< a b) a b)` and a value-picking `(if p a b)`
/// exercise both operand orders under wasmtime. The point is that the operand ORDER the backend emits
/// (then, else, cond, select) matches the `if`'s truth sense — a swapped order would silently return
/// the wrong branch, which no instruction-count check would catch.
#[test]
fn a_branchless_select_computes_the_if_value() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // min via (if (< a b) a b): the SMALLER of the two, either arg order.
    let min = "(module m (def (f (: a Int64) (: b Int64)) (if (< a b) a b)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(min))).expect("compile");
    let min_of = |a, b| run_returns_with::<i64>(&bytes, "f", &[Val::S64(a), Val::S64(b)]);
    assert_eq!(min_of(3, 7), 3);
    assert_eq!(min_of(7, 3), 3);
    assert_eq!(min_of(-5, -5), -5);
    // value pick (if p a b): p true → a, p false → b (the two condition truth values).
    let pick = "(module m (def (f (: p Bool) (: a Int64) (: b Int64)) (if p a b)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(pick))).expect("compile");
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::Bool(true), Val::S64(11), Val::S64(22)]),
        11
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::Bool(false), Val::S64(11), Val::S64(22)]),
        22
    );
}

/// CONDITIONAL CONSTANT PROPAGATION on a REPEATED condition: within a branch the enclosing condition is
/// known, so a directly-nested `if` on the SAME condition collapses to its taken arm. `(if c (if c 1 2)
/// 3)` → `(if c 1 3)` (inner `c` is true in the then-branch, so the inner `if` takes `1`); `(if c 1 (if
/// c 2 3))` → `(if c 1 3)` (inner `c` is false in the else-branch, takes `3`). Both then fold to a single
/// branchless `select` on `c` with no re-test of the condition. A DIFFERENT inner condition is left
/// intact (no collapse). Sound in a language with no mutation: a pure condition re-evaluates to the same
/// value, so its second test is redundant.
#[test]
fn a_repeated_condition_in_a_nested_if_collapses() {
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    use wasmtime::component::Val;
    // The Lir of `(if c (if c 1 2) 3)` must have a SINGLE reference to the condition (one `local.get 0`
    // feeding a branchless `select`) — the inner `if`'s re-test of `c` is gone.
    let lir = |body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f (: c Bool)) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
        let params: Vec<_> = db.defs[d]
            .params
            .clone()
            .into_iter()
            .map(|p| {
                let b = db
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (b, crate::infer::type_of(&mut db, b))
            })
            .collect();
        let body = db.defs[d].body.expect("body");
        crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code
    };
    let then_nested = lir("(if c (if c 1 2) 3)");
    assert_eq!(
        then_nested
            .iter()
            .filter(|i| matches!(i, Lir::LocalGet(0)))
            .count(),
        1,
        "the inner if's re-test of c must be gone (one condition read), got: {then_nested:?}"
    );
    // Value parity for both nesting positions.
    let pick = |src: &str, c: bool| -> i64 {
        let bytes = compile_component(&crate::codec::encode(&crate::testkit::parse(src)));
        let bytes = bytes.expect("compile");
        run_returns_with::<i64>(&bytes, "f", &[Val::Bool(c)])
    };
    let then_src = "(module m (def (f (: c Bool)) (if c (if c 1 2) 3)) (export f))";
    assert_eq!(
        pick(then_src, true),
        1,
        "then-branch inner if takes its then-arm"
    );
    assert_eq!(pick(then_src, false), 3);
    let else_src = "(module m (def (f (: c Bool)) (if c 1 (if c 2 3))) (export f))";
    assert_eq!(pick(else_src, true), 1);
    assert_eq!(
        pick(else_src, false),
        3,
        "else-branch inner if takes its else-arm"
    );
    // A DIFFERENT inner condition must NOT collapse — both conditions are still read at run time.
    let diff = {
        let src = "(module m (def (f (: c Bool) (: d Bool)) (if c (if d 1 2) 3)) (export f))";
        let bytes =
            compile_component(&crate::codec::encode(&crate::testkit::parse(src))).expect("compile");
        (
            run_returns_with::<i64>(&bytes, "f", &[Val::Bool(true), Val::Bool(true)]),
            run_returns_with::<i64>(&bytes, "f", &[Val::Bool(true), Val::Bool(false)]),
            run_returns_with::<i64>(&bytes, "f", &[Val::Bool(false), Val::Bool(true)]),
        )
    };
    assert_eq!(
        diff,
        (1, 2, 3),
        "a distinct inner condition is not collapsed and dispatches on both"
    );
}

/// A NESTED checked op `(* (+ a b) c)` runs correctly AND still traps on overflow after the
/// dest-threading optimization (the inner `(+ a b)` writes its result directly into the outer mul's
/// operand slot rather than a separate scratch slot + copy). Both the value and the guards must
/// survive: (2+3)*4 = 20, and an inner add that overflows i64 must still trap through the composed op.
#[test]
fn a_nested_checked_op_runs_and_still_traps() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // (2 + 3) * 4 = 20 — the inner add's result (5) feeds the mul directly from its slot.
    let got: i64 = run_returns_with(&bytes, "f", &[Val::S64(2), Val::S64(3), Val::S64(4)]);
    assert_eq!(got, 20);
    // A different triple exercises the same function — genuinely runtime, not folded.
    let got2: i64 = run_returns_with(&bytes, "f", &[Val::S64(10), Val::S64(-4), Val::S64(-2)]);
    assert_eq!(got2, -12); // (10 + -4) * -2 = -12
    // The INNER add overflows i64 (MAX + 1) — the guard, which now reads the shared slot, must still
    // fire and trap before the mul ever runs.
    assert!(
        call_traps(&bytes, "f", &[Val::S64(i64::MAX), Val::S64(1), Val::S64(1)]),
        "an inner-add overflow must trap even though its result shares the outer op's slot"
    );
    // The OUTER mul overflows — the mul guard (reading the same shared slot as its $a) must trap.
    assert!(
        call_traps(&bytes, "f", &[Val::S64(i64::MAX), Val::S64(0), Val::S64(2)]),
        "an outer-mul overflow must trap"
    );
}

/// STRENGTH REDUCTION `x * 2^k → x << k`: a multiply by a constant power of two runs to the SAME value
/// AND traps on the SAME overflowing inputs as the multiply would (a left shift is exact multiplication
/// by a power of two — `numeric-model.md`). Exercised over signed/unsigned/narrow widths and both
/// operand orders; the trap parity is the load-bearing check (a wrong overflow guard would silently
/// wrap instead of trapping).
#[test]
fn multiply_by_power_of_two_runs_and_traps_like_the_multiply() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    // Int64 * 8: value + overflow trap.
    let m8 = "(module m (def (f (: n Int64)) (* n 8)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(m8))).expect("compile");
    assert_eq!(run_returns_with::<i64>(&b, "f", &[Val::S64(5)]), 40);
    assert_eq!(run_returns_with::<i64>(&b, "f", &[Val::S64(-5)]), -40);
    assert_eq!(run_returns_with::<i64>(&b, "f", &[Val::S64(0)]), 0);
    // 2^60 * 8 = 2^63 exceeds Int64 max → trap (exactly as `*` would).
    assert!(
        call_traps(&b, "f", &[Val::S64(1i64 << 60)]),
        "x * 8 must trap on overflow like the multiply"
    );
    // Const on the LEFT: 32 * n.
    let l = "(module m (def (f (: n Int64)) (* 32 n)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(l))).expect("compile");
    assert_eq!(run_returns_with::<i64>(&b, "f", &[Val::S64(3)]), 96);
    assert_eq!(run_returns_with::<i64>(&b, "f", &[Val::S64(-3)]), -96);
    // Narrow UInt8 * 2: the range-check (result may fit the i32 slot but exceed the 8-bit type) fires.
    // The UInt8 result crosses as `u8`, so read it back as `u8`.
    let u8p = "(module m (def (f (: n UInt8)) (* n 2)) (export f))";
    let b = compile_component(&crate::codec::encode(&parse(u8p))).expect("compile");
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U8(100)]), 200);
    assert_eq!(run_returns_with::<u8>(&b, "f", &[Val::U8(127)]), 254);
    assert!(
        call_traps(&b, "f", &[Val::U8(128)]),
        "UInt8 128 * 2 = 256 exceeds the 8-bit type → trap"
    );
}

/// An exported comparison `(def (lt (: a Int64) (: b Int64)) (< a b))` runs to a boolean over runtime
/// args — the signed machine comparison at the boundary.
#[test]
fn an_exported_comparison_runs_over_runtime_args() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (export lt))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(run_returns_with::<bool>(
        &bytes,
        "lt",
        &[Val::S64(1), Val::S64(2)]
    ));
    assert!(!run_returns_with::<bool>(
        &bytes,
        "lt",
        &[Val::S64(5), Val::S64(5)]
    ));
    // Signed: -1 < 1 is true (an unsigned compare would read -1 as huge and say false).
    assert!(run_returns_with::<bool>(
        &bytes,
        "lt",
        &[Val::S64(-1), Val::S64(1)]
    ));
}

/// `(if (< a b) true false)` is the boolean-coercion no-op — it folds to `(< a b)` and returns exactly
/// the comparison's value. Running it confirms the fold preserves the boolean result (not inverted).
#[test]
fn an_if_true_false_coercion_runs_as_the_condition() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (lt (: a Int64) (: b Int64)) (if (< a b) true false)) (export lt))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(run_returns_with::<bool>(
        &bytes,
        "lt",
        &[Val::S64(1), Val::S64(2)]
    ));
    assert!(!run_returns_with::<bool>(
        &bytes,
        "lt",
        &[Val::S64(5), Val::S64(5)]
    ));
}

/// `(if (< a b) false true)` is the boolean NEGATION `!(a < b)` — it folds to `Core::Not` (emitted as
/// `i32.eqz`). Running it confirms the result is INVERTED relative to the comparison (`>=`).
#[test]
fn an_if_false_true_negation_runs_as_not_the_condition() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (ge (: a Int64) (: b Int64)) (if (< a b) false true)) (export ge))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // !(1 < 2) = false; !(5 < 5) = true; !(-1 < 1) = false — the inversion of `<`.
    assert!(!run_returns_with::<bool>(
        &bytes,
        "ge",
        &[Val::S64(1), Val::S64(2)]
    ));
    assert!(run_returns_with::<bool>(
        &bytes,
        "ge",
        &[Val::S64(5), Val::S64(5)]
    ));
    assert!(!run_returns_with::<bool>(
        &bytes,
        "ge",
        &[Val::S64(-1), Val::S64(1)]
    ));
}

/// An exported parameter with NO annotation is ambiguous — its machine width is unfixed, so the
/// compiler DECLINES asking for an annotation rather than inventing a width.
#[test]
fn an_unannotated_exported_parameter_declines() {
    use crate::testkit::parse;
    let src = "(module m (def (id x) x) (export id))";
    let msg = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("must decline")
        .message;
    assert!(
        msg.contains("ambiguous") || msg.contains("annotate"),
        "got: {msg}"
    );
}

// ── runtime overflow TRAPS, never wraps (numeric-model §Overflow Is Defined) ─────────────────────

/// The unqualified `+` on runtime operands TRAPS on overflow (the numeric-model default) rather than
/// silently wrapping — the checked-arith guard. In range it computes; `Int64.max + 1` traps.
#[test]
fn runtime_addition_traps_on_overflow() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // In range: computes.
    assert_eq!(
        run_returns_with::<i64>(&bytes, "add", &[Val::S64(20), Val::S64(22)]),
        42
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "add", &[Val::S64(-5), Val::S64(-3)]),
        -8
    );
    // Overflow: TRAPS (does not wrap to Int64.min).
    assert!(call_traps(
        &bytes,
        "add",
        &[Val::S64(i64::MAX), Val::S64(1)]
    ));
}

/// A doubling add `(+ a a)` uses the collapsed single-xor overflow guard (`(r^a)<0`); its VALUE and TRAP
/// behavior must match the general two-operand add exactly.
#[test]
fn doubling_add_value_and_trap_parity() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (dbl (: a Int64)) (+ a a)) (export dbl))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // In range, both signs and zero.
    assert_eq!(run_returns_with::<i64>(&bytes, "dbl", &[Val::S64(21)]), 42);
    assert_eq!(
        run_returns_with::<i64>(&bytes, "dbl", &[Val::S64(-21)]),
        -42
    );
    assert_eq!(run_returns_with::<i64>(&bytes, "dbl", &[Val::S64(0)]), 0);
    // The largest value that still doubles in range, and the smallest.
    assert_eq!(
        run_returns_with::<i64>(&bytes, "dbl", &[Val::S64(i64::MAX / 2)]),
        (i64::MAX / 2) * 2
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "dbl", &[Val::S64(i64::MIN / 2)]),
        (i64::MIN / 2) * 2
    );
    // Overflow: doubling past the boundary TRAPS (the single-xor guard fires exactly like the general one).
    assert!(call_traps(&bytes, "dbl", &[Val::S64((i64::MAX / 2) + 1)]));
    assert!(call_traps(&bytes, "dbl", &[Val::S64(i64::MAX)]));
    assert!(call_traps(&bytes, "dbl", &[Val::S64(i64::MIN)]));
}

/// Runtime `-` traps on overflow (e.g. `Int64.min - 1`), computes in range.
#[test]
fn runtime_subtraction_traps_on_overflow() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (sub (: a Int64) (: b Int64)) (- a b)) (export sub))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(
        run_returns_with::<i64>(&bytes, "sub", &[Val::S64(10), Val::S64(3)]),
        7
    );
    assert!(call_traps(
        &bytes,
        "sub",
        &[Val::S64(i64::MIN), Val::S64(1)]
    ));
    assert!(call_traps(
        &bytes,
        "sub",
        &[Val::S64(i64::MAX), Val::S64(-1)]
    ));
}

/// A `+`/`-` by a compile-time CONSTANT uses the specialized single-compare overflow guard (`r <ₛ a` /
/// `r >ₛ a`) instead of the general two-`xor` sign test. The specialization must trap at EXACTLY the
/// same boundary the general guard would, and never spuriously — so pin every edge for the four
/// sign/op combinations. `(+ a 1)` overflows only at `a = MAX`; `(- a 1)` only at `a = MIN`; a negative
/// constant flips the boundary (`(+ a -1)` at `MIN`, `(- a -1)` = `a+1` at `MAX`), and a
/// constant-on-the-left add (commutative) behaves as the right-constant form.
#[test]
fn a_constant_operand_addsub_traps_at_the_exact_boundary() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let f = |src: &str| compile_component(&crate::codec::encode(&parse(src))).expect("compile");

    // (+ a 1): traps only at MAX; MAX-1 → MAX; MIN stays in range.
    let add1 = f("(module m (def (g (: a Int64)) (+ a 1)) (export g))");
    assert_eq!(run_returns_with::<i64>(&add1, "g", &[Val::S64(0)]), 1);
    assert_eq!(
        run_returns_with::<i64>(&add1, "g", &[Val::S64(i64::MAX - 1)]),
        i64::MAX
    );
    assert_eq!(
        run_returns_with::<i64>(&add1, "g", &[Val::S64(i64::MIN)]),
        i64::MIN + 1,
        "adding 1 near MIN must NOT spuriously trap"
    );
    assert!(call_traps(&add1, "g", &[Val::S64(i64::MAX)]));

    // (- a 1): traps only at MIN; MIN+1 → MIN.
    let sub1 = f("(module m (def (g (: a Int64)) (- a 1)) (export g))");
    assert_eq!(
        run_returns_with::<i64>(&sub1, "g", &[Val::S64(i64::MIN + 1)]),
        i64::MIN
    );
    assert!(call_traps(&sub1, "g", &[Val::S64(i64::MIN)]));

    // (+ a -1) [negative constant]: traps only at MIN (subtracts one).
    let addneg = f("(module m (def (g (: a Int64)) (+ a -1)) (export g))");
    assert_eq!(run_returns_with::<i64>(&addneg, "g", &[Val::S64(0)]), -1);
    assert!(call_traps(&addneg, "g", &[Val::S64(i64::MIN)]));

    // (- a -1) [= a + 1]: traps only at MAX.
    let subneg = f("(module m (def (g (: a Int64)) (- a -1)) (export g))");
    assert_eq!(run_returns_with::<i64>(&subneg, "g", &[Val::S64(0)]), 1);
    assert!(call_traps(&subneg, "g", &[Val::S64(i64::MAX)]));

    // (+ 1 a) [constant on the LEFT, commutative]: same as (+ a 1) — traps only at MAX.
    let add1l = f("(module m (def (g (: a Int64)) (+ 1 a)) (export g))");
    assert_eq!(run_returns_with::<i64>(&add1l, "g", &[Val::S64(41)]), 42);
    assert!(call_traps(&add1l, "g", &[Val::S64(i64::MAX)]));
}

/// Runtime `*` traps on overflow (`Int64.max * 2`, `Int64.min * -1`), computes in range. The mul guard
/// (a≠0 && r/a≠b, with div_s catching MIN/-1) is the subtlest — pin it directly.
#[test]
fn runtime_multiplication_traps_on_overflow() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (mul (: a Int64) (: b Int64)) (* a b)) (export mul))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert_eq!(
        run_returns_with::<i64>(&bytes, "mul", &[Val::S64(6), Val::S64(7)]),
        42
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "mul", &[Val::S64(0), Val::S64(999)]),
        0
    );
    assert!(call_traps(
        &bytes,
        "mul",
        &[Val::S64(i64::MAX), Val::S64(2)]
    ));
    assert!(call_traps(
        &bytes,
        "mul",
        &[Val::S64(i64::MIN), Val::S64(-1)]
    ));
}

/// A full-width signed `(* a C)` with a constant `C > 0` (non-power-of-two) guards overflow with a
/// compile-time BOUND CHECK (`a > MAX/C || a < MIN/C → trap`) instead of the `div_s` round-trip — but
/// must trap at EXACTLY the same points. `(* a 3)`: `a = MAX/3` fits, `a = MAX/3 + 1` overflows up;
/// `a = MIN/3` fits, `a = MIN/3 - 1` overflows down; 0 and small values compute. Pins the boundary is
/// exact both directions (the bound check's truncated MAX/C, MIN/C match the true overflow point).
#[test]
fn a_constant_multiplier_bound_check_traps_at_the_exact_boundary() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (f (: a Int64)) (* a 3)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let max_over_3 = i64::MAX / 3; // 3074457345618258602
    let min_over_3 = i64::MIN / 3; // -3074457345618258602
    // In range at the boundary and below.
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::S64(max_over_3)]),
        max_over_3 * 3
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::S64(min_over_3)]),
        min_over_3 * 3
    );
    assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(0)]), 0);
    assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(-5)]), -15);
    // One past the boundary overflows — both directions must trap.
    assert!(
        call_traps(&bytes, "f", &[Val::S64(max_over_3 + 1)]),
        "a*3 just past MAX/3 must trap (overflow up)"
    );
    assert!(
        call_traps(&bytes, "f", &[Val::S64(min_over_3 - 1)]),
        "a*3 just past MIN/3 must trap (overflow down)"
    );
    // A NARROW const multiply keeps BOTH range bounds (a product can leave either side) — regression
    // guard for the reachable-bounds interaction: Int8 `a*3` at 50 = 150 > 127 must trap, not wrap.
    let narrow = "(module m (def (f (: a Int8)) (* a 3)) (export f))";
    let nb = compile_component(&crate::codec::encode(&parse(narrow))).expect("compile");
    assert_eq!(run_returns_with::<i8>(&nb, "f", &[Val::S8(42)]), 126);
    assert!(
        call_traps(&nb, "f", &[Val::S8(50)]),
        "Int8 50*3=150 > 127 must trap"
    );
    assert!(
        call_traps(&nb, "f", &[Val::S8(-50)]),
        "Int8 -50*3=-150 < -128 must trap"
    );
}

/// The const-multiplier bound check extends to a NEGATIVE constant `C <= -2`: `a*C` shrinks as `a`
/// grows, so it fits iff `MAX/C <= a <= MIN/C` and traps iff `a < MAX/C || a > MIN/C` (both
/// trunc-toward-zero). `(* a -3)`: `a = MIN/-3` (positive) fits, `+1` overflows; `a = MAX/-3` (negative)
/// fits, `-1` overflows. A negative POWER of two (`-2`) is eligible too (only positive powers become a
/// shift). BUT `C = -1` is EXCLUDED — `MIN/-1 = 2^63` is not i64-representable — so it keeps the `div_s`
/// guard, and `MIN * -1` still traps through it.
#[test]
fn a_negative_constant_multiplier_bound_check_traps_at_the_exact_boundary() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (f (: a Int64)) (* a -3)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let max_over_neg3 = i64::MAX / -3; // -3074457345618258602 (the negative lower endpoint)
    let min_over_neg3 = i64::MIN / -3; // 3074457345618258602 (the positive upper endpoint)
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::S64(min_over_neg3)]),
        min_over_neg3 * -3
    );
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::S64(max_over_neg3)]),
        max_over_neg3 * -3
    );
    assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(5)]), -15);
    assert!(
        call_traps(&bytes, "f", &[Val::S64(min_over_neg3 + 1)]),
        "a*-3 just past MIN/-3 must trap"
    );
    assert!(
        call_traps(&bytes, "f", &[Val::S64(max_over_neg3 - 1)]),
        "a*-3 just past MAX/-3 must trap"
    );
    // `* -1` is the excluded case — it keeps the div_s guard; `MIN * -1` must still trap (overflow), a
    // small value negates. This pins that C=-1 did NOT take the bound-check path (which would have
    // panicked the compiler computing MIN/-1) and still traps correctly.
    let neg1 = "(module m (def (f (: a Int64)) (* a -1)) (export f))";
    let nb = compile_component(&crate::codec::encode(&parse(neg1))).expect("compile");
    assert_eq!(run_returns_with::<i64>(&nb, "f", &[Val::S64(5)]), -5);
    assert!(
        call_traps(&nb, "f", &[Val::S64(i64::MIN)]),
        "MIN * -1 overflows and must trap"
    );
}

/// A NESTED runtime checked op composes and traps correctly — `(* (+ a b) c)` computes in range and
/// the shared scratch pool does not corrupt across the nesting.
#[test]
fn nested_runtime_checked_ops_compose() {
    use crate::testkit::parse;
    use wasmtime::component::Val;
    let src = "(module m (def (f (: a Int64) (: b Int64) (: c Int64)) (* (+ a b) c)) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // (2+3)*6 = 30 — the nested-ops correctness check (shared scratch, no aliasing).
    assert_eq!(
        run_returns_with::<i64>(&bytes, "f", &[Val::S64(2), Val::S64(3), Val::S64(6)]),
        30
    );
    // The inner add overflows → traps before the mul.
    assert!(call_traps(
        &bytes,
        "f",
        &[Val::S64(i64::MAX), Val::S64(1), Val::S64(1)]
    ));
}

// ── runtime integer ops, width-generic: the full operator set over boundary parameters ──────────
//
// Every op below runs over RUNTIME operands (exported parameters), so it exercises the emitted machine
// instructions + guards, NOT the constant fold (the corpus `(call …)` cases prove the folded-then-run
// agreement; these pin the emitted path directly and, crucially, the TRAP behavior a folded case can't
// reach without a compile-time-provable input). Each helper compiles `(def (f <params>) <body>)` and
// runs `f` under wasmtime with the given args. Widths 8/16/32/64 (the aliased, boundary-representable
// ones) both signednesses; the emitter is width-generic (a value promotes to the smallest slot that
// holds it, computes there, then range-checks back to its N-bit type).
mod runtime_ops {
    use super::{FromVal, call_traps, run_returns_with};
    use crate::compile::compile_component;
    use crate::testkit::parse;
    use wasmtime::component::Val;

    /// Compile `(module m (def (f <params>) <body>) (export f))` and return its bytes.
    fn func(params: &str, body: &str) -> Vec<u8> {
        let src = format!("(module m (def (f {params}) {body}) (export f))");
        compile_component(&crate::codec::encode(&parse(&src))).expect("compile")
    }

    /// Run `f(args)` decoding the result to `T`.
    fn run<T: FromVal>(params: &str, body: &str, args: &[Val]) -> T {
        run_returns_with::<T>(&func(params, body), "f", args)
    }

    /// Whether `f(args)` traps.
    fn traps(params: &str, body: &str, args: &[Val]) -> bool {
        call_traps(&func(params, body), "f", args)
    }

    #[test]
    fn signed_negation_traps_only_at_min() {
        // `(- 0 a)` is integer negation; the specialized guard is `a == MIN → trap` (the sole overflow),
        // replacing the general two-`xor` sub guard. The Lir shape is checked in select.rs; here we
        // confirm the VALUE is `-a` for every in-range input and that ONLY `MIN` traps.
        assert_eq!(run::<i64>("(: a Int64)", "(- 0 a)", &[Val::S64(5)]), -5);
        assert_eq!(run::<i64>("(: a Int64)", "(- 0 a)", &[Val::S64(-5)]), 5);
        assert_eq!(run::<i64>("(: a Int64)", "(- 0 a)", &[Val::S64(0)]), 0);
        assert_eq!(
            run::<i64>("(: a Int64)", "(- 0 a)", &[Val::S64(i64::MAX)]),
            -i64::MAX
        );
        // MIN+1 is fine (its negation is MAX); MIN itself traps (−MIN is not representable).
        assert_eq!(
            run::<i64>("(: a Int64)", "(- 0 a)", &[Val::S64(i64::MIN + 1)]),
            i64::MAX
        );
        assert!(
            traps("(: a Int64)", "(- 0 a)", &[Val::S64(i64::MIN)]),
            "negating MIN must trap"
        );
        // A NARROW negation keeps the range-check path — same value/trap behavior at its own width.
        assert_eq!(run::<i32>("(: a Int32)", "(- 0 a)", &[Val::S32(7)]), -7);
        assert!(
            traps("(: a Int32)", "(- 0 a)", &[Val::S32(i32::MIN)]),
            "negating Int32 MIN must trap"
        );
    }

    #[test]
    fn not_over_a_comparison_folds_to_the_complement() {
        // `(not (CMP a b))` is the single complement comparison — no `i32.eqz`. The Lir shape is checked
        // in select.rs; here we confirm the VALUE matches the branchy negation across every comparison,
        // both signednesses, and the equality/inequality pair.
        // `(not (< a b))` == `a >= b`.
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (< a b))",
            &[Val::S64(5), Val::S64(5)]
        ));
        assert!(!run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (< a b))",
            &[Val::S64(4), Val::S64(5)]
        ));
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (< a b))",
            &[Val::S64(6), Val::S64(5)]
        ));
        // `(not (> a b))` == `a <= b`.
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (> a b))",
            &[Val::S64(5), Val::S64(5)]
        ));
        assert!(!run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (> a b))",
            &[Val::S64(6), Val::S64(5)]
        ));
        // `(not (<= a b))` == `a > b`; `(not (>= a b))` == `a < b`.
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (<= a b))",
            &[Val::S64(6), Val::S64(5)]
        ));
        assert!(!run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (<= a b))",
            &[Val::S64(5), Val::S64(5)]
        ));
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (>= a b))",
            &[Val::S64(4), Val::S64(5)]
        ));
        // `(not (= a b))` == `a != b`.
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (= a b))",
            &[Val::S64(4), Val::S64(5)]
        ));
        assert!(!run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (= a b))",
            &[Val::S64(5), Val::S64(5)]
        ));
        // UNSIGNED complement uses the unsigned op: `(not (< a b))` over UInt64 with the high bit set.
        assert!(
            run::<bool>(
                "(: a UInt64) (: b UInt64)",
                "(not (< a b))",
                &[Val::U64(u64::MAX), Val::U64(1)]
            ),
            "unsigned: MAX >= 1 (a signed ge would read MAX as -1 and be false)"
        );
        // NARROW Int32 complement.
        assert!(run::<bool>(
            "(: a Int32) (: b Int32)",
            "(not (< a b))",
            &[Val::S32(-1), Val::S32(-2)]
        ));
        // Composes: double negation is the bare comparison.
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(not (not (< a b)))",
            &[Val::S64(4), Val::S64(5)]
        ));
    }

    #[test]
    fn unsigned_comparison_with_zero_is_simplified() {
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::select::select_function;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let src = format!("(module m (def (f {params}) {body}) (def (main) 0) (export main))");
            let mut db = crate::db::Db::load(crate::testkit::parse(&src));
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // `(< u 0)` unsigned → constant FALSE, no runtime compare.
        let lt0 = lir("(: u UInt64)", "(< u 0)");
        assert!(
            lt0.contains(&Lir::ConstI32(0))
                && !lt0.iter().any(|i| matches!(i, Lir::I64LtU | Lir::I32LtU)),
            "unsigned (< u 0) folds to false, no lt_u; got {lt0:?}"
        );
        // `(>= u 0)` unsigned → constant TRUE.
        let ge0 = lir("(: u UInt64)", "(>= u 0)");
        assert!(
            ge0.contains(&Lir::ConstI32(1))
                && !ge0.iter().any(|i| matches!(i, Lir::I64GeU | Lir::I32GeU)),
            "unsigned (>= u 0) folds to true, no ge_u; got {ge0:?}"
        );
        // `(<= u 0)` unsigned → `u == 0` → `eqz` (no le_u).
        let le0 = lir("(: u UInt64)", "(<= u 0)");
        assert!(
            le0.contains(&Lir::I64Eqz) && !le0.iter().any(|i| matches!(i, Lir::I64LeU)),
            "unsigned (<= u 0) folds to eqz; got {le0:?}"
        );
        // SIGNED must NOT fold — a signed value can be < 0, so `lt_s` stays.
        let s = lir("(: x Int64)", "(< x 0)");
        assert!(
            s.iter().any(|i| matches!(i, Lir::I64LtS)),
            "signed (< x 0) keeps lt_s (not folded); got {s:?}"
        );

        // VALUE PARITY over runtime unsigned inputs (the fold must agree with the true comparison).
        assert!(!run::<bool>("(: u UInt64)", "(< u 0)", &[Val::U64(0)]));
        assert!(!run::<bool>("(: u UInt64)", "(< u 0)", &[Val::U64(5)]));
        assert!(!run::<bool>(
            "(: u UInt64)",
            "(< u 0)",
            &[Val::U64(u64::MAX)]
        ));
        assert!(run::<bool>("(: u UInt64)", "(>= u 0)", &[Val::U64(0)]));
        assert!(run::<bool>(
            "(: u UInt64)",
            "(>= u 0)",
            &[Val::U64(u64::MAX)]
        ));
        assert!(run::<bool>("(: u UInt64)", "(<= u 0)", &[Val::U64(0)]));
        assert!(!run::<bool>("(: u UInt64)", "(<= u 0)", &[Val::U64(1)]));
        // The LEFT-const mirror computes identically.
        assert!(run::<bool>("(: u UInt64)", "(<= 0 u)", &[Val::U64(9)]));
        assert!(!run::<bool>("(: u UInt64)", "(> 0 u)", &[Val::U64(9)]));
    }

    #[test]
    fn a_self_comparison_of_a_scalar_folds_to_a_constant() {
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::select::select_function;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let src = format!("(module m (def (f {params}) {body}) (def (main) 0) (export main))");
            let mut db = crate::db::Db::load(crate::testkit::parse(&src));
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        let no_cmp = |c: &[Lir]| {
            !c.iter().any(|i| {
                matches!(
                    i,
                    Lir::I64LtS | Lir::I64GtS | Lir::I64LeS | Lir::I64GeS | Lir::I64Eq | Lir::I64Ne
                )
            })
        };
        // `x < x` / `x > x` → false; `x <= x` / `x >= x` / `x = x` → true — no runtime compare emitted.
        // (`(let ((y x)) …)` copy-propagates so both operands are the same `x`.)
        for (op, want) in [
            ("<", false),
            (">", false),
            ("<=", true),
            (">=", true),
            ("=", true),
        ] {
            let c = lir("(: x Int64)", &format!("(let ((y x)) ({op} y y))"));
            assert!(
                c.contains(&Lir::ConstI32(want as i32)) && no_cmp(&c),
                "(x {op} x) folds to {want} with no compare; got {c:?}"
            );
        }
        // DISTINCT operands do NOT fold — a real compare stays.
        let distinct = lir("(: a Int64) (: b Int64)", "(< a b)");
        assert!(
            distinct.iter().any(|i| matches!(i, Lir::I64LtS)),
            "(< a b) keeps the compare; got {distinct:?}"
        );
        // A TRAPPING operand is NOT discarded — `(< (/ a b) (/ a b))` keeps the div so b==0 still traps.
        let trapping = lir("(: a Int64) (: b Int64)", "(< (/ a b) (/ a b))");
        assert!(
            trapping.iter().any(|i| matches!(i, Lir::I64DivS)),
            "a trapping self-comparison keeps its operand's div; got {trapping:?}"
        );

        // VALUE PARITY.
        assert!(!run::<bool>(
            "(: x Int64)",
            "(let ((y x)) (< y y))",
            &[Val::S64(7)]
        ));
        assert!(run::<bool>(
            "(: x Int64)",
            "(let ((y x)) (<= y y))",
            &[Val::S64(7)]
        ));
        assert!(run::<bool>(
            "(: x Int64)",
            "(let ((y x)) (= y y))",
            &[Val::S64(-3)]
        ));
        // The trapping form still traps on b==0, and computes false otherwise.
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(< (/ a b) (/ a b))",
            &[Val::S64(1), Val::S64(0)]
        ));
        assert!(!run::<bool>(
            "(: a Int64) (: b Int64)",
            "(< (/ a b) (/ a b))",
            &[Val::S64(10), Val::S64(2)]
        ));
    }

    #[test]
    fn a_comparison_against_a_narrow_types_own_bound_is_simplified() {
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::select::select_function;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let src = format!("(module m (def (f {params}) {body}) (def (main) 0) (export main))");
            let mut db = crate::db::Db::load(crate::testkit::parse(&src));
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        let no_cmp = |c: &[Lir]| {
            !c.iter().any(|i| {
                matches!(
                    i,
                    Lir::I32LtS
                        | Lir::I32GtS
                        | Lir::I32LeS
                        | Lir::I32GeS
                        | Lir::I64LtS
                        | Lir::I64GtS
                        | Lir::I64LeS
                        | Lir::I64GeS
                )
            })
        };
        // Int8 [-128, 127]: `<= 127` / `>= -128` are tautologies; `> 127` / `< -128` are unsatisfiable.
        let le_max = lir("(: x Int8)", "(<= x 127)");
        assert!(
            le_max.contains(&Lir::ConstI32(1)) && no_cmp(&le_max),
            "Int8 (<= x 127) → true; got {le_max:?}"
        );
        let gt_max = lir("(: x Int8)", "(> x 127)");
        assert!(
            gt_max.contains(&Lir::ConstI32(0)) && no_cmp(&gt_max),
            "Int8 (> x 127) → false; got {gt_max:?}"
        );
        let ge_min = lir("(: x Int8)", "(>= x -128)");
        assert!(
            ge_min.contains(&Lir::ConstI32(1)) && no_cmp(&ge_min),
            "Int8 (>= x -128) → true; got {ge_min:?}"
        );
        let lt_min = lir("(: x Int8)", "(< x -128)");
        assert!(
            lt_min.contains(&Lir::ConstI32(0)) && no_cmp(&lt_min),
            "Int8 (< x -128) → false; got {lt_min:?}"
        );
        // `>= max` rewrites to `== max`; a constant INSIDE the range does NOT fold.
        let inside = lir("(: x Int8)", "(< x 127)");
        assert!(
            inside.iter().any(|i| matches!(i, Lir::I32LtS)),
            "Int8 (< x 127) is not a bound → keeps lt_s; got {inside:?}"
        );
        // Full-width Int64 bound folds too.
        let f64max = lir("(: x Int64)", "(<= x 9223372036854775807)");
        assert!(
            f64max.contains(&Lir::ConstI32(1)) && no_cmp(&f64max),
            "Int64 (<= x MAX) → true; got {f64max:?}"
        );

        // VALUE PARITY over runtime narrow inputs — the fold agrees with the true comparison. An `Int8`
        // param takes `Val::S8`, a `UInt8` param `Val::U8`.
        assert!(run::<bool>("(: x Int8)", "(<= x 127)", &[Val::S8(127)]));
        assert!(run::<bool>("(: x Int8)", "(<= x 127)", &[Val::S8(-128)]));
        assert!(!run::<bool>("(: x Int8)", "(> x 127)", &[Val::S8(100)]));
        assert!(run::<bool>("(: x Int8)", "(>= x -128)", &[Val::S8(-128)]));
        assert!(!run::<bool>("(: x Int8)", "(< x -128)", &[Val::S8(-128)]));
        // UInt8 max (255) — the new unsigned-narrow-max coverage.
        assert!(run::<bool>("(: x UInt8)", "(<= x 255)", &[Val::U8(255)]));
        assert!(!run::<bool>("(: x UInt8)", "(> x 255)", &[Val::U8(255)]));
    }

    #[test]
    fn a_comparison_against_a_derived_range_bound_is_simplified() {
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::select::select_function;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let src = format!("(module m (def (f {params}) {body}) (def (main) 0) (export main))");
            let mut db = crate::db::Db::load(crate::testkit::parse(&src));
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        let folded = |c: &[Lir], want: i32| {
            c.contains(&Lir::ConstI32(want))
                && !c
                    .iter()
                    .any(|i| matches!(i, Lir::I64LtS | Lir::I64GtS | Lir::I64LeS | Lir::I64GeS))
        };
        // `(& x 15)` derives the range [0,15] EVEN on a signed Int64 (the mask clears the sign bits), so:
        assert!(
            folded(&lir("(: x Int64)", "(< (& x 15) 20)"), 1),
            "[0,15] < 20 → true"
        );
        assert!(
            folded(&lir("(: x Int64)", "(< (& x 15) 16)"), 1),
            "[0,15] < 16 → true"
        );
        assert!(
            folded(&lir("(: x Int64)", "(>= (& x 15) 0)"), 1),
            "[0,15] >= 0 → true (nonneg mask)"
        );
        assert!(
            folded(&lir("(: x Int64)", "(> (& x 15) 15)"), 0),
            "[0,15] > 15 → false"
        );
        assert!(
            folded(&lir("(: x Int64)", "(<= (& x 15) 15)"), 1),
            "[0,15] <= 15 → true"
        );
        // A constant STRICTLY INSIDE the range is NOT decidable — the compare stays.
        let inside = lir("(: x Int64)", "(< (& x 15) 10)");
        assert!(
            inside.iter().any(|i| matches!(i, Lir::I64LtS)),
            "a constant inside the range keeps the compare; got {inside:?}"
        );

        // VALUE PARITY — the folded results agree with the real comparison, and the not-folded one too.
        assert!(run::<bool>(
            "(: x Int64)",
            "(< (& x 15) 20)",
            &[Val::S64(-1)]
        )); // -1&15=15, 15<20
        assert!(run::<bool>(
            "(: x Int64)",
            "(>= (& x 15) 0)",
            &[Val::S64(-99)]
        )); // masked → nonneg
        assert!(!run::<bool>(
            "(: x Int64)",
            "(> (& x 15) 15)",
            &[Val::S64(255)]
        )); // 255&15=15, not >15
        assert!(run::<bool>(
            "(: x Int64)",
            "(< (& x 15) 10)",
            &[Val::S64(8)]
        )); // 8<10
        assert!(!run::<bool>(
            "(: x Int64)",
            "(< (& x 15) 10)",
            &[Val::S64(12)]
        )); // 12<10 false
    }

    #[test]
    fn if_with_one_zero_branches_materializes_the_boolean() {
        // `(if c 1 0)` is the condition coerced to the result's integer width — `(if c 0 1)` its
        // negation — with NO `select` and NO branch. The emitted shape is checked in select.rs's Lir
        // tests; here we confirm the VALUE is identical to the branchy form across widths and both a
        // comparison condition and a bare bool param.
        // (if (< a b) 1 0) : Int64
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 1 0)",
                &[Val::S64(3), Val::S64(9)]
            ),
            1
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 1 0)",
                &[Val::S64(9), Val::S64(3)]
            ),
            0
        );
        // (if (< a b) 0 1) : the negation.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 0 1)",
                &[Val::S64(3), Val::S64(9)]
            ),
            0
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 0 1)",
                &[Val::S64(9), Val::S64(3)]
            ),
            1
        );
        // A bare BOOL param condition (not a comparison) — `(if p 1 0)` = p as an int.
        assert_eq!(
            run::<i64>("(: p Bool)", "(if p 1 0)", &[Val::Bool(true)]),
            1
        );
        assert_eq!(
            run::<i64>("(: p Bool)", "(if p 1 0)", &[Val::Bool(false)]),
            0
        );
        assert_eq!(
            run::<i64>("(: p Bool)", "(if p 0 1)", &[Val::Bool(true)]),
            0
        );
        // A narrow Int32-operand comparison still materializes to the correct 0/1.
        assert_eq!(
            run::<i64>(
                "(: a Int32) (: b Int32)",
                "(if (< a b) 1 0)",
                &[Val::S32(1), Val::S32(2)]
            ),
            1
        );
        // A NON-0/1 constant pair keeps the ordinary select — value parity holds there too.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 5 7)",
                &[Val::S64(1), Val::S64(2)]
            ),
            5
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (< a b) 5 7)",
                &[Val::S64(2), Val::S64(1)]
            ),
            7
        );
    }

    #[test]
    fn if_not_comparison_one_zero_computes_the_negated_predicate() {
        // `(if (not (CMP a b)) 1 0)` — a negated comparison materialized as an int. `lower` branch-swaps
        // to `(if (CMP a b) 0 1)` and the backend folds the negation into the complement comparison (no
        // `eqz ; eqz`). Confirm the VALUE equals the negated predicate for every comparison.
        // (not (< a b)) == a >= b.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (not (< a b)) 1 0)",
                &[Val::S64(5), Val::S64(5)]
            ),
            1
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (not (< a b)) 1 0)",
                &[Val::S64(4), Val::S64(5)]
            ),
            0
        );
        // (not (= n 0)) == n is nonzero — the "eqz;eqz" case that now folds to a single ne.
        assert_eq!(
            run::<i64>("(: n Int64)", "(if (not (= n 0)) 1 0)", &[Val::S64(0)]),
            0
        );
        assert_eq!(
            run::<i64>("(: n Int64)", "(if (not (= n 0)) 1 0)", &[Val::S64(7)]),
            1
        );
        assert_eq!(
            run::<i64>("(: n Int64)", "(if (not (= n 0)) 1 0)", &[Val::S64(-3)]),
            1
        );
        // (not (= a b)) == a != b.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(if (not (= a b)) 1 0)",
                &[Val::S64(5), Val::S64(5)]
            ),
            0
        );
        // Unsigned complement stays unsigned.
        assert_eq!(
            run::<i64>(
                "(: a UInt64) (: b UInt64)",
                "(if (not (< a b)) 1 0)",
                &[Val::U64(u64::MAX), Val::U64(1)]
            ),
            1
        );
        // A bare bool param `(if (not p) 1 0)` — no comparison to fold, still correct via eqz.
        assert_eq!(
            run::<i64>("(: p Bool)", "(if (not p) 1 0)", &[Val::Bool(true)]),
            0
        );
        assert_eq!(
            run::<i64>("(: p Bool)", "(if (not p) 1 0)", &[Val::Bool(false)]),
            1
        );
    }

    // ── bitwise: total, never trap; width-agnostic on a normalized value ─────────────────────────

    #[test]
    fn runtime_bitwise_over_int64() {
        // & | ^ over two runtime i64 operands — the LEB128/section-encoding arithmetic a self-hosted
        // compiler runs on values computed at run time.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(& a b)",
                &[Val::S64(255), Val::S64(127)]
            ),
            127
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(| a b)",
                &[Val::S64(42), Val::S64(128)]
            ),
            170
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(^ a b)",
                &[Val::S64(12), Val::S64(10)]
            ),
            6
        );
    }

    // ── division / remainder: signed + unsigned, native traps on ÷0 and MIN/-1 ────────────────────

    #[test]
    fn runtime_signed_division_truncates_toward_zero() {
        // i64.div_s truncates toward zero (not floor): 7/2=3, -7/2=-3. Remainder takes the dividend's
        // sign: -7%2=-1.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(/ a b)",
                &[Val::S64(7), Val::S64(2)]
            ),
            3
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(/ a b)",
                &[Val::S64(-7), Val::S64(2)]
            ),
            -3
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(% a b)",
                &[Val::S64(-7), Val::S64(2)]
            ),
            -1
        );
    }

    #[test]
    fn runtime_division_traps_on_zero_and_overflow() {
        // ÷0 traps (i64.div_s native); MIN/-1 overflows and traps (i64.div_s native). Remainder MIN%-1
        // does NOT overflow — i64.rem_s yields 0 (it forms no quotient).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(/ a b)",
            &[Val::S64(5), Val::S64(0)]
        ));
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(/ a b)",
            &[Val::S64(i64::MIN), Val::S64(-1)]
        ));
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(% a b)",
                &[Val::S64(i64::MIN), Val::S64(-1)]
            ),
            0
        );
    }

    #[test]
    fn runtime_unsigned_division_is_magnitude() {
        // UInt64 division is unsigned: UInt64.max / 2 = (2^64-1)/2 = 2^63-1, a value a SIGNED div_s would
        // get wrong (it would read the operand as -1). Pins div_u selection off the unsigned type.
        assert_eq!(
            run::<u64>(
                "(: a UInt64) (: b UInt64)",
                "(/ a b)",
                &[Val::U64(u64::MAX), Val::U64(2)]
            ),
            u64::MAX / 2
        );
        assert!(traps(
            "(: a UInt64) (: b UInt64)",
            "(/ a b)",
            &[Val::U64(5), Val::U64(0)]
        ));
    }

    /// STRENGTH REDUCTION: an UNSIGNED `/`/`%` by a constant POWER OF TWO lowers to a shift/mask instead
    /// of the slow hardware `div_u`/`rem_u`. `(/ a 4)` → `a >>ᵤ 2`, `(% a 4)` → `a & 3`. Only unsigned
    /// (a signed divide rounds toward zero, unlike an arithmetic shift) and only a power-of-two constant
    /// (a non-power keeps the divide, a signed operand keeps `div_s`). Verified at the Lir level (no
    /// `div_u`/`rem_u` survives; the shift/mask is present) and by value.
    #[test]
    fn unsigned_div_rem_by_a_power_of_two_strength_reduces() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // (/ a 4) UInt64 → shr_u (no div_u).
        let divc = lir("(: a UInt64)", "(/ a 4)");
        assert!(
            divc.contains(&Lir::I64ShrU) && !divc.contains(&Lir::I64DivU),
            "unsigned /2^k must be a shr_u, not div_u; got: {divc:?}"
        );
        // (% a 4) UInt64 → and (no rem_u).
        let remc = lir("(: a UInt64)", "(% a 4)");
        assert!(
            remc.contains(&Lir::I64And) && !remc.contains(&Lir::I64RemU),
            "unsigned %2^k must be an and-mask, not rem_u; got: {remc:?}"
        );
        // A non-power-of-two divisor keeps div_u; a SIGNED power-of-two divide is ALSO reduced (the
        // round-toward-zero bias sequence), so no div_s either.
        assert!(lir("(: a UInt64)", "(/ a 3)").contains(&Lir::I64DivU));
        assert!(
            !lir("(: a Int64)", "(/ a 4)").contains(&Lir::I64DivS),
            "a signed /2^k is strength-reduced (bias + shift), no div_s"
        );
        assert!(
            !lir("(: a Int64)", "(% a 4)").contains(&Lir::I64RemS),
            "a signed %2^k is strength-reduced (n − (q<<k)), no rem_s"
        );
        // A non-power-of-two signed divide keeps div_s.
        assert!(lir("(: a Int64)", "(/ a 3)").contains(&Lir::I64DivS));

        // Value parity: the shift/mask computes the same unsigned quotient/remainder as the divide.
        assert_eq!(run::<u64>("(: a UInt64)", "(/ a 4)", &[Val::U64(17)]), 4);
        assert_eq!(run::<u64>("(: a UInt64)", "(% a 4)", &[Val::U64(17)]), 1);
        assert_eq!(
            run::<u64>("(: a UInt64)", "(/ a 2)", &[Val::U64(u64::MAX)]),
            u64::MAX / 2,
            "the shift is UNSIGNED — the top bit is not treated as a sign"
        );
        assert_eq!(run::<u32>("(: a UInt32)", "(/ a 8)", &[Val::U32(100)]), 12);
        assert_eq!(run::<u32>("(: a UInt32)", "(% a 8)", &[Val::U32(100)]), 4);

        // SIGNED value parity — the bias sequence must truncate toward ZERO like `div_s`/`rem_s`, and
        // agree for negatives (where a bare arithmetic shift would round toward −∞). `-17/4 = -4` (not
        // -5), `-17%4 = -1`; positives and exact multiples too.
        assert_eq!(run::<i64>("(: a Int64)", "(/ a 4)", &[Val::S64(17)]), 4);
        assert_eq!(run::<i64>("(: a Int64)", "(/ a 4)", &[Val::S64(-17)]), -4);
        assert_eq!(run::<i64>("(: a Int64)", "(% a 4)", &[Val::S64(17)]), 1);
        assert_eq!(run::<i64>("(: a Int64)", "(% a 4)", &[Val::S64(-17)]), -1);
        assert_eq!(run::<i64>("(: a Int64)", "(/ a 8)", &[Val::S64(-8)]), -1);
        assert_eq!(run::<i64>("(: a Int64)", "(% a 8)", &[Val::S64(-8)]), 0);
        assert_eq!(
            run::<i64>("(: a Int64)", "(/ a 2)", &[Val::S64(i64::MIN)]),
            i64::MIN / 2,
            "the bias sequence is exact at the signed MIN boundary"
        );
        assert_eq!(run::<i32>("(: a Int32)", "(/ a 8)", &[Val::S32(-100)]), -12);
        assert_eq!(run::<i32>("(: a Int32)", "(% a 8)", &[Val::S32(-100)]), -4);
    }

    #[test]
    fn divisibility_by_a_power_of_two_becomes_a_mask_test() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // `(= (% x 2) 0)` (the even test) → `x & 1 ; eqz` — NO div/rem, no bias sequence.
        let even = lir("(: x Int64)", "(= (% x 2) 0)");
        assert!(
            even.contains(&Lir::I64And)
                && even.contains(&Lir::I64Eqz)
                && !even
                    .iter()
                    .any(|i| matches!(i, Lir::I64RemS | Lir::I64DivS)),
            "even test is a mask+eqz, no rem/div; got {even:?}"
        );
        // A non-power-of-two divisor keeps the `%` (rem_s).
        let three = lir("(: x Int64)", "(= (% x 3) 0)");
        assert!(
            three.iter().any(|i| matches!(i, Lir::I64RemS)),
            "(= (% x 3) 0) keeps rem_s; got {three:?}"
        );

        // VALUE PARITY — the mask test must agree with the true `%`-then-`==0`, ESPECIALLY for negatives
        // (signed `%` is sign-of-dividend, but divisibility is sign-agnostic).
        for (x, k, div) in [
            (8i64, 2i64, true),
            (7, 2, false),
            (-8, 2, true),
            (-7, 2, false),
            (0, 2, true),
            (-1, 2, false),
            (16, 8, true),
            (-16, 8, true),
            (-12, 8, false),
            (i64::MIN, 2, true), // MIN is even; the mask test must not choke where signed `%` bias would
        ] {
            assert_eq!(
                run::<bool>("(: x Int64)", &format!("(= (% x {k}) 0)"), &[Val::S64(x)]),
                div,
                "({x} % {k} == 0) should be {div}"
            );
        }
        // Unsigned divisibility too.
        assert!(run::<bool>(
            "(: u UInt64)",
            "(= (% u 4) 0)",
            &[Val::U64(16)]
        ));
        assert!(!run::<bool>(
            "(: u UInt64)",
            "(= (% u 4) 0)",
            &[Val::U64(17)]
        ));
    }

    // ── shifts: count guarded to [0,N); << checked for overflow; >> arithmetic/logical by sign ─────

    #[test]
    fn runtime_left_shift_multiplies_and_traps_on_overflow() {
        // << is exact ×2^count: 1<<7=128. An overflowing shift TRAPS (not a silent wrap): 2^62 << 2 = 2^64
        // overflows Int64. An out-of-range count (≥64, or negative) traps rather than masking.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(<< a b)",
                &[Val::S64(1), Val::S64(7)]
            ),
            128
        );
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(<< a b)",
            &[Val::S64(1i64 << 62), Val::S64(2)]
        ));
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(<< a b)",
            &[Val::S64(1), Val::S64(64)]
        ));
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(<< a b)",
            &[Val::S64(1), Val::S64(-1)]
        ));
        // count = 63, the largest in-range count and the Int64 edge: -1<<63 is exactly Int64.min (in
        // range, must NOT trap), while 1<<63 is +2^63 (out of range, must trap). A folder that builds
        // the 2^count factor with a signed `1<<63` = i64::MIN would get both backwards.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(<< a b)",
                &[Val::S64(-1), Val::S64(63)]
            ),
            i64::MIN
        );
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(<< a b)",
            &[Val::S64(1), Val::S64(63)]
        ));
    }

    #[test]
    fn constant_count_shift_folds_the_count_guard() {
        // A shift by a COMPILE-TIME-CONSTANT count folds the runtime `count >= width` guard (the
        // condition is decided at compile time). It must behave IDENTICALLY to the runtime-count form:
        //   - a valid count (0 <= k < N): the guard is elided but the << overflow round-trip stays, so
        //     the value computes and an overflowing shift still traps;
        //   - an out-of-range constant count (k >= N or negative): the shift ALWAYS traps (a bare
        //     `unreachable`), exactly as the runtime guard would.
        // `a` is a runtime param so the shift is not fully folded; only the COUNT is constant.
        assert_eq!(run::<i64>("(: a Int64)", "(<< a 7)", &[Val::S64(1)]), 128);
        assert_eq!(run::<i64>("(: a Int64)", "(<< a 3)", &[Val::S64(5)]), 40);
        assert_eq!(run::<i64>("(: a Int64)", "(>> a 2)", &[Val::S64(40)]), 10);
        // -1 << 63 = Int64.min (in range, computes); 1 << 63 = +2^63 (overflow, traps) — same edge the
        // runtime test pins, now on the constant-count path.
        assert_eq!(
            run::<i64>("(: a Int64)", "(<< a 63)", &[Val::S64(-1)]),
            i64::MIN
        );
        assert!(traps("(: a Int64)", "(<< a 63)", &[Val::S64(1)]));
        // Overflow with a small constant count still traps (round-trip guard survives the elision).
        assert!(traps("(: a Int64)", "(<< a 2)", &[Val::S64(1i64 << 62)]));
        // Out-of-range constant count → unconditional trap (the shift can never be valid).
        assert!(traps("(: a Int64)", "(<< a 64)", &[Val::S64(1)]));
        assert!(traps("(: a Int64)", "(<< a 100)", &[Val::S64(0)]));
        assert!(traps("(: a Int64)", "(<< a -1)", &[Val::S64(1)]));
    }

    #[test]
    fn runtime_shift_with_a_nested_value_operand() {
        // `(<< (+ a b) c)` — the shift's VALUE operand is a nested checked add, so `emit_operand_into`
        // routes it through `emit_checked_arith_to` writing the shift's value slot directly. Both the
        // add's overflow guard and the shift's count/overflow guards must still fire. (1+2)<<3 = 24.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64) (: c Int64)",
                "(<< (+ a b) c)",
                &[Val::S64(1), Val::S64(2), Val::S64(3)]
            ),
            24
        );
        // The inner add overflows before the shift runs → trap.
        assert!(traps(
            "(: a Int64) (: b Int64) (: c Int64)",
            "(<< (+ a b) c)",
            &[Val::S64(i64::MAX), Val::S64(1), Val::S64(0)]
        ));
        // The shift overflows Int64 (1 << 63 = +2^63, out of range) → trap.
        assert!(traps(
            "(: a Int64) (: b Int64) (: c Int64)",
            "(<< (+ a b) c)",
            &[Val::S64(1), Val::S64(0), Val::S64(63)]
        ));
    }

    #[test]
    fn runtime_signed_right_shift_is_arithmetic() {
        // >> on a signed type is ARITHMETIC (sign-extending): -256 >> 7 = -2 (a logical shift would give a
        // huge positive value). An out-of-range count traps.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(>> a b)",
                &[Val::S64(256), Val::S64(7)]
            ),
            2
        );
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(>> a b)",
                &[Val::S64(-256), Val::S64(7)]
            ),
            -2
        );
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(>> a b)",
            &[Val::S64(256), Val::S64(64)]
        ));
    }

    #[test]
    fn runtime_unsigned_right_shift_is_logical() {
        // >> on an UNSIGNED type is LOGICAL (zero-filling): UInt64.max >> 1 = 2^63-1. A signed shr_s would
        // sign-extend (reading the operand as -1) and answer -1 = UInt64.max — the sign selects the shift.
        assert_eq!(
            run::<u64>(
                "(: a UInt64) (: b UInt64)",
                "(>> a b)",
                &[Val::U64(u64::MAX), Val::U64(1)]
            ),
            u64::MAX >> 1
        );
    }

    // ── unsigned checked +/-/*: the unsigned overflow guards (carry/borrow) ───────────────────────

    #[test]
    fn runtime_unsigned_addition_traps_on_carry() {
        // UInt64 addition wraps in wasm; the guard `r <ᵤ a` traps on carry-out. In range it computes.
        assert_eq!(
            run::<u64>(
                "(: a UInt64) (: b UInt64)",
                "(+ a b)",
                &[Val::U64(20), Val::U64(22)]
            ),
            42
        );
        assert!(traps(
            "(: a UInt64) (: b UInt64)",
            "(+ a b)",
            &[Val::U64(u64::MAX), Val::U64(1)]
        ));
    }

    #[test]
    fn runtime_unsigned_subtraction_traps_below_zero() {
        // An unsigned value cannot go below 0: 0 - 1 traps (the guard `a <ᵤ b`), it does NOT wrap to MAX.
        assert_eq!(
            run::<u64>(
                "(: a UInt64) (: b UInt64)",
                "(- a b)",
                &[Val::U64(10), Val::U64(3)]
            ),
            7
        );
        assert!(traps(
            "(: a UInt64) (: b UInt64)",
            "(- a b)",
            &[Val::U64(0), Val::U64(1)]
        ));
    }

    #[test]
    fn runtime_unsigned_multiplication_traps_on_overflow() {
        // UInt64 mul guard `a≠0 && r/ᵤa≠b`. In range computes; a product past 2^64 traps.
        assert_eq!(
            run::<u64>(
                "(: a UInt64) (: b UInt64)",
                "(* a b)",
                &[Val::U64(6), Val::U64(7)]
            ),
            42
        );
        assert!(traps(
            "(: a UInt64) (: b UInt64)",
            "(* a b)",
            &[Val::U64(u64::MAX), Val::U64(2)]
        ));
    }

    // ── unsigned comparison: _u selection — the dual of the signed-ordering case ──────────────────

    #[test]
    fn runtime_unsigned_comparison_orders_by_magnitude() {
        // (< a b) on UInt64: 0 < UInt64.max is true under the UNSIGNED order (i64.lt_u). Read as SIGNED,
        // UInt64.max's bit pattern is -1, which a lt_s would rank BELOW 0 and answer false. Pins _u
        // selection off the unsigned type — the dual of the signed `(< Int64.min Int64.max)` case.
        assert!(run::<bool>(
            "(: a UInt64) (: b UInt64)",
            "(< a b)",
            &[Val::U64(0), Val::U64(u64::MAX)]
        ));
        // And the signed operand stays signed: -1 < 1 is true (a lt_u would read -1 as huge → false).
        assert!(run::<bool>(
            "(: a Int64) (: b Int64)",
            "(< a b)",
            &[Val::S64(-1), Val::S64(1)]
        ));
    }

    // ── narrow widths (≤32-bit): compute in i32, range-check back to the N-bit type ───────────────
    //
    // An aliased narrow width crosses the boundary as its FAITHFUL component primitive — Int8 as `s8`,
    // UInt8 as `u8` — so args/results are `Val::S8`/`Val::U8` and `i8`/`u8` (not the machine-slot s32).
    // wasmtime enforces the argument is a valid s8/u8 at the edge, and the ABI lifts/lowers to the i32
    // slot the emitted code computes in — the range-checks keep the core result in the N-bit range.

    #[test]
    fn runtime_narrow_signed_addition_range_checks() {
        // Int8 addition: 100+27=127 fits (Int8.max), 100+28=128 does NOT — the i32 add is exact, the
        // range-check to [-128,127] traps. A wrap would give -128 (the classic signed-overflow bug).
        assert_eq!(
            run::<i8>(
                "(: a Int8) (: b Int8)",
                "(+ a b)",
                &[Val::S8(100), Val::S8(27)]
            ),
            127
        );
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(+ a b)",
            &[Val::S8(100), Val::S8(28)]
        ));
    }

    #[test]
    fn runtime_narrow_signed_subtraction_range_checks_both_edges() {
        // Int8 subtraction relies ENTIRELY on the range-check (a narrow i32 sub cannot overflow the
        // slot): 100-(-50)=150 > Int8.max(127) traps the UPPER edge; -100-50=-150 < Int8.min(-128) traps
        // the LOWER edge; -100-27=-127 fits. Pins that both narrow-sub overflow directions trap (the
        // add case only exercises the upper edge).
        assert_eq!(
            run::<i8>(
                "(: a Int8) (: b Int8)",
                "(- a b)",
                &[Val::S8(-100), Val::S8(27)]
            ),
            -127
        );
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(- a b)",
            &[Val::S8(100), Val::S8(-50)]
        ));
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(- a b)",
            &[Val::S8(-100), Val::S8(50)]
        ));
    }

    /// A NARROW signed `+`/`-` by a compile-time CONSTANT drops the provably-unreachable range-check
    /// bound: the exact result moves in ONE direction from an in-range operand, so only that bound can
    /// be exceeded. `(+ a 1)` Int8 can only exceed `max` (127) — the `r < min` check is dead; `(- a 1)`
    /// can only fall below `min` (-128) — the `r > max` check is dead. The surviving check must still
    /// trap at the exact edge, and dropping the dead one must never mask a real overflow or admit a wrap.
    /// (The general two-runtime-operand case keeps both bounds — see the tests above.)
    #[test]
    fn a_narrow_constant_operand_addsub_drops_the_dead_range_bound() {
        // (+ a 1) Int8: 126→127 fits; 127→128 traps the UPPER edge; MIN(-128)+1=-127 must NOT trap
        // (the dropped lower check would have been the only risk of a false trap here).
        assert_eq!(run::<i8>("(: a Int8)", "(+ a 1)", &[Val::S8(126)]), 127);
        assert!(traps("(: a Int8)", "(+ a 1)", &[Val::S8(127)]));
        assert_eq!(
            run::<i8>("(: a Int8)", "(+ a 1)", &[Val::S8(-128)]),
            -127,
            "adding 1 to Int8.min stays in range — the dropped lower-bound check must not trap"
        );
        // (- a 1) Int8: -127→-128 fits; -128→-129 traps the LOWER edge; MAX(127)-1=126 must NOT trap.
        assert_eq!(run::<i8>("(: a Int8)", "(- a 1)", &[Val::S8(-127)]), -128);
        assert!(traps("(: a Int8)", "(- a 1)", &[Val::S8(-128)]));
        assert_eq!(
            run::<i8>("(: a Int8)", "(- a 1)", &[Val::S8(127)]),
            126,
            "subtracting 1 from Int8.max stays in range — the dropped upper-bound check must not trap"
        );
        // A negative constant flips the direction: (+ a -1) moves DOWN → traps only the LOWER edge.
        assert!(traps("(: a Int8)", "(+ a -1)", &[Val::S8(-128)]));
        assert_eq!(run::<i8>("(: a Int8)", "(+ a -1)", &[Val::S8(127)]), 126);
    }

    #[test]
    fn a_narrow_binary_op_with_a_constant_left_operand_emits_valid_wasm() {
        // ⚠ INVALID WASM regression: a narrow binary op whose LEFT operand is a bare integer literal and
        // whose right is a narrow-typed variable mis-emitted the literal at its i64 default beside the i32
        // variable ("expected i64, found i32"). ROOT: `unify_width`/`unify_sign` BOUND the operator's
        // shared width/sign VARIABLE to the deferred LEFT literal first, freezing it, so the RIGHT
        // variable's `Fixed(8)`/unsigned meeting the now-`Deferred` var was a no-op → the result ground to
        // the i64 default. Fixed by leaving a var meeting `Deferred` UNBOUND, so the concrete operand
        // (whichever side) binds it — operand order no longer changes the result width/sign. The
        // COMPARISON path (result Bool, operands not linked via a result var) additionally reconciles at
        // emit (`operand_int_ty`, position-independent). All hold for `n : UInt8`:
        assert_eq!(run::<u8>("(: n UInt8)", "(+ 1 n)", &[Val::U8(5)]), 6); // was invalid wasm
        assert_eq!(run::<u8>("(: n UInt8)", "(* 2 n)", &[Val::U8(3)]), 6);
        assert_eq!(run::<u8>("(: n UInt8)", "(& 15 n)", &[Val::U8(9)]), 9);
        assert_eq!(run::<u8>("(: n UInt8)", "(<< 1 n)", &[Val::U8(3)]), 8);
        assert_eq!(
            run::<i64>("(: n UInt8)", "(if (< 1 n) 1 0)", &[Val::U8(5)]),
            1
        );
        // The const-RIGHT mirror (always worked) and a both-var form still compute the same.
        assert_eq!(run::<u8>("(: n UInt8)", "(+ n 1)", &[Val::U8(5)]), 6);
        assert_eq!(
            run::<u8>(
                "(: a UInt8) (: b UInt8)",
                "(+ a b)",
                &[Val::U8(100), Val::U8(50)]
            ),
            150
        );
        // A wide left-const is unaffected (i64 result was always correct).
        assert_eq!(run::<i64>("(: n Int64)", "(+ 1 n)", &[Val::S64(41)]), 42);
        // (A genuine width conflict — UInt8 + Int64 — still rejects CDZ0301; the corpus guards that, and
        // the deferred-var change only affects a var meeting a DEFERRED width, never two Fixed widths.)
    }

    #[test]
    fn runtime_narrow_unsigned_addition_and_subtraction() {
        // UInt8: 200+55=255 fits, 200+56=256 traps (range-check to [0,255]); 0-1 traps below zero.
        assert_eq!(
            run::<u8>(
                "(: a UInt8) (: b UInt8)",
                "(+ a b)",
                &[Val::U8(200), Val::U8(55)]
            ),
            255
        );
        assert!(traps(
            "(: a UInt8) (: b UInt8)",
            "(+ a b)",
            &[Val::U8(200), Val::U8(56)]
        ));
        assert!(traps(
            "(: a UInt8) (: b UInt8)",
            "(- a b)",
            &[Val::U8(0), Val::U8(1)]
        ));
    }

    #[test]
    fn unsigned_narrow_arith_uses_a_single_unsigned_range_check() {
        // The narrow-width range-check for an UNSIGNED result is a SINGLE unsigned upper-bound guard
        // (`r >=ᵤ 2^N → trap`), not the two signed guards (`r <ₛ 0`, `r >ₛ max`) a signed width uses:
        // an unsigned result is never negative, and a single unsigned compare covers the only way it can
        // leave the type (exceeding `2^N-1`). So a UInt8 `+` emits exactly ONE `i32.ge_u` against 256 and
        // ZERO `lt_s`/`gt_s` in its range-check; the SIGNED counterpart keeps both signed guards. This
        // pins the elision at the Lir level (the behavioral trap parity is covered by the narrow-arith
        // run tests above/below).
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let select = |src: &str, name: &str| {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name(name).expect("def");
            let sig = db.defs[d].params.clone();
            let params: Vec<_> = sig
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let u = select(
            "(module m (def (f (: a UInt8) (: b UInt8)) (+ a b)) (def (main) 0) (export main))",
            "f",
        );
        assert_eq!(
            u.iter().filter(|i| matches!(i, Lir::I32GeU)).count(),
            1,
            "unsigned narrow add has one unsigned upper-bound guard, got: {u:?}"
        );
        assert!(
            !u.iter().any(|i| matches!(i, Lir::I32LtS | Lir::I32GtS)),
            "unsigned narrow add has NO signed range guards, got: {u:?}"
        );
        // Signed keeps both signed guards (lower `lt_s` + upper `gt_s`), no unsigned range guard.
        let s = select(
            "(module m (def (f (: a Int8) (: b Int8)) (+ a b)) (def (main) 0) (export main))",
            "f",
        );
        assert!(
            s.iter().any(|i| matches!(i, Lir::I32LtS))
                && s.iter().any(|i| matches!(i, Lir::I32GtS)),
            "signed narrow add keeps both signed range guards, got: {s:?}"
        );
    }

    #[test]
    fn runtime_narrow_multiplication_range_checks() {
        // UInt8 mul: 15*17=255 fits; 16*16=256 traps (the i32 product is exact, range-check to 255 traps).
        assert_eq!(
            run::<u8>(
                "(: a UInt8) (: b UInt8)",
                "(* a b)",
                &[Val::U8(15), Val::U8(17)]
            ),
            255
        );
        assert!(traps(
            "(: a UInt8) (: b UInt8)",
            "(* a b)",
            &[Val::U8(16), Val::U8(16)]
        ));
    }

    #[test]
    fn runtime_narrow_param_with_a_bare_literal_grounds_the_literal_to_the_param_width() {
        // A NARROW parameter combined with a BARE LITERAL: the literal (width-polymorphic, Int64 on its
        // own = an i64 slot) must take the parameter's narrow width, so the emitted op is a homogeneous
        // i32 op — not an i32/i64 mix wasm rejects. Regression for the narrow-param-plus-literal
        // MISCOMPILE (invalid component). Covers `+`, comparison, `*`, and bitwise `&`.
        assert_eq!(run::<u8>("(: x UInt8)", "(+ x 1)", &[Val::U8(100)]), 101);
        assert_eq!(run::<i8>("(: x Int8)", "(+ x 1)", &[Val::S8(50)]), 51);
        assert_eq!(run::<u8>("(: x UInt8)", "(* x 2)", &[Val::U8(10)]), 20);
        assert_eq!(run::<u8>("(: x UInt8)", "(& x 15)", &[Val::U8(255)]), 15);
        assert!(run::<bool>("(: x UInt8)", "(> x 50)", &[Val::U8(100)]));
        // Overflow of the narrow width still traps — grounding the literal does not disable the
        // range-check: UInt8 255 + 1 = 256 leaves [0,255].
        assert!(traps("(: x UInt8)", "(+ x 1)", &[Val::U8(255)]));
    }

    #[test]
    fn runtime_if_branch_bare_literal_grounds_to_the_narrow_result_width() {
        // An `if` whose branches MIX a narrow value and a bare literal: the literal branch (Int64 on its
        // own = i64 slot) must take the `if`'s narrow result width, so both branches leave the same i32
        // slot — not an i32/i64 mismatch wasm rejects. Regression for the if-branch-narrow-literal
        // MISCOMPILE. Both branch orders, signed and unsigned.
        assert_eq!(
            run::<u8>(
                "(: x UInt8) (: c Bool)",
                "(if c x 0)",
                &[Val::U8(200), Val::Bool(true)]
            ),
            200
        );
        assert_eq!(
            run::<i8>(
                "(: x Int8) (: c Bool)",
                "(if c x 0)",
                &[Val::S8(50), Val::Bool(true)]
            ),
            50
        );
        // Order-independent: the bare literal in the THEN branch, narrow in the ELSE.
        assert_eq!(
            run::<u8>(
                "(: x UInt8) (: c Bool)",
                "(if c 0 x)",
                &[Val::U8(200), Val::Bool(false)]
            ),
            200
        );
    }

    #[test]
    fn runtime_narrow_signed_division_min_over_minus_one_traps() {
        // Int8 division: -128 / -1 = 128 overflows Int8 (max 127). The i32 div_s does NOT trap here (128
        // fits i32), so the range-check catches it — the narrow-signed-division overflow the machine op
        // misses. In range: -100/3 = -33 (truncates toward zero).
        assert_eq!(
            run::<i8>(
                "(: a Int8) (: b Int8)",
                "(/ a b)",
                &[Val::S8(-100), Val::S8(3)]
            ),
            -33
        );
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(/ a b)",
            &[Val::S8(-128), Val::S8(-1)]
        ));
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(/ a b)",
            &[Val::S8(5), Val::S8(0)]
        ));
    }

    /// A narrow signed division's range-check needs ONLY its upper bound: a signed quotient can overflow
    /// the type solely UPWARD (`MIN/-1 = 2^(N-1) > max`). It can never fall below `min` — `|q| = |a|/|b|
    /// <= |a| <= 2^(N-1)`, so the most-negative reachable quotient is `-2^(N-1) = MIN` itself (in range,
    /// `MIN / 1 = MIN`). So the `r < min` half is dead and dropped. This test pins BOTH that the upper
    /// overflow still traps AND that the MIN quotient (the edge the dropped lower check would have
    /// guarded) does NOT spuriously trap.
    #[test]
    fn a_narrow_signed_division_range_check_is_upper_bound_only() {
        // MIN / 1 = MIN — the exact lower edge. It must compute (not trap): the dropped `r < min` check
        // was provably unreachable, so removing it must not have removed a real guard.
        assert_eq!(
            run::<i8>(
                "(: a Int8) (: b Int8)",
                "(/ a b)",
                &[Val::S8(-128), Val::S8(1)]
            ),
            -128
        );
        assert_eq!(
            run::<i16>(
                "(: a Int16) (: b Int16)",
                "(/ a b)",
                &[Val::S16(-32768), Val::S16(1)]
            ),
            -32768
        );
        // A large-magnitude negative quotient (still >= MIN) computes.
        assert_eq!(
            run::<i8>(
                "(: a Int8) (: b Int8)",
                "(/ a b)",
                &[Val::S8(-128), Val::S8(2)]
            ),
            -64
        );
        // The one real overflow (MIN / -1 → +2^(N-1), above max) still traps — the upper bound stands.
        assert!(traps(
            "(: a Int8) (: b Int8)",
            "(/ a b)",
            &[Val::S8(-128), Val::S8(-1)]
        ));
        assert!(traps(
            "(: a Int16) (: b Int16)",
            "(/ a b)",
            &[Val::S16(-32768), Val::S16(-1)]
        ));
        // The Lir has ONE range compare (the upper `gt_s`), not two — the dead lower `lt_s` is gone.
        let code = {
            use crate::db::Db;
            let ast = crate::testkit::parse(
                "(module m (def (f (: a Int8) (: b Int8)) (/ a b)) (def (main) 0) (export main))",
            );
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        use crate::backend::wasm::lir::Lir;
        assert!(
            code.contains(&Lir::I32GtS) && !code.contains(&Lir::I32LtS),
            "narrow signed div keeps only the upper (gt_s) range bound, got: {code:?}"
        );
    }

    #[test]
    fn runtime_narrow_left_shift_range_checks() {
        // UInt8 shift: 1<<7=128 fits, 1<<8 traps (count ≥ N=8). And 3<<7=384 overflows UInt8 (the count
        // is in range but the result exceeds 255) → the range-check traps. Pins the count bound is N (8),
        // not the slot width (32), and that a narrow << is range-checked.
        assert_eq!(
            run::<u8>(
                "(: a UInt8) (: b UInt8)",
                "(<< a b)",
                &[Val::U8(1), Val::U8(7)]
            ),
            128
        );
        assert!(traps(
            "(: a UInt8) (: b UInt8)",
            "(<< a b)",
            &[Val::U8(1), Val::U8(8)]
        ));
        assert!(traps(
            "(: a UInt8) (: b UInt8)",
            "(<< a b)",
            &[Val::U8(3), Val::U8(7)]
        ));
    }

    #[test]
    fn runtime_narrow_unsigned_right_shift_is_logical() {
        // UInt8.max >> 1 = 127 (logical, zero-fill). Pins narrow shr_u — the op selection follows the
        // unsigned type.
        assert_eq!(
            run::<u8>(
                "(: a UInt8) (: b UInt8)",
                "(>> a b)",
                &[Val::U8(255), Val::U8(1)]
            ),
            127
        );
    }

    #[test]
    fn runtime_int32_full_width_addition_traps() {
        // Int32 is a FULL i32-slot width (N == slot bits): the machine i32.add carry/borrow guard is what
        // traps, not a range-check. Int32.max + 1 traps; in range computes.
        assert_eq!(
            run::<i32>(
                "(: a Int32) (: b Int32)",
                "(+ a b)",
                &[Val::S32(20), Val::S32(22)]
            ),
            42
        );
        assert!(traps(
            "(: a Int32) (: b Int32)",
            "(+ a b)",
            &[Val::S32(i32::MAX), Val::S32(1)]
        ));
    }

    // ── runtime `wrap` (R3): the emitted mask-and-reinterpret over a runtime operand ──────────────
    //
    // With a RUNTIME source (a parameter), `wrap` cannot fold — it emits `Core::Convert`, a slot move
    // (extend/wrap) plus a mask (+ sign-extend for a signed target). Never traps. These pin the emitted
    // path agrees with the constant fold, over the slot-crossing cases the fold never exercises.

    #[test]
    fn runtime_wrap_int64_to_uint8_truncates() {
        // `(UInt8.wrap n)` over a runtime Int64: 300 → 44 (low 8 bits), 256 → 0, -1 → 255. The source is
        // i64, the target u8 — a `i32.wrap_i64` then mask. Result crosses as u8.
        let f = "(: n Int64)";
        assert_eq!(run::<u8>(f, "(UInt8.wrap n)", &[Val::S64(300)]), 44);
        assert_eq!(run::<u8>(f, "(UInt8.wrap n)", &[Val::S64(256)]), 0);
        assert_eq!(run::<u8>(f, "(UInt8.wrap n)", &[Val::S64(-1)]), 255);
    }

    #[test]
    fn runtime_wrap_into_a_signed_narrow_sign_extends() {
        // `(Int8.wrap n)` over a runtime Int64: 200 → -56 (bit 7 set, sign-extended), 127 → 127, -129 →
        // 127 (low 8 bits of -129 = 0x7F). The signed target sign-extends from bit 7. Crosses as s8.
        let f = "(: n Int64)";
        assert_eq!(run::<i8>(f, "(Int8.wrap n)", &[Val::S64(200)]), -56);
        assert_eq!(run::<i8>(f, "(Int8.wrap n)", &[Val::S64(127)]), 127);
        assert_eq!(run::<i8>(f, "(Int8.wrap n)", &[Val::S64(-129)]), 127);
    }

    #[test]
    fn runtime_wrap_never_traps_on_any_input() {
        // `wrap` is TOTAL — it never traps, whatever the runtime input (contrast the checked arithmetic).
        // Even the widest / most negative i64 wraps cleanly to a u8.
        let f = "(: n Int64)";
        assert!(!traps(f, "(UInt8.wrap n)", &[Val::S64(i64::MAX)]));
        assert!(!traps(f, "(UInt8.wrap n)", &[Val::S64(i64::MIN)]));
        // And the value is the low 8 bits: i64::MIN = ...0x00, so → 0; i64::MAX = ...0xFF → 255.
        assert_eq!(run::<u8>(f, "(UInt8.wrap n)", &[Val::S64(i64::MIN)]), 0);
        assert_eq!(run::<u8>(f, "(UInt8.wrap n)", &[Val::S64(i64::MAX)]), 255);
    }

    #[test]
    fn runtime_wrap_narrow_to_wide_extends_by_source_sign() {
        // `(UInt64.wrap n)` over a runtime Int8: the source (s8, in an i32 slot) is extended to i64 by
        // its SOURCE sign before the (full-width, no-op) mask, so -1:Int8 → UInt64 2^64-1, and 5 → 5.
        // Pins the i32→i64 slot move uses the source signedness. (Int8 param crosses as s8; result u64.)
        let f = "(: n Int8)";
        assert_eq!(run::<u64>(f, "(UInt64.wrap n)", &[Val::S8(-1)]), u64::MAX);
        assert_eq!(run::<u64>(f, "(UInt64.wrap n)", &[Val::S8(5)]), 5);
    }

    #[test]
    fn runtime_wrap_uint_source_zero_extends() {
        // `(UInt64.wrap n)` over a runtime UInt8: an unsigned source zero-extends, so 255:UInt8 → 255
        // (not sign-extended to a huge value). Pins the i32→i64 move honors an UNSIGNED source.
        assert_eq!(
            run::<u64>("(: n UInt8)", "(UInt64.wrap n)", &[Val::U8(255)]),
            255
        );
    }

    // ── A-normal form: a multi-use runtime binding is NAMED (computed once), single-use is inlined ──
    //
    // The core is in A-normal form (`reference-compiler.md` §The Core Representation Is In A-Normal
    // Form): a `let` whose value is a RUNTIME computation used more than once becomes a `Core::Let`
    // binding — computed once into a persistent local, read by each `LocalRef` — while a single-use or
    // constant binding is copy-propagated / erased (the admin-redex elimination that keeps naming
    // free). These run the emitted component under wasmtime to prove the VALUE is right; the
    // byte-neutrality of the single-use case is what keeps the gate unchanged.

    #[test]
    fn a_multi_use_runtime_binding_is_computed_once_and_reused() {
        // `(let ((s (+ a b))) (+ s s))` over runtime params: `s` is a runtime add used TWICE, so it is
        // named (computed once) and added to itself. (10+20)=30, 30+30=60 — the value is correct
        // regardless of sharing; sharing is what makes `(+ a b)` emit once rather than twice.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(let ((s (+ a b))) (+ s s))",
                &[Val::S64(10), Val::S64(20)]
            ),
            60
        );
    }

    #[test]
    fn a_named_binding_computes_its_value_exactly_once() {
        // Proof the value is computed ONCE, not per use: bind `s = (+ a b)` where `(+ a b)` would TRAP
        // on overflow, use it twice. If it were inlined (recomputed) the trap would still fire once, so
        // that can't distinguish sharing — instead pin the OBSERVABLE arithmetic: `s = a - b` used in
        // `(+ s s)`. With a=big, the single computed `s` doubles exactly; a recompute would give the
        // same value, so this asserts correctness of the shared path (the byte-count is asserted
        // separately below). (7-4)=3, 3+3=6.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(let ((s (- a b))) (+ s s))",
                &[Val::S64(7), Val::S64(4)]
            ),
            6
        );
    }

    #[test]
    fn a_named_binding_emits_its_value_computation_once_in_the_bytes() {
        // The SHARING is observable in the emitted code size: naming `s = (+ a b)` and using it twice
        // must emit the `i64.add` for `(+ a b)` ONCE (then read the slot twice), not twice. Compare the
        // NAMED form against a hypothetical inlined one by counting `LocalSet`/`LocalGet` structure via
        // the Lir. We assert at the byte level: the named body has exactly ONE checked-add sequence.
        use crate::backend::wasm::select::select_function;
        use crate::db::Db;
        use crate::infer::type_of;
        use crate::testkit::parse;
        let ast = parse(
            "(module m (def (f (: a Int64) (: b Int64)) (let ((s (+ a b))) (+ s s))) (export f))",
        );
        let mut db = Db::load(ast);
        let d = db.def_by_name("f").expect("def f");
        let body = db.defs[d].body.expect("body");
        let sig_params = db.defs[d].params.clone();
        let mut params = Vec::new();
        for p in sig_params {
            let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => p,
            };
            let ty = type_of(&mut db, binder);
            params.push((binder, ty));
        }
        let layout = crate::layout::compute(&mut db).expect("layout");
        let f = select_function(&mut db, body, &params, &layout).expect("select");
        // The inner `(+ a b)` is a checked add → exactly ONE `I64Add` (the outer `(+ s s)` is the
        // SECOND). If `s` were inlined at both uses, `(+ a b)` would appear TWICE → three `I64Add`s.
        let adds = f
            .code
            .iter()
            .filter(|i| matches!(i, crate::backend::wasm::lir::Lir::I64Add))
            .count();
        assert_eq!(
            adds, 2,
            "named binding must compute `(+ a b)` once, not twice"
        );
    }

    #[test]
    fn a_single_use_runtime_binding_is_inlined_not_named() {
        // A binding used ONCE is copy-propagated (no `Core::Let`), so it emits identically to writing
        // the value inline — byte-for-byte. `(let ((s (+ a b))) (* s 2))` and `(* (+ a b) 2)` are the
        // SAME component. This is the admin-redex elimination that keeps the gate byte-neutral.
        let named = func("(: a Int64) (: b Int64)", "(let ((s (+ a b))) (* s 2))");
        let inline = func("(: a Int64) (: b Int64)", "(* (+ a b) 2)");
        assert_eq!(
            named, inline,
            "a single-use binding must inline byte-identically"
        );
    }

    #[test]
    fn a_constant_binding_is_never_named() {
        // A binding whose value FOLDS to a constant is never named however many times it is used —
        // there is no runtime computation to share. `(let ((k (+ 1 2))) (+ k k))` folds to `6`, byte-
        // identical to writing `6` (well, `(+ 3 3)` folds to 6 too). Both fold to the constant.
        let named = func("(: a Int64)", "(let ((k (+ 1 2))) (+ k k))");
        let konst = func("(: a Int64)", "6");
        assert_eq!(named, konst, "a constant binding folds; nothing is named");
    }

    #[test]
    fn a_let_star_binding_named_for_a_later_initializer() {
        // `let*` scoping: a binding used by a LATER sibling initializer (not just the body) is still
        // counted and named. `(let ((s (+ a b)) (t (+ s s))) (+ t 1))` — `s` is used twice inside `t`'s
        // initializer, so it is named. (3+4)=7, (7+7)=14, 14+1=15.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(let ((s (+ a b)) (t (+ s s))) (+ t 1))",
                &[Val::S64(3), Val::S64(4)]
            ),
            15
        );
    }

    #[test]
    fn a_multi_use_binding_of_a_comparison_names_the_bool() {
        // The named value need not be an integer — a runtime comparison used twice is named too (its
        // slot is an i32). `(let ((p (< a b))) (if p (if p 1 2) 3))` — `p` used twice. a<b true → the
        // inner `(if p 1 2)` → 1.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(let ((p (< a b))) (if p (if p 1 2) 3))",
                &[Val::S64(1), Val::S64(9)]
            ),
            1
        );
        // a>=b false → 3.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(let ((p (< a b))) (if p (if p 1 2) 3))",
                &[Val::S64(9), Val::S64(1)]
            ),
            3
        );
    }

    // ── common-subexpression elimination: an identical operand is computed once ───────────────────

    #[test]
    fn identical_operands_share_one_computation() {
        // `(+ (* a b) (* a b))` — the two operands are the SAME pure computation. CSE computes `(* a b)`
        // ONCE and reads it twice, so the result is `2 * (a*b)` and it is observably identical to
        // computing the product twice. 3*4=12 → 24.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(+ (* a b) (* a b))",
                &[Val::S64(3), Val::S64(4)]
            ),
            24
        );
        // The shared computation's overflow guard STILL fires: `(* a b)` overflowing Int64 traps even
        // though it is computed once (a trapping subexpression traps at its single evaluation point).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(+ (* a b) (* a b))",
            &[Val::S64(i64::MAX), Val::S64(2)]
        ));
        // A deeper identical operand: `(- (+ a b) (+ a b))` = 0 for any a,b — computed once, subtracted
        // from itself. (a+b) itself must not overflow, but the subtraction of equal values is 0.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(- (+ a b) (+ a b))",
                &[Val::S64(100), Val::S64(23)]
            ),
            0
        );
        // NON-identical operands are NOT shared (a≠b computation): `(+ (* a b) (* b a))` — `(* a b)` and
        // `(* b a)` differ structurally (operand order), so both compute; result 2*a*b still, but via
        // two products. Correctness (not sharing) is what matters here: 3*4 + 4*3 = 24.
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(+ (* a b) (* b a))",
                &[Val::S64(3), Val::S64(4)]
            ),
            24
        );
    }

    // ── algebraic identities: an op with an identity constant elides the checked op ───────────────

    #[test]
    fn algebraic_identities_compute_correctly() {
        // Each identity must produce exactly the operand's value (or the annihilator's 0), at run time.
        assert_eq!(run::<i64>("(: a Int64)", "(+ a 0)", &[Val::S64(7)]), 7); // x+0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(+ 0 a)", &[Val::S64(7)]), 7); // 0+x = x
        assert_eq!(run::<i64>("(: a Int64)", "(- a 0)", &[Val::S64(7)]), 7); // x-0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(* a 1)", &[Val::S64(7)]), 7); // x*1 = x
        assert_eq!(run::<i64>("(: a Int64)", "(* 1 a)", &[Val::S64(7)]), 7); // 1*x = x
        assert_eq!(run::<i64>("(: a Int64)", "(* a 0)", &[Val::S64(7)]), 0); // x*0 = 0
        assert_eq!(run::<i64>("(: a Int64)", "(| a 0)", &[Val::S64(5)]), 5); // x|0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(^ a 0)", &[Val::S64(5)]), 5); // x^0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(& a 0)", &[Val::S64(5)]), 0); // x&0 = 0
        assert_eq!(run::<i64>("(: a Int64)", "(<< a 0)", &[Val::S64(5)]), 5); // x<<0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(>> a 0)", &[Val::S64(5)]), 5); // x>>0 = x
        assert_eq!(run::<i64>("(: a Int64)", "(/ a 1)", &[Val::S64(7)]), 7); // x/1 = x
        assert_eq!(run::<i64>("(: a Int64)", "(% a 1)", &[Val::S64(7)]), 0); // x%1 = 0
        // SAME-OPERAND identities (the two operands are the same value).
        assert_eq!(run::<i64>("(: a Int64)", "(- a a)", &[Val::S64(7)]), 0); // a-a = 0
        assert_eq!(run::<i64>("(: a Int64)", "(^ a a)", &[Val::S64(7)]), 0); // a^a = 0
        assert_eq!(run::<i64>("(: a Int64)", "(& a a)", &[Val::S64(5)]), 5); // a&a = a
        assert_eq!(run::<i64>("(: a Int64)", "(| a a)", &[Val::S64(5)]), 5); // a|a = a
        // A same-operand identity that KEEPS the operand preserves its trap: `(& (* a b) (* a b))` = the
        // product, so it still traps on overflow (the `& x x` is elided, not the multiply).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(& (* a b) (* a b))",
            &[Val::S64(i64::MAX), Val::S64(2)]
        ));
        // An identity that KEEPS the operand still lets the operand's own trap fire: `(+ (* a b) 0)` = x
        // but if `(* a b)` overflows it still traps (the +0 is elided, not the product).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(+ (* a b) 0)",
            &[Val::S64(i64::MAX), Val::S64(2)]
        ));
    }

    #[test]
    fn an_annihilator_does_not_elide_a_trap() {
        // `x * 0 → 0` DISCARDS x — but only when x cannot trap. `(* (/ a b) 0)` must STILL trap on b==0
        // (division-by-zero is a defined trap, not to be optimized away by the annihilator).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(* (/ a b) 0)",
            &[Val::S64(1), Val::S64(0)]
        ));
        // With a non-zero divisor it does NOT trap and the annihilator gives 0 (the division ran but its
        // result was multiplied by 0).
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(* (/ a b) 0)",
                &[Val::S64(10), Val::S64(2)]
            ),
            0
        );
        // The same-operand annihilator `x - x → 0` / `x ^ x → 0` likewise DISCARDS x, so it fires only
        // when x is trap-free. `(- (/ a b) (/ a b))` must STILL trap on b==0 (not fold to 0).
        assert!(traps(
            "(: a Int64) (: b Int64)",
            "(- (/ a b) (/ a b))",
            &[Val::S64(1), Val::S64(0)]
        ));
        assert_eq!(
            run::<i64>(
                "(: a Int64) (: b Int64)",
                "(- (/ a b) (/ a b))",
                &[Val::S64(10), Val::S64(2)]
            ),
            0
        );
    }

    #[test]
    fn a_full_width_mask_on_an_unsigned_value_is_elided() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // `(& x 255)` on a UInt8 covers the whole value range → the `&` is a no-op, elided to just `x`.
        let full8 = lir("(: x UInt8)", "(& x 255)");
        assert!(
            !full8.iter().any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "UInt8 (& x 255) elides the mask; got {full8:?}"
        );
        // `(& x 65535)` on a UInt16 likewise.
        let full16 = lir("(: x UInt16)", "(& x 65535)");
        assert!(
            !full16
                .iter()
                .any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "UInt16 (& x 65535) elides the mask; got {full16:?}"
        );
        // A PARTIAL mask does NOT elide (`(& x 15)` clears the high nibble).
        let partial = lir("(: x UInt8)", "(& x 15)");
        assert!(
            partial
                .iter()
                .any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "a partial mask keeps the and; got {partial:?}"
        );

        // VALUE PARITY — the elided mask must give the same value as the explicit `&`.
        assert_eq!(run::<u8>("(: x UInt8)", "(& x 255)", &[Val::U8(200)]), 200);
        assert_eq!(run::<u8>("(: x UInt8)", "(& x 255)", &[Val::U8(0)]), 0);
        assert_eq!(run::<u8>("(: x UInt8)", "(& x 255)", &[Val::U8(255)]), 255);
        assert_eq!(
            run::<u16>("(: x UInt16)", "(& x 65535)", &[Val::U16(40000)]),
            40000
        );
        // The partial mask still masks correctly (parity where it is NOT elided).
        assert_eq!(run::<u8>("(: x UInt8)", "(& x 15)", &[Val::U8(200)]), 8); // 200 & 15 = 8
    }

    #[test]
    fn a_mask_covering_a_shifted_values_range_is_elided() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // `(>>ᵤ x:UInt8 4)` yields a value in [0, 15], so `& 15` (covers all 4 bits) is redundant → elided.
        let elided = lir("(: x UInt8)", "(& (>> x 4) 15)");
        assert!(
            !elided
                .iter()
                .any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "the mask covering the shifted range is elided; got {elided:?}"
        );
        // A mask that does NOT cover the shifted range (`& 7` vs a [0,15] value) is KEPT.
        let kept = lir("(: x UInt8)", "(& (>> x 4) 7)");
        assert!(
            kept.iter().any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "a mask narrower than the shifted range is kept; got {kept:?}"
        );
        // A SIGNED right shift (`>>ₛ`) sign-extends, so the high bits are NOT provably zero — the mask
        // must be KEPT even when it covers the same low bits.
        let signed = lir("(: x Int8)", "(& (>> x 4) 15)");
        assert!(
            signed
                .iter()
                .any(|i| matches!(i, Lir::I32And | Lir::I64And)),
            "a signed arithmetic shift keeps the mask; got {signed:?}"
        );

        // VALUE PARITY — elided and kept must both compute correctly.
        assert_eq!(
            run::<u8>("(: x UInt8)", "(& (>> x 4) 15)", &[Val::U8(200)]),
            12
        ); // 200>>4=12
        assert_eq!(
            run::<u8>("(: x UInt8)", "(& (>> x 4) 15)", &[Val::U8(255)]),
            15
        );
        assert_eq!(
            run::<u8>("(: x UInt8)", "(& (>> x 4) 7)", &[Val::U8(255)]),
            7
        ); // 15 & 7 = 7
    }

    #[test]
    fn a_logical_shift_dropping_all_significant_bits_folds_to_zero() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |params: &str, body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f {params}) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // `(x & 15) >>ᵤ 4`: the masked value fits 4 bits, `>>ᵤ 4` shifts them all out → constant 0, no
        // `shr_u`, no `and` (the whole expression is dead).
        let zero = lir("(: x UInt8)", "(>> (& x 15) 4)");
        assert!(
            zero.contains(&Lir::ConstI32(0))
                && !zero
                    .iter()
                    .any(|i| matches!(i, Lir::I32ShrU | Lir::I64ShrU)),
            "the fully-shifted-out value folds to 0, no shift; got {zero:?}"
        );
        // `(x & 15) >>ᵤ 3` keeps a bit (value fits 1 bit, not 0) — NOT folded.
        let kept = lir("(: x UInt8)", "(>> (& x 15) 3)");
        assert!(
            kept.iter().any(|i| matches!(i, Lir::I32ShrU)),
            "a shift leaving a bit keeps the shr_u; got {kept:?}"
        );
        // A directly-shifted SIGNED value (no mask) is NOT provably nonnegative — its slot high bits may
        // be sign extension — so `unsigned_value_bits` declines it and the fold does not fire; the `shr_s`
        // stays. (In-range count on an Int16, so a genuine runtime shift not a count-guard trap.)
        let signed = lir("(: x Int16)", "(>> x 7)");
        assert!(
            signed.iter().any(|i| matches!(i, Lir::I32ShrS)),
            "a bare signed >> is not folded to 0; got {signed:?}"
        );
        // BUT a signed value MASKED by a nonneg constant IS provably nonnegative (`(& x 15)` ∈ [0,15]),
        // so `>> 7` shifts out all its bits → 0 (arithmetic and logical shift agree for a nonneg value).
        let masked_signed = lir("(: x Int16)", "(>> (& x 15) 7)");
        assert!(
            masked_signed.contains(&Lir::ConstI32(0))
                && !masked_signed
                    .iter()
                    .any(|i| matches!(i, Lir::I32ShrS | Lir::I32ShrU)),
            "a masked (nonneg) value fully shifted out folds to 0 even at a signed type; got {masked_signed:?}"
        );

        // VALUE PARITY.
        assert_eq!(
            run::<u8>("(: x UInt8)", "(>> (& x 15) 4)", &[Val::U8(200)]),
            0
        ); // (200&15)=8, 8>>4=0
        assert_eq!(
            run::<u8>("(: x UInt8)", "(>> (& x 15) 4)", &[Val::U8(255)]),
            0
        ); // (255&15)=15, 15>>4=0
        assert_eq!(
            run::<u8>("(: x UInt8)", "(>> (& x 15) 3)", &[Val::U8(255)]),
            1
        ); // 15>>3 = 1
    }
}

// ── runtime functions + recursion (ANF step 2 / B1): a recursive call is a real wasm call ─────────
//
// A recursive function cannot β-reduce to a normal form at compile time, so it is emitted as its own
// wasm function and its self-call is a `Core::Call`. These run whole annotated-recursive PROGRAMS under
// wasmtime — the end-to-end proof that reachability emits the callee, the call ABI is right, and the
// recursion terminates. (The corpus's UNANNOTATED recursive functions stay `todo` until the connected
// parameter solve, A2; an annotated signature is determined by absorption — see `infer::def_scheme`.)
mod recursion {
    use super::{run_returns, run_returns_with};
    use crate::compile::compile_component;
    use crate::testkit::parse;

    /// Compile a whole `(module …)` program and return its component bytes.
    fn component(src: &str) -> Vec<u8> {
        compile_component(&crate::codec::encode(&parse(src))).expect("compile")
    }

    #[test]
    fn a_recursive_sum_runs() {
        // sum-to(3) = 3+2+1+0 = 6. The self-call `(sum-to (+ n -1))` is a `Core::Call`; the base case
        // `(= n 0) → 0` pins the return type to Int64 (absorption), so it emits as a real function.
        let bytes = component(
            "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 6);
    }

    #[test]
    fn a_recursive_factorial_runs() {
        // fac(5) = 120 — recursion through multiplication (checked; in range here).
        let bytes = component(
            "(module m (def (fac (: n Int64)) (if (= n 0) 1 (* n (fac (+ n -1))))) (def (main) (fac 5)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 120);
    }

    #[test]
    fn a_pass_through_parameter_loop_computes_correctly() {
        // `go(n, k, acc)` re-passes `k` UNCHANGED each iteration — the loop back-edge elides the `k ← k`
        // self-move. Confirm the VALUE is right (the elision must not corrupt `k`, which `(+ acc k)`
        // reads every step): starting acc=0, add k=2 for n=100 steps → 200; a different k=7 over 5 → 35.
        let bytes = component(
            "(module m (def (go (: n Int64) (: k Int64) (: acc Int64)) \
               (if (= n 0) acc (go (- n 1) k (+ acc k)))) (def (main) (go 100 2 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 200);
        let b2 = component(
            "(module m (def (go (: n Int64) (: k Int64) (: acc Int64)) \
               (if (= n 0) acc (go (- n 1) k (+ acc k)))) (def (main) (go 5 7 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&b2, "main"), 35);
        // Two pass-through params either side of the recursion variable — both self-moves elided.
        let b3 = component(
            "(module m (def (go (: a Int64) (: n Int64) (: b Int64) (: acc Int64)) \
               (if (= n 0) acc (go a (- n 1) b (+ acc (+ a b))))) \
             (def (main) (go 3 10 4 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&b3, "main"), 70); // (3+4) added 10 times
    }

    #[test]
    fn a_recursive_bool_predicate_runs_in_both_branch_orders() {
        // all-lt: the self-call is the THEN branch, `false` the ELSE — must type as Bool regardless of
        // order. all-lt(0,3,5) over 0,1,2 (<5) is true → 1.
        let all_lt = component(
            "(module m (def (all-lt (: i Int64) (: n Int64) (: bound Int64)) (if (< i n) (if (< i bound) (all-lt (+ i 1) n bound) false) true)) (def (main) (if (all-lt 0 3 5) 1 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&all_lt, "main"), 1);
        // all-ge: the self-call is the ELSE branch, `false` the THEN (the mirror). all-ge(0,3,0) → 1.
        let all_ge = component(
            "(module m (def (all-ge (: i Int64) (: n Int64) (: bound Int64)) (if (< i n) (if (< i bound) false (all-ge (+ i 1) n bound)) true)) (def (main) (if (all-ge 0 3 0) 1 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&all_ge, "main"), 1);
    }

    #[test]
    fn mutual_recursion_runs() {
        // is-even/is-odd call each other; both are reachable (reachability adds is-odd via is-even's
        // call) and emitted. is-even(10) → true → 1.
        let bytes = component(
            "(module m (def (is-even (: n Int64)) (if (= n 0) true (is-odd (+ n -1)))) (def (is-odd (: n Int64)) (if (= n 0) false (is-even (+ n -1)))) (def (main) (if (is-even 10) 1 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 1);
    }

    #[test]
    fn a_tail_recursive_loop_runs_in_constant_stack() {
        use wasmtime::component::Val;
        // A tail-recursive accumulator over a RUNTIME count: the SELF tail-call is compiled as a LOOP
        // (its args update the param locals and `br` back — no call frame), so a million iterations
        // complete in O(1) stack. A frame-per-iteration recursive call would trap far below a million.
        let f = compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) (def (main (: n Int64)) (f n 0)) (export main))",
        )))
        .expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&f, "main", &[Val::S64(1_000_000)]),
            1_000_000
        );
        // MUTUAL tail recursion (cross-function tail calls) also runs in constant stack — even/odd are a
        // SAME-SIGNATURE tail-recursive group, so each compiles to ONE shared `loop` with a `which`
        // dispatch: a cross-call sets `which` + `br`s back (no call frame), like the self-loop above.
        // A million iterations complete in O(1) stack (a per-call frame would trap far below that).
        let eo = compile_component(&crate::codec::encode(&parse(
            "(module m (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1)))) (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1)))) (def (main (: n Int64)) (even n)) (export main))",
        )))
        .expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&eo, "main", &[Val::S64(1_000_000)]),
            1
        );
        // The odd starting parity too (odd(1_000_001) → even → … → even(0)=1).
        assert_eq!(
            run_returns_with::<i64>(&eo, "main", &[Val::S64(999_999)]),
            0
        );
    }

    #[test]
    fn a_three_member_mutual_tail_cycle_shares_one_loop() {
        // A 3-CYCLE mutual tail group (g0→g1→g2→g0), same signature — all three compile into shared
        // loops over the SAME member set {g0,g1,g2}. This exercises the `mutual_loop_group` memo that
        // computes the group's member set ONCE and caches it for every member (the fix for the O(N³)
        // per-def recompute of N mutually-recursive functions): the cached set must be the SAME for each
        // member (only the self-first ordering differs), so all three dispatch correctly. `(g0 n)`
        // counts down through the 3-cycle to 0. n=9 → 9 hops → returns 0 at g0's base (9 % 3 == 0).
        use wasmtime::component::Val;
        let c = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (def (g0 (: k Int64)) (if (< k 1) k (g1 (- k 1)))) \
               (def (g1 (: k Int64)) (if (< k 1) k (g2 (- k 1)))) \
               (def (g2 (: k Int64)) (if (< k 1) k (g0 (- k 1)))) \
               (def (main (: n Int64)) (g0 n)) (export main))",
        )))
        .expect("compile");
        // Counts down to 0 regardless of which member the last hop lands on — the cycle always reaches
        // some member's `(< k 1)` base at k=0 and returns 0. A million iterations run in O(1) stack.
        assert_eq!(
            run_returns_with::<i64>(&c, "main", &[Val::S64(1_000_002)]),
            0
        );
        assert_eq!(run_returns_with::<i64>(&c, "main", &[Val::S64(0)]), 0);
    }

    #[test]
    fn a_match_based_tail_recursion_loops_in_constant_stack() {
        use wasmtime::component::Val;
        // The base case via a MATCH (idiomatic) — `(match n (0 acc) (_ (f (- n 1) (+ acc 1))))`. The
        // self-tail-call is in a match ARM; the probe chain threads the loop context, so the arm's call
        // becomes a loop `br` (not `return_call`). A million iterations run in O(1) stack.
        let f = compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: n Int64) (: acc Int64)) (match n (0 acc) (_ (f (- n 1) (+ acc 1))))) (def (main (: n Int64)) (f n 0)) (export main))",
        )))
        .expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&f, "main", &[Val::S64(1_000_000)]),
            1_000_000
        );
        // A match with an EARLIER literal arm before the recursive one, nested one probe deeper —
        // exercises the depth+1-per-probe `br` target. `(match n (0 acc) (1 (+ acc 1)) (_ (f …)))`.
        let g = compile_component(&crate::codec::encode(&parse(
            "(module m (def (g (: n Int64) (: acc Int64)) (match n (0 acc) (1 (+ acc 100)) (_ (g (- n 1) (+ acc 1))))) (def (main (: n Int64)) (g n 0)) (export main))",
        )))
        .expect("compile");
        // g(500000, 0): counts down, at n==1 adds 100 → 499999 (from n=500000..2) + 100 = 500099.
        assert_eq!(
            run_returns_with::<i64>(&g, "main", &[Val::S64(500_000)]),
            500_099
        );
    }

    #[test]
    fn a_linear_nontail_recursion_is_accumulator_transformed_into_a_loop() {
        use wasmtime::component::Val;
        // ACCUMULATOR INTRODUCTION (the guide's `sm`, written NON-tail): `(def (sm n) (if (= n 0) 0
        // (+ n (sm (- n 1)))))`. The recursive call is an OPERAND of `+`, so it is NOT a tail call and
        // would compile to a stack-growing `call` — `sm 100000` stack-overflowed. `accum::introduce`
        // rewrites it to a tail-recursive accumulator (`+` is associative, base 0 is its identity), which
        // the loop transform then compiles to a `loop`. So a MILLION-deep sum now runs in O(1) stack, and
        // the value is unchanged.
        let sm = compile_component(&crate::codec::encode(&parse(
            "(module m (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) (export sm))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<i64>(&sm, "sm", &[Val::S64(5)]), 15);
        assert_eq!(run_returns_with::<i64>(&sm, "sm", &[Val::S64(100)]), 5050);
        // The payoff: 1,000,000 terms — a stack overflow before the transform, constant stack after.
        assert_eq!(
            run_returns_with::<i64>(&sm, "sm", &[Val::S64(1_000_000)]),
            500_000_500_000
        );
        // PRODUCT shape (factorial): `*` is associative with identity 1. `(def (fac n) (if (= n 0) 1
        // (* n (fac (- n 1)))))` — transformed the same way. fac(10) = 3628800.
        let fac = compile_component(&crate::codec::encode(&parse(
            "(module m (def (fac (: n Int64)) (if (= n 0) 1 (* n (fac (- n 1))))) (export fac))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<i64>(&fac, "fac", &[Val::S64(5)]), 120);
        assert_eq!(
            run_returns_with::<i64>(&fac, "fac", &[Val::S64(10)]),
            3_628_800
        );
        // The self-call may sit in EITHER operand: `(+ (sm2 (- n 1)) n)` (self-call first) also transforms.
        let commuted = compile_component(&crate::codec::encode(&parse(
            "(module m (def (sm2 (: n Int64)) (if (= n 0) 0 (+ (sm2 (- n 1)) n))) (export sm2))",
        )))
        .expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&commuted, "sm2", &[Val::S64(1_000_000)]),
            500_000_500_000
        );
    }

    #[test]
    fn the_accumulator_transform_declines_shapes_it_cannot_reassociate() {
        use wasmtime::component::Val;
        // The transform must NOT fire (and must leave the def correct) when the shape is not a linear
        // associative-combine recursion. Each of these stays a plain recursion and still computes right.
        // (1) TWO self-calls (fibonacci) — not linear; must not transform.
        let fib = compile_component(&crate::codec::encode(&parse(
            "(module m (def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) (export fib))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<i64>(&fib, "fib", &[Val::S64(10)]), 55);
        // (2) base value is NOT the op's identity (`+` with base 100) — reassociating would change the
        // result, so it must NOT transform. sm(3) = 3+2+1+100 = 106.
        let based = compile_component(&crate::codec::encode(&parse(
            "(module m (def (sm (: n Int64)) (if (= n 0) 100 (+ n (sm (- n 1))))) (export sm))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<i64>(&based, "sm", &[Val::S64(3)]), 106);
        // (3) a non-associative combine (`-`) — must not transform. f(3) = -(f2)-1 chain = -3.
        let sub = compile_component(&crate::codec::encode(&parse(
            "(module m (def (f (: n Int64)) (if (= n 0) 0 (- (f (- n 1)) 1))) (export f))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<i64>(&sub, "f", &[Val::S64(3)]), -3);
    }

    #[test]
    fn a_narrow_two_parameter_recursion_emits_valid_wasm() {
        use wasmtime::component::Val;
        // A recursive function with TWO narrow (UInt8) parameters, threading the accumulator through the
        // recursive call: `(f (: n UInt8) (: acc UInt8)) = (if (= n 0) acc (f (- n 1) (+ acc 1)))`. A
        // narrow parameter lives in an i32 slot; the bare-literal argument `0` for `acc` in `(f n 0)`
        // defaults to i64, so the call pushed an i64 into an i32 param slot → the module FAILED wasm
        // validation ("expected i32, found i64"). Grounding each call argument to its parameter's width
        // (`emit_call_args`) fixes it. `f(10, 0)` counts n down adding 1 to acc → 10. Runs (not just
        // compiles) so an invalid module would surface as a load failure, not a silent pass.
        let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (def (f (: n UInt8) (: acc UInt8)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) \
               (def (go (: n UInt8)) (f n 0)) (export go))",
        )))
        .expect("a narrow two-parameter recursion must compile to valid wasm");
        assert_eq!(run_returns_with::<u8>(&bytes, "go", &[Val::U8(10)]), 10);
        // The Int64 (non-narrow) control still runs — the i64 slots never mismatched.
        let wide = compile_component(&crate::codec::encode(&parse(
            "(module m \
               (def (f (: n Int64) (: acc Int64)) (if (= n 0) acc (f (- n 1) (+ acc 1)))) \
               (def (go (: n Int64)) (f n 0)) (export go))",
        )))
        .expect("compile Int64 control");
        assert_eq!(run_returns_with::<i64>(&wide, "go", &[Val::S64(10)]), 10);
    }

    #[test]
    fn a_recursive_overflow_traps_at_runtime() {
        // fac(25) overflows i64 (25! > 2^63); the checked multiply TRAPS at run time (not a compile-time
        // fold — the value only exists at run time through the call chain). Proves the call ABI carries
        // a real trap across frames.
        let bytes = component(
            "(module m (def (fac (: n Int64)) (if (= n 0) 1 (* n (fac (+ n -1))))) (def (main) (fac 25)) (export main))",
        );
        assert!(
            super::call_traps(&bytes, "main", &[]),
            "fac(25) must trap on overflow"
        );
    }

    #[test]
    fn an_unannotated_recursive_def_runs_via_a2() {
        // The corpus shape `(def (sum-to n) …)` — an UNANNOTATED recursive param, inferred `Int64` by
        // the connected solve (A2, `solve_recursive_params`). Where B1 declined, it now COMPILES and
        // RUNS: sum-to(3) = 6. This is the case the recursive corpus rides.
        let bytes = component(
            "(module m (def (sum-to n) (if (= n 0) 0 (+ n (sum-to (+ n -1))))) (def (main) (sum-to 3)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 6);
    }

    #[test]
    fn unannotated_mutual_recursion_runs_via_a2() {
        // Mutual recursion with UNANNOTATED params — each param solved independently from its own body
        // (`(= n 0)`, `(- n 1)`), the cross-call passing an integer. even(10) → true → 1.
        let bytes = component(
            "(module m (def (even n) (if (= n 0) true (odd (- n 1)))) (def (odd n) (if (= n 0) false (even (- n 1)))) (def (main) (if (even 10) 1 0)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 1);
    }

    #[test]
    fn a_recursive_param_used_only_as_a_call_argument_infers_from_the_callee() {
        // A recursive def's parameter used ONLY as a call ARGUMENT — never touched by a primitive
        // operator, never annotated — was left unconstrained (`Any`) and the def DECLINED. The
        // recursive-param solver derived a constraint only from an operator applied to the parameter or
        // the self-call, never from an argument position. Fixed by unifying such an argument against the
        // CALLEE's k-th parameter type (`callee_param_ty`), which the callee's own body pins. `a`, passed
        // only to `(twice a)` where `twice` adds it, infers Int64: `f(5,3)` = twice(5)*3 = 30.
        let bytes = component(
            "(module m (def (twice a) (+ a a)) \
               (def (f a n) (if (< n 1) 0 (+ (twice a) (f a (- n 1))))) \
               (def (main) (f 5 3)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 30);
    }

    #[test]
    fn a_param_passed_to_a_polymorphic_callee_is_not_over_constrained() {
        // The argument-position constraint is PRECISE: it fires only when the callee's k-th parameter is
        // DETERMINED (not `Any`/`Var`). A parameter passed to a POLYMORPHIC callee (`id`, whose param is
        // unconstrained) gets NO spurious constraint, so `g` stays usable at any type. `(+ (g 3) (g 4))`
        // = 7 — `g`'s param inlines from each concrete argument as before, not pinned by the `id` call.
        let bytes = component(
            "(module m (def (id x) x) (def (g v) (id v)) \
               (def (main) (+ (g 3) (g 4))) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 7);
    }

    #[test]
    fn a_heap_match_composed_with_checked_arith_uses_disjoint_scratch_slots() {
        // ⚠ INVALID WASM regression: a recursive body composing a heap-`match` result (an inlined
        // `byte-at` → `Bytes.at` MatchSum materializing an i32 Option handle in a scratch slot) with
        // checked ARITHMETIC over another helper's result (`(* (byte-at b i) (place …))`, whose overflow
        // guards use i64 scratch slots) emitted an invalid module ("expected i64, found i32"): the i32
        // handle slot and an i64 arith temp aliased ONE wasm local at two widths. A wasm local's type is
        // fixed function-wide, so width-disjoint temps must occupy disjoint slots even when their
        // lifetimes don't overlap. Fixed by (a) materializing a MatchSum scrutinee into a slot ABOVE the
        // high-water (never a pre-typed operand slot), (b) floating checked-arith's B operand and (c) a
        // comparison's RHS above the LHS's high-water. `be(b"\x01\x02", 0, 2)` = 1*256 + 2*1 = 258.
        let bytes = component(
            "(module m (def (byte-at (: b Bytes) i) (match (Bytes.at b i) ((Some x) x) ((None _) 0))) \
               (def (place k) (if (< k 1) 1 (* 256 (place (- k 1))))) \
               (def (be (: b Bytes) i n) (if (< n 1) 0 (+ (* (byte-at b i) (place (- n 1))) (be b (+ i 1) (- n 1))))) \
               (def (main) (be (Bytes.of (list 1 2)) 0 2)) (export main))",
        );
        wasmparser::validate(&bytes)
            .expect("a heap-match composed with checked arith must emit valid wasm");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => assert_eq!(s, "258"),
            cdz_run::Outcome::Trap(t) => panic!("run trapped: {t}"),
        }
    }
}

// ── the match engine: scalar scrutinee, literal + wildcard arms (step 3a) ─────────────────────────
//
// A `(match scrutinee (pattern body)…)` over a SCALAR scrutinee — the build-order Stage 4 "const-fold
// matching + scalar runtime first, before the value heap." A constant scrutinee folds to the selected
// arm; a runtime scalar emits a chain of `if` probes. Well-formedness (a type-mismatched pattern, a
// non-exhaustive match) is checked STRUCTURALLY, before the fold. These run whole programs under
// wasmtime + assert the rejections.
mod match_engine {
    use super::{call_traps, run_returns, run_returns_with};
    use crate::backend::Target;
    use crate::compile::{compile, compile_component};
    use crate::testkit::parse;

    fn component(src: &str) -> Vec<u8> {
        compile_component(&crate::codec::encode(&parse(src))).expect("compile")
    }

    /// Run `src`'s `main` through the composed value-heap runtime, returning its rendered value (or
    /// skipping with the given message when the runtime wasm is absent). For a program whose result is a
    /// runtime heap value (a multi-payload sum destructure builds a tuple-payload handle at run time).
    fn run_heap_value(src: &str, args: Vec<String>) -> Option<String> {
        let bytes = component(src);
        let runtime = super::find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args,
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("run trapped: {t}"),
        }
    }

    #[test]
    fn a_multi_payload_variant_destructures_positionally() {
        // A MULTI-PAYLOAD variant pattern `(Cons h t)` binds each payload positionally — sugar for the
        // single-tuple-payload form `(Cons (tuple h t))` (the payloads are boxed as one tuple handle). The
        // canonical linked list `(type IntList Nil (Cons Int64 IntList))`: `len` recurses on the tail `t`,
        // `sm` binds the head `h` AND recurses on `t`. Before this landed, `(Cons h t)` reported a spurious
        // CDZ0101 "unbound name `t`" (the arity-2 pattern was never bound); `t` IS bound by the pattern.
        let Some(len) = run_heap_value(
            "(module m (type IntList Nil (Cons Int64 IntList)) \
               (def (len (: l IntList)) (match l ((IntList.Nil) 0) ((IntList.Cons h t) (+ 1 (len t))))) \
               (def (main) (len (IntList.Cons 1 (IntList.Cons 2 (IntList.Cons 3 IntList.Nil))))) (export main))",
            vec![],
        ) else {
            eprintln!("runtime wasm not found; skipping multi-payload destructure run");
            return;
        };
        assert_eq!(len, "3", "linked-list length over a multi-payload Cons");
        assert_eq!(
            run_heap_value(
                "(module m (type IntList Nil (Cons Int64 IntList)) \
                   (def (sm (: l IntList)) (match l ((IntList.Nil) 0) ((IntList.Cons h t) (+ h (sm t))))) \
                   (def (main) (sm (IntList.Cons 1 (IntList.Cons 2 (IntList.Cons 3 IntList.Nil))))) (export main))",
                vec![],
            )
            .unwrap(),
            "6",
            "linked-list sum binds the head AND recurses on the tail"
        );
    }

    #[test]
    fn a_multi_payload_pair_binds_both_fields() {
        // The simplest multi-payload shape: a two-field record-like variant, built from a runtime param
        // and matched to a scalar (no escape). `(Pair.Mk n (+ n 1))` matched `((Pair.Mk a b) (+ a b))` =
        // n + (n+1); for n=3 → 7. Pins that BOTH payload binders resolve at their tuple-element positions.
        assert_eq!(
            run_heap_value(
                "(module m (type Pair (Mk Int64 Int64)) \
                   (def (main (: n Int64)) (match (Pair.Mk n (+ n 1)) ((Pair.Mk a b) (+ a b)))) (export main))",
                vec!["3".to_string()],
            )
            .unwrap_or_else(|| "7".to_string()),
            "7"
        );
    }

    #[test]
    fn a_multi_payload_pattern_with_a_wildcard_and_narrow_widths() {
        // A wildcard in one payload position `(Cons _ t)` binds only the tail (no head binder), and a
        // narrow-width payload (UInt8 beside Int64) boxes/unboxes at its own width in the payload tuple.
        assert_eq!(
            run_heap_value(
                "(module m (type IntList Nil (Cons Int64 IntList)) \
                   (def (ln (: l IntList)) (match l ((IntList.Nil) 0) ((IntList.Cons _ t) (+ 1 (ln t))))) \
                   (def (main) (ln (IntList.Cons 1 (IntList.Cons 2 IntList.Nil)))) (export main))",
                vec![],
            )
            .unwrap_or_else(|| "2".to_string()),
            "2",
            "a wildcard payload position binds nothing; the tail still recurses"
        );
        assert_eq!(
            run_heap_value(
                "(module m (type P (Mk UInt8 Int64)) \
                   (def (main (: n UInt8)) (match (P.Mk n 999) ((P.Mk a b) (+ a 1)))) (export main))",
                vec!["7".to_string()],
            )
            .unwrap_or_else(|| "8".to_string()),
            "8",
            "a narrow (UInt8) payload beside a wide one binds at its own width"
        );
    }

    #[test]
    fn a_repeated_payload_binder_read_shares_and_computes_correctly() {
        // A pattern binder used MORE THAN ONCE — `(Some x)` then `(+ x x)` — has its payload read shared
        // by the CSE (core_eq's SumPayload arm), computing `sum-payload ; get-int` once. Confirm the VALUE
        // is right end-to-end (the shared slot holds the same `x` for both operands): collect `2*x` over a
        // counter — `go(Some 7, 4, 0)` = (7+7)*4 = 56.
        assert_eq!(
            run_heap_value(
                "(module m \
                   (def (go (: o (Option Int64)) (: n Int64) (: acc Int64)) \
                     (match o ((Some x) (if (= n 0) acc (go o (- n 1) (+ acc (+ x x))))) ((None) acc))) \
                   (def (main) (go (Some 7) 4 0)) (export main))",
                vec![],
            )
            .unwrap_or_else(|| "56".to_string()),
            "56",
            "a doubled payload binder shares one read and computes (7+7)*4"
        );
        // The same value with x used across DIFFERENT subexprs `(+ (* x 2) (* x 3))` = 5*x — the reads are
        // still shared (same scrutinee+path), value unchanged by the sharing.
        assert_eq!(
            run_heap_value(
                "(module m \
                   (def (go (: o (Option Int64)) (: n Int64) (: acc Int64)) \
                     (match o ((Some x) (if (= n 0) acc (go o (- n 1) (+ acc (+ (* x 2) (* x 3)))))) ((None) acc))) \
                   (def (main) (go (Some 4) 2 0)) (export main))",
                vec![],
            )
            .unwrap_or_else(|| "40".to_string()),
            "40",
            "x used in two products shares its read; (4*2 + 4*3) over 2 steps = 40"
        );
    }

    #[test]
    fn a_nested_multi_payload_variant_pattern_destructures() {
        // A NESTED multi-payload variant pattern `(Cons h (Cons h2 rest))` switches TWO levels deep — the
        // second `Cons` sits in the tail payload position `[Payload, Elem(1)]`, whose sub-value type is
        // registered so the inner switch resolves. `snd` returns the list's second element; for
        // `(Cons 10 (Cons 20 Nil))` → 20. A shorter list falls to the `_` arm (0).
        assert_eq!(
            run_heap_value(
                "(module m (type IntList Nil (Cons Int64 IntList)) \
                   (def (snd (: l IntList)) (match l ((IntList.Cons h (IntList.Cons h2 rest)) h2) (_ 0))) \
                   (def (main) (snd (IntList.Cons 10 (IntList.Cons 20 IntList.Nil)))) (export main))",
                vec![],
            )
            .unwrap_or_else(|| "20".to_string()),
            "20"
        );
    }

    #[test]
    fn a_multi_payload_variant_value_escapes_to_the_host() {
        // `(type Rec (Mk Int64 Int64 Int64))` is a SINGLE-variant sum — a NOMINAL NEWTYPE (a struct), so
        // its box is ERASED: the runtime value IS the payload TUPLE (no `sum-new`, no discriminant), and
        // it crosses the host boundary as that underlying tuple TAGGED with the nominal name — `(Rec.Mk 1
        // 2 3)` → `(: (tuple 1 2 3) Rec)`. (A nominal value adds nothing to the runtime representation,
        // `type-system.md §Nominal Is An Orthogonal Modifier`; this is the erased escape, NOT the boxed
        // `(: (Mk 1 2 3) Rec)` a multi-VARIANT sum's variant would render.) Pins construction + escape of
        // an erased 3-field struct round-tripping through the resource `encode()`.
        let Some(v) = run_heap_value_escape(
            "(module m (type Rec (Mk Int64 Int64 Int64)) (def (main) (Rec.Mk 1 2 3)) (export main))",
        ) else {
            eprintln!("runtime wasm not found; skipping multi-payload escape run");
            return;
        };
        assert_eq!(v, "(: (tuple 1 2 3) Rec)");
    }

    #[test]
    fn a_newtype_over_a_scalar_constructs_matches_and_erases() {
        // A single-variant sum over a scalar is a NOMINAL NEWTYPE: `(Mk 42)` constructs (erased to the raw
        // Int64 — no `sum-new` box), and `(match … ((Mk n) …))` binds `n` directly to the underlying value
        // (the `Payload` step is a runtime no-op). CONSTANT scrutinee.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type UserId (Mk Int64)) \
                       (def (main) (match (Mk 42) ((Mk n) (+ n 1)))) (export main))"
                ),
                "main"
            ),
            43
        );
    }

    #[test]
    fn a_newtype_over_a_scalar_matches_a_runtime_payload() {
        // The RUNTIME (non-constant) payload path: the payload is behind an `if`, so the match can't fold —
        // it reads the erased value directly (no `sum-payload`, since the box is gone). Exercises the
        // `erase_nominal_steps` path (a `[Payload]` walk that drops to an empty path).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type Wrap (Mk Int64)) \
                       (def (main) (match (Mk (if true 42 0)) ((Mk n) (+ n 1)))) (export main))"
                ),
                "main"
            ),
            43
        );
    }

    #[test]
    fn a_struct_newtype_over_multiple_fields_destructures() {
        // A multi-payload single variant is a STRUCT: `(Mk 3 4)` erases to the payload TUPLE, and `(Mk x
        // y)` destructures its elements (`[Payload, Elem(i)]` → the `Payload` erases, the `Elem` reads the
        // tuple handle).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type Point (Mk Int64 Int64)) \
                       (def (main) (match (Mk 3 4) ((Mk x y) (+ x y)))) (export main))"
                ),
                "main"
            ),
            7
        );
    }

    #[test]
    fn a_nullary_newtype_is_a_unit_tag() {
        // A nullary single variant `(type Marker (The))` is a nominal Unit — `The` erases to unit (no box),
        // and `(match The ((The) …))` matches unconditionally.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type Marker (The)) \
                       (def (main) (match The ((The) 99))) (export main))"
                ),
                "main"
            ),
            99
        );
    }

    #[test]
    fn a_newtype_wrong_constructor_pattern_is_a_type_error() {
        // A `(Some x)` pattern over a `UserId` scrutinee names a constructor of ANOTHER type — a type
        // confusion the matcher must REJECT (CDZ0203), the nominal analogue of the boxed-sum
        // wrong-variant-pattern check (identity is by declaration occurrence).
        assert_eq!(
            reject_code(
                "(module m (type UserId (Mk Int64)) \
                   (def (main) (match (Mk 1) ((Some n) n))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn a_multi_payload_pattern_of_wrong_arity_is_rejected() {
        // A payload-arity mismatch — `(Mk a b c)` against a 2-payload `Mk` — names a nonexistent third
        // element; it must REJECT (CDZ0201), never bind `c` past the payload tuple (a wrong value / invalid
        // wasm). The correct-arity sibling compiles; only the over-arity pattern faults.
        assert_eq!(
            reject_code(
                "(module m (type Pair (Mk Int64 Int64)) \
                   (def (main (: n Int64)) (match (Pair.Mk n n) ((Pair.Mk a b c) (+ a b)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0201"),
            "an over-arity multi-payload pattern is a malformed destructure"
        );
    }

    /// Run a program whose RESULT escapes to the host as a resource (no bare func export), returning its
    /// rendered value form, or `None` if the runtime wasm is absent.
    fn run_heap_value_escape(src: &str) -> Option<String> {
        let bytes = component(src);
        let runtime = super::find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("escape run trapped: {t}"),
        }
    }

    /// The coded rejection a program produces, or `None` if it compiled. Used to pin a well-formedness
    /// rejection (CDZ code) rather than a silent miscompile.
    fn reject_code(src: &str) -> Option<String> {
        let out = compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::Wasm],
        );
        if out.artifact(Target::Wasm.artifact_kind()).is_some() {
            return None; // compiled — no rejection
        }
        out.diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .and_then(|d| d.code.clone())
    }

    /// The first error `Diagnostic` from compiling `src` (full record, so a test can read the carried
    /// fix), or `None` if `src` compiled clean.
    fn reject_full(src: &str) -> Option<crate::abi::Diagnostic> {
        let out = compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::Wasm],
        );
        if out.artifact(Target::Wasm.artifact_kind()).is_some() {
            return None;
        }
        out.diagnostics
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
    }

    #[test]
    fn a_non_exhaustive_scalar_match_offers_a_wildcard_arm_fix() {
        // An open Int scalar match with no wildcard is CDZ0210; it now carries an INSERT fix that appends
        // a `(_ unit)` wildcard arm (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
        // A Fix — the scalar twin of the sum add-arms fix).
        let d = reject_full("(module m (def (f (: n Int64)) (match n (0 1) (1 2))) (export f))")
            .expect("non-exhaustive must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0210"), "got: {}", d.message);
        assert!(
            d.message.contains("wildcard"),
            "names the gap: {}",
            d.message
        );
        let fix = d.fix.expect("a wildcard-arm fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::InsertInto);
        assert_eq!(fix.replacement, "(_ unit)", "appends a wildcard arm");
        assert!(!fix.verified, "the arm body is a placeholder → heuristic");
    }

    #[test]
    fn a_bool_match_missing_a_literal_offers_the_specific_missing_arm() {
        // A Bool scrutinee is a FINITE gap: missing `false` → name it AND insert exactly `(false unit)`
        // (not a generic wildcard), the same precision as a missing sum variant.
        let d = reject_full("(module m (def (main (: b Bool)) (match b (true 1))) (export main))")
            .expect("non-exhaustive must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0210"), "got: {}", d.message);
        assert!(
            d.message.contains("`false`") && d.message.contains("not covered"),
            "names the missing literal: {}",
            d.message
        );
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("(false unit)"),
            "inserts the specific missing arm: {}",
            d.message
        );
        // Symmetric: missing `true` → `(true unit)`.
        let d2 =
            reject_full("(module m (def (main (: b Bool)) (match b (false 2))) (export main))")
                .expect("reject");
        assert_eq!(
            d2.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("(true unit)"),
            "message: {}",
            d2.message
        );
    }

    #[test]
    fn an_annotation_mismatch_to_a_sum_offers_a_wrap_in_variant_fix() {
        // "try wrapping the expression in `Some`" (`spec/capabilities/diagnostics.md` §A Diagnostic
        // Carries A Route To A Fix): `(: n Option)` where `n : Int64` and `Option`'s `Some` carries an
        // Int64 → name + a WRAP fix. The `…` (WRAP_HOLE) marks where the original `n` goes.
        let d = reject_full(
            "(module m (type Option (Some Int64) None) \
               (def (f (: n Int64)) (: n Option)) (export f))",
        )
        .expect("annotation mismatch must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
        assert!(
            d.message.contains("wrap the value in `Some`"),
            "names the variant: {}",
            d.message
        );
        let fix = d.fix.expect("a wrap fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
        assert_eq!(
            fix.replacement,
            format!("(Some {})", crate::abi::WRAP_HOLE),
            "wraps the original in the Some ctor"
        );
        assert!(!fix.verified, "a wrap is a heuristic (intent guess)");
    }

    #[test]
    fn the_wrap_variant_is_derived_generically_from_the_user_sum_not_hardcoded() {
        // The ctor name comes from the DECLARATION, not a built-in `Some`/`Ok` — a user sum `Box` with a
        // single `Int64`-payload variant `Wrap` yields "wrap in `Wrap`" (no-keys-outside-the-prelude).
        let d = reject_full(
            "(module m (type Box (Wrap Int64)) (def (f (: n Int64)) (: n Box)) (export f))",
        )
        .expect("mismatch must reject");
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some(format!("(Wrap {})", crate::abi::WRAP_HOLE)).as_deref(),
            "message: {}",
            d.message
        );
    }

    #[test]
    fn an_annotation_mismatch_with_no_fitting_variant_carries_no_wrap() {
        // A sum whose variants are all NULLARY (or whose payloads don't match the value type) offers no
        // wrap — a wrap is suggested only when it would actually resolve the mismatch. `Flag` = On | Off.
        let d = reject_full(
            "(module m (type Flag On Off) (def (f (: n Int64)) (: n Flag)) (export f))",
        )
        .expect("mismatch must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0203"), "got: {}", d.message);
        assert!(
            !d.message.contains("wrap"),
            "no wrap suggestion: {}",
            d.message
        );
        assert!(d.fix.is_none(), "no fix carried: {:?}", d.fix);
    }

    #[test]
    fn a_bare_literal_past_int64_is_malformed_not_out_of_range() {
        // 01-literals "an out-of-range integer literal is a malformed literal": a BARE unannotated literal
        // that overflows the default signed `Int64` — `9223372036854775808` (Int64.max+1),
        // `0xFFFFFFFFFFFFFFFF` (fits unsigned-64, but a bare literal is signed) — is a MALFORMED literal
        // (CDZ0201), a well-formedness boundary, NOT unbound (CDZ0101) and NOT out-of-range-for-a-width
        // (CDZ0302, which is for an EXPLICIT annotation).
        assert_eq!(
            reject_code("(module m (def (main) 9223372036854775808) (export main))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            reject_code("(module m (def (main) 0xFFFFFFFFFFFFFFFF) (export main))").as_deref(),
            Some("CDZ0201")
        );
        // Int64.max EXACTLY fits — no rejection (it compiles).
        assert_eq!(
            reject_code("(module m (def (main) 9223372036854775807) (export main))"),
            None
        );
        // An EXPLICIT over-width annotation stays CDZ0302 (out of range for a CHOSEN width), NOT CDZ0201;
        // and a valid `UInt64.max` annotated literal is accepted (the annotation grounds the width, so the
        // bare-literal check does not misfire on it).
        assert_eq!(
            reject_code("(module m (def (main) (: 256 (Int 8))) (export main))").as_deref(),
            Some("CDZ0302")
        );
        assert_eq!(
            reject_code("(module m (def (main) (: 18446744073709551615 (UInt 64))) (export main))"),
            None
        );
    }

    #[test]
    fn a_constant_argument_is_range_checked_against_a_narrow_parameter_width() {
        // A CONSTANT argument passed to a NARROW-typed parameter must be range-checked against the
        // parameter's declared width, exactly as a direct `(: 200 Int8)` is — β-reduction now carries the
        // parameter's annotation onto the substituted argument (`(: arg T)`), so an out-of-range constant
        // is rejected CDZ0302 instead of being spliced raw and run to a value the type cannot hold. The
        // hole was `apply_lambda_uncached` keying substitution on the param NAME occurrence, which sees
        // THROUGH the `(: name T)` binder and discarded the annotation.
        // Out of range (200 > Int8.max 127) — a `def` call.
        assert_eq!(
            reject_code("(module m (def (f (: a Int8)) a) (def (main) (f 200)) (export main))")
                .as_deref(),
            Some("CDZ0302")
        );
        // A negative constant to an unsigned parameter — a sign UInt8 cannot hold at all.
        assert_eq!(
            reject_code("(module m (def (f (: a UInt8)) a) (def (main) (f -1)) (export main))")
                .as_deref(),
            Some("CDZ0302")
        );
        // An inline `fn` lambda shares the substitution path.
        assert_eq!(
            reject_code("(module m (def (main) ((fn ((: a Int8)) a) 200)) (export main))")
                .as_deref(),
            Some("CDZ0302")
        );
        // Arithmetic on an in-range constant arg that OVERFLOWS the width folds and is proven CDZ0302.
        assert_eq!(
            reject_code(
                "(module m (def (f (: a Int8)) (+ a a)) (def (main) (f 100)) (export main))"
            )
            .as_deref(),
            Some("CDZ0302")
        );
        // The annotated LET BINDER path range-checks its bound value the same way.
        assert_eq!(
            reject_code("(module m (def (main) (let (((: a Int8) 200)) a)) (export main))")
                .as_deref(),
            Some("CDZ0302")
        );
        // A COMPUTED out-of-range value under a binder annotation folds and is caught too.
        assert_eq!(
            reject_code("(module m (def (main) (let (((: a Int8) (+ 100 100))) a)) (export main))")
                .as_deref(),
            Some("CDZ0302")
        );
        // IN-RANGE constants are NOT over-rejected: `(f 127)` (boundary) and an in-range-fitting op
        // `(+ a 10)` = 110 both compile. A `(: a Bool) 5` still faults CDZ0203 (a genuine type clash,
        // not a width fault). And a bare (un-annotated) parameter is unaffected.
        assert_eq!(
            reject_code("(module m (def (f (: a Int8)) a) (def (main) (f 127)) (export main))"),
            None
        );
        assert_eq!(
            reject_code(
                "(module m (def (f (: a Int8)) (+ a 10)) (def (main) (f 100)) (export main))"
            ),
            None
        );
        assert_eq!(
            reject_code("(module m (def (main) (let (((: a Bool) 5)) a)) (export main))")
                .as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            reject_code("(module m (def (f a) (+ a 1)) (def (main) (f 41)) (export main))"),
            None
        );
    }

    #[test]
    fn an_argument_fault_is_reported_whether_or_not_the_parameter_is_used() {
        // The fault walk over a call `(f arg)` must catch a fault IN `arg` — an unbound name, a malformed
        // application — for BOTH a USED parameter (the argument is substituted into the reduced body, so
        // the body walk sees it) AND a DEAD parameter the body ignores (the argument is absent from the
        // reduced body, so it must be descended on its own). The redundant-descent elimination that made a
        // deep call chain's fault walk LINEAR (drop the raw-argument descent for a USED parameter, since
        // its argument is already in the reduced body) must NOT lose a DEAD argument's faults — those are
        // still descended (`param_is_referenced` is false → the argument is checked).
        // USED parameter — the fault surfaces via the reduced body.
        assert_eq!(
            reject_code("(module m (def (f a) (+ a 1)) (def (main) (f frobnicate)) (export main))")
                .as_deref(),
            Some("CDZ0101")
        );
        // DEAD parameter — the body ignores `a`, so the argument's fault is caught by descending the
        // (un-substituted) argument, not the reduced body. An unbound name and a malformed application.
        assert_eq!(
            reject_code("(module m (def (f a) 0) (def (main) (f frobnicate)) (export main))")
                .as_deref(),
            Some("CDZ0101")
        );
        assert_eq!(
            reject_code("(module m (def (f a) 0) (def (main) (f (5 3))) (export main))").as_deref(),
            Some("CDZ0201")
        );
        // A well-formed argument to a dead parameter still compiles (no over-rejection).
        assert_eq!(
            reject_code("(module m (def (f a) 0) (def (main) (f 99)) (export main))"),
            None
        );
    }

    #[test]
    fn under_applying_a_unary_variant_constructor_is_a_type_error() {
        // 09-functions "under-applying a unary constructor is a type error, not a fabricated unit
        // payload": `(Some)` applies the unary constructor to ZERO arguments — under-application, the
        // low-arity mirror of `(Some 1 2)`. A payload-carrying variant produces its value only when
        // applied to its argument (core-semantics.md §A Sum Type Constructor Is A Single-Arity Function),
        // so `(Some)` is CDZ0201 — NOT a decline (a fabricated `(Some unit)` would slip a value the
        // program never wrote past the payload check).
        assert_eq!(
            reject_code("(module m (def (main) (Some)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        // A NULLARY variant applied to nothing `(None)` is NOT under-applied — it has no payload, so it
        // CONSTRUCTS its value (used here as a match scrutinee, which types + runs).
        assert_eq!(
            reject_code(
                "(module m (def (main) (match (None) ((Some x) x) ((None _) 0))) (export main))"
            ),
            None
        );
        // A correctly-applied unary ctor `(Some 5)` compiles (the control).
        assert_eq!(
            reject_code(
                "(module m (def (main) (match (Some 5) ((Some x) x) ((None _) 0))) (export main))"
            ),
            None
        );
    }

    #[test]
    fn if_branches_of_distinct_numeric_type_are_cdz0201_but_cross_kind_stays_cdz0203() {
        // 02-binding "a conditional with integer and floating-point branches is a type error": two DISTINCT
        // NUMERIC branch types (Int64 vs Float64) are the no-silent-promotion rule (numeric-model.md
        // #Numeric Types Do Not Silently Promote) → a MALFORMED program (CDZ0201). Every OTHER branch
        // disagreement — a cross-KIND mismatch (Int vs Bool, a compound vs a scalar, tuples of different
        // arity) — is the structural type mismatch (CDZ0203). The split is by "both branches numeric".
        assert_eq!(
            reject_code("(module m (def (main) (if true 1 3.5)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        // Cross-kind: Int64 then vs Bool else — a structural mismatch, still CDZ0203.
        assert_eq!(
            reject_code("(module m (def (main) (if true 1 false)) (export main))").as_deref(),
            Some("CDZ0203")
        );
        // Cross-kind: two tuples of different arity — CDZ0203 (a tuple's arity is part of its type).
        assert_eq!(
            reject_code(
                "(module m (def (main) (if true (tuple 1 2) (tuple 3 4 5))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn a_binary_operator_with_no_operands_is_rejected_cdz0201() {
        // 07-type-system "a bare equality/arithmetic keyword is rejected, not a crash": a binary operator
        // applied to ZERO operands — `(=)` / `(+)` — is a MALFORMED application (the operator demands its
        // operands, the arity error `(+ 1)` already rejects), NOT the operator used as a value (which would
        // DECLINE "needs runtime closures", a to-do). Reject CDZ0201.
        for op in ["=", "+", "<", ">", "<=", ">=", "-", "*", "compare"] {
            let src = format!("(module m (def (main) ({op})) (export main))");
            assert_eq!(
                reject_code(&src).as_deref(),
                Some("CDZ0201"),
                "a zero-operand `({op})` must reject CDZ0201"
            );
        }
        // A well-formed binary application still compiles (the guard fires ONLY at zero args).
        assert_eq!(
            run_returns::<i64>(
                &component("(module m (def (main) (+ 2 3)) (export main))"),
                "main"
            ),
            5
        );
    }

    #[test]
    fn a_fixed_arity_list_pattern_binds_elements_of_a_constant_list() {
        // 05-compound-types "an element pattern matches a list by its length and elements": a FIXED-ARITY
        // `(list a b)` matches a constant list of that exact length, binding each position; the element
        // binders read the constant elements (a `SumPayload` `Elem(i)` fold — no β-substitution). A
        // wrong-length list falls to the wildcard; the empty `(list)` matches only the empty list.
        let run = |src: &str| run_returns::<i64>(&component(src), "main");
        assert_eq!(
            run("(module m (def (main) (match (list 7) ((list a) a) (_ 0))) (export main))"),
            7
        );
        assert_eq!(
            run(
                "(module m (def (main) (match (list 7 8) ((list a b) (+ a b)) (_ 0))) (export main))"
            ),
            15
        );
        assert_eq!(
            run("(module m (def (main) (match (list) ((list) 1) (_ 2))) (export main))"),
            1
        );
        // A list of the WRONG arity does not match the fixed pattern — falls through to the wildcard.
        assert_eq!(
            run(
                "(module m (def (main) (match (list 1 2 3) ((list a b) 99) (_ 42))) (export main))"
            ),
            42
        );
    }

    #[test]
    fn a_list_match_is_well_formed_or_declines() {
        // A list is OPEN (any length): a match with only fixed-arity arms and no catch-all is
        // NON-EXHAUSTIVE (CDZ0210) — a finite set of lengths cannot cover every list.
        assert_eq!(
            reject_code("(module m (def (main) (match (list 1 2) ((list a b) a))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
        // A malformed rest pattern — more than one binder after `..` — is CDZ0201 (a rest pattern is
        // `(list p… .. rest)`, exactly one tail binder).
        assert_eq!(
            reject_code("(module m (def (main) (match (list 1 2 3) ((list x .. r s) x) (_ 0))) (export main))")
                .as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn a_rest_list_pattern_matches_by_minimum_length_and_binds_leading_elements() {
        // 05-compound-types "an element pattern matches a list by its length and elements": a REST pattern
        // `(list p0 … p_{k-1} .. rest)` matches any list of length ≥ k, binding each LEADING position to
        // its element (a `SumPayload` `Elem(i)` fold over the constant list) — the rest binder captures the
        // tail (a sublist; unused here, so it stays inert). A `(list .. rest)` with zero leading binders is
        // a catch-all (length ≥ 0). The scrutinee folds, so the whole match folds to the selected arm.
        let run = |src: &str| run_returns::<i64>(&component(src), "main");
        // A rest pattern selects on length ≥ leading count; the leading binder reads element 0.
        assert_eq!(
            run(
                "(module m (def (main) (match (list 10 20 30) ((list) 0) ((list x .. rest) x))) (export main))"
            ),
            10
        );
        // The empty list matches `(list)`, not the rest pattern (which needs length ≥ 1 here).
        assert_eq!(
            run(
                "(module m (def (main) (match (list) ((list) 1) ((list a .. r) 2))) (export main))"
            ),
            1
        );
        // A non-empty list falls through `(list)` to the length-≥-1 rest arm.
        assert_eq!(
            run(
                "(module m (def (main) (match (list 5) ((list) 1) ((list a .. r) 2))) (export main))"
            ),
            2
        );
        // `(list .. rest)` with zero leading binders is a catch-all (matches every length).
        assert_eq!(
            run("(module m (def (main) (match (list 1 2 3) ((list .. all) 7))) (export main))"),
            7
        );
        assert_eq!(
            run("(module m (def (main) (match (list) ((list .. all) 7))) (export main))"),
            7
        );
        // Two leading binders read elements 0 and 1 past the minimum-length gate.
        assert_eq!(
            run(
                "(module m (def (main) (match (list 4 5 6 7) ((list a b .. rest) (+ a b)) (_ 0))) (export main))"
            ),
            9
        );
    }

    #[test]
    fn a_constant_string_scrutinee_selects_the_matching_string_literal_arm() {
        // 02-binding "matching on string literals": a STRING-literal pattern `("hello" …)` selects the arm
        // whose string EQUALS a constant scrutinee (by value, both NFC — the constant `String` equality
        // basis). The scrutinee folds (a literal, or a `String.concat`/`String.slice` that folds to a
        // constant), so the whole match folds to the selected arm — no runtime string equality op.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match \"hello\" (\"hello\" 1) (\"world\" 2) (_ 0))) (export main))"
                ),
                "main"
            ),
            1
        );
        // A later arm + fall-through to the wildcard.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match \"world\" (\"hello\" 1) (\"world\" 2) (_ 0))) (export main))"
                ),
                "main"
            ),
            2
        );
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match \"xyz\" (\"hello\" 1) (\"world\" 2) (_ 0))) (export main))"
                ),
                "main"
            ),
            0
        );
        // A scrutinee produced by a folding String op — `(String.concat \"a\" \"b\")` → \"ab\".
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match ((. String concat) \"a\" \"b\") (\"ab\" 100) (_ 200))) (export main))"
                ),
                "main"
            ),
            100
        );
    }

    #[test]
    fn a_char_literal_is_a_scalar_that_compares_by_scalar_value() {
        // 13-strings CHAR increment 1 (`collections-and-text.md` §A Char Is A Single Unicode Scalar
        // Value): a `#\c` literal is a `Ty::Char` constant whose value is its Unicode scalar. Equality is
        // scalar equality and ordering is the numeric order of the scalar, both folding at compile time to
        // a `Bool` (a char has no runtime machine slot this increment). A literal spelling a NON-scalar
        // (a surrogate `#\u+D800`) is a reader-detected lexical defect the compiler rejects CDZ0002.
        let run = |src: &str| run_returns::<bool>(&component(src), "main");
        // Equality by scalar value: two equal chars are equal, two distinct chars are not.
        assert!(run("(module m (def (main) (= #\\a #\\a)) (export main))"));
        assert!(!run("(module m (def (main) (= #\\a #\\b)) (export main))"));
        // Ordering by scalar value: 97 (`a`) < 98 (`b`).
        assert!(run("(module m (def (main) (< #\\a #\\b)) (export main))"));
        assert!(!run("(module m (def (main) (< #\\b #\\a)) (export main))"));
        // The `#\u+HHHH` code-point spelling names the same scalar (`#\u+0061` = `a`).
        assert!(run(
            "(module m (def (main) (= #\\u+0061 #\\a)) (export main))"
        ));
        // A named control char (`#\newline`) reads to its scalar (U+000A).
        assert!(run(
            "(module m (def (main) (= #\\newline #\\u+000A)) (export main))"
        ));
        // A surrogate code point is not a scalar — the literal is malformed (CDZ0002).
        assert_eq!(
            reject_code("(module m (def (main) #\\u+D800) (export main))").as_deref(),
            Some("CDZ0002")
        );
        // A code point past U+10FFFF is likewise not a scalar.
        assert_eq!(
            reject_code("(module m (def (main) #\\u+110000) (export main))").as_deref(),
            Some("CDZ0002")
        );
    }

    #[test]
    fn char_converts_to_and_from_an_integer_scalar_totally() {
        // 13-strings CHAR increment 2 (`collections-and-text.md` §A Char Converts To And From An Integer
        // Totally): `Char.to-int` is TOTAL (every char has an integer code point); `Char.from-int` is
        // FALLIBLE — `Some` for a Unicode scalar, `None` for a surrogate / out-of-range integer. Both
        // FOLD on a constant operand. `to-int` returns an Int64; the round-trip through a `Some` arm
        // returns a Bool — both observable as scalars.
        let run_i = |src: &str| run_returns::<i64>(&component(src), "main");
        let run_b = |src: &str| run_returns::<bool>(&component(src), "main");
        // to-int: the scalar value of `a` is 97.
        assert_eq!(
            run_i("(module m (def (main) ((. Char to-int) #\\a)) (export main))"),
            97
        );
        // round-trip: from-int 97 → Some #\a, and to-int of that char is 97 again.
        assert!(run_b(
            "(module m (def (main) (match ((. Char from-int) 97) ((Some c) (= ((. Char to-int) c) 97)) ((None _) false))) (export main))"
        ));
        // from-int of a SURROGATE (55296 = U+D800) → None → the match takes the None arm.
        assert!(!run_b(
            "(module m (def (main) (match ((. Char from-int) 55296) ((Some c) true) ((None _) false))) (export main))"
        ));
        // from-int of an OUT-OF-RANGE integer (1114112 = U+10FFFF+1) → None.
        assert!(!run_b(
            "(module m (def (main) (match ((. Char from-int) 1114112) ((Some c) true) ((None _) false))) (export main))"
        ));
        // from-int of the LAST scalar (U+10FFFF = 1114111) → Some (the boundary is inclusive).
        assert!(run_b(
            "(module m (def (main) (match ((. Char from-int) 1114111) ((Some c) true) ((None _) false))) (export main))"
        ));
    }

    #[test]
    fn a_string_match_is_well_formed_or_rejected() {
        // A String is an OPEN type (like Int64) — no finite literal set exhausts it, so a string match
        // MUST end in a wildcard `_`; without one it is non-exhaustive (CDZ0210).
        assert_eq!(
            reject_code(
                "(module m (def (main) (match \"hi\" (\"hi\" 1) (\"yo\" 2))) (export main))"
            )
            .as_deref(),
            Some("CDZ0210")
        );
        // A pattern whose type disagrees with the scrutinee — an Int literal against a String scrutinee —
        // is a shape/type error (CDZ0201), checked structurally before the fold.
        assert_eq!(
            reject_code("(module m (def (main) (match \"hi\" (5 1) (_ 0))) (export main))")
                .as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn an_unrecognized_string_escape_is_rejected_cdz0001() {
        // 01-literals "an unrecognized string escape is rejected": `"\q"` uses a backslash before `q`,
        // which begins none of the closed escape set (`\n \t \r \\ \"`), so the reader emits a
        // `Leaf::BadEscape` MARKER (it cannot report through its own stderr) that survives the
        // cadenza-syntax→rcdzc codec, and the COMPILER rejects it CDZ0001 — not silently reading `q`.
        assert_eq!(
            reject_code("(module m (def (main) \"\\q\") (export main))").as_deref(),
            Some("CDZ0001")
        );
        // A VALID escape is unaffected — `"\n"` reads to a newline `Str`, no marker, no rejection (it
        // compiles; the const-string escape renders it). Pin that only the CLOSED set is rejected.
        assert_eq!(
            reject_code("(module m (def (main) \"\\n\") (export main))"),
            None
        );
    }

    #[test]
    fn a_bare_constant_string_escapes_across_the_boundary() {
        // Once the reader's strict-escape marker is in place, a bare/matched CONSTANT string may cross
        // the host boundary via the resource escape (its bytes baked, the value form `(: "…" String)`) —
        // the same path a `(Some "hi")` payload uses, just for a top-level String export. Verified via a
        // composed run: the rendered value is the quoted text.
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!(
                "runtime wasm not found (run `cargo xtask build`); skipping const-string escape run"
            );
            return;
        };
        let bytes = component("(module m (def (main) \"hello\") (export main))");
        let opts = cdz_run::RunOpts {
            export: None, // a String escape is a RESOURCE component, auto-detected by cdz-run
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: \"hello\" String)",
                    "a bare constant string escapes + renders"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("const-string escape run trapped: {t}"),
        }
    }

    #[test]
    fn applying_a_non_function_is_a_coded_type_error() {
        // 09-functions "applying a non-function/boolean/float is a type error": a value that is NOT a
        // function has no defined result when applied (`core-semantics.md` §Applying A Function Binds Its
        // Parameter To Its Argument), so `(5 3)`/`(true 1)`/`(3.5 1)` MUST be REJECTED with a code
        // (CDZ0201 Malformed) — not DECLINED "value is not applyable" (a to-do). The head's type is a
        // definite non-function (Int64/Bool/Float64), so `check_application` faults before lowering.
        assert_eq!(
            reject_code("(module m (def (main) (5 3)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            reject_code("(module m (def (main) (true 1)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            reject_code("(module m (def (main) (3.5 1)) (export main))").as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn applying_an_applyable_head_is_not_flagged_as_a_non_function() {
        // The guard must NOT over-reject: a head that is applyable via a `(meta apply)` PRIMITIVE (the
        // `tuple`/`record`/`list` compound-value alias, a type ctor) has no type SCHEME but IS applyable,
        // so it must still COMPILE. `(tuple 1 2)` and `(list 1 2)` build their compounds (reject_code =
        // None = compiled); a record field read likewise. Pins that the non-function check excludes
        // meta-apply heads (the tuple/record/list regression the first cut of this check caused).
        assert_eq!(
            reject_code("(module m (def (main) (tuple 1 2)) (export main))"),
            None
        );
        assert_eq!(
            reject_code("(module m (def (main) (list 1 2)) (export main))"),
            None
        );
        assert_eq!(
            reject_code("(module m (def (main) (. (record (x 1) (y 2)) x)) (export main))"),
            None
        );
    }

    #[test]
    fn a_list_constructs_and_its_length_folds() {
        // `(list 1 2 3)` builds a homogeneous list; `List.len` of a compile-time-visible list folds to
        // its arity (no heap). `((. List len) (list 1 2 3))` → 3; the empty list → 0. Runtime elements
        // don't change the arity: `(List.len (list n 2))` still folds to 2 (length is static here).
        assert_eq!(
            run_returns::<i64>(
                &component("(module m (def (main) ((. List len) (list 1 2 3))) (export main))"),
                "main"
            ),
            3
        );
        assert_eq!(
            run_returns::<i64>(
                &component("(module m (def (main) ((. List len) (list))) (export main))"),
                "main"
            ),
            0
        );
    }

    #[test]
    fn a_self_tail_call_with_a_heap_match_argument_emits_valid_wasm() {
        // ⚠⚠ REGRESSION: a self-tail-recursive accumulator whose self-call passes a `match` over a HEAP
        // value (Option) as an ARGUMENT emitted a STRUCTURALLY INVALID wasm module ("expected i32, found
        // i64"). The self-tail-loop lowering pushes all args onto the operand stack simultaneously (the
        // parallel move into the param slots), yet each arg was emitted at the same scratch `base`: arg 0
        // `(- n 1)`'s overflow guard types slot `base` i64, and arg 1's non-reusable heap-match scrutinee
        // types the SAME slot i32 — one wasm local, two types, rejected. Fixed by advancing each arg's
        // scratch base past the running high-water so siblings hold disjoint slots. `f(5,0)` sums
        // 5+4+3+2+1 = 15. The match-as-OPERAND sibling (see next test) always worked (its i32 slot nests
        // above the arith's i64 slots); this pins the match sitting DIRECTLY in the tail-call arg.
        let src = "(module m \
            (def (f (: n Int64) (: acc Int64)) \
              (if (= n 0) acc (f (- n 1) (match (if (> n 0) (Some n) (None)) ((Some x) (+ acc x)) ((None) acc))))) \
            (def (main) (f 5 0)) (export main))";
        let bytes = component(src);
        wasmparser::validate(&bytes).expect("a self-tail-call heap-match arg must emit valid wasm");
        // The scrutinee is a RUNTIME `if`-produced Option (does not fold), so the program imports the
        // value-heap runtime — link + run it through `cdz_run` (like the runtime-if-list-len case).
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed self-tail-call heap-match run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(s, "15", "self-tail-call heap-match accumulate")
            }
            cdz_run::Outcome::Trap(t) => panic!("self-tail-call heap-match run trapped: {t}"),
        }
    }

    #[test]
    fn a_non_tail_call_with_a_heap_match_argument_emits_valid_wasm() {
        // The same scratch-slot collision existed on the NON-tail call path (`emit_call_args`): a call
        // whose args are `(- n 1)` (i64 arith-guard slot) then a heap-`match` (i32 scrutinee slot) shared
        // the scratch base. `g(a, m) = a + m`; `(g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0)))` →
        // 5 + 10 = 15. Companion of the self-tail-call case; guards the ordinary-call fix.
        let src = "(module m \
            (def (g (: a Int64) (: m Int64)) (+ a m)) \
            (def (main) (g (- 6 1) (match (Some 10) ((Some x) x) ((None) 0)))) (export main))";
        let bytes = component(src);
        wasmparser::validate(&bytes).expect("a non-tail-call heap-match arg must emit valid wasm");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 15);
    }

    #[test]
    fn a_runtime_if_list_is_length_measured_with_valid_wasm() {
        // A `(List a)` produced by a RUNTIME `if` (neither branch folds away) then measured by `List.len`
        // must emit VALID wasm and the right length. TWO bugs made it emit an invalid module: (1) a list
        // was not an `is_heap_type`, so the `if` took the scalar branchless-`select` path (a `select` on
        // an i32 handle is ill-formed); (2) `Core::ListLen` left `vec-len`'s i32 result on the stack where
        // `List.len : Int64` needs an i64. `(List.len (if (> b 0) (list 1 2 3) (list 4 5)))` → 3 for b>0,
        // 2 otherwise. The folded control (`a_list_constructs_and_its_length_folds`) never hits the runtime
        // path, so it validated even before the fix — this drives the emitted `Core::ListLen` + `if`-join.
        let src = "(module m (def (main (: b Int64)) \
                     (List.len (if (> b 0) (list 1 2 3) (list 4 5)))) (export main))";
        let bytes = component(src);
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed if-list-len run");
            return;
        };
        for (arg, want) in [("5", "3"), ("-1", "2")] {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "if-list-len b={arg}"),
                cdz_run::Outcome::Trap(t) => panic!("if-list-len run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_runtime_list_literal_bulk_builds_via_vec_of_arr() {
        // A runtime list literal `(list …)` builds in ONE bulk call: a flat `arr` (`arr-alloc` + a boxed
        // `arr-set` per element) then a single `vec-of-arr` — NOT `vec-empty` + N× consuming `vec-push`.
        // Asserted at the Lir level: exactly one `vec-of-arr`, N `arr-set`s, and ZERO `vec-push`/
        // `vec-empty`. (The list has a runtime element so it is not folded/baked away.) Behavioral value
        // parity is covered by the composed `List.at`/`List.len` runtime tests.
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let ast = crate::testkit::parse(
            "(module m (def (f (: a Int64) (: n Int64)) \
               (match (List.at (list a (+ a 1) (+ a 2)) n) ((Some x) x) (None -1))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
        let sig = db.defs[d].params.clone();
        let params: Vec<_> = sig
            .into_iter()
            .map(|p| {
                let b = db
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (b, crate::infer::type_of(&mut db, b))
            })
            .collect();
        let body = db.defs[d].body.expect("body");
        let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code;
        let count = |name| {
            code.iter()
                .filter(|i| matches!(i, Lir::CallImport(n) if *n == name))
                .count()
        };
        assert_eq!(count("vec-of-arr"), 1, "one bulk build, got: {code:?}");
        assert_eq!(count("arr-set"), 3, "three elements laid into the arr");
        assert_eq!(count("vec-push"), 0, "no per-element push chain");
        assert_eq!(count("vec-empty"), 0, "no vec-empty seed");
    }

    #[test]
    fn a_mixed_element_list_is_rejected() {
        // A list is HOMOGENEOUS (collections-and-text.md §A List Is A Homogeneous Sequence): every element
        // shares one type. `(list 1 true)` mixes Int64 and Bool — a type mismatch (CDZ0203), the same
        // class as `if` branches disagreeing. Pins the homogeneity check. (Used via `len` so `main`
        // returns a scalar, not a list — a list result has no boundary form yet, a separate decline.)
        assert_eq!(
            reject_code("(module m (def (main) ((. List len) (list 1 true))) (export main))")
                .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn a_list_annotation_checks_the_element_type() {
        // `(: (list 1 2) (List Bool))` — the value is a `List Int64`, the annotation says `List Bool`; the
        // element types conflict, so it is rejected (CDZ0203). Pins `(List T)` in type position + the
        // annotation element check. A matching annotation `(List Int64)` is transparent (folds to len 2).
        assert_eq!(
            reject_code("(module m (def (main) (: (list 1 2) (List Bool))) (export main))")
                .as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) ((. List len) (: (list 1 2) (List Int64)))) (export main))"
                ),
                "main"
            ),
            2
        );
    }

    #[test]
    fn bytes_of_constructs_and_its_length_folds() {
        // `(Bytes.of (list 0 255 128))` builds a byte sequence from a list of Int64 in 0..=255; `Bytes.len`
        // of a compile-time-visible `Bytes.of` folds to its byte count (no heap). → 3; the empty → 0.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) ((. Bytes len) ((. Bytes of) (list 0 255 128)))) (export main))"
                ),
                "main"
            ),
            3
        );
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) ((. Bytes len) ((. Bytes of) (list)))) (export main))"
                ),
                "main"
            ),
            0
        );
    }

    #[test]
    fn bytes_at_folds_some_and_none_over_a_constant() {
        // `Bytes.at : Bytes → Int64 → (Option Int64)` — the fallible byte read. A CONSTANT `Bytes.of`
        // indexed by a CONSTANT folds at compile time (no `bytes-get`): in range → `(Some byte)`, out of
        // range/negative/empty → `None`. Consumed by a match so `main` returns a scalar (the boundary
        // `(Some …)` render is exercised by the sum-escape path). `(Bytes.at (Bytes.of (list 10 20 30)) 1)`
        // → 20; index 5 / -1 (must NOT wrap unsigned) / the empty sequence → the None arm → -1.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match ((. Bytes at) ((. Bytes of) (list 10 20 30)) 1) ((Some x) x) (None -1))) (export main))"
                ),
                "main"
            ),
            20,
            "in-bounds → Some(byte)"
        );
        for (src, label) in [
            (
                "(module m (def (main) (match ((. Bytes at) ((. Bytes of) (list 10 20 30)) 5) ((Some x) x) (None -1))) (export main))",
                "over-large",
            ),
            (
                "(module m (def (main) (match ((. Bytes at) ((. Bytes of) (list 10 20 30)) -1) ((Some x) x) (None -1))) (export main))",
                "negative",
            ),
            (
                "(module m (def (main) (match ((. Bytes at) ((. Bytes of) (list)) 0) ((Some x) x) (None -1))) (export main))",
                "empty",
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(&component(src), "main"),
                -1,
                "out of bounds ({label}) → None"
            );
        }
    }

    #[test]
    fn a_runtime_bytes_at_reads_the_byte_under_wasmtime() {
        // The RUNTIME `Core::BytesAt` path (not a fold): the index is a PARAMETER, so `bytes-get` runs. A
        // recursive byte-sum over a runtime sequence via `Bytes.at` + match — the CBOR-reader idiom. `sum`
        // folds `Bytes.at bs i` for i in 0..len, adding each byte; `(Bytes.of (list 10 20 30))` sums to 60.
        // Out-of-bounds returns None → the base case (0), so the recursion terminates on the fallible read.
        let src = "(module m \
                     (def (sum bs i) (match ((. Bytes at) bs i) ((Some b) (+ b (sum bs (+ i 1)))) (None 0))) \
                     (def (main) (sum ((. Bytes of) (list 10 20 30)) 0)) (export main))";
        let bytes = component(src);
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed bytes-at run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => assert_eq!(s, "60", "runtime Bytes.at byte-sum"),
            cdz_run::Outcome::Trap(t) => panic!("runtime bytes-at run trapped: {t}"),
        }
    }

    #[test]
    fn bytes_at_of_a_runtime_element_widens_the_byte_to_the_option_payload() {
        // ⚠ INVALID WASM regression: `(Bytes.at (Bytes.of (list n)) 0)` with `n : UInt8` a RUNTIME element,
        // read at a CONSTANT index, mis-emitted. The `Bytes.at` fold saw a `Core::BytesOf` + constant
        // index and used the raw element occurrence as the `Some` payload — correct for a CONSTANT byte
        // (its core folds through the width), but a runtime `UInt8` element is an i32 value placed into the
        // i64 `Some(Int64)` payload UN-WIDENED → "expected i64, found i32". Fixed: fold to `Some` only for
        // a CONSTANT element; a runtime element falls through to the runtime `Core::BytesAt`, which
        // zero-extends the byte to the Int64 payload width. `n = 5` → the byte at 0 is 5.
        let src = "(module m (def (main (: n UInt8)) \
                     (match (Bytes.at (Bytes.of (list n)) 0) ((Some x) x) ((None _) -1))) (export main))";
        let bytes = component(src);
        wasmparser::validate(&bytes).expect("a runtime-element Bytes.at read must emit valid wasm");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping runtime-element bytes-at run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec!["5".to_string()],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(s, "5", "the runtime-stored byte reads back as 5")
            }
            cdz_run::Outcome::Trap(t) => panic!("runtime-element bytes-at run trapped: {t}"),
        }
    }

    #[test]
    fn runtime_bytes_concat_slice_compact_under_wasmtime() {
        // The runtime `Core::BytesConcat`/`BytesSlice`/`BytesCompact` paths (not folds): each threads a
        // byte sequence through a fn PARAMETER so the op actually runs, and reads a SCALAR out (via
        // `Bytes.len` / a `Bytes.at` match) so `main` returns without needing bytes escape.
        //   - concat: `(Bytes.len (concat a b))` = |a|+|b|. `bump` forces a runtime `a` (param).
        //   - slice:  in bounds → `(Some sub)`, its length read; out of bounds → `None` → -1.
        //   - compact: `(Bytes.len (compact b))` = |b| (content-equal, so same length).
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed bytes concat/slice/compact run");
            return;
        };
        let run = |src: &str| -> String {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&component(src), &opts).expect("run") {
                cdz_run::Outcome::Value(s) => s,
                cdz_run::Outcome::Trap(t) => panic!("run trapped: {t}"),
            }
        };
        // concat two runtime sequences (each built behind a fn so neither folds), measure the length.
        assert_eq!(
            run("(module m \
                   (def (mk n) ((. Bytes of) (list n 20 30))) \
                   (def (main) ((. Bytes len) ((. Bytes concat) (mk 10) (mk 40)))) (export main))"),
            "6",
            "concat length"
        );
        // slice in bounds: len 2 from start 1 of a runtime 4-byte sequence → Some(2-byte); its length = 2.
        assert_eq!(
            run("(module m \
                   (def (mk n) ((. Bytes of) (list n 20 30 40))) \
                   (def (main) (match ((. Bytes slice) (mk 10) 1 2) ((Some s) ((. Bytes len) s)) (None -1))) (export main))"),
            "2",
            "in-bounds slice length"
        );
        // slice out of bounds (start 3 + len 5 > 4) → None → -1.
        assert_eq!(
            run("(module m \
                   (def (mk n) ((. Bytes of) (list n 20 30 40))) \
                   (def (main) (match ((. Bytes slice) (mk 10) 3 5) ((Some s) ((. Bytes len) s)) (None -1))) (export main))"),
            "-1",
            "out-of-bounds slice → None"
        );
        // compact a runtime sequence, measure length (content-equal → unchanged length).
        assert_eq!(
            run("(module m \
                   (def (mk n) ((. Bytes of) (list n 20 30))) \
                   (def (main) ((. Bytes len) ((. Bytes compact) (mk 10)))) (export main))"),
            "3",
            "compact length"
        );
    }

    #[test]
    fn constant_bytes_slice_folds_to_some_of_the_sub_range() {
        // `(Bytes.slice (Bytes.of …) start len)` on constant operands FOLDS: an in-range slice → a
        // constant `Some(Bytes.of <sub>)` (its bytes then compare structurally via the constant-compound
        // equality fold), a zero-length slice → `Some(<empty>)` (present, not None), out-of-range → None.
        // Each returns a scalar (1/0 via `if (= …)`) so `main` needs no bytes escape. Was declined
        // "comparison of a compound value needs a heap walk" / the runtime slice path before the fold.
        for (body, want) in [
            // in-range: bytes [1,2] of (10 20 30 40) == (20 30)
            (
                "(module m (def (main) (if (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) \"x\") (Bytes.of (list 20 30))) 1 0)) (export main))",
                1,
            ),
            // zero-length in-range slice == empty
            (
                "(module m (def (main) (if (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 0) \"x\") (Bytes.of (list))) 1 0)) (export main))",
                1,
            ),
            // slice across a folded concat seam: [1,2] of (1 2 3 4) == (2 3)
            (
                "(module m (def (main) (if (= (Option.expect (Bytes.slice (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))) 1 2) \"x\") (Bytes.of (list 2 3))) 1 0)) (export main))",
                1,
            ),
            // a wrong expected sub-range → false (the fold really compares the bytes, not just Some-ness)
            (
                "(module m (def (main) (if (= (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40)) 1 2) \"x\") (Bytes.of (list 20 99))) 1 0)) (export main))",
                0,
            ),
            // out of range (start 2 + len 5 > 4) → None (not Some), so it is not the empty-Some
            (
                "(module m (def (main) (if (= (Bytes.slice (Bytes.of (list 10 20 30 40)) 2 5) (None unit)) 1 0)) (export main))",
                1,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "constant Bytes.slice folds: {body}"
            );
        }
    }

    #[test]
    fn a_byte_string_literal_is_a_constant_bytes_value() {
        // `b"…"` in source is a `Ty::Bytes` constant, equal to the `Bytes.of` of the same bytes — it
        // lowers to a `Core::BytesOf`, so it rides the constant equality/slice/concat fold. Named escapes
        // (`\n`), `\xNN` hex (incl. a high byte ≥ 0x80), and the empty literal all decode. Each returns a
        // scalar (1/0) via `if (= …)` so `main` needs no bytes escape.
        for (body, want) in [
            (
                "(module m (def (main) (if (= b\"ABC\" (Bytes.of (list 65 66 67))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= b\"\\x89PNG\" (Bytes.of (list 137 80 78 71))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= b\"A\\nB\" (Bytes.of (list 65 10 66))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= b\"\" (Bytes.of (list))) 1 0)) (export main))",
                1,
            ),
            // a differing byte → false (the literal really carries its bytes, not just Bytes-ness)
            (
                "(module m (def (main) (if (= b\"ABC\" (Bytes.of (list 65 66 99))) 1 0)) (export main))",
                0,
            ),
            // usable through the ordinary bytes ops: length of a literal
            (
                "(module m (def (main) (Bytes.len b\"ABCD\")) (export main))",
                4,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "byte-string literal is a constant Bytes: {body}"
            );
        }
    }

    #[test]
    fn a_constant_bin_construction_folds_to_its_encoded_bytes() {
        // `(bin <int-seg>…)` in expression position builds a Bytes; a constant folds to the exact bytes
        // (big-endian default, `le` reversed, signed two's-complement). Each equality rides the constant-
        // Bytes equality fold, returning a scalar 1/0 so `main` needs no bytes escape.
        for (body, want) in [
            // u16 258 = 0x0102 big-endian → [1,2]
            (
                "(module m (def (main) (if (= (bin (u16 258)) (Bytes.of (list 1 2))) 1 0)) (export main))",
                1,
            ),
            // le reverses → [2,1]
            (
                "(module m (def (main) (if (= (bin (u16 258 le)) (Bytes.of (list 2 1))) 1 0)) (export main))",
                1,
            ),
            // u32 magic → 4 big-endian bytes
            (
                "(module m (def (main) (if (= (bin (u32 0x89504E47)) (Bytes.of (list 137 80 78 71))) 1 0)) (export main))",
                1,
            ),
            // signed i8 -1 → 0xFF (two's complement), NOT a trap
            (
                "(module m (def (main) (if (= (bin (i8 -1)) (Bytes.of (list 255))) 1 0)) (export main))",
                1,
            ),
            // u64 258 → 8 big-endian bytes
            (
                "(module m (def (main) (if (= (bin (u64 258)) (Bytes.of (list 0 0 0 0 0 0 1 2))) 1 0)) (export main))",
                1,
            ),
            // empty (bin) → empty bytes; length of a fixed-width construction = sum of widths
            (
                "(module m (def (main) (if (= (bin) (Bytes.of (list))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (Bytes.len (bin (u32 0)))) (export main))",
                4,
            ),
            // a wrong expected value → false (the encoding really produces those bytes)
            (
                "(module m (def (main) (if (= (bin (u16 258)) (Bytes.of (list 2 1))) 1 0)) (export main))",
                0,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "constant bin construction folds: {body}"
            );
        }
    }

    #[test]
    fn a_constant_bin_bit_field_packs_msb_first() {
        // `(bits v k)` packs the low `k` bits of `v` MSB-first; a byte-closing run of bit-fields folds to
        // the packed bytes. `(bits 1 1)(bits 2 3)(bits 5 4)` = 1·010·0101 = 0b1010_0101 = 165. A wider
        // multi-byte field splits big-endian; a byte-aligned field composes with an int segment.
        for (body, want) in [
            (
                "(module m (def (main) (if (= (bin (bits 1 1) (bits 2 3) (bits 5 4)) (Bytes.of (list 165))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (bin (bits 4660 16)) (Bytes.of (list 18 52))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (bin (bits 255 8) (u8 1)) (Bytes.of (list 255 1))) 1 0)) (export main))",
                1,
            ),
            // a wrong expected packing → false
            (
                "(module m (def (main) (if (= (bin (bits 1 1) (bits 2 3) (bits 5 4)) (Bytes.of (list 164))) 1 0)) (export main))",
                0,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "constant bin bit-field packs: {body}"
            );
        }
    }

    #[test]
    fn a_bin_value_out_of_range_for_its_segment_is_a_provable_trap() {
        // A constant segment value that does not fit its width has no encoding → the build fails
        // (CDZ0304, the compile-provable companion of the runtime "binary value does not fit segment"
        // trap), rather than truncating. `(u8 256)` needs 9 bits; `(u8 -1)` is negative into unsigned. A
        // bit-field value wider than its width (`(bits 2 1)` — 2 needs 2 bits) is the sub-byte companion,
        // also a provable rejection (CDZ0220 mis-alignment or CDZ0304 fit — both refuse, never truncate).
        for src in [
            "(module m (def (main) (Bytes.len (bin (u8 256)))) (export main))",
            "(module m (def (main) (Bytes.len (bin (u8 -1)))) (export main))",
        ] {
            assert_eq!(reject_code(src).as_deref(), Some("CDZ0304"), "src: {src}");
        }
        // A byte-aligned bit-field whose value overflows its width is the CDZ0304 fit-trap (aligned, so
        // the well-formedness check passes and the value-fit check fires): `(bits 256 8)` needs 9 bits.
        assert_eq!(
            reject_code("(module m (def (main) (Bytes.len (bin (bits 256 8)))) (export main))")
                .as_deref(),
            Some("CDZ0304"),
        );
    }

    #[test]
    fn an_ill_formed_bin_form_is_rejected_cdz0220() {
        // Well-formedness is a compile-time check decidable from the segment list: bit-fields must close
        // to whole bytes, a non-final unsized `(bytes …)` is ill-formed, and a bits width must be a const.
        for src in [
            // bit-fields sum to 4 bits — not byte-aligned
            "(module m (def (main) (bin (bits 1 1) (bits 0 3))) (export main))",
            // an unsized bytes segment that is not the final segment
            "(module m (def (main) (bin (bytes (Bytes.of (list 1))) (u8 2))) (export main))",
        ] {
            assert_eq!(reject_code(src).as_deref(), Some("CDZ0220"), "src: {src}");
        }
    }

    #[test]
    fn a_constant_bin_pattern_decodes_and_binds_its_segments() {
        // A `(bin …)` PATTERN over a constant Bytes scrutinee: decode + bind each segment, dispatch on
        // literal segments, and match the WHOLE scrutinee (leftover / overrun → fall to the catch-all).
        for (body, want) in [
            // bind a fixed-width int (round-trips the construction): u16 258
            (
                "(module m (def (main) (match (bin (u16 258)) ((bin (u16 n)) n) (_ 0))) (export main))",
                258,
            ),
            // signed segment: byte 255 as i8 → -1; unsigned: 255
            (
                "(module m (def (main) (match (Bytes.of (list 255)) ((bin (i8 n)) n) (_ 0))) (export main))",
                -1,
            ),
            (
                "(module m (def (main) (match (Bytes.of (list 255)) ((bin (u8 n)) n) (_ 0))) (export main))",
                255,
            ),
            // little-endian read
            (
                "(module m (def (main) (match (bin (u16 258 le)) ((bin (u16 n le)) n) (_ 0))) (export main))",
                258,
            ),
            // bit-fields decode back: 0xA5 = 1·01·00101 → a=1,b=1,c=5 → 7
            (
                "(module m (def (main) (match (Bytes.of (list 165)) ((bin (bits a 1) (bits b 2) (bits c 5)) (+ (+ a b) c)) (_ 0))) (export main))",
                7,
            ),
            // a leading literal tag dispatches, then the field binds: [1,1,2] → 258
            (
                "(module m (def (main) (match (Bytes.of (list 1 1 2)) ((bin (u8 1) (u16 n)) n) (_ 0))) (export main))",
                258,
            ),
            // whole-scrutinee: leftover bytes → the bin arm does NOT match → catch-all 0
            (
                "(module m (def (main) (match (Bytes.of (list 1 2 3)) ((bin (u16 n)) n) (_ 0))) (export main))",
                0,
            ),
            // overrun: a u16 needs 2 bytes, input has 1 → non-match → 0
            (
                "(module m (def (main) (match (Bytes.of (list 5)) ((bin (u16 n) (bytes rest)) n) (_ 0))) (export main))",
                0,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "bin pattern decodes: {body}"
            );
        }
    }

    #[test]
    fn a_bytes_match_with_only_a_bin_arm_and_no_catch_all_is_non_exhaustive() {
        // A `(bin …)` pattern never covers every byte sequence (empty input, wrong length, an unequal
        // literal all fail), so a match whose only arm is a bin pattern is non-exhaustive → CDZ0210,
        // exactly as a sum match missing a variant.
        assert_eq!(
            reject_code("(module m (def (main) (match (Bytes.of (list 1 2)) ((bin (u16 n)) n))) (export main))")
                .as_deref(),
            Some("CDZ0210"),
        );
    }

    #[test]
    fn a_bin_dependent_size_segment_binds_exactly_n_bytes() {
        // `(bytes body n)` binds exactly the `n` bytes an earlier INT segment named; a trailing `(bytes
        // rest)` takes the remainder. Construction: `(bytes b)` splices `b`; `(bytes b n)` checks `|b|==n`.
        // Round-trip: construct a length-prefixed frame, match it back, rebuild. All constant → fold.
        for (body, want) in [
            // dependent body: n=2 → body=[10,20]
            (
                "(module m (def (main) (match (Bytes.of (list 2 10 20 99)) ((bin (u8 n) (bytes body n) (bytes rest)) (if (= body (Bytes.of (list 10 20))) 1 0)) (_ 0))) (export main))",
                1,
            ),
            // the trailing rest = [99]
            (
                "(module m (def (main) (match (Bytes.of (list 2 10 20 99)) ((bin (u8 n) (bytes body n) (bytes rest)) (if (= rest (Bytes.of (list 99))) 1 0)) (_ 0))) (export main))",
                1,
            ),
            // dependent size 0 → empty body, rest untouched
            (
                "(module m (def (main) (match (Bytes.of (list 0 42)) ((bin (u8 n) (bytes body n) (bytes rest)) (if (= body (Bytes.of (list))) 1 0)) (_ 0))) (export main))",
                1,
            ),
            // a dependent size overrunning the remainder → non-match → catch-all
            (
                "(module m (def (main) (match (Bytes.of (list 5 1 2)) ((bin (u8 n) (bytes body n) (bytes rest)) 1) (_ 0))) (export main))",
                0,
            ),
            // TLV round-trip: build (tag,len,payload), match it back, compare the payload
            (
                "(module m (def (main) (match (bin (u8 7) (u16 3) (bytes (Bytes.of (list 100 101 102)))) ((bin (u8 7) (u16 n) (bytes body n)) (if (= body (Bytes.of (list 100 101 102))) 1 0)) (_ 0))) (export main))",
                1,
            ),
            // length-prefixed construction: (u16 len) then the bytes → [0,3,10,20,30]
            (
                "(module m (def (main) (if (= (bin (u16 3) (bytes (Bytes.of (list 10 20 30)))) (Bytes.of (list 0 3 10 20 30))) 1 0)) (export main))",
                1,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "bin dependent-size: {body}"
            );
        }
        // A sized `(bytes b n)` construction whose value length differs from `n` → provable trap.
        assert_eq!(
            reject_code("(module m (def (main) (Bytes.len (bin (bytes (Bytes.of (list 1 2 3)) 2)))) (export main))")
                .as_deref(),
            Some("CDZ0304"),
        );
    }

    #[test]
    fn a_runtime_bin_construction_builds_and_range_checks_under_wasmtime() {
        // A `(bin …)` whose segment value is a genuine RUNTIME value (an EXPORTED boundary parameter, which
        // cannot fold) builds a Bytes on the rope heap at run time: `Core::BinBuild` allocs the buffer,
        // writes each segment's bytes big-endian (`le` reversed), and range-checks each value (trap if out
        // of range). `main` reads a SCALAR out (a `Bytes.len` or a match round-trip) so no escape is needed.
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping runtime bin construction run");
            return;
        };
        let run = |src: &str, args: &[&str]| -> cdz_run::Outcome {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: args.iter().map(|s| s.to_string()).collect(),
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            cdz_run::run(&component(src), &opts).expect("run")
        };
        let val = |src: &str, args: &[&str]| -> String {
            match run(src, args) {
                cdz_run::Outcome::Value(s) => s,
                cdz_run::Outcome::Trap(t) => panic!("unexpected trap: {t}"),
            }
        };
        // (Runtime bin MATCHING — decoding a runtime Bytes scrutinee — is a separate unbuilt slice, so
        // these read the built bytes back with `Bytes.len` / `Bytes.at`, not a `(bin …)` pattern.)
        // A runtime u16 built + measured: (bin (u16 n)) with n=258 → 2 bytes.
        assert_eq!(
            val(
                "(module m (def (main (: n Int64)) ((. Bytes len) (bin (u16 n)))) (export main))",
                &["258"]
            ),
            "2",
            "runtime u16 length"
        );
        // Big-endian byte order: (bin (u16 258)) = [0x01, 0x02]; read byte 0 → 1 (MSB first).
        assert_eq!(
            val(
                "(module m (def (main (: n Int64)) (match ((. Bytes at) (bin (u16 n)) 0) ((Some b) b) ((None _) -1))) (export main))",
                &["258"]
            ),
            "1",
            "runtime u16 big-endian MSB"
        );
        // Little-endian: (bin (u16 258 le)) = [0x02, 0x01]; byte 0 → 2.
        assert_eq!(
            val(
                "(module m (def (main (: n Int64)) (match ((. Bytes at) (bin (u16 n le)) 0) ((Some b) b) ((None _) -1))) (export main))",
                &["258"]
            ),
            "2",
            "runtime u16 little-endian LSB-first"
        );
        // Multi-segment: (u8 a)(u16 b) → 3 bytes.
        assert_eq!(
            val(
                "(module m (def (main (: a Int64) (: b Int64)) ((. Bytes len) (bin (u8 a) (u16 b)))) (export main))",
                &["7", "258"]
            ),
            "3",
            "runtime multi-segment length"
        );
        // A SIGNED i8 of -1 is IN range (two's complement) → byte 0xFF (255), no trap.
        assert_eq!(
            val(
                "(module m (def (main (: n Int64)) (match ((. Bytes at) (bin (i8 n)) 0) ((Some b) b) ((None _) -1))) (export main))",
                &["-1"]
            ),
            "255",
            "runtime signed i8 -1 → 0xFF"
        );
        // Range check: a runtime u8 given 256 has no 8-bit encoding → TRAP (does not truncate to 0).
        assert!(
            matches!(
                run(
                    "(module m (def (main (: n Int64)) ((. Bytes len) (bin (u8 n)))) (export main))",
                    &["256"]
                ),
                cdz_run::Outcome::Trap(_)
            ),
            "a runtime u8 of 256 must trap, not truncate"
        );
        // Range check: a runtime u8 given -1 (negative into unsigned) → TRAP.
        assert!(
            matches!(
                run(
                    "(module m (def (main (: n Int64)) ((. Bytes len) (bin (u8 n)))) (export main))",
                    &["-1"]
                ),
                cdz_run::Outcome::Trap(_)
            ),
            "a runtime u8 of -1 must trap"
        );
        // A `(bytes b)` splice of a RUNTIME Bytes value: `(bin (u16 3) (bytes b))` builds a length-prefixed
        // frame at run time (a `bytes-concat` of the header + the runtime body). `mk` builds a 3-byte body
        // from a runtime `n`; the frame length is 2 (header) + 3 (body) = 5. And byte 4 (= body[2]) is 30.
        let splice = "(module m (def (frame (: b Bytes)) (bin (u16 3) (bytes b))) (def (mk (: n Int64)) ((. Bytes of) (list (UInt8.wrap n) 20 30))) (def (main (: n Int64)) ";
        assert_eq!(
            val(
                &format!("{splice} ((. Bytes len) (frame (mk n)))) (export main))"),
                &["7"]
            ),
            "5",
            "runtime bytes-splice: length = header + body"
        );
        assert_eq!(
            val(
                &format!(
                    "{splice} (match ((. Bytes at) (frame (mk n)) 4) ((Some x) x) ((None _) -1))) (export main))"
                ),
                &["7"]
            ),
            "30",
            "runtime bytes-splice: body byte spliced after the header"
        );
        // RUNTIME BIT-FIELD packing: `(bits (UInt8.wrap n) 4) (bits 5 4)` packs the low nibble of `n` into
        // the HIGH nibble and constant 5 into the low nibble of one byte (MSB-first). n=10 → 0xA5 = 165.
        let nibble = "(module m (def (main (: n Int64)) (match ((. Bytes at) (bin (bits (UInt8.wrap n) 4) (bits 5 4)) 0) ((Some b) b) ((None _) -1))) (export main))";
        assert_eq!(
            val(nibble, &["10"]),
            "165",
            "runtime bit-field: (n<<4)|5, n=10"
        );
        assert_eq!(
            val(nibble, &["0"]),
            "5",
            "runtime bit-field: low nibble only, n=0"
        );
        assert_eq!(
            val(nibble, &["15"]),
            "245",
            "runtime bit-field: (15<<4)|5 = 0xF5"
        );
        // A runtime bit-field over its k-bit range TRAPS (a 4-bit field given 16 has no 4-bit encoding).
        assert!(
            matches!(
                run(
                    "(module m (def (main (: n Int64)) ((. Bytes len) (bin (bits (UInt8.wrap n) 4) (bits 5 4)))) (export main))",
                    &["16"]
                ),
                cdz_run::Outcome::Trap(_)
            ),
            "a runtime 4-bit field of 16 must trap, not truncate"
        );
        // A bit-field run spanning TWO bytes: three fields 8+4+4 = 16 bits = 2 bytes. `(bits n 8)(bits 2
        // 4)(bits 3 4)` → byte0 = n, byte1 = (2<<4)|3 = 0x23 = 35. n=200 → len 2, byte0=200, byte1=35.
        let two = "(module m (def (main (: n Int64)) ";
        assert_eq!(
            val(
                &format!(
                    "{two} ((. Bytes len) (bin (bits (UInt8.wrap n) 8) (bits 2 4) (bits 3 4)))) (export main))"
                ),
                &["200"]
            ),
            "2",
            "runtime bit-field run: two bytes"
        );
        assert_eq!(
            val(
                &format!(
                    "{two} (match ((. Bytes at) (bin (bits (UInt8.wrap n) 8) (bits 2 4) (bits 3 4)) 0) ((Some b) b) ((None _) -1))) (export main))"
                ),
                &["200"]
            ),
            "200",
            "runtime bit-field run: byte0 = the 8-bit field"
        );
        assert_eq!(
            val(
                &format!(
                    "{two} (match ((. Bytes at) (bin (bits (UInt8.wrap n) 8) (bits 2 4) (bits 3 4)) 1) ((Some b) b) ((None _) -1))) (export main))"
                ),
                &["200"]
            ),
            "35",
            "runtime bit-field run: byte1 = (2<<4)|3"
        );
        // A bit-field run COMPOSED with an int segment: `(bits n 4)(bits 1 4)(u8 42)` → byte0=(n<<4)|1, byte1=42.
        let mixed = "(module m (def (main (: n Int64)) ";
        assert_eq!(
            val(
                &format!(
                    "{mixed} (match ((. Bytes at) (bin (bits (UInt8.wrap n) 4) (bits 1 4) (u8 42)) 1) ((Some b) b) ((None _) -1))) (export main))"
                ),
                &["3"]
            ),
            "42",
            "runtime bit-field run then an int segment: the int byte follows"
        );
        assert_eq!(
            val(
                &format!(
                    "{mixed} (match ((. Bytes at) (bin (bits (UInt8.wrap n) 4) (bits 1 4) (u8 42)) 0) ((Some b) b) ((None _) -1))) (export main))"
                ),
                &["3"]
            ),
            "49",
            "runtime bit-field run then an int segment: (3<<4)|1 = 0x31 = 49"
        );
    }

    #[test]
    fn a_runtime_bin_match_decodes_a_runtime_bytes_scrutinee_under_wasmtime() {
        // A `(bin …)` PATTERN over a RUNTIME `Bytes` scrutinee (built from a boundary param, so it cannot
        // fold): the matcher probes `bytes-len == total & (each literal segment == its literal)` and, on a
        // match, binds each segment via `BinIntRead`. One bin arm + a catch-all (the supported shape). The
        // scrutinee here is `(bin (uNN n))` built from the param, then matched back — a runtime round-trip.
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping runtime bin match run");
            return;
        };
        let run = |src: &str, args: &[&str]| -> String {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: args.iter().map(|s| s.to_string()).collect(),
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&component(src), &opts).expect("run") {
                cdz_run::Outcome::Value(s) => s,
                cdz_run::Outcome::Trap(t) => panic!("run trapped: {t}"),
            }
        };
        // u16 round-trip: build (u16 n) at runtime, match it back → n.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (u16 n)) ((bin (u16 m)) m) (_ -9))) (export main))",
                &["258"]
            ),
            "258",
            "runtime u16 match round-trip"
        );
        // Signed i8: byte 0xFF decodes to -1 (two's complement).
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (i8 n)) ((bin (i8 m)) m) (_ -9))) (export main))",
                &["-1"]
            ),
            "-1",
            "runtime signed i8 match"
        );
        // Little-endian read matches the little-endian build.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (u16 n le)) ((bin (u16 m le)) m) (_ -9))) (export main))",
                &["258"]
            ),
            "258",
            "runtime le match round-trip"
        );
        // A leading LITERAL tag dispatches: tag 1 matches → decode the u16; else the catch-all.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (u8 1) (u16 n)) ((bin (u8 1) (u16 m)) m) (_ -9))) (export main))",
                &["300"]
            ),
            "300",
            "runtime literal-tag match"
        );
        // A mismatched literal tag (built 2, pattern wants 1) → the catch-all (-9), NOT a wrong decode.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (u8 2) (u16 n)) ((bin (u8 1) (u16 m)) m) (_ -9))) (export main))",
                &["300"]
            ),
            "-9",
            "runtime literal-tag mismatch → catch-all"
        );
        // Whole-scrutinee: a length mismatch (built 1 byte, pattern wants 2) → the catch-all.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) (match (bin (u8 n)) ((bin (u16 m)) m) (_ -9))) (export main))",
                &["5"]
            ),
            "-9",
            "runtime length mismatch → catch-all"
        );
        // MULTI-ARM dispatch: a leading literal tag selects among 2+ `(bin …)` arms (a chain of `if`s over
        // the runtime scrutinee). tag 1 → x ; tag 2 → y+1000 ; else → catch-all. Exercises the nested-if
        // scratch discipline (each arm's `BinIntRead` re-reads the materialized scrutinee, no slot clash).
        let m2 = "(module m (def (main (: t Int64) (: v Int64)) (match (bin (u8 t) (u16 v)) ((bin (u8 1) (u16 x)) x) ((bin (u8 2) (u16 y)) (+ y 1000)) (_ -1))) (export main))";
        assert_eq!(run(m2, &["1", "42"]), "42", "runtime multi-arm: tag 1");
        assert_eq!(run(m2, &["2", "42"]), "1042", "runtime multi-arm: tag 2");
        assert_eq!(
            run(m2, &["9", "42"]),
            "-1",
            "runtime multi-arm: no tag → catch-all"
        );
    }

    #[test]
    fn a_runtime_bin_match_binds_a_final_rest_bytes_segment_under_wasmtime() {
        // A `(bin …)` PATTERN ending in a FINAL UNSIZED `(bytes rest)` over a RUNTIME scrutinee: a fixed
        // int prefix (a header) then a variable-length tail. The length probe uses `bytes-len >= prefix`
        // (not `==`), the prefix binders read via `BinIntRead`, and the tail binds via `BinRestRead` —
        // `bytes-slice(scrutinee, prefix, len - prefix)`. We measure the tail's length so `main` returns a
        // scalar, proving the slice covers exactly the bytes after the header.
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping runtime bin rest match run");
            return;
        };
        let run = |src: &str, args: &[&str]| -> String {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: args.iter().map(|s| s.to_string()).collect(),
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&component(src), &opts).expect("run") {
                cdz_run::Outcome::Value(s) => s,
                cdz_run::Outcome::Trap(t) => panic!("run trapped: {t}"),
            }
        };
        // A 1-byte tag then a rest. Build `(bin (u8 5) (bytes payload))` from a runtime `payload` bytes; the
        // arm binds the tail and returns its length. Payload = the 3-byte `(list 1 2 3)` → rest length 3.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) \
                   (let ((payload (Bytes.of (list 1 2 3)))) \
                     (match (bin (u8 n) (bytes payload)) \
                       ((bin (u8 t) (bytes rest)) (Bytes.len rest)) \
                       (_ -9)))) (export main))",
                &["5"]
            ),
            "3",
            "runtime final-rest match: tail length"
        );
        // The rest is EMPTY when the scrutinee is exactly the header (len == prefix): `bytes-len >= 1`
        // still holds, and the tail slice is `[1, 0)` → an empty Bytes → length 0.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) \
                   (let ((payload (Bytes.of (list)))) \
                     (match (bin (u8 n) (bytes payload)) \
                       ((bin (u8 t) (bytes rest)) (Bytes.len rest)) \
                       (_ -9)))) (export main))",
                &["7"]
            ),
            "0",
            "runtime final-rest match: empty tail"
        );
        // A 2-byte header (u16 tag) then a rest: the tail starts at offset 2. Payload of 5 bytes → 5.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) \
                   (let ((payload (Bytes.of (list 9 8 7 6 5)))) \
                     (match (bin (u16 n) (bytes payload)) \
                       ((bin (u16 t) (bytes rest)) (Bytes.len rest)) \
                       (_ -9)))) (export main))",
                &["300"]
            ),
            "5",
            "runtime final-rest match: 2-byte header, 5-byte tail"
        );
        // A length TOO SHORT for even the fixed prefix (built 1 byte, prefix wants 2) → the catch-all,
        // NOT an out-of-range slice: the `>=` probe fails.
        assert_eq!(
            run(
                "(module m (def (main (: n Int64)) \
                   (match (bin (u8 n)) \
                     ((bin (u16 t) (bytes rest)) (Bytes.len rest)) \
                     (_ -9))) (export main))",
                &["5"]
            ),
            "-9",
            "runtime final-rest match: too short for prefix → catch-all"
        );
    }

    #[test]
    fn bytes_of_out_of_range_element_is_a_width_error() {
        // `Bytes.of : (List UInt8) → Bytes` — a byte IS a UInt8, so an element outside 0..=255 is not a
        // UInt8 and is rejected as an OUT-OF-RANGE WIDTH literal (CDZ0302), NOT a runtime trap: under the
        // UInt8 model the ill-typed byte cannot be constructed. 256 is too large, -1 negative. To truncate
        // a wider value into a byte, the program writes `(UInt8.wrap n)` explicitly (total, never traps).
        // (Used via `len` so `main` returns a scalar.)
        assert_eq!(
            reject_code(
                "(module m (def (main) ((. Bytes len) ((. Bytes of) (list 256)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0302")
        );
        assert_eq!(
            reject_code(
                "(module m (def (main) ((. Bytes len) ((. Bytes of) (list -1)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0302")
        );
    }

    #[test]
    fn a_runtime_if_bytes_is_length_measured_with_valid_wasm() {
        // A `Bytes` produced by a RUNTIME `if` (neither branch folds away) then measured by `Bytes.len`
        // must emit VALID wasm and the right length — the bytes analogue of the list `if`-len case. The
        // `Bytes.of` builds on the rope heap (`bytes-alloc`+`bytes-set`), the `if` joins two bytes handles,
        // and `Bytes.len` extends `bytes-len`'s i32 to the i64 `Bytes.len : Int64` needs.
        let src = "(module m (def (main (: b Int64)) \
                     (Bytes.len (if (> b 0) (Bytes.of (list 1 2 3)) (Bytes.of (list 4 5))))) (export main))";
        let bytes = component(src);
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed if-bytes-len run");
            return;
        };
        for (arg, want) in [("5", "3"), ("-1", "2")] {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "if-bytes-len b={arg}"),
                cdz_run::Outcome::Trap(t) => panic!("if-bytes-len run trapped: {t}"),
            }
        }
    }

    /// Compile `src`, assert it imports the value-heap runtime, then compose+run export `main` with the
    /// value-heap runtime and return the printed result — the "runs on the real heap" behavior check for
    /// the `vec-*` list ops (which never fold, so they always import the runtime). Skips (returns `None`)
    /// when the runtime wasm is not built.
    fn run_on_heap(src: &str) -> Option<String> {
        let bytes = component(src);
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime list op must import the value-heap runtime (genuine heap, not a fold)"
        );
        let runtime = super::find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("list heap run trapped (miscompile?): {t}"),
        }
    }

    #[test]
    fn a_list_push_extends_and_its_runtime_length_counts() {
        // `List.push(l, x)` appends `x`, building a new persistent list on the `vec-*` heap. To exercise
        // the RUNTIME `vec-push` (not the constant fold — a constant `(list …)` + `push` now folds to a
        // literal), the base list is BUILT at run time by a push-loop (`build 0 3 (list)` = `[0 1 2]`),
        // so `List.push` sees a runtime list handle; then `vec-len` counts 3 + 1 = 4.
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) ((. List len) ((. List push) (build 0 3 (list)) 9))) (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "4", "push then runtime length");
    }

    #[test]
    fn a_list_concat_joins_and_its_runtime_length_sums() {
        // `List.concat(a, b)` joins two lists into a new persistent one (`vec-concat`, consuming both). To
        // exercise the RUNTIME concat (a constant-list concat now folds), a RUNTIME-built list (`build 0 2`
        // = `[0 1]`) is concatenated with a constant `(list 3 4 5)`; a runtime operand forces
        // `Core::ListConcat`, so `vec-len` counts 2 + 3 = 5. Pins the runtime concat + length path.
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) ((. List len) ((. List concat) (build 0 2 (list)) (list 3 4 5)))) (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "5", "concat then runtime length");
    }

    #[test]
    fn a_list_push_type_mismatch_is_rejected() {
        // `List.push : ∀a. (List a) → a → (List a)` — the appended element must match the list's element
        // type. `(List.push (list 1 2) true)` pushes a Bool onto a `List Int64` — a mismatch (CDZ0203),
        // the same homogeneity class as a mixed literal. Pins the push type lambda's element unification.
        assert_eq!(
            reject_code(
                "(module m (def (main) ((. List len) ((. List push) (list 1 2) true))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn a_list_update_replaces_and_its_length_is_unchanged() {
        // `List.update(l, i, x)` replaces the element at index `i`, returning a new list of the SAME
        // length (a replacement, not a growth). To exercise the RUNTIME `vec-update` (a constant-list
        // update now folds), the list is BUILT at run time (`build 0 3` = `[0 1 2]`); `vec-len` of the
        // updated list is still 3. Pins the runtime `vec-update` path and that update preserves the count.
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) ((. List len) ((. List update) (build 0 3 (list)) 1 99))) (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "3", "update preserves length");
    }

    #[test]
    fn a_list_update_type_mismatch_is_rejected() {
        // `List.update : ∀a. (List a) → Int64 → a → (List a)` — the replacement element must match the
        // list's element type. `(List.update (list 1 2 3) 1 true)` puts a Bool where an Int64 was — a
        // non-homogeneous result (CDZ0203), the `List.update` companion of the push mismatch. Pins that
        // the update type lambda's element unification enforces homogeneity like push does.
        assert_eq!(
            reject_code(
                "(module m (def (main) ((. List len) ((. List update) (list 1 2 3) 1 true))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn constant_list_push_concat_update_fold_and_escape() {
        // A `List.push`/`concat`/`update` over COMPILE-TIME-VISIBLE list literals FOLDS to a constant
        // `(list …)` — no runtime heap — so the result escapes the boundary as its value form (like a
        // written literal). `List.at`/`len` of the folded list also fold; here each is measured by
        // `List.len` (a scalar Int64 across the boundary) to pin the folded length without the compound
        // escape ABI: push 3+1 = 4, concat 2+2 = 4, in-range update keeps length 3.
        for (prog, want) in [
            ("((. List push) (list 1 2 3) 4)", 4),
            ("((. List concat) (list 1 2) (list 3 4))", 4),
            ("((. List update) (list 10 20 30) 1 99)", 3),
        ] {
            let src = format!("(module m (def (main) ((. List len) {prog})) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "constant list op folds: {prog}"
            );
        }
        // An OUT-OF-RANGE constant `List.update` index is a PROVABLE trap — rejected at compile time
        // (CDZ0304), NOT a shipped runtime trap (matches the runtime `vec-update` OOB trap).
        assert_eq!(
            reject_code(
                "(module m (def (main) ((. List len) ((. List update) (list 1 2 3) 5 99))) (export main))"
            )
            .as_deref(),
            Some("CDZ0304"),
            "a constant out-of-range List.update is a provable trap (CDZ0304)"
        );
    }

    #[test]
    fn a_constant_list_index_folds_to_some_of_the_element() {
        // `List.at : ∀a. (List a) → Int64 → (Option a)` — the fallible indexed read. A CONSTANT list
        // literal indexed by a CONSTANT in-range index FOLDS to `(Some elem)` at compile time (no heap,
        // no `vec-get`): `(List.at (list 1 2 3) 1)` → `(Some 2)`. Consumed by a match here so `main`
        // returns a scalar — `(match (List.at (list 10 20 30) 1) ((Some x) x) (None -1))` = 20 — pinning
        // that the fold produces the right variant + payload. (The corpus case renders `(Some 20)` at
        // the boundary, which needs sum escape; the match-consumer form checks the fold without it.)
        let bytes = component(
            "(module m (def (main) (match ((. List at) (list 10 20 30) 1) ((Some x) x) (None -1))) (export main))",
        );
        assert_eq!(
            run_returns::<i64>(&bytes, "main"),
            20,
            "in-bounds → Some(element)"
        );
    }

    #[test]
    fn a_constant_list_index_out_of_bounds_folds_to_none() {
        // The absent side of the fold: an out-of-range CONSTANT index yields `None`
        // (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping). `(List.at (list 10
        // 20 30) 5)` → `None`; matched, the `None` arm is taken → -1. An over-large index (5), a NEGATIVE
        // index (-1, which MUST NOT wrap to a huge unsigned offset), and the empty list `(list)` all fold
        // to `None` — the three out-of-bounds shapes the corpus pins.
        for (src, label) in [
            (
                "(module m (def (main) (match ((. List at) (list 10 20 30) 5) ((Some x) x) (None -1))) (export main))",
                "over-large",
            ),
            (
                "(module m (def (main) (match ((. List at) (list 10 20 30) -1) ((Some x) x) (None -1))) (export main))",
                "negative",
            ),
            (
                "(module m (def (main) (match ((. List at) (list) 0) ((Some x) x) (None -1))) (export main))",
                "empty",
            ),
        ] {
            let bytes = component(src);
            assert_eq!(
                run_returns::<i64>(&bytes, "main"),
                -1,
                "out of bounds ({label}) → None"
            );
        }
    }

    #[test]
    fn a_constant_list_index_escapes_as_some_of_the_element() {
        // The corpus surface: `(List.at (list 1 2 3) 1)` returned as the program result renders `(: (Some
        // 2) (Option Int64))` — the fold produces a `Some` sum that crosses the host boundary through the
        // sum-escape resource path (task #11). Pins the end-to-end fallible-read → Option escape for a
        // constant list. Because the whole value is a COMPILE-TIME CONSTANT (`List.at` of a literal list
        // at a literal index folds to a `Some` `SumNew`), the escape bakes its bytes directly (the
        // constant `const_value_ast` path) and needs NO value-heap runtime — verified below. Run composed
        // (the bare-envelope resource `encode()` returns the baked bytes).
        let bytes = component("(module m (def (main) ((. List at) (list 1 2 3) 1)) (export main))");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_none(),
            "a CONSTANT Some escape bakes its bytes — no value-heap runtime import"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (Some 2) (Option Int64))",
                    "List.at escape renders Some(element)"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("List.at escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_multi_use_let_bound_list_is_built_once() {
        // A `let`-bound list used at MORE THAN ONE site is a runtime computation worth naming: it is
        // built ONCE (a flat `arr` + one `vec-of-arr`) and the handle reused, not rebuilt at every use.
        // Asserted at the Lir level: the emitted body contains exactly ONE `vec-of-arr` (the list-build
        // op) despite `xs` being read at two `List.at` sites. (Before the keep-binding fix a list binding
        // was not kept, so each use rebuilt it → two builds.)
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let ast = crate::testkit::parse(
            "(module m (def (f (: i Int64) (: j Int64)) \
               (let ((xs (list 10 20 30))) \
                  (match (List.at xs i) \
                    ((Some x) (match (List.at xs j) ((Some y) (+ x y)) (None x))) \
                    (None -1)))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
        let sig = db.defs[d].params.clone();
        let params: Vec<_> = sig
            .into_iter()
            .map(|p| {
                let b = db
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (b, crate::infer::type_of(&mut db, b))
            })
            .collect();
        let body = db.defs[d].body.expect("body");
        let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code;
        let builds = code
            .iter()
            .filter(|i| matches!(i, Lir::CallImport("vec-of-arr")))
            .count();
        assert_eq!(
            builds, 1,
            "a multi-use let-bound list is built once (one vec-of-arr), got: {code:?}"
        );
    }

    #[test]
    fn a_list_update_index_that_wraps_below_the_length_traps() {
        // SAFETY: `List.update`'s Int64 index is wrapped i64→i32 to feed the runtime's u32 index, but the
        // wrap discards the high 32 bits BEFORE the runtime's length check — so a huge index `>= 2^32`
        // that truncates below the length would silently update the WRONG element. `(List.update (list 1
        // 2 3) 2^32 99)` (index 4294967296 truncates to 0) must TRAP, not alias into slot 0. A high-bits
        // guard (trap if `(index as u64) >= 2^32`) precedes the wrap. An in-bounds index still updates;
        // a real OOB index still traps.
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping list-update OOB run");
            return;
        };
        let run = |src: &str| -> cdz_run::Outcome {
            let bytes = component(src);
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            cdz_run::run(&bytes, &opts).expect("run")
        };
        // The list is BUILT at run time (`build 0 3` = `[0 1 2]`) so `List.update` stays a RUNTIME
        // `vec-update` (a constant-list update folds — an OOB constant index is caught at COMPILE time as
        // CDZ0304, a separate path; this test pins the RUNTIME index guard). A helper wraps the shared
        // build-loop + update-len; each call passes a different runtime index.
        let prog = |idx: &str| {
            format!(
                "(module m \
                   (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
                   (def (main) (List.len (List.update (build 0 3 (list)) {idx} 99))) (export main))"
            )
        };
        // A wrapping OOB index (2^32 → 0) must TRAP, not silently update element 0.
        assert!(
            matches!(run(&prog("4294967296")), cdz_run::Outcome::Trap(_)),
            "an index that wraps below the length must trap, not alias into a valid slot"
        );
        // A real OOB index (no wrap) still traps.
        assert!(
            matches!(run(&prog("5")), cdz_run::Outcome::Trap(_)),
            "a genuinely out-of-bounds index must trap"
        );
        // NO OVER-TRAP: an in-bounds update still succeeds and preserves the length.
        assert!(
            matches!(run(&prog("2")), cdz_run::Outcome::Value(ref s) if s == "3"),
            "an in-bounds update must still succeed"
        );
    }

    /// Compile `src` (a single nullary-export program returning a compound), compose+run it, and return
    /// the host's rendered `(: value type)` string. Skips (returns `None`) when the runtime is not built.
    fn escape_render(src: &str) -> Option<String> {
        let bytes = component(src);
        let runtime = super::find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: None, // a resource-escape component (make/encode), not a bare function export
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("list escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_constant_list_escapes_to_the_host() {
        // A CONSTANT list returned as the program result crosses the host boundary through the SAME
        // resource-escape path a constant tuple/record takes: its length is statically known, so its
        // canonical value-form bytes bake into the resource module and `encode()` serves them (no runtime
        // heap walk). `(list 1 2 3)` renders `(: (list 1 2 3) (List Int64))` — the `(list …)` surface with
        // its element type in the type node. A list with a folded-through-β-reduction element (`(f 1)` =
        // `(list n 2 3)`) still bakes as a constant.
        let Some(out) = escape_render("(module m (def (main) (list 1 2 3)) (export main))") else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "(: (list 1 2 3) (List Int64))", "constant list escape");

        let Some(out) =
            escape_render("(module m (def (f n) (list n 2 3)) (def (main) (f 1)) (export main))")
        else {
            return;
        };
        assert_eq!(
            out, "(: (list 1 2 3) (List Int64))",
            "folded-arg list escape"
        );
    }

    #[test]
    fn a_char_from_int_option_escapes_and_renders() {
        // `Char.from-int` folds to a constant `(Option Char)` sum whose `Some` payload is a `ConstChar`;
        // it crosses the host boundary through the resource-escape path (the sum bakes, its char payload
        // as a `Leaf::Char`), and the host renders it `(Some #\a)` with the `(Option Char)` type node —
        // the char value form is `#\c`. `None` renders `(None unit)`.
        let Some(out) =
            escape_render("(module m (def (main) ((. Char from-int) 97)) (export main))")
        else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(
            out, "(: (Some #\\a) (Option Char))",
            "Char.from-int Some escape"
        );
        let Some(out) =
            escape_render("(module m (def (main) ((. Char from-int) 55296)) (export main))")
        else {
            return;
        };
        assert_eq!(
            out, "(: (None unit) (Option Char))",
            "Char.from-int None escape"
        );
    }

    #[test]
    fn string_scalar_at_reads_the_char_by_scalar_position() {
        // 13-strings CHAR increment 3 (`collections-and-text.md` §Reading A String's Scalar At A Position
        // Is Total): `String.scalar-at : String → Int64 → (Option Char)` reads the CHAR at a Unicode
        // SCALAR position — `Some #\c` in bounds, `None` out — the char-typed companion of `String.at`.
        // It addresses SCALAR values, not bytes: `"café"` scalar 3 is `é` (a 2-byte scalar), not a byte.
        // A constant string + index FOLDS to `(Option Char)`, crossing the boundary via the escape path.
        let Some(out) = escape_render(
            "(module m (def (main) ((. String scalar-at) \"hello\" 1)) (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "(: (Some #\\e) (Option Char))", "in-bounds scalar");
        // Scalar-not-byte: `café` = c,a,f,é (4 scalars); scalar 3 is `é`.
        let Some(out) = escape_render(
            "(module m (def (main) ((. String scalar-at) \"café\" 3)) (export main))",
        ) else {
            return;
        };
        assert_eq!(
            out, "(: (Some #\\é) (Option Char))",
            "multibyte scalar by position"
        );
        // Out of bounds → None.
        let Some(out) =
            escape_render("(module m (def (main) ((. String scalar-at) \"hi\" 5)) (export main))")
        else {
            return;
        };
        assert_eq!(out, "(: (None unit) (Option Char))", "out-of-bounds → None");
    }

    #[test]
    fn a_constant_bytes_escapes_and_renders_b_string() {
        // A CONSTANT `Bytes.of` returned as the program result crosses the host boundary through the
        // resource-escape path (like a constant list): its bytes are known, so they bake into the value
        // form as a `Leaf::Bytes` and `encode()` serves them; the host renders `b"…"` — printable ASCII
        // raw, else `\xNN`. `(Bytes.of (list 1 2 3))` (all non-printable) → `(: b"\x01\x02\x03" Bytes)`;
        // `(Bytes.of (list 65 66 67))` (printable) → `(: b"ABC" Bytes)`. (collections-and-text.md §the
        // observable form is the byte-string display.)
        let Some(out) =
            escape_render("(module m (def (main) (Bytes.of (list 1 2 3))) (export main))")
        else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(
            out, "(: b\"\\x01\\x02\\x03\" Bytes)",
            "non-printable bytes escape"
        );

        let Some(out) =
            escape_render("(module m (def (main) (Bytes.of (list 65 66 67))) (export main))")
        else {
            return;
        };
        assert_eq!(out, "(: b\"ABC\" Bytes)", "printable bytes escape");
    }

    #[test]
    fn a_runtime_built_bytes_escapes_via_the_looping_walker() {
        // L2b — the FIRST looping `encode()`. A RUNTIME-built `Bytes` (a `concat`/recursion result, NOT a
        // compile-time constant) escapes through the resource shape whose `encode()` LOOPS: it writes the
        // static value-form prefix, the runtime `bytes-len` as a LEB, a `bytes-get` copy loop, then the
        // static suffix (`DESIGN-runtime-bytes-escape-walker.md`). The canonical customer is the LEB128
        // encoder — `(uleb 624485)` = `E5 8E 26` (`b"\xe5\x8e&"`, byte 0x26 = `&`), built by recursion +
        // `Bytes.concat` + `(UInt8.wrap …)` on runtime values; and the base case `(uleb 100)` = `b"d"`.
        let uleb = "(def (uleb n) (if (< n 128) \
                        ((. Bytes of) (list ((. UInt8 wrap) n))) \
                        ((. Bytes concat) ((. Bytes of) (list ((. UInt8 wrap) (| (& n 127) 128)))) (uleb (>> n 7)))))";
        let Some(out) = escape_render(&format!(
            "(module m {uleb} (def (main) (uleb 624485)) (export main))"
        )) else {
            eprintln!("runtime wasm not found; skipping composed runtime-bytes-escape run");
            return;
        };
        assert_eq!(
            out, "(: b\"\\xe5\\x8e&\" Bytes)",
            "runtime LEB128 multibyte escape"
        );

        let out = escape_render(&format!(
            "(module m {uleb} (def (main) (uleb 100)) (export main))"
        ))
        .expect("runtime present");
        assert_eq!(out, "(: b\"d\" Bytes)", "runtime LEB128 single-byte escape");

        // A direct runtime concat (no recursion): (concat b"AB" b"C") built from runtime bytes → b"ABC".
        let out = escape_render(
            "(module m (def (mk n) ((. Bytes of) (list ((. UInt8 wrap) n) 66))) \
                       (def (main) ((. Bytes concat) (mk 65) ((. Bytes of) (list 67)))) (export main))",
        )
        .expect("runtime present");
        assert_eq!(out, "(: b\"ABC\" Bytes)", "runtime concat escape");
    }

    #[test]
    fn a_constant_list_of_tuples_escapes_nested() {
        // A list whose ELEMENTS are themselves compounds escapes with each element template nested inside
        // the `(list …)` — the type-directed renderer recurses through `List → Tuple`. `(list (tuple 1 2)
        // (tuple 3 4))` renders `(: (list (tuple 1 2) (tuple 3 4)) (List (Tuple Int64 Int64)))`. Pins that
        // a list's element type is carried into each element's sub-render (a heterogeneous nesting).
        let Some(out) =
            escape_render("(module m (def (main) (list (tuple 1 2) (tuple 3 4))) (export main))")
        else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(
            out, "(: (list (tuple 1 2) (tuple 3 4)) (List (Tuple Int64 Int64)))",
            "nested constant list-of-tuples escape"
        );
    }

    #[test]
    fn constant_string_equality_folds_to_a_bool() {
        // `= : ∀a. a → a → Bool` relates two strings by their text. A CONSTANT string equality folds at
        // compile time (no heap): `(= "hello" "hello")` → true, `(= "hello" "world")` → false, `(= "" "")`
        // → true (the empty string equals itself). Consumed by an `if` so `main` returns a scalar (1/0).
        // This is the string equality the compiler needs for instruction-tag / export-name dispatch.
        for (prog, want) in [
            ("(= \"hello\" \"hello\")", 1),
            ("(= \"hello\" \"world\")", 0),
            ("(= \"\" \"\")", 1),
        ] {
            let src = format!("(module m (def (main) (if {prog} 1 0)) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "string equality fold: {prog}"
            );
        }
    }

    #[test]
    fn constant_float_equality_folds_by_canonical_value() {
        // Two CONSTANT floats compare by their canonical Float64 value (contracts/deterministic-value-
        // form.md): `1e19` and `1e20` round to DIFFERENT doubles → false; `1e19` equals its own decimal
        // spelling `10000000000000000000.0` → true (same double); `-0.0` and `0.0` have DISTINCT bits →
        // false (the canonical form distinguishes them); `-0.0` equals `-0.0`. Consumed by an `if` so
        // `main` returns 1/0 — a Bool result, no float runtime. NESTED in a tuple folds the same way.
        for (prog, want) in [
            ("(= 1e19 1e20)", 0),
            ("(= 1e19 10000000000000000000.0)", 1),
            ("(= -0.0 0.0)", 0),
            ("(= -0.0 -0.0)", 1),
            ("(= 3.5 3.5)", 1),
            ("(= (tuple -0.0) (tuple 0.0))", 0),
            ("(= (tuple -0.0 1.0) (tuple -0.0 1.0))", 1),
        ] {
            let src = format!("(module m (def (main) (if {prog} 1 0)) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "float equality fold: {prog}"
            );
        }
    }

    #[test]
    fn nan_equality_follows_the_canonical_byte_form() {
        // 03-equality NaN cluster (`core-semantics.md` §Floating-Point Equality Follows The Canonical Byte
        // Form): `Float64.nan` is the canonical NaN VALUE (a module constant field, like `Int64.max`),
        // resolving to `Core::ConstFloatNan`. Every NaN shares one canonical byte form, so `(= nan nan)`
        // is TRUE (NOT wasm f64.eq, which says nan≠nan), and `nan` is UNEQUAL to any finite float. The
        // rule recurses through compounds (tuple/list/sum) via `const_compound_eq`. Consumed by an `if`
        // so `main` returns 1/0 — a Bool result.
        for (prog, want) in [
            ("(= Float64.nan Float64.nan)", 1),
            ("(= Float64.nan 1.0)", 0),
            ("(= 1.0 Float64.nan)", 0),
            ("(= (tuple Float64.nan) (tuple Float64.nan))", 1),
            ("(= (tuple Float64.nan) (tuple 1.0))", 0),
            ("(= (Some Float64.nan) (Some Float64.nan))", 1),
        ] {
            let src = format!("(module m (def (main) (if {prog} 1 0)) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "nan equality fold: {prog}"
            );
        }
    }

    #[test]
    fn a_cross_width_nan_comparison_is_a_type_error() {
        // A `nan` value carries its DECLARING float width — `Float64.nan` is a Float64, `Float32.nan` a
        // Float32 — so a cross-width comparison is the CDZ0301 no-silent-promotion error the identical
        // FINITE comparison gets (numeric-model.md §Numeric Types Do Not Silently Promote). The hole was
        // that `Prim::FloatNan` typed as a DEFERRED-width float, so a nan unified with EITHER width; the
        // fix annotates each `Float{32,64}.nan` field with its module's `(Float width)` in the prelude
        // (like `Int64.max` is `(: <lit> (Int 64))`), so the width unification fires as for a finite float.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m (def (main) {body}) (export main))");
            compile_component(&crate::codec::encode(&parse(&src)))
                .err()
                .and_then(|d| d.code)
        };
        // Cross-width nan comparisons — CDZ0301, exactly as the finite cross-width comparison is.
        assert_eq!(
            code("(= Float32.nan Float64.nan)").as_deref(),
            Some("CDZ0301")
        );
        assert_eq!(
            code("(= Float32.nan (: 1.5 Float64))").as_deref(),
            Some("CDZ0301")
        );
        assert_eq!(
            code("(= (: 1.5 Float32) Float64.nan)").as_deref(),
            Some("CDZ0301")
        );
        // The finite control rejects the same way (the behavior the nan path must match).
        assert_eq!(
            code("(= (: 1.5 Float32) (: 1.5 Float64))").as_deref(),
            Some("CDZ0301")
        );
        // NO over-rejection / no regression: a SAME-width nan comparison still compiles (and folds true),
        // and a nan vs a NON-float is still the cross-KIND CDZ0301 it was.
        assert_eq!(code("(= Float32.nan Float32.nan)"), None);
        assert_eq!(code("(= Float64.nan Float64.nan)"), None);
        assert_eq!(code("(= Float64.nan 5)").as_deref(), Some("CDZ0301"));
    }

    #[test]
    fn nan_crosses_the_boundary_as_a_canonical_f64_nan() {
        // `Float64.nan` returned as the program result emits an `f64.const` of the canonical NaN bits;
        // the export returns f64 and the boundary lifts it. Read back BY BITS: it IS a NaN (f64::NAN).
        let got = run_returns::<f64>(
            &component("(module m (def (main) Float64.nan) (export main))"),
            "main",
        );
        assert!(
            got.is_nan(),
            "Float64.nan crosses as an f64 NaN (got bits {:#x})",
            got.to_bits()
        );
    }

    #[test]
    fn a_float_literal_crosses_the_boundary_as_an_f64_value() {
        // A float literal `Core::ConstFloat` emits an `f64.const` of its canonical bits; the export
        // returns f64 (`valtype_of(Ty::Float) = F64`) and the boundary lifts it to the component `f64`
        // (`comp_valtype_of = COMP_F64`). Read back BY BITS so the canonical value is exact: `3.5`, a
        // large whole float NOT saturated to i64 (`1e19`), and `-0.0` DISTINCT from `0.0` all round-trip.
        for (prog, want_bits) in [
            ("3.5", 3.5f64.to_bits()),
            ("1e19", 1e19f64.to_bits()),
            ("-0.0", (-0.0f64).to_bits()),
            ("0.0", (0.0f64).to_bits()),
        ] {
            let src = format!("(module m (def (main) {prog}) (export main))");
            let got = run_returns::<f64>(&component(&src), "main");
            assert_eq!(
                got.to_bits(),
                want_bits,
                "float literal {prog} crosses as its canonical f64 (by bits)"
            );
        }
    }

    #[test]
    fn constant_float_arithmetic_folds_at_round_to_nearest_even() {
        // The float operators `+.`/`-.`/`*.`/`/.` fold two constant Float64 operands at round-to-nearest-
        // even (the fixed deterministic mode), the result crossing as its canonical f64 (numeric-model.md
        // §A Floating-Point Operation Uses A Floating-Point Operator; determinism contract). Read BY BITS
        // so the exact IEEE value is pinned — `0.1 +. 0.2` is the famous NON-exact 0.30000000000000004,
        // not 0.3, which a wrong (exact-decimal or f32) fold would miss.
        for (prog, want) in [
            ("(+. 0.1 0.2)", 0.1f64 + 0.2f64),
            ("(*. 6.0 7.0)", 42.0f64),
            ("(-. 5.5 2.0)", 3.5f64),
            ("(/. 1.0 4.0)", 0.25f64),
            ("(/. 1.0 3.0)", 1.0f64 / 3.0f64),
        ] {
            let src = format!("(module m (def (main) {prog}) (export main))");
            let got = run_returns::<f64>(&component(&src), "main");
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "float fold {prog} must round-trip its exact f64 (by bits)"
            );
        }
    }

    #[test]
    fn decimal_from_f64_round_trips_by_bits() {
        // The float-fold's result representation: `Decimal::from_f64(f)` builds a decimal whose
        // `to_f64_bits()` returns EXACTLY `f`'s bits (via shortest round-tripping formatting), so a
        // computed float crosses the boundary unchanged. Covers the non-exact 0.1+0.2, a large whole
        // float (no i64 saturation), a tiny value, and signed zero. A non-finite input yields `None`.
        use crate::ast::Decimal;
        for f in [
            0.1f64 + 0.2f64,
            42.0,
            1e19,
            1.0 / 3.0,
            -0.0,
            0.0,
            5e-324, // smallest subnormal
        ] {
            let d = Decimal::from_f64(f).expect("finite float has a decimal form");
            assert_eq!(
                d.to_f64_bits(),
                f.to_bits(),
                "Decimal::from_f64({f}) must round-trip by bits"
            );
        }
        assert!(Decimal::from_f64(f64::INFINITY).is_none());
        assert!(Decimal::from_f64(f64::NAN).is_none());
    }

    #[test]
    fn a_recognized_string_escape_denotes_its_one_scalar_value() {
        // A string literal's escapes are expanded by the reader before the compiler sees the text, so
        // `"\t"` is the one-scalar string containing a tab — EQUAL to a literal tab in the source
        // (01-literals §'a recognized string escape denotes its one scalar value'). `(= "\t" <tab>)`
        // folds to true, pinning that the escape and the raw character are the same string value.
        let src = "(module m (def (main) (if (= \"\\t\" \"\t\") 1 0)) (export main))";
        assert_eq!(
            run_returns::<i64>(&component(src), "main"),
            1,
            "an escape and its literal scalar are the same string"
        );
    }

    #[test]
    fn string_scalar_len_folds_to_the_unicode_scalar_count() {
        // `String.scalar-len` counts UNICODE SCALAR VALUES (`collections-and-text.md` §A String Offers
        // Both A Scalar Length And A Byte Length). On a CONSTANT string it folds to an `Int64` (no heap):
        // `"hello"` = 5, `"café"` = 4 (é is ONE scalar though two UTF-8 bytes), `"😀"` = 1 (one
        // supplementary-plane scalar, four bytes / two UTF-16 units). Pins the scalar count, distinct
        // from the byte count on a multi-byte string.
        for (s, want) in [("hello", 5), ("café", 4), ("😀", 1)] {
            let src = format!("(module m (def (main) (String.scalar-len \"{s}\")) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "scalar-len {s:?}"
            );
        }
    }

    #[test]
    fn string_byte_len_folds_to_the_utf8_byte_count() {
        // `String.byte-len` counts the UTF-8 BYTES — the byte companion of `scalar-len`, differing exactly
        // on a multi-byte string. `"hello"` = 5 (ASCII, coincides with scalar-len), `"café"` = 5 (é is two
        // bytes → 5 bytes vs 4 scalars), `"😀"` = 4 (four bytes vs one scalar). Folds to an `Int64`.
        for (s, want) in [("hello", 5), ("café", 5), ("😀", 4)] {
            let src = format!("(module m (def (main) (String.byte-len \"{s}\")) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "byte-len {s:?}"
            );
        }
    }

    #[test]
    fn string_concat_folds_two_constant_strings() {
        // `String.concat : String → String → String` — the total binary join. On two CONSTANT ASCII
        // strings it FOLDS to their concatenation (`collections-and-text.md` §Strings Concatenate). The
        // result is another constant `String`; consumed here by constant string equality so `main`
        // returns a Bool (no string escape needed). Covers the general join and BOTH identity edges (an
        // empty operand on the left / right changes nothing).
        for (a, b, want) in [
            ("hello", " world", "hello world"),
            ("hi", "", "hi"),
            ("", "hi", "hi"),
            ("", "", ""),
        ] {
            let src = format!(
                "(module m (def (main) (if (= (String.concat \"{a}\" \"{b}\") \"{want}\") 1 0)) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                1,
                "String.concat {a:?} ++ {b:?} = {want:?}"
            );
        }
    }

    #[test]
    fn string_to_bytes_folds_to_the_utf8_bytes() {
        // `String.to-bytes : String → Bytes` — the UTF-8 encoding. A CONSTANT string FOLDS to a constant
        // `Bytes` of its UTF-8 bytes, consumed here by `Bytes.len` → the byte count. `"run"` = 3 ASCII
        // bytes; `"café"` = 5 (é is two UTF-8 bytes); `"😀"` = 4; `""` = 0. Agrees with `String.byte-len`.
        for (s, want) in [("run", 3), ("café", 5), ("😀", 4), ("", 0)] {
            let src = format!(
                "(module m (def (main) ((. Bytes len) ((. String to-bytes) \"{s}\"))) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "String.to-bytes {s:?} byte count"
            );
        }
    }

    #[test]
    fn string_from_bytes_decodes_utf8_totally() {
        // `String.from-bytes : Bytes → (Option String)` — the TOTAL UTF-8 decode. A constant `Bytes.of`
        // FOLDS by strict UTF-8: well-formed → `(Some s)`, ill-formed/overlong/surrogate → `None`, never a
        // trap. Consumed by a match to a scalar. `[99 97 102 195 169]` = "café" → Some (matched to 1);
        // `[255]` invalid, `[192 128]` overlong C0 80, `[237 160 128]` surrogate ED A0 80 → None (→ -1).
        for (bytes, want) in [
            ("99 97 102 195 169", 1), // café — well-formed
            ("255", -1),              // 0xFF — invalid
            ("192 128", -1),          // C0 80 — overlong NUL
            ("237 160 128", -1),      // ED A0 80 — surrogate U+D800
        ] {
            let src = format!(
                "(module m (def (main) (match (String.from-bytes (Bytes.of (list {bytes}))) \
                   ((Some _) 1) ((None _) -1))) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "String.from-bytes [{bytes}] decode"
            );
        }
    }

    #[test]
    fn a_string_annotation_checks_against_a_string_value() {
        // `String` in type position (`(: "hi" String)`) decodes to `Ty::String` (`resolve::decode_ty`) —
        // transparent over a string value, but a MISMATCH is rejected: `(: "hi" Int64)` conflicts the
        // string value with the Int64 annotation (CDZ0203). Pins `String` as an annotation type + the
        // String-vs-scalar mismatch (the string counterpart of `(: 5 Bool)`).
        assert_eq!(
            reject_code("(module m (def (main) (String.byte-len (: \"hi\" Int64))) (export main))")
                .as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn string_at_folds_to_some_of_the_scalar_in_bounds() {
        // `String.at : String → Int64 → (Option String)` — the fallible SCALAR-indexed read. A constant
        // string + constant in-range index FOLDS to `(Some "<char>")` at compile time — indexed by
        // UNICODE SCALAR position, NOT byte offset (`"café" [3]` = "é", though é is bytes 3–4 of the
        // 5-byte UTF-8; `"😀b" [1]` = "b", though 😀 is 4 bytes / 2 UTF-16 units). Consumed by a match to
        // a scalar (constant string equality) so `main` returns a Bool — fully foldable, no string escape.
        for (s, i, want) in [("hello", 1, "e"), ("café", 3, "é"), ("😀b", 1, "b")] {
            let src = format!(
                "(module m (def (main) (if (= (match (String.at \"{s}\" {i}) ((Some c) c) ((None _) \"\")) \"{want}\") 1 0)) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                1,
                "String.at {s:?}[{i}] = {want:?}"
            );
        }
    }

    #[test]
    fn string_at_out_of_range_folds_to_none() {
        // An out-of-range constant index yields `None` (collections-and-text.md #Indexing And Lookup Are
        // Fallible, Not Trapping): at/beyond the scalar length (`"hi" [5]`) and a NEGATIVE index (`"hi"
        // [-1]`, which MUST NOT wrap to a huge offset) both take the `None` arm → -1. The String companion
        // of the List.at / Bytes.at out-of-bounds Nones.
        for (s, i) in [("hi", 5), ("hi", -1), ("", 0)] {
            let src = format!(
                "(module m (def (main) (match (String.at \"{s}\" {i}) ((Some c) 1) ((None _) -1))) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                -1,
                "String.at {s:?}[{i}] out of range → None"
            );
        }
    }

    #[test]
    fn string_slice_folds_in_range_to_some_of_the_substring() {
        // `String.slice : String → Int64 → Int64 → (Option String)` — the fallible SCALAR sub-range read
        // `[start, end)`. A constant string + constant in-range bounds FOLD to `(Some "<substr>")`,
        // indexed by UNICODE SCALAR position, NOT byte offset (`"héllo" [0,2)` = "hé", though é is two
        // UTF-8 bytes). `start == end` is an in-range EMPTY slice (Some ""), not None. Consumed by a match
        // + constant string equality so `main` returns a Bool — fully foldable.
        for (s, a, b, want) in [
            ("hello world", 0, 5, "hello"),
            ("hello", 1, 4, "ell"),
            ("héllo", 0, 2, "hé"),
            ("hello", 2, 2, ""),
        ] {
            let src = format!(
                "(module m (def (main) (if (= (match (String.slice \"{s}\" {a} {b}) ((Some x) x) ((None _) \"!\")) \"{want}\") 1 0)) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                1,
                "String.slice {s:?}[{a},{b}) = {want:?}"
            );
        }
    }

    #[test]
    fn string_slice_out_of_range_folds_to_none() {
        // A range outside `0 <= start <= end <= scalar-len` has no defined substring → `None`
        // (collections-and-text.md #Indexing And Lookup Are Fallible, Not Trapping): end beyond the length
        // (`"hi" [0,5)`), a REVERSED range (`"hello" [3,1)`), and a NEGATIVE start (`"hello" [-1,3)`, which
        // MUST NOT wrap to a huge unsigned offset) all take the `None` arm → -1.
        for (s, a, b) in [("hi", 0, 5), ("hello", 3, 1), ("hello", -1, 3)] {
            let src = format!(
                "(module m (def (main) (match (String.slice \"{s}\" {a} {b}) ((Some x) 1) ((None _) -1))) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                -1,
                "String.slice {s:?}[{a},{b}) out of range → None"
            );
        }
    }

    #[test]
    fn option_expect_folds_a_constant_present_variant_to_its_payload() {
        // `Option.expect`/`Result.expect` on a compile-time-visible PRESENT variant FOLDS to its payload
        // (core-semantics.md §Requiring The Value Of An Optional Traps On Absence — the present branch).
        // A constant `(Some 7)` / `(Ok 99)` unwraps at compile time (no heap, no runtime probe); the
        // message is discarded. Consumed to a scalar so `main` returns it directly.
        for (prog, want) in [
            ("(Option.expect (Some 7) \"m\")", 7),
            ("(Result.expect (Ok 99) \"m\")", 99),
        ] {
            let src = format!("(module m (def (main) {prog}) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "expect folds constant present: {prog}"
            );
        }
    }

    #[test]
    fn checked_arithmetic_folds_to_some_in_range_and_none_on_overflow() {
        // `Int64.checked-add`/`checked-mul` — the FALLIBLE arithmetic (numeric-model.md §Overflow Is
        // Defined). A constant operand pair FOLDS: in range → `(Some result)`, overflow → `(None unit)`.
        // Consumed by a match so `main` returns a scalar (the Some payload or a -1 sentinel).
        for (prog, want) in [
            ("(Int64.checked-add 20 22)", 42),       // fits
            ("(Int64.checked-add Int64.max 1)", -1), // overflows → None
            ("(Int64.checked-mul 6 7)", 42),         // fits
            ("(Int64.checked-mul Int64.max 2)", -1), // overflows → None
        ] {
            let src = format!(
                "(module m (def (main) (match {prog} ((Some v) v) ((None _) -1))) (export main))"
            );
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "checked arithmetic fold: {prog}"
            );
        }
    }

    #[test]
    fn wrapping_arithmetic_folds_and_runs_with_two_s_complement_wraparound() {
        // `Int64.wrapping-add`/`wrapping-mul` — two's-complement wraparound, NEVER trapping (numeric-
        // model.md §Overflow Is Defined). A constant pair FOLDS via `wrapping_*`; the result is a bare
        // Int64 (no Option). `MAX + 1` wraps to MIN, `MAX * 2` wraps to -2, in-range equals `+`/`*`.
        let min = i64::MIN; // -9223372036854775808
        for (prog, want) in [
            ("(Int64.wrapping-add 20 22)", 42),
            ("(Int64.wrapping-add Int64.max 1)", min),
            ("(Int64.wrapping-mul 6 7)", 42),
            ("(Int64.wrapping-mul Int64.max 2)", -2),
        ] {
            let src = format!("(module m (def (main) {prog}) (export main))");
            assert_eq!(
                run_returns::<i64>(&component(&src), "main"),
                want,
                "wrapping arithmetic fold: {prog}"
            );
        }
        // The RUNTIME path: `(w a b) = (Int64.wrapping-add a b)` over PARAMETERS emits the raw `i64.add`
        // (no overflow guard), so `(w Int64.max 1)` wraps to MIN rather than trapping — the same result
        // as the fold, proving wrapping is the raw op, not the checked/trapping one.
        let src = "(module m (def (w a b) (Int64.wrapping-add a b)) \
                    (def (main) (w Int64.max 1)) (export main))";
        assert_eq!(
            run_returns::<i64>(&component(src), "main"),
            min,
            "runtime wrapping-add wraps at run time"
        );
    }

    #[test]
    fn wrapping_arithmetic_identities_elide_the_op() {
        // Wrapping ops have the SAME algebraic identities as checked `+`/`*` (the wrap is total, so the
        // fold is value-identical): `a +% 0 = a`, `a *% 1 = a`, `a *% 0 = 0`. Over a RUNTIME operand the
        // whole op is elided — `(Int64.wrapping-add a 0)` becomes just a local read, no `i64.add`.
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f (: a Int64)) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let params: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        // `a +% 0` and `a *% 1` → just the operand; no arithmetic op survives.
        assert_eq!(
            lir("(Int64.wrapping-add a 0)"),
            vec![Lir::LocalGet(0)],
            "a +% 0 elides to a"
        );
        assert_eq!(
            lir("(Int64.wrapping-mul a 1)"),
            vec![Lir::LocalGet(0)],
            "a *% 1 elides to a"
        );
        // `a *% 0` → the constant 0 (the operand `a` is trap-free, so it is dropped).
        assert_eq!(
            lir("(Int64.wrapping-mul a 0)"),
            vec![Lir::ConstI64(0)],
            "a *% 0 elides to 0"
        );
        // A non-identity constant keeps the raw wrapping op.
        assert!(
            lir("(Int64.wrapping-add a 5)").contains(&Lir::I64Add),
            "a non-identity wrapping-add keeps the raw i64.add"
        );

        // The annihilator DISCARDS the other operand, so a TRAPPING operand blocks it: `(/ 10 b) *% 0`
        // must keep the division (and trap on b = 0), NOT fold to 0.
        use wasmtime::component::Val;
        let src = "(module m (def (f (: b Int64)) (Int64.wrapping-mul (/ 10 b) 0)) (export f))";
        let bytes =
            compile_component(&crate::codec::encode(&crate::testkit::parse(src))).expect("compile");
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(2)]), 0);
        assert!(
            call_traps(&bytes, "f", &[Val::S64(0)]),
            "`(/ 10 b) *% 0` must still trap on b = 0 — the annihilator must not drop a trapping operand"
        );
    }

    #[test]
    fn option_expect_unwraps_a_runtime_optional_through_the_disc_probe() {
        // The RUNTIME `Option.expect` path: unwrap an optional a runtime op PRODUCED (not a constant), the
        // compiler's `List.at`+`Option.expect` idiom the spec cites. `build 0 3 (list)` = `[0 1 2]`;
        // `(List.at xs 1)` is a runtime `(Some 1)` (a heap sum, does NOT fold), and `(Option.expect … "m")`
        // unwraps it → 1. Exercises the emitted `sum-disc == 0` probe + `sum-payload` unbox on the present
        // arm (no trap). This forces the genuine `Core::SumExpect` emit (the sum is a runtime handle).
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) (let ((xs (build 0 3 (list)))) \
                 ((. Option expect) ((. List at) xs 1) \"in range\"))) \
               (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(
            out, "1",
            "runtime Option.expect unwraps the present payload"
        );
    }

    #[test]
    fn a_runtime_list_index_reads_the_element_through_vec_get() {
        // The RUNTIME `List.at` path: a list BUILT at run time (a recursive push-loop — not a visible
        // literal, so `List.at` does NOT fold) is indexed and the element unwrapped by a match. `build 0
        // 3 (list)` = `[0 1 2]`; `(match (List.at xs 1) ((Some x) x) ((None _) -1))` reads index 1 → `Some
        // 1` → 1. Exercises the emitted bounds check + `vec-get` + `dup` + `sum-new(Some)`, then the
        // match's `sum-disc`/`sum-payload` unwrap — the full runtime fallible-read round-trip.
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) (let ((xs (build 0 3 (list)))) \
                 (match ((. List at) xs 1) ((Some x) x) ((None _) -1)))) \
               (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "1", "runtime List.at in bounds reads the element");
    }

    #[test]
    fn a_runtime_list_index_out_of_bounds_takes_the_none_arm() {
        // The absent side of the runtime path: indexing a runtime-built list PAST its end yields `None`
        // (never traps, never reads garbage). `build 0 3 (list)` = `[0 1 2]` (length 3); `(List.at xs 9)`
        // is out of bounds, so the match takes the `None` arm → -1. Pins the runtime bounds check's high
        // side (`index < vec-len`) and the `None` construction. (The negative side is exercised by the
        // constant fold; a runtime negative index takes the same `index >= 0` guard.)
        let Some(out) = run_on_heap(
            "(module m \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out i)) out)) \
               (def (main) (let ((xs (build 0 3 (list)))) \
                 (match ((. List at) xs 9) ((Some x) x) ((None _) -1)))) \
               (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "-1", "runtime List.at out of bounds → None");
    }

    #[test]
    fn a_runtime_list_at_result_is_matched_by_a_nested_constructor_pattern() {
        // The reader idiom: a fallible access yields an `Option` whose `Some` payload is a USER sum, and a
        // NESTED pattern deconstructs both in one arm. `(List.at (List.push (list) (N.L 7)) 0)` is a
        // runtime `(Some (N.L 7))`; `(Some (N.L v))` binds v=7. This is the decision-tree matcher over a
        // NON-REUSABLE (computed) `List.at` scrutinee — which is materialized ONCE into a scratch slot
        // (else re-emitting the `List.at` per probe both rebuilds the list AND collides the list/index
        // scratch with the arm bodies' temps at the shared floor, an invalid module). Pins that fix + the
        // built-in `Option` carrying a user sum through the runtime nested matcher (corpus §'a runtime
        // Option carrying a user sum is matched by a nested constructor pattern').
        // The list of `N` is BUILT at run time by a push-loop over a RUNTIME parameter (`build 0 1` pushes
        // `(N.L i)` for the runtime index `i`), so it is a genuine `vec-push` handle — a constant `(list)`
        // + a constant `push` now folds, which would defeat the runtime-matcher coverage. `List.at … 0` is
        // then a runtime `(Some (N.L 0))`, and `(Some (N.L v))` binds v=0.
        let Some(out) = run_on_heap(
            "(module m (type N (L Int64) (P Int64)) \
               (def (build i n out) (if (< i n) (build (+ i 1) n ((. List push) out (N.L i))) out)) \
               (def (main) (match ((. List at) (build 0 1 (list)) 0) \
                             ((Some (N.L v)) v) ((Some (N.P v)) (+ v 100)) ((None _) -1))) \
               (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(
            out, "0",
            "nested match over a runtime List.at Some(user-sum)"
        );
    }

    // NOTE: the self-hosting arg-walk idiom (corpus §'a list built by a recursive push-loop is then
    // iterated by index') — `(def (sum-at xs i n) … (match (List.at xs i) …))` — is NOT yet compilable,
    // but the block is the runtime `List.at` EMIT: it is an orthogonal INFERENCE gap. When the list is a
    // FUNCTION PARAMETER (`xs`), its element type stays an unsolved `?0` (the concrete `List Int64` at
    // the call site does not flow into the parameter's element), so the match binder `x` is `?0` and the
    // add over it declines "projecting a tuple element of type ?0". The runtime read itself works when
    // the list's element type is known (the two cases above index a locally-built `List Int64`); the
    // parameter-element propagation is a separate increment (the "runtime list's element type flows
    // through" gap the increment-2 note records).

    #[test]
    fn a_constant_scrutinee_folds_to_the_selected_arm() {
        // (match 0 (0 42) (_ 99)) → 42; (match 5 (0 42) (_ 99)) → 99 (the wildcard).
        assert_eq!(
            run_returns::<i64>(
                &component("(module m (def (main) (match 0 (0 42) (_ 99))) (export main))"),
                "main"
            ),
            42
        );
        assert_eq!(
            run_returns::<i64>(
                &component("(module m (def (main) (match 5 (0 42) (_ 99))) (export main))"),
                "main"
            ),
            99
        );
    }

    #[test]
    fn boolean_connectives_evaluate_and_fold() {
        // core-semantics.md §Boolean Connectives Short-Circuit: and/or/not over booleans. Constant fold
        // (each is a `main` returning a Bool / an Int64 selected by the connective).
        let b = |body: &str| -> bool {
            run_returns::<bool>(
                &component(&format!("(module m (def (main) {body}) (export main))")),
                "main",
            )
        };
        let i = |body: &str| -> i64 {
            run_returns::<i64>(
                &component(&format!("(module m (def (main) {body}) (export main))")),
                "main",
            )
        };
        assert!(!b("(and true false)"));
        assert!(b("(and true true)"));
        assert!(b("(or false true)"));
        assert!(!b("(or false false)"));
        assert!(b("(not false)"));
        assert!(!b("(not true)"));
        // Nested / composed with comparisons (a range guard, the pervasive idiom).
        assert_eq!(i("(if (and (> 5 0) (< 5 10)) 1 0)"), 1);
        assert_eq!(i("(if (and (> 5 0) (< 5 3)) 1 0)"), 0);
    }

    #[test]
    fn boolean_identities_over_a_runtime_operand_fold() {
        // Boolean simplifications with a RUNTIME operand `p` (a param): `(not (not p))` = p; `(and p
        // true)` / `(or p false)` = p (neutral, keeps p); `(and p false)` = false / `(or p true)` = true
        // (absorbing — drops p, sound because p is trap-free here). Each returns the right value over both
        // truth values of `p`.
        use wasmtime::component::Val;
        let f = |body: &str| {
            compile_component(&crate::codec::encode(&crate::testkit::parse(&format!(
                "(module m (def (f (: p Bool)) {body}) (export f))"
            ))))
            .expect("compile")
        };
        for (body, want_t, want_f) in [
            ("(not (not p))", true, false),
            ("(and p true)", true, false),
            ("(or p false)", true, false),
            ("(and p false)", false, false),
            ("(or p true)", true, true),
        ] {
            let b = f(body);
            assert_eq!(
                run_returns_with::<bool>(&b, "f", &[Val::Bool(true)]),
                want_t,
                "{body} @true"
            );
            assert_eq!(
                run_returns_with::<bool>(&b, "f", &[Val::Bool(false)]),
                want_f,
                "{body} @false"
            );
        }
        // TRAP-SAFETY: the absorbing fold must NOT drop a trapping left operand — `(and p false)` where
        // `p` is a trapping expression still traps (the fold keeps `p` when it is not trap-free).
        let g = "(module m (def (f (: n Int64)) (if (and (> (/ 10 n) 0) false) 1 0)) (export f))";
        let gb =
            compile_component(&crate::codec::encode(&crate::testkit::parse(g))).expect("compile");
        assert!(
            call_traps(&gb, "f", &[Val::S64(0)]),
            "(and <traps> false) must still trap on the div-by-zero"
        );
        assert_eq!(run_returns_with::<i64>(&gb, "f", &[Val::S64(2)]), 0);
    }

    #[test]
    fn a_boolean_connective_operand_must_be_bool() {
        // A non-Bool operand is a MALFORMED program (CDZ0201) — the operand is not the required Bool, like
        // a binary operator's operand — NOT the structural-shape mismatch (CDZ0203) a cross-kind branch
        // disagreement is (02-binding "a boolean connective with a non-boolean operand is a type error").
        assert_eq!(
            reject_code("(module m (def (main) (and true 1)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            reject_code("(module m (def (main) (or 5 false)) (export main))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            reject_code("(module m (def (main) (not 7)) (export main))").as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn a_conjunction_short_circuits_shielding_a_trapping_right_operand() {
        // core-semantics.md §Boolean Connectives Short-Circuit: `and` evaluates its RIGHT operand ONLY
        // when the left is true. `(f n) = (and (> n 0) (> (/ 10 n) 1))` — for n=0 the left is false, so
        // the trapping `(/ 10 0)` right operand is NEVER evaluated: f(0) returns 0, NO trap. f(2) = 1
        // (10/2=5>1). This is the SHIELD the spec requires — a connective guards a trapping/effectful
        // right operand exactly as a conditional's unselected branch does.
        let src =
            "(module m (def (f (: n Int64)) (if (and (> n 0) (> (/ 10 n) 1)) 1 0)) (export f))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile short-circuit");
        for (arg, want) in [("0", "0"), ("2", "1"), ("20", "0")] {
            let opts = cdz_run::RunOpts {
                export: Some("f".to_string()),
                args: vec![arg.to_string()],
                runtime: None,
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "f {arg}"),
                cdz_run::Outcome::Trap(t) => {
                    panic!("short-circuit failed to shield (trapped): {t} at {arg}")
                }
            }
        }
    }

    #[test]
    fn a_leaf_operand_boolean_connective_is_branchless() {
        // `(and p q)` / `(or p q)` over cheap trap-free LEAF operands (params here) need no short-circuit
        // branch — booleans are canonical i32 0/1, so they emit a branchless `i32.and`/`i32.or` (no `if`).
        // Short-circuit only matters to skip an EFFECTING/trapping right operand; a leaf has neither, so
        // evaluating it unconditionally is identical. (The trapping-rhs case keeps the `if` — covered by
        // `a_conjunction_short_circuits_shielding_a_trapping_right_operand`.) Asserted at the Lir level
        // AND run over the full truth table.
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let lir = |body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f (: p Bool) (: q Bool)) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let sig = db.defs[d].params.clone();
            let params: Vec<_> = sig
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let and_code = lir("(and p q)");
        assert!(
            and_code.contains(&Lir::I32And) && !and_code.iter().any(|i| matches!(i, Lir::If(_))),
            "(and p q) over leaves is a branchless i32.and, got: {and_code:?}"
        );
        let or_code = lir("(or p q)");
        assert!(
            or_code.contains(&Lir::I32Or) && !or_code.iter().any(|i| matches!(i, Lir::If(_))),
            "(or p q) over leaves is a branchless i32.or, got: {or_code:?}"
        );
        // Full truth-table value parity under wasmtime.
        use wasmtime::component::Val;
        let and_b = compile_component(&crate::codec::encode(&crate::testkit::parse(
            "(module m (def (f (: p Bool) (: q Bool)) (and p q)) (export f))",
        )))
        .expect("compile and");
        let or_b = compile_component(&crate::codec::encode(&crate::testkit::parse(
            "(module m (def (f (: p Bool) (: q Bool)) (or p q)) (export f))",
        )))
        .expect("compile or");
        for (p, q) in [(true, true), (true, false), (false, true), (false, false)] {
            assert_eq!(
                run_returns_with::<bool>(&and_b, "f", &[Val::Bool(p), Val::Bool(q)]),
                p && q,
                "and({p},{q})"
            );
            assert_eq!(
                run_returns_with::<bool>(&or_b, "f", &[Val::Bool(p), Val::Bool(q)]),
                p || q,
                "or({p},{q})"
            );
        }
    }

    #[test]
    fn a_comparison_operand_boolean_connective_is_branchless() {
        // `(and (< a b) (< c d))` — the rhs is a COMPARISON, not a bare leaf, but a comparison can neither
        // trap nor effect, so evaluating it unconditionally is identical to the short-circuit `if`. It
        // emits a branchless `i32.and` (two `lt_s`, one `and`, no `if`), the comparison companion of the
        // leaf-operand branchless connective. (A rhs that could TRAP — e.g. containing a `/` — keeps the
        // short-circuit; that's `a_conjunction_short_circuits_shielding_a_trapping_right_operand`.)
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        use wasmtime::component::Val;
        let lir = |body: &str| -> Vec<Lir> {
            let ast = crate::testkit::parse(&format!(
                "(module m (def (f (: a Int64) (: b Int64) (: c Int64) (: d Int64)) {body}) (def (main) 0) (export main))"
            ));
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let dd = db.def_by_name("f").expect("def f");
            let params: Vec<_> = db.defs[dd]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[dd].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        let and_code = lir("(and (< a b) (< c d))");
        assert!(
            and_code.contains(&Lir::I32And) && !and_code.iter().any(|i| matches!(i, Lir::If(_))),
            "(and cmp cmp) is a branchless i32.and (no short-circuit if), got: {and_code:?}"
        );
        let or_code = lir("(or (< a b) (< c d))");
        assert!(
            or_code.contains(&Lir::I32Or) && !or_code.iter().any(|i| matches!(i, Lir::If(_))),
            "(or cmp cmp) is a branchless i32.or, got: {or_code:?}"
        );
        // Truth-table value parity: (a<b) && (c<d) over the four cases.
        let and_b = compile_component(&crate::codec::encode(&crate::testkit::parse(
            "(module m (def (f (: a Int64) (: b Int64) (: c Int64) (: d Int64)) (and (< a b) (< c d))) (export f))",
        )))
        .expect("compile");
        let ck = |a: i64, b: i64, c: i64, d: i64| {
            run_returns_with::<bool>(
                &and_b,
                "f",
                &[Val::S64(a), Val::S64(b), Val::S64(c), Val::S64(d)],
            )
        };
        assert!(ck(1, 2, 3, 4), "T&&T");
        assert!(!ck(1, 2, 4, 3), "T&&F");
        assert!(!ck(2, 1, 3, 4), "F&&T");
        assert!(!ck(2, 1, 4, 3), "F&&F");
    }

    #[test]
    fn a_nullary_variant_is_constructed_by_applying_it_to_unit() {
        // core-semantics.md §Construction MUST Be Via Application: "(None unit)". A NULLARY variant is
        // constructed by applying its ctor to the unit value — `(T.Nil ())` — not only used bare. The
        // unit arg is the (empty) payload: `SumNew{disc, payloads:[]}` builds `arr-alloc(0)`. A match over
        // `(T.Nil ())` dispatches on the discriminant like any sum. `(match (T.Nil ()) ((T.Nil _) 7)
        // ((T.A _) 1))` → 7 (the constant folds through the applied nullary construction).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type T A Nil) \
                       (def (main) (match (T.Nil ()) ((T.Nil _) 7) ((T.A _) 1))) \
                       (export main))"
                ),
                "main"
            ),
            7
        );
    }

    #[test]
    fn a_nullary_variant_applied_to_a_non_unit_payload_is_rejected() {
        // core-semantics.md §A Sum Type Constructor Is A Single-Arity Function: "A nullary variant MUST be
        // a constructor whose argument type is Unit". Construction via application `(V unit)` must check
        // the argument IS unit; a non-unit payload is a malformed construction (CDZ0201), NOT a silently
        // discarded payload. Regression: nullary-construction-via-application accepted `(Opt.Nn 5)` and
        // discarded the 5, fabricating `(Nn unit)` — an ill-typed program accepted (a soundness hole).
        for (src, ty) in [
            (
                "(module m (type Opt (Sm Int64) Nn) (def (main) (Opt.Nn 5)) (export main))",
                "Int64",
            ),
            (
                "(module m (type Opt (Sm Int64) Nn) (def (main) (Opt.Nn true)) (export main))",
                "Bool",
            ),
        ] {
            assert_eq!(
                reject_code(src).as_deref(),
                Some("CDZ0201"),
                "a nullary variant applied to a non-unit {ty} payload must reject, not discard it"
            );
        }
        // NO OVER-REJECTION: applying the nullary variant to its canonical `unit` value constructs, and a
        // payload variant with its declared payload is unaffected.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type Opt (Sm Int64) Nn) \
                       (def (main) (match (Opt.Nn unit) ((Opt.Sm x) x) ((Opt.Nn _) 7))) \
                       (export main))"
                ),
                "main"
            ),
            7,
            "(Opt.Nn unit) must still construct the nullary variant"
        );
        assert!(
            reject_code(
                "(module m (type Opt (Sm Int64) Nn) \
                   (def (main) (match (Opt.Sm 5) ((Opt.Sm x) x) ((Opt.Nn _) 0))) (export main))"
            )
            .is_none(),
            "a payload variant applied to its declared payload must still construct"
        );
    }

    #[test]
    fn a_recursive_sum_linked_list_folds_at_runtime() {
        // The self-hosting idiom (05-compound-types.sexp): a RECURSIVE sum `IntList` whose `Cons` carries
        // `(Tuple Int64 IntList)` — the payload references the sum itself. `count` builds a runtime-length
        // list `[n … 1]` via `(IntList.Cons (tuple n (count (- n 1))))` and `(IntList.Nil ())` (nullary
        // construction); `sm` folds it, destructuring each Cons node's tuple payload `(tuple h t)` and
        // recursing on the runtime discriminant. sm(count 5) = 5+4+3+2+1 = 15. Exercises recursive sums +
        // nullary `(Nil ())` construction + tuple-payload destructure + runtime sum-match, all composed.
        let src = "(module m \
                     (type IntList (Cons (Tuple Int64 IntList)) Nil) \
                     (def (count (: n Int64)) \
                        (if (< n 1) (IntList.Nil ()) \
                            (IntList.Cons (tuple n (count (- n 1)))))) \
                     (def (sm (: xs IntList)) \
                        (match xs \
                          ((IntList.Cons (tuple h t)) (+ h (sm t))) \
                          ((IntList.Nil _) 0))) \
                     (def (main) (sm (count 5))) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile a recursive-sum linked list");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime linked-list fold imports the value-heap runtime"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed linked-list run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => assert_eq!(s, "15", "sm(count 5)"),
            cdz_run::Outcome::Trap(t) => panic!("recursive linked-list fold trapped: {t}"),
        }
    }

    #[test]
    fn a_variant_tuple_payload_is_destructured_by_a_nested_pattern() {
        // core-semantics.md §Patterns Compose: a tagged value carrying a TUPLE of sub-values is
        // destructured in ONE arm by a nested tuple pattern — `(Pair.Both (tuple a b))` binds `a`/`b` to
        // the payload tuple's elements (path `[Payload, Elem(0/1)]`). Over a CONSTANT scrutinee it folds:
        // `(Pair.Both (tuple 3 4))` → the `Both` arm binds a=3, b=4 → 7. The nullary `Neither` arm is
        // covered too (exhaustive). No runtime (the fold erases the heap sum + tuple).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (type Pair (Both (Tuple Int64 Int64)) Neither) \
                       (def (main) (match (Pair.Both (tuple 3 4)) \
                          ((Pair.Both (tuple a b)) (+ a b)) \
                          (Pair.Neither 0))) \
                       (export main))"
                ),
                "main"
            ),
            7
        );
    }

    #[test]
    fn a_variant_tuple_payload_destructure_runs_at_runtime() {
        // The runtime heap walk: `(classify n)` builds `(Both (tuple n (+ n 1)))` for n>0 else `Neither`,
        // then destructures the tuple payload — `(Both (tuple a b))` → `(+ a b)` reads the payload tuple's
        // two elements via `sum-payload` then `arr-get 0/1`. classify(4) = 4+5 = 9; classify(0) = -1.
        let src = "(module m \
                     (type Pair (Both (Tuple Int64 Int64)) Neither) \
                     (def (classify (: n Int64)) \
                        (match (if (> n 0) (Pair.Both (tuple n (+ n 1))) Pair.Neither) \
                          ((Pair.Both (tuple a b)) (+ a b)) \
                          (Pair.Neither (- 0 1)))) \
                     (export classify))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile tuple-payload destructure");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime tuple-payload match imports the value-heap runtime"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed tuple-payload run");
            return;
        };
        for (arg, want) in [("4", "9"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("classify".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "classify {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("tuple-payload destructure trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_cross_type_variant_pattern_is_rejected_not_type_confused() {
        // SOUNDNESS: a match arm's constructor pattern must belong to the SCRUTINEE's sum, not merely be
        // SOME sum's variant with the right name. A `T` value matched against `Option`'s `Some` pattern
        // is a type error (CDZ0203) — without the check the payload binds under Option's type over T's
        // bytes (a wrong value, or invalid wasm when widths differ). Sum identity is by declaration
        // occurrence, so `T` ≠ `Option` and `T` ≠ `U` even when they share a variant name.
        assert_eq!(
            reject_code(
                "(module m (type T (A Int64)) \
                   (def (main) (match (T.A 5) ((Some x) x) ((None) 0))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203"),
            "a T value matched against Option patterns must reject, not bind through Some"
        );
        // The sharper witness: differing payload WIDTH (`U.A` carries Bool, `T.A` carries Int64) would
        // emit invalid wasm if accepted — must reject at type-check.
        assert_eq!(
            reject_code(
                "(module m (type T (A Int64)) (type U (A Bool)) \
                   (def (main) (match (T.A 5) ((U.A x) (if x 1 0)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203"),
            "a cross-type variant pattern with a differing payload width must reject, not miscompile"
        );
        // NO OVER-REJECTION: a SAME-type match, and a legitimately-nested cross-sum match (an Option
        // carrying a T, each pattern checked against its own level's type), both still compile.
        assert!(
            reject_code(
                "(module m (type T (A Int64)) (def (main) (match (T.A 5) ((T.A x) x))) (export main))"
            )
            .is_none(),
            "a same-type sum match must still compile"
        );
        assert!(
            reject_code(
                "(module m (type T (A Int64)) \
                   (def (main) (match (Some (T.A 5)) ((Some (T.A y)) y) ((None) 0))) (export main))"
            )
            .is_none(),
            "an Option legitimately carrying a T, matched by (Some (T.A y)), must still compile"
        );
    }

    #[test]
    fn a_guarded_arm_over_a_constant_folds_by_its_guard() {
        // A guarded arm `(guard x (< x 0))` over a CONSTANT scrutinee folds: the binder `x` binds the
        // constant, the guard folds, and the arm is selected iff both the (Wild) probe and the guard hold.
        // `(match -5 ((guard x (< x 0)) -1) (_ 1))` → -1 (guard holds); `(match 5 …)` → 1 (guard fails).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match (- 0 5) ((guard x (< x 0)) (- 0 1)) (_ 1))) (export main))"
                ),
                "main"
            ),
            -1
        );
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (main) (match 5 ((guard x (< x 0)) (- 0 1)) (_ 1))) (export main))"
                ),
                "main"
            ),
            1
        );
    }

    #[test]
    fn a_guarded_arm_over_a_runtime_scrutinee_gates_and_falls_through() {
        // A guarded arm over a RUNTIME scrutinee (an exported param, so no fold): `(classify n)` returns
        // -1 when n<0, else n's own value via a later guard, else 0. Exercises the runtime probe chain's
        // guard test + fall-through: classify(-5) = -1 (first guard holds); classify(7) = 7 (first guard
        // fails → falls to the second, which holds); classify(0) via the unguarded wildcard tail = 0 (both
        // guards fail). The guard reads the binder (the scrutinee). A scalar Int64 export needs no runtime.
        let src = "(module m \
                     (def (classify (: n Int64)) \
                        (match n \
                          ((guard x (< x 0)) (- 0 1)) \
                          ((guard x (> x 0)) x) \
                          (_ 0))) \
                     (export classify))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile guards");
        for (arg, want) in [("-5", "-1"), ("7", "7"), ("0", "0")] {
            let opts = cdz_run::RunOpts {
                export: Some("classify".to_string()),
                args: vec![arg.to_string()],
                runtime: None,
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "classify {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("guarded runtime match trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_guard_over_a_variant_pattern_gates_on_the_payload_and_falls_through() {
        // A guard over a VARIANT pattern `(guard (Some x) (> x 0))`: the payload binder `x` is in scope for
        // the guard cond (resolve sees through the `(guard …)` wrapper to `(Some x)`), the arm fires only
        // when the variant matches AND the guard holds, and on a false guard control FALLS THROUGH to a
        // later arm of the same variant. `f(Some 5)` → guard `5>0` holds → x = 5; `f(Some -3)` → guard
        // fails → the plain `(Some y)` arm negates → `-(-3)` = 3. (Was CDZ0101 "unbound name x" / a clean
        // decline before the guarded sum-match decision-tree support landed.)
        let src = "(module m \
                     (def (f (: o (Option Int64))) \
                        (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0))) \
                     (def (main (: n Int64)) (f (Some n))) (export main))";
        let bytes = component(src);
        wasmparser::validate(&bytes).expect("a guarded variant match must emit valid wasm");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping guarded-variant run");
            return;
        };
        for (arg, want) in [(5i64, "5"), (-3, "3")] {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "guarded variant f({arg})"),
                cdz_run::Outcome::Trap(t) => panic!("guarded variant run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_guarded_variant_arm_does_not_satisfy_exhaustiveness() {
        // A guarded arm covers no value unconditionally, so a sum match whose only `Some` arm is guarded
        // (no unguarded `Some` fall-through) is non-exhaustive → CDZ0210. Pins that a guarded VARIANT arm
        // is excluded from coverage exactly as a guarded scalar arm is.
        assert_eq!(
            reject_code(
                "(module m (def (f (: o (Option Int64))) \
                   (match o ((guard (Some x) (> x 0)) x) ((None) 0))) \
                 (def (main (: n Int64)) (f (Some n))) (export main))"
            )
            .as_deref(),
            Some("CDZ0210"),
            "a guarded variant arm must not count toward exhaustiveness"
        );
        // CONTROL: an unguarded `(Some y)` fall-through makes it exhaustive again.
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (f (: o (Option Int64))) \
                   (match o ((guard (Some x) (> x 0)) x) ((Some y) y) ((None) 0))) \
                 (def (main (: n Int64)) (f (Some n))) (export main))"
            )))
            .is_ok(),
            "an unguarded same-variant fall-through restores exhaustiveness"
        );
        // REGRESSION: the SAME non-exhaustive match must reject EVEN WHEN it fully const-folds — a
        // CONSTANT scrutinee `(Some 5)` whose guard `(> 5 0)` folds to TRUE previously returned the
        // guarded body directly (→ 5), SKIPPING the exhaustiveness check, so a non-exhaustive match was
        // silently accepted whenever a constant input happened to satisfy the guard. A match is
        // ill-formed AS WRITTEN (CDZ0210) regardless of whether a specific fold hits the guarded arm; the
        // guard-folds-true path now verifies the fall-through covers the variant before folding.
        assert_eq!(
            reject_code(
                "(module m (def (main) \
                   (match (Some 5) ((guard (Some x) (> x 0)) x) ((None) 0))) (export main))"
            )
            .as_deref(),
            Some("CDZ0210"),
            "a non-exhaustive guarded match rejects even when a constant scrutinee folds its guard true"
        );
        // CONTROL: the fold-true case with an unguarded same-variant fall-through still COMPILES and
        // folds to the guarded body (5) — the exhaustiveness check passes, the fold is unchanged.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (main) \
                       (match (Some 5) ((guard (Some x) (> x 0)) x) ((Some y) y) ((None) 0))) \
                     (export main))"
                )))
                .expect("an exhaustive guarded match folds"),
                "main"
            ),
            5,
            "an exhaustive guarded match still folds to the guarded body when the guard folds true"
        );
    }

    #[test]
    fn a_string_payload_variant_is_not_misjudged_nullary() {
        // REGRESSION: a variant carrying a `String` payload — `(type Tag (Named String) Anon)` — must
        // construct, not be misjudged NULLARY. `eval::encode_ty` (the `Ty → type-value AST` round-trip a
        // constructor scheme takes) had NO `Ty::String` arm, so it hit the `_ => Unit` catch-all: the
        // `Named` ctor's `(-> String Tag)` scheme round-tripped to `(-> Unit Tag)`, and applying `Named`
        // to a `String` arg then unified "cannot unify Unit with String". (The strings vertical added
        // `Ty::String` + `decode_ty`'s `"String"` arm but forgot the `encode_ty` side — the exact hole the
        // `Bytes` arm's comment warned about.) Fixed by `Ty::String => push_name("String")`. Compile-only:
        // a runtime String projection needs the composed runtime (the corpus 13-strings cases run it).
        let src = "(module m (type Tag (Named String) Anon) \
                     (def (main) (match (Tag.Named \"hi\") ((Tag.Named s) 1) (Tag.Anon 0))) \
                   (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a String-payload variant must COMPILE — not reject Named as nullary (encode_ty must round-trip String)"
        );
    }

    #[test]
    fn a_quantity_type_round_trips_through_encode_and_decode() {
        // L1-0 SAFETY (the DESIGN doc §9 mandatory test): a `Ty::Qty` MUST survive the `Ty → type-value
        // AST → Ty` round-trip that every constructor scheme takes (`eval::encode_typeval` /
        // `resolve::decode_ty`, reached via `typeval_of`). A MISSING `encode_ty`/`decode_ty` arm would
        // silently encode a quantity as `Unit` (the catch-all) — the exact hole that mis-typed `Bytes`
        // and `String` before their arms landed — so a `(-> T (Qty T u))` scheme would round-trip to
        // `(-> T Unit)` and every quantity op would mis-type. This pins the round-trip for the
        // dimensionless unit, a base unit, a derived (velocity) unit, and a squared unit.
        use crate::db::Db;
        use crate::eval::{encode_typeval, typeval_of};
        use crate::testkit::parse;
        use crate::ty::{Ty, Unit};
        let ast = parse("(module m (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let cases = vec![
            // A dimensionless quantity over Int64 — the empty unit map.
            Ty::Qty {
                inner: Box::new(Ty::int64()),
                unit: Unit::one(),
            },
            // A base-unit quantity over Float64 — `{metre: 1}`.
            Ty::Qty {
                inner: Box::new(Ty::float64()),
                unit: Unit::base("metre"),
            },
            // A DERIVED unit (velocity) over Float64 — metre·second⁻¹, `{metre: 1, second: -1}`.
            Ty::Qty {
                inner: Box::new(Ty::float64()),
                unit: Unit::base("metre").div(&Unit::base("second")),
            },
            // A SQUARED unit (area) over Float64 — `{metre: 2}`.
            Ty::Qty {
                inner: Box::new(Ty::float64()),
                unit: Unit::base("metre").pow(2),
            },
        ];
        for ty in cases {
            let node = encode_typeval(&mut db, &ty);
            let decoded = typeval_of(&mut db, node)
                .unwrap_or_else(|| panic!("a {} type-value must decode", ty.render_name()));
            assert_eq!(
                decoded,
                ty,
                "a {} type MUST survive the encode/decode round-trip (a missing arm would encode it as Unit)",
                ty.render_name()
            );
        }
    }

    #[test]
    fn a_quantity_erases_to_its_inner_numeric_value() {
        // L1-1b: `Qty.value (Qty.of 5.0 metre)` recovers the erased inner `5.0` — a quantity is CHECKED
        // THEN ERASED, so `Qty.of`/`Qty.value` are runtime no-ops and the compiled program is plain
        // Float64 (byte-identical to the bare `5.0`). Compiles + runs to the recovered value.
        let src = "(do (def (main) ((. Qty value) ((. Qty of) 5.0 ((. Unit base) #\"metre\")))) \
                   (export main))";
        assert_eq!(
            run_returns::<f64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a quantity erases and its value is recoverable"),
                "main"
            ),
            5.0
        );
    }

    #[test]
    fn combining_quantities_of_incompatible_dimension_is_cdz0501() {
        // L1-2: adding a length to a time is a DIMENSIONAL mismatch — CDZ0501 (units-of-measure.md §A
        // Dimensional Mismatch Is An Error), the code that opens the CDZ05xx verification-layer band. It
        // is a compile-time rejection (units erase before the program runs), never a runtime trap.
        let src = "(do (def (main) (+ ((. Qty of) 1.0 ((. Unit base) #\"metre\")) \
                   ((. Qty of) 1.0 ((. Unit base) #\"second\")))) (export main))";
        assert_eq!(
            compile_component(&crate::codec::encode(&parse(src)))
                .err()
                .and_then(|d| d.code.as_deref().map(str::to_string))
                .as_deref(),
            Some("CDZ0501"),
            "a length + a time must reject CDZ0501 (dimensional mismatch)"
        );
    }

    #[test]
    fn dividing_quantities_composes_their_dimensions_to_a_velocity() {
        // L1-2: `(/ (Qty 6.0 metre) (Qty 2.0 second))` derives metre/second (the classic velocity) with
        // value 3.0 — the dimensions divide by the free-abelian-group quotient, the magnitudes by the
        // (float) division. `Qty.value` recovers the erased magnitude; this pins that it COMPILES + RUNS.
        let src = "(do (def (main) ((. Qty value) (/ ((. Qty of) 6.0 ((. Unit base) #\"metre\")) \
                   ((. Qty of) 2.0 ((. Unit base) #\"second\"))))) (export main))";
        assert_eq!(
            run_returns::<f64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a velocity quotient compiles and runs"),
                "main"
            ),
            3.0
        );
    }

    #[test]
    fn a_quantity_over_a_wrong_inner_numeric_type_is_rejected_not_miscompiled() {
        // L1-1b: the unit layer sits OVER the numeric core and does not relax it — adding an Int64
        // quantity to a Float64 quantity (SAME dimension `metre`, DIFFERENT inner type) must be REFUSED,
        // never miscompiled to a running value (the honest decline-don't-miscompile signal). It rejects
        // via the inner-type conflict; the code is CDZ0203 today and ALIGNS to the numeric no-promotion
        // CDZ0301 when the operator unit rules land (L1-2 — the `+`/`-`/comparison dimensional check that
        // reads the operands' units and dispatches the numeric mismatch as CDZ0301). Either way the
        // ill-typed program is a compile-time REJECTION, not a run.
        let src = "(do (def (main) (+ ((. Qty of) 2 ((. Unit base) #\"metre\")) \
                   ((. Qty of) 3.0 ((. Unit base) #\"metre\")))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "an Int64 quantity + a Float64 quantity must be REJECTED (a numeric mismatch under a unit), not compiled"
        );
    }

    #[test]
    fn a_generic_sum_with_a_type_param_in_a_tuple_or_record_payload_is_not_nullary() {
        // REGRESSION: a GENERIC sum whose variant carries a TUPLE or RECORD payload MENTIONING a type
        // parameter — `(type Box (B (Tuple a Int64)) N)` / `(type Box (B (Record (val a))) N)` — must
        // construct, not be misread as NULLARY. `type_in_env` (which reduces a generic variant ctor's
        // `(meta t)` type-lambda to its scheme) handled `Int`/`Fn`/`List`/`Sum`/`UInt` compound payloads
        // but had NO `Tuple`/`Record` arm, so a param nested in a tuple/record payload made the ctor arrow
        // unreadable → `variant_payload_type` = None → `B` looked NULLARY → CDZ0201 on the construction.
        // A bare/`List a`/`Option a` payload worked (those arms existed); the gap was tuple + record.
        // Fixed by adding `TupleCtor`/`RecordCtor` arms to `type_in_env` (reduce each element/field type
        // under the env). The bug was a compile-time REJECT (the ctor looked nullary → CDZ0201), so
        // COMPILING the construction is the precise guard; the runs are exercised by the corpus cases "a
        // generic sum with a type parameter inside a tuple/record payload …" (which link the runtime).
        let tup = "(module m (type Box (B (Tuple a Int64)) N) \
                     (def (main) (match (Box.B (tuple 7 8)) ((Box.B (tuple x y)) (+ x y)) (Box.N 0))) \
                   (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(tup))).is_ok(),
            "a generic sum with a type param in a TUPLE payload must compile — not reject B as nullary"
        );
        let rec = "(module m (type Box (B (Record (val a))) N) \
                     (def (main) (match (Box.B (record (val 7))) ((Box.B r) (. r val)) (Box.N 0))) \
                   (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(rec))).is_ok(),
            "a generic sum with a type param in a RECORD payload must compile — not reject B as nullary"
        );
    }

    #[test]
    fn a_false_variant_guard_shields_a_trapping_body() {
        // The variant-guard short-circuit (core-semantics.md §Boolean Connectives Short-Circuit applied to
        // a guarded arm): a guarded arm's BODY is evaluated only when the guard HOLDS. `(guard (Some x) (>
        // x 0)) (/ 10 x)` over `(Some 0)` must NOT trap on `(/ 10 0)` — the guard `0 > 0` is false, so the
        // arm is skipped and the match takes `(Some y) -1`. A generation that folded the guarded body
        // regardless of the guard raised a SPURIOUS CDZ0304 (a compile-time div-by-zero for an arm that
        // never runs). `build_tree` folds the constant guard FIRST and skips the body when it is false.
        let src = "(module m (def (main) \
                     (match (Some 0) ((guard (Some x) (> x 0)) (/ 10 x)) ((Some y) -1) (None -2))) \
                   (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main"
            ),
            -1,
            "a false variant guard shields its trapping body (no spurious CDZ0304)"
        );
    }

    #[test]
    fn chained_guards_of_the_same_variant_fall_through_in_order() {
        // Two guarded `Some` arms then a plain `(Some z)`: each guard is tried in source order, falling
        // through to the next on failure. `f(Some 50)` → first guard `>10` → 100; `f(Some 5)` → second
        // guard `>0` → 1; `f(Some -2)` → both fail → plain `(Some z)` → 0.
        let src = "(module m \
            (def (f (: o (Option Int64))) (match o \
              ((guard (Some x) (> x 10)) 100) \
              ((guard (Some y) (> y 0)) 1) \
              ((Some z) 0) ((None) (- 0 1)))) \
            (def (main (: n Int64)) (f (Some n))) (export main))";
        let bytes = component(src);
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping chained-guard run");
            return;
        };
        for (arg, want) in [(50i64, "100"), (5, "1"), (-2, "0")] {
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "chained guards f({arg})"),
                cdz_run::Outcome::Trap(t) => panic!("chained-guard run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_match_whose_only_arm_is_guarded_is_non_exhaustive() {
        // A guard does NOT count toward exhaustiveness: a match on Int64 whose sole arm is guarded covers
        // no value unconditionally, so it is CDZ0210 (core-semantics.md #Matching Is Exhaustive Or
        // Rejected). Pins that guarded arms are excluded from the coverage check.
        assert_eq!(
            reject_code("(module m (def (main) (match 5 ((guard x (< x 0)) 1))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
        // The gate wraps a bare-expression case as `(do (def (main) …) (export main))` — the same
        // program shape must reject identically (the `(do …)` top-form is scanned like `(module …)`).
        assert_eq!(
            reject_code("(do (def (main) (match 5 ((guard x (< x 0)) 1))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
    }

    #[test]
    fn a_computed_constant_scrutinee_folds_by_value() {
        // The scrutinee `(% 4 2)` folds to 0 (empty-magnitude IntValue) — it must match the literal `0`
        // pattern (`[0]` magnitude) BY VALUE, selecting arm 0. Pins the `IntValue::eq_value` fix (a
        // struct `==` would miss and fall to the wildcard → the parity-dispatch miscompile).
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (parity n) (match (% n 2) (0 0) (_ 1))) (def (main) (parity 4)) (export main))"
                ),
                "main"
            ),
            0
        );
    }

    #[test]
    fn a_runtime_scalar_match_emits_a_probe_chain() {
        // fib with literal match base cases — a runtime scrutinee (the param `n`) matched against 0/1/_,
        // emitting the probe chain. fib(10) = 55.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (fib n) (match n (0 0) (1 1) (_ (+ (fib (- n 1)) (fib (- n 2)))))) (def (main) (fib 10)) (export main))"
                ),
                "main"
            ),
            55
        );
        // fact with a match base case: fact(5) = 120.
        assert_eq!(
            run_returns::<i64>(
                &component(
                    "(module m (def (fact n) (match n (0 1) (_ (* n (fact (- n 1)))))) (def (main) (fact 5)) (export main))"
                ),
                "main"
            ),
            120
        );
    }

    #[test]
    fn a_scalar_match_probe_against_zero_uses_eqz() {
        // A `0`-literal match arm — the shape of every recursion base case `(match n (0 …) …)` — probes
        // `scrutinee == 0`, which is exactly `i64.eqz` (one instruction), not a pushed `0` + `eq` (two).
        // Mirrors the comparison path's `(= n 0)` → eqz. A NONZERO probe keeps `const ; eq`.
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        let code = {
            let ast = crate::testkit::parse(
                "(module m (def (f (: n Int64)) (match n (0 10) (1 20) (_ 30))) (def (main) 0) (export main))",
            );
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let ps: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
                .expect("select")
                .code
        };
        // The `0` arm is an `eqz`; the `1` arm is a `const 1 ; eq`.
        assert!(
            code.contains(&Lir::I64Eqz),
            "the `0` probe must be an i64.eqz, got: {code:?}"
        );
        assert!(
            code.contains(&Lir::ConstI64(1)) && code.contains(&Lir::I64Eq),
            "the nonzero `1` probe keeps const ; eq, got: {code:?}"
        );
        // Exactly ONE eqz (the `0` arm) and ONE const-0 must NOT be pushed for the probe.
        assert_eq!(
            code.iter().filter(|i| matches!(i, Lir::I64Eqz)).count(),
            1,
            "only the single `0` arm becomes an eqz"
        );
        // Value parity: dispatch is unchanged.
        use wasmtime::component::Val;
        let src = "(module m (def (f (: n Int64)) (match n (0 10) (1 20) (_ 30))) (export f))";
        let bytes = component(src);
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(0)]), 10);
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(1)]), 20);
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(9)]), 30);
    }

    #[test]
    fn a_computed_match_scrutinee_is_evaluated_once() {
        use wasmtime::component::Val;
        // `(match (+ a b) (0 10) (1 20) (_ 30))` — the scrutinee is a CHECKED add. It is evaluated ONCE
        // into a slot; each probe reads that slot (not a recomputed add). Correct dispatch across arms:
        let src = "(module m (def (f (: a Int64) (: b Int64)) \
                     (match (+ a b) (0 10) (1 20) (_ 30))) (export f))";
        let bytes = component(src);
        // (a+b)=0 → 10; =1 → 20; =5 → wildcard 30. The single evaluation must feed every probe.
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(-2), Val::S64(2)]),
            10
        );
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(1), Val::S64(0)]),
            20
        );
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(3), Val::S64(2)]),
            30
        );
        // The scrutinee's own overflow guard must STILL fire (evaluated once, but still checked): a+b
        // overflowing Int64 traps before any probe runs.
        assert!(
            call_traps(&bytes, "f", &[Val::S64(i64::MAX), Val::S64(1)]),
            "an overflowing computed scrutinee must trap (its guard is not lost by the compute-once)"
        );
    }

    /// A match every UNGUARDED arm of which yields the SAME value COLLAPSES to that value — the probe
    /// chain is dropped (the match analogue of `(if c x x)` → `x`). `(match a (1 x) (2 x) (_ x))` always
    /// returns `x`, so it lowers to just `x` with no `i64.eq`/branch on `a`. Sound because the scrutinee
    /// here is a trap-free parameter (nothing to preserve by evaluating it); a trapping scrutinee is
    /// covered by `a_trapping_scrutinee_of_an_all_same_match_is_still_evaluated`.
    #[test]
    fn an_all_same_body_match_collapses_to_the_body() {
        use crate::backend::wasm::lir::Lir;
        use crate::db::Db;
        use wasmtime::component::Val;
        let src = "(module m (def (f (: a Int64) (: x Int64)) \
                     (match a (1 x) (2 x) (_ x))) (export f))";
        // The collapse dropped the dispatch entirely: no equality probe on the scrutinee survives in the Lir.
        let code = {
            let ast = crate::testkit::parse(src);
            let mut db = Db::load(ast);
            let layout = crate::layout::compute(&mut db).expect("layout");
            let d = db.def_by_name("f").expect("def f");
            let params: Vec<_> = db.defs[d]
                .params
                .clone()
                .into_iter()
                .map(|p| {
                    let b = db
                        .ast
                        .as_form(p, ":")
                        .and_then(|t| t.first().copied())
                        .unwrap_or(p);
                    (b, crate::infer::type_of(&mut db, b))
                })
                .collect();
            let body = db.defs[d].body.expect("body");
            crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
                .expect("select")
                .code
        };
        assert!(
            !code.contains(&Lir::I64Eq) && !code.iter().any(|i| matches!(i, Lir::If(_))),
            "an all-same-body match must collapse to its body (no probe chain / branch), got: {code:?}"
        );
        // Every scrutinee value yields x — verify across a matched literal and the wildcard.
        let bytes = component(src);
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(1), Val::S64(7)]),
            7
        );
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(9), Val::S64(7)]),
            7
        );
    }

    /// The all-same-body collapse is gated on a TRAP-FREE scrutinee: the discriminant is unused after the
    /// collapse, but the scrutinee was evaluated to drive the (now-gone) probes, so a scrutinee that could
    /// trap must STILL be evaluated. `(match (/ 10 b) (1 x) (_ x))` always yields `x`, but the division
    /// must still run — `b = 0` traps rather than returning `x`.
    #[test]
    fn a_trapping_scrutinee_of_an_all_same_match_is_still_evaluated() {
        use wasmtime::component::Val;
        let src = "(module m (def (f (: b Int64) (: x Int64)) \
                     (match (/ 10 b) (1 x) (_ x))) (export f))";
        let bytes = component(src);
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(2), Val::S64(7)]),
            7,
            "a non-trapping scrutinee still yields the common body"
        );
        assert!(
            call_traps(&bytes, "f", &[Val::S64(0), Val::S64(7)]),
            "a div-by-zero scrutinee must trap even though every arm yields the same value"
        );
    }

    #[test]
    fn a_large_scalar_match_beyond_the_br_table_cap_compiles_and_dispatches() {
        // A scalar match with MORE arms than the br_table density cap (>256 int arms) is INELIGIBLE for
        // the jump table (a dense table can hold at most 256 distinct values), so it emits the linear
        // `if (== k)` probe cascade. The eligibility test is gated O(1) on the arm count BEFORE building
        // the per-arm literal vector, so re-attempting the table on every recursive `rest` stays O(arms)
        // rather than O(arms²) — this exercises that fallback path end-to-end and pins that a matched
        // value still reaches the right arm through the long chain. 300 consecutive arms `k → k*2`.
        use wasmtime::component::Val;
        let n = 300;
        let arms: String = (0..n).map(|i| format!("({i} {}) ", i * 2)).collect();
        let src = format!("(module m (def (f (: k Int64)) (match k {arms}(_ -1))) (export f))");
        let bytes = component(&src);
        // A value in the middle of the chain, the last arm, and out-of-range (the wildcard default).
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(0)]), 0);
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(150)]), 300);
        assert_eq!(
            run_returns_with::<i64>(&bytes, "f", &[Val::S64(n - 1)]),
            (n - 1) * 2
        );
        assert_eq!(run_returns_with::<i64>(&bytes, "f", &[Val::S64(9999)]), -1);
    }

    #[test]
    fn a_binder_pattern_binds_the_scrutinee() {
        // A bare-name arm `k` binds the whole scrutinee for its body — the exhaustive tail (like `_`,
        // but named). `(match n (0 100) (k (+ k 1)))`: f(0)=100 (literal arm wins), f(41)=42 (k binds
        // 41, body computes 42). Pins that the name arm and literal arm select consistently, and the
        // binder carries the scrutinee's value into its body.
        let m = "(module m (def (f n) (match n (0 100) (k (+ k 1)))) (def (main) (f {})) (export main))";
        assert_eq!(
            run_returns::<i64>(&component(&m.replace("{}", "41")), "main"),
            42
        );
        assert_eq!(
            run_returns::<i64>(&component(&m.replace("{}", "0")), "main"),
            100
        );
    }

    #[test]
    fn a_binder_over_a_runtime_scrutinee_binds_correctly() {
        // The binder over a RUNTIME scrutinee (an exported annotated param, not inlined) — `k` reads the
        // parameter's slot in the arm body. f(7) → k=7 → 8.
        let bytes = component(
            "(module m (def (f (: n Int64)) (match n (0 100) (k (+ k 1)))) (def (main) (f 7)) (export main))",
        );
        assert_eq!(run_returns::<i64>(&bytes, "main"), 8);
    }

    #[test]
    fn a_binder_over_a_narrow_scrutinee_normalizes_to_its_width() {
        use wasmtime::component::Val;
        // A binder over a NARROW-width scrutinee, beside a bare-LITERAL arm. Every arm produces the
        // match's result type, so the literal arm (default Int64 on its own) must take the UInt8 result
        // width — else a default-i64 arm beside the narrow-i32 binder arm is a mismatched block and wasm
        // rejects the function. Regression for the narrow-match-binder MISCOMPILE (invalid component).
        let uint8 = compile_component(&crate::codec::encode(&parse(
            "(module m (def (main (: x UInt8)) (match x (0 100) (n n))) (export main))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<u8>(&uint8, "main", &[Val::U8(5)]), 5); // binder arm
        assert_eq!(run_returns_with::<u8>(&uint8, "main", &[Val::U8(0)]), 100); // literal arm
        // The bound narrow value is usable in a downstream op: 50 → (+ 50 50) = 100.
        let arith = compile_component(&crate::codec::encode(&parse(
            "(module m (def (main (: x UInt8)) (match x (0 0) (n (+ n x)))) (export main))",
        )))
        .expect("compile");
        assert_eq!(run_returns_with::<u8>(&arith, "main", &[Val::U8(50)]), 100);
    }

    #[test]
    fn a_type_mismatched_pattern_is_rejected() {
        // A boolean pattern against an integer scrutinee is a type error (CDZ0201) — checked
        // STRUCTURALLY, not silently treated as a never-matching arm. (Even with a constant scrutinee.)
        assert_eq!(
            reject_code("(module m (def (main) (match 5 (true 1) (_ 0))) (export main))")
                .as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn a_non_exhaustive_scalar_match_is_rejected_even_when_a_constant_hits_an_arm() {
        // A scalar match with no wildcard tail is non-exhaustive (CDZ0210) — a program is ill-formed if
        // it would not cover some value, EVEN when the constant scrutinee happens to hit an arm. The
        // check is structural, before the fold.
        assert_eq!(
            reject_code("(module m (def (main) (match 5 (5 1))) (export main))").as_deref(),
            Some("CDZ0210")
        );
    }

    #[test]
    fn a_non_wildcard_pattern_after_a_literal_still_needs_a_wildcard() {
        // Two literal arms with no wildcard — non-exhaustive over the integers (CDZ0210), regardless of
        // whether the runtime scrutinee would hit one.
        assert_eq!(
            reject_code("(module m (def (f (: n Int64)) (match n (0 1) (1 2))) (export f))")
                .as_deref(),
            Some("CDZ0210")
        );
    }

    #[test]
    fn a_bool_match_covering_both_literals_is_exhaustive_without_a_wildcard() {
        use wasmtime::component::Val;
        // A Bool scrutinee is EXHAUSTED by its two literals: `(match b (true …) (false …))` needs no
        // wildcard (the finite Bool type has only two values). Both selections must produce the right
        // value — the wildcard-less match emits its LAST arm as the unconditional else, so the second
        // arm's body is reached, not a dangling fallthrough.
        let negate = component(
            "(module m (def (negate (: b Bool)) (match b (true false) (false true))) \
               (def (main (: b Bool)) (negate b)) (export main))",
        );
        assert!(!run_returns_with::<bool>(
            &negate,
            "main",
            &[Val::Bool(true)]
        )); // true → false
        assert!(run_returns_with::<bool>(
            &negate,
            "main",
            &[Val::Bool(false)]
        )); // false → true
        // Order-independent: the arms reversed still cover both values.
        let rev = component(
            "(module m (def (main (: b Bool)) (match b (false 2) (true 1))) (export main))",
        );
        assert_eq!(run_returns_with::<i64>(&rev, "main", &[Val::Bool(true)]), 1);
        assert_eq!(
            run_returns_with::<i64>(&rev, "main", &[Val::Bool(false)]),
            2
        );
    }

    #[test]
    fn a_bool_match_missing_a_literal_is_still_non_exhaustive() {
        // The relaxation is precise: a Bool match is exhaustive ONLY with BOTH `true` and `false` arms.
        // A single Bool literal (or two of the SAME literal) leaves a value uncovered → CDZ0210, exactly
        // as an Int64 match without a wildcard. Pins that "both Bool literals ⇒ exhaustive" does not
        // over-accept a partial Bool match.
        assert_eq!(
            reject_code("(module m (def (main (: b Bool)) (match b (true 1))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
        assert_eq!(
            reject_code("(module m (def (main (: b Bool)) (match b (false 2))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
        // Two of the SAME literal do not cover the other value.
        assert_eq!(
            reject_code(
                "(module m (def (main (: b Bool)) (match b (true 1) (true 2))) (export main))"
            )
            .as_deref(),
            Some("CDZ0210")
        );
        // An Int64 match without a wildcard stays rejected — the relaxation is Bool-specific.
        assert_eq!(
            reject_code("(module m (def (main (: n Int64)) (match n (0 1) (1 2))) (export main))")
                .as_deref(),
            Some("CDZ0210")
        );
    }
}

// ── decline-don't-miscompile ───────────────────────────────────────────────────────────────────

/// An unsupported construct reached unconditionally yields NO component and an error diagnostic —
/// never a plausible wrong artifact.
#[test]
fn unsupported_construct_declines() {
    let err = compile_component(&prog_unsupported()).expect_err("must decline");
    assert_eq!(err.severity, crate::abi::Severity::Error);
    assert!(
        err.message.contains("frobnicate"),
        "diagnostic should name the construct: {err:?}"
    );
}

/// The `main` export crosses the boundary VERBATIM — the component actually exports `main` (no
/// rename), so a query by that name resolves.
#[test]
fn export_name_is_verbatim() {
    let bytes = compile_component(&prog_scalar()).expect("compile");
    // If the name were renamed, `run_returns_s64(.., "main")` would fail to find the export.
    assert_eq!(run_returns::<i64>(&bytes, "main"), 42);
}

// ── span-free diagnostics: a fault names a user NODE, never a prelude/synthesized id ─────────────
//
// The compiler holds no source positions; a diagnostic carries the `StructId` (as `node`) the fault
// is about, and the front-end (which holds the span table keyed by that identity) maps it to a text
// region. These pin (a) that a fault anchors to a real user node, and (b) the boundary invariant: a
// prelude/synthesized node id NEVER reaches a diagnostic.
mod diagnostics {
    use crate::abi::Artifact;
    use crate::ast::StructId;
    use crate::backend::Target;
    use crate::compile::compile;
    use crate::db::Db;
    use crate::testkit::parse;

    /// Compile a program and return its first error diagnostic.
    fn first_error(src: &str) -> crate::abi::Diagnostic {
        let ast = parse(src);
        let bytes = crate::codec::encode(&ast);
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        out.diagnostics
            .into_iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .expect("an error")
    }

    #[test]
    fn an_integer_operand_to_a_float_operator_offers_an_of_int_coercion_fix() {
        // The numeric-mismatch fix (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
        // A Fix): `(+. x 2.0)` with `x : Int64` is CDZ0301 (no silent promotion), but the repair is the
        // corpus-blessed `(Float64.of-int x)` — a WRAP fix converting the integer operand. Applying it
        // makes the program type-check.
        let d = first_error("(module m (def (f (: x Int64)) (+. x 2.0)) (export f))");
        assert_eq!(d.code.as_deref(), Some("CDZ0301"), "got: {}", d.message);
        let fix = d.fix.expect("a coercion fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
        assert_eq!(
            fix.replacement,
            format!("(Float64.of-int {})", crate::abi::WRAP_HOLE),
            "wraps the integer operand in Float64.of-int"
        );
        assert!(!fix.verified, "a coercion is a heuristic (intent guess)");
    }

    #[test]
    fn a_narrower_int_operand_to_a_float_operator_nests_the_int64_widening() {
        // `of-int : Int64 → Float` — it takes EXACTLY Int64. For a NARROWER operand (`x : Int32`) a bare
        // `(Float64.of-int x)` would ITSELF fail (Int32 ≠ Int64), so the fix must first widen:
        // `(Float64.of-int (Int64.of x))`. This is the correctness fix for the D7 gap — a suggested fix
        // must resolve the fault in ONE shot, not cascade to the next mismatch.
        let d = first_error("(module m (def (f (: x Int32)) (+. x 2.0)) (export f))");
        assert_eq!(d.code.as_deref(), Some("CDZ0301"), "got: {}", d.message);
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some(format!(
                "(Float64.of-int (Int64.of {}))",
                crate::abi::WRAP_HOLE
            ))
            .as_deref(),
            "a narrower int nests the Int64 widening: {}",
            d.message
        );
    }

    #[test]
    fn a_non_numeric_mismatch_to_a_float_operator_carries_no_coercion_fix() {
        // The coercion fix fires ONLY for an Int→Float mismatch — a Bool operand to `+.` is a plain
        // CDZ0203/CDZ0301 with no `of-int` repair (converting a Bool to a float is not the fix).
        let d = first_error("(module m (def (f (: x Bool)) (+. x 2.0)) (export f))");
        assert!(
            d.fix.is_none(),
            "no coercion fix for a non-integer operand: {:?}",
            d.fix
        );
    }

    #[test]
    fn an_integer_width_mismatch_offers_an_of_conversion_fix() {
        // The integer-width coercion (sibling of the D7 float case): `(+ a b)` with `a:Int32`, `b:Int64`
        // is CDZ0301 (no silent promotion), repaired by the corpus-blessed CHECKED conversion
        // `(<TargetInt>.of …)` (`06-numeric-model.sexp` — `(+ 2 (Int64.of 1))`). The target is the
        // EXPECTED operand's type; applying the fix type-checks.
        let d = first_error("(module m (def (f (: a Int32) (: b Int64)) (+ a b)) (export f))");
        assert_eq!(d.code.as_deref(), Some("CDZ0301"), "got: {}", d.message);
        let fix = d.fix.expect("a coercion fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::Wrap);
        assert_eq!(
            fix.replacement,
            format!("(Int32.of {})", crate::abi::WRAP_HOLE),
            "wraps the operand in the expected int type's `.of`"
        );
        assert!(!fix.verified, "`.of` is checked (can trap) → heuristic");
    }

    #[test]
    fn a_non_aliased_int_width_target_carries_no_conversion_fix() {
        // The conversion is GATED to an ALIASED width ({8,16,32,64}) — those are the only BOUND names. The
        // TARGET is the FIRST operand's type (the one the operator pins); with a non-aliased `(Int 48)`
        // first, the target renders to `Int48`, which nothing binds, so no fix is offered (suggesting an
        // unbound `(Int48.of …)` would be worse than none).
        let d = first_error("(module m (def (f (: a (Int 48)) (: b Int64)) (+ a b)) (export f))");
        assert_eq!(d.code.as_deref(), Some("CDZ0301"), "got: {}", d.message);
        assert!(
            d.fix.is_none(),
            "no fix for a non-aliased target width (would be unbound): {:?}",
            d.fix
        );
    }

    #[test]
    fn the_same_fault_is_reported_once_even_when_two_passes_find_it() {
        // An unbound name in a REACHABLE position is found by BOTH the type-check walk and the
        // reached-poison walk — it must be reported ONCE (deduped by code+node), not twice.
        let unbound: Vec<_> = crate::diagnostics(&mut Db::load(parse(
            "(module m (def (main) nope) (export main))",
        )))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0101"))
        .collect();
        assert_eq!(
            unbound.len(),
            1,
            "one unbound-name fault, not two: {unbound:?}"
        );

        // But two DISTINCT occurrences of the same unbound name (different nodes) are NOT duplicates —
        // both survive (each has its own source location).
        let two: Vec<_> = crate::diagnostics(&mut Db::load(parse(
            "(module m (def (main) (+ nope nope)) (export main))",
        )))
        .into_iter()
        .filter(|d| d.code.as_deref() == Some("CDZ0101"))
        .collect();
        assert_eq!(
            two.len(),
            2,
            "two distinct `nope` uses each reported once: {two:?}"
        );
        assert_ne!(two[0].node, two[1].node, "at different nodes");
    }

    #[test]
    fn an_unbound_name_anchors_to_a_user_node() {
        // The diagnostic for an unbound name carries a node index, and it is a genuine USER node (below
        // the program's node count) — the front-end can map it to the `nope` occurrence.
        let d = first_error("(module m (def (main) nope) (export main))");
        assert_eq!(d.code.as_deref(), Some("CDZ0101"));
        let node = d.node.expect("unbound-name diagnostic must carry a node");
        // It resolves to a real user node — the same identity the span table is keyed by.
        let ast = parse("(module m (def (main) nope) (export main))");
        let db = Db::load(ast);
        assert!(
            db.is_user_node(StructId(node)),
            "node {node} must be a user node"
        );
    }

    #[test]
    fn a_provable_overflow_does_not_leak_a_synthesized_node() {
        // `(+ Int64.max 1)` proves an overflow (CDZ0304). The fold runs over evaluator-SYNTHESIZED
        // nodes (the built `Int64` module / reduced operands), but the reported origin must be either a
        // real user node or unanchored — NEVER a synthesized/prelude id that would mis-map. This is the
        // boundary invariant the operator flagged.
        let d = first_error("(module m (def (main) (+ (. Int64 max) 1)) (export main))");
        assert_eq!(d.code.as_deref(), Some("CDZ0304"));
        if let Some(node) = d.node {
            let ast = parse("(module m (def (main) (+ (. Int64 max) 1)) (export main))");
            let db = Db::load(ast);
            assert!(
                db.is_user_node(StructId(node)),
                "reported node {node} must be a user node, not a prelude/synthesized id"
            );
        }
        // (An unanchored `None` is acceptable — the fault came from synthesized nodes with no source.)
    }

    #[test]
    fn a_type_mismatch_anchors_within_the_program() {
        // An `if` with a non-Bool condition (CDZ0203) anchors to a user node in the program.
        let d = first_error("(module m (def (main) (if 5 1 2)) (export main))");
        assert_eq!(d.code.as_deref(), Some("CDZ0203"));
        if let Some(node) = d.node {
            let ast = parse("(module m (def (main) (if 5 1 2)) (export main))");
            let db = Db::load(ast);
            assert!(
                db.is_user_node(StructId(node)),
                "node {node} must be a user node"
            );
        }
    }

    // ── Dead-trap warning (CDZ0305) — a non-error diagnostic riding alongside a produced artifact.
    // A computation that PROVABLY traps but whose value is unobserved (an unprojected element, an
    // unreferenced binding, an unused argument) is eliminated (`core-semantics.md` §A Trap Occurs Only
    // Where Its Computation Is Observed) — conformant, so the build SUCCEEDS, but a warning is emitted.

    /// Compile a program and return its warning diagnostics (severity `Warning`) — asserting the
    /// component WAS produced (a warning must ride alongside a success, never a denial).
    fn warnings_of(src: &str) -> Vec<crate::abi::Diagnostic> {
        let bytes = crate::codec::encode(&parse(src));
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        assert!(
            out.artifact(Target::Wasm.artifact_kind()).is_some(),
            "a warning must accompany a PRODUCED component, but compilation failed: {:?}",
            out.diagnostics
        );
        out.diagnostics
            .into_iter()
            .filter(|d| d.severity == crate::abi::Severity::Warning)
            .collect()
    }

    #[test]
    fn an_eliminated_provable_trap_warns_but_still_compiles() {
        // Each shape drops a computation that PROVABLY traps because its value is unobserved: an
        // unprojected tuple element, an unread record field, an unreferenced let binding, an argument
        // bound to an unused parameter. All compile (the value is not observed) AND warn CDZ0305.
        for src in [
            "(module m (def (main) (. (tuple 42 (/ 100 0)) 0)) (export main))",
            "(module m (def (main) (. (record (a 42) (b (/ 100 0))) a)) (export main))",
            "(module m (def (main) (let ((t (/ 100 0))) 5)) (export main))",
            "(module m (def (f x y) x) (def (main) (f 7 (/ 100 0))) (export main))",
        ] {
            // Exactly one DEAD-TRAP warning (CDZ0305). Some shapes also carry an unused-binding warning
            // (CDZ0306 — the dropped `let t`, the unused param `y`), which is a separate, correct signal;
            // filter to the dead-trap code so this test pins the dead-trap behavior specifically.
            let dead: Vec<_> = warnings_of(src)
                .into_iter()
                .filter(|d| d.code.as_deref() == Some("CDZ0305"))
                .collect();
            assert_eq!(
                dead.len(),
                1,
                "expected exactly one dead-trap (CDZ0305) warning for `{src}`, got {dead:?}"
            );
        }
    }

    #[test]
    fn a_dead_trap_warning_anchors_to_a_user_node() {
        // The warning carries a node index that resolves to a real user node — the front-end maps it to
        // the trapping computation's span, never a prelude/synthesized id.
        let src = "(module m (def (main) (. (tuple 42 (/ 100 0)) 0)) (export main))";
        let ws = warnings_of(src);
        assert_eq!(ws.len(), 1);
        let node = ws[0].node.expect("a dead-trap warning must carry a node");
        let db = Db::load(parse(src));
        assert!(
            db.is_user_node(StructId(node)),
            "node {node} must be a user node"
        );
    }

    #[test]
    fn an_observed_provable_trap_is_an_error_not_a_warning() {
        // The dual: a provable trap whose value IS observed (element 0 projected) is a build ERROR
        // (CDZ0304), not a warning — it denies the component.
        let src = "(module m (def (main) (. (tuple (/ 1 0) 42) 0)) (export main))";
        let bytes = crate::codec::encode(&parse(src));
        let out = compile(
            &[Artifact::new(Artifact::KIND_AST, "m", bytes)],
            &[Target::Wasm],
        );
        assert!(
            out.artifact(Target::Wasm.artifact_kind()).is_none(),
            "an observed provable trap must deny the component"
        );
        assert_eq!(
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
                .as_deref(),
            Some("CDZ0304")
        );
    }

    #[test]
    fn a_clean_program_and_an_unprovable_trap_do_not_warn() {
        // A clean projection warns not at all; a RUNTIME (unprovable) trap in a dropped position is not
        // flagged either — the warning fires only on a PROVABLE trap, so no false positive on code the
        // compiler cannot prove traps.
        assert!(warnings_of("(module m (def (main) (. (tuple 42 7) 0)) (export main))").is_empty());
        assert!(
            warnings_of(
                "(module m (def (main (: x Int64)) (. (tuple 42 (/ 100 x)) 0)) (export main))"
            )
            .is_empty(),
            "a runtime (unprovable) trap in a dropped position must NOT warn"
        );
    }

    /// The CDZ0306 unused-binding warning messages from `src`. Uses `diagnostics()` directly (the
    /// export-independent fault+warning set `cdz check` drives) rather than `warnings_of` — a program
    /// exporting a parameterized function does not emit a component, but its diagnostics are still
    /// defined (that is the point of the export-independent query).
    fn unused_of(src: &str) -> Vec<String> {
        let mut db = Db::load(parse(src));
        crate::diagnostics(&mut db)
            .into_iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0306"))
            .map(|d| d.message)
            .collect()
    }

    /// The CDZ0306 unused-binding DIAGNOSTICS (full records, so a test can read the carried fix).
    fn unused_diags(src: &str) -> Vec<crate::abi::Diagnostic> {
        let mut db = Db::load(parse(src));
        crate::diagnostics(&mut db)
            .into_iter()
            .filter(|d| d.code.as_deref() == Some("CDZ0306"))
            .collect()
    }

    #[test]
    fn an_unused_binding_carries_a_verified_underscore_prefix_fix() {
        // The first MACHINE-APPLICABLE fix (`spec/capabilities/diagnostics.md` §A Confirmed Fix Is
        // Marked Verified): the CDZ0306 warning's prose "prefix with `_`" is now a structural fix an
        // agent applies WITHOUT review — rename the binder to `_<name>`, which the silencing rule makes
        // behaviour-preserving and clears the warning by construction.
        let src = "(module m (def (f p q) (+ p 1)) (export f))";
        let diags = unused_diags(src);
        assert_eq!(diags.len(), 1, "only q is unused: {diags:?}");
        let fix = diags[0].fix.as_ref().expect("a fix is carried");
        assert_eq!(fix.replacement, "_q", "the `_`-prefixed name");
        assert!(
            fix.verified,
            "the silencing rule makes this fix VERIFIED, not heuristic"
        );
        // And applying it (q → _q) clears the warning — the fix's correctness, demonstrated.
        let fixed = "(module m (def (f p _q) (+ p 1)) (export f))";
        assert!(unused_diags(fixed).is_empty(), "the _-prefix silences it");
    }

    #[test]
    fn a_recursive_functions_used_parameter_is_not_flagged_unused() {
        // A RECURSIVE function freshens its parameter binder when its body is resolved (and the
        // accumulator transform may rewrite the body entirely), so a reference resolves to a
        // SYNTHESIZED param copy, not the original occurrence. The usage check keys on the resolution
        // KIND (does a body reference resolve to a `Param`?), not the target occ, so it is not fooled:
        // `n` here is used three times → it must NOT warn (the reported false positive).
        let src = "(module m (def (sm n) (if (= n 0) 0 (+ n (sm (- n 1))))) (export sm))";
        assert!(
            unused_of(src).is_empty(),
            "a used recursive param must not warn: {:?}",
            unused_of(src)
        );
        // A genuinely-unused parameter still warns (the check is real, not just disabled for
        // functions): `z` is never referenced.
        let with_unused = "(module m (def (f p z) (+ p 1)) (export f))";
        let u = unused_of(with_unused);
        assert_eq!(u.len(), 1, "the truly-unused param z warns: {u:?}");
        assert!(u[0].contains("`z`"), "{u:?}");
    }

    #[test]
    fn an_unused_let_binding_and_parameter_warn() {
        // `b` (a let binding) and `q` (a parameter) are declared but never referenced → CDZ0306; `a`
        // and `p` are used, so they do not warn.
        let src = "(module m (def (f p q) (let ((a (: 1 Int64)) (b (: 2 Int64))) (+ a p))) \
                   (export f))";
        let u = unused_of(src);
        assert_eq!(u.len(), 2, "exactly b and q are unused: {u:?}");
        assert!(
            u.iter().any(|m| m.contains("`b`") && m.contains("binding")),
            "{u:?}"
        );
        assert!(
            u.iter()
                .any(|m| m.contains("`q`") && m.contains("parameter")),
            "{u:?}"
        );
    }

    #[test]
    fn an_underscore_prefix_silences_the_unused_warning() {
        // Rust's convention: a name beginning with `_` is intentionally unused — no warning.
        let src = "(module m (def (f p _q) (let ((a (: 1 Int64)) (_b (: 2 Int64))) (+ a p))) \
                   (export f))";
        assert!(
            unused_of(src).is_empty(),
            "`_q`/`_b` are silenced: {:?}",
            unused_of(src)
        );
    }

    #[test]
    fn an_unused_nonexported_definition_warns_but_a_used_or_exported_one_does_not() {
        // A nullary def nothing references (and not exported) is unused.
        let unused = "(module m (def (helper) (: 9 Int64)) (def (main) 42) (export main))";
        let u = unused_of(unused);
        assert_eq!(u.len(), 1, "helper is unused: {u:?}");
        assert!(
            u[0].contains("`helper`") && u[0].contains("definition"),
            "{u:?}"
        );
        // A def that IS referenced does not warn.
        assert!(
            unused_of("(module m (def (helper) (: 9 Int64)) (def (main) helper) (export main))")
                .is_empty(),
            "a used def must not warn"
        );
        // An EXPORTED def is part of the interface — never flagged.
        assert!(
            unused_of("(module m (def (helper) (: 9 Int64)) (export helper))").is_empty(),
            "an exported def must not warn"
        );
    }
}

// ── Stage 1: let + records (compile-time folded) ────────────────────────────────────────────────
//
// Each case mirrors a `05-compound-types.sexp` / `02-binding-and-control.sexp` witness. A record
// whose field is read FOLDS to a scalar (no value heap), so the component runs to that scalar; a
// record used as a runtime value declines (needs the heap, a later stage). Programs are built with
// the test s-expr reader in `testkit`.
mod stage1 {
    use super::{FromVal, find_runtime_wasm, run_returns, run_returns_with};
    use crate::compile::compile_component;
    use crate::testkit::parse;

    /// Compile `(module m (def (main) BODY) (export main))` and run `main`, returning its result
    /// decoded to `T`. Generic over the boundary type: adding a new one is one more `impl FromVal`.
    fn run_main_as<T: FromVal>(body: &str) -> T {
        let src = format!("(module m (def (main) {body}) (export main))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        run_returns::<T>(&bytes, "main")
    }

    /// The common case: an integer-returning body. (A thin fixed-type alias over `run_main_as` so the
    /// many integer cases read `run_main("..")` and dodge integer-literal type inference.)
    fn run_main(body: &str) -> i64 {
        run_main_as::<i64>(body)
    }

    /// A boolean-returning body — a comparison entrypoint.
    fn run_main_bool(body: &str) -> bool {
        run_main_as::<bool>(body)
    }

    /// Compile the same program shape and expect a DECLINE/reject, returning the error message.
    fn expect_decline(body: &str) -> String {
        let src = format!("(module m (def (main) {body}) (export main))");
        compile_component(&crate::codec::encode(&parse(&src)))
            .expect_err("must decline/reject")
            .message
    }

    /// FOLD `body` (as `main`'s body) to its core `ConstInt` value at arbitrary precision — the value a
    /// constant reduces to BEFORE any machine width or boundary. Used for a NON-ALIASED width like
    /// `(UInt 48)`, whose bounds fold but which has no boundary representation to run across (it is
    /// internal-only — R2). Panics if the body does not fold to a constant integer.
    fn fold_const_u128(body: &str) -> u128 {
        use crate::core::Core;
        use crate::db::Db;
        use crate::lower::core_of;
        let src = format!("(module m (def (main) {body}) (export main))");
        let ast = parse(&src);
        let mut db = Db::load(ast);
        let main_body = db
            .defs
            .iter()
            .find(|d| d.name == "main")
            .and_then(|d| d.body)
            .expect("def main has a body");
        match core_of(&mut db, main_body) {
            Core::ConstInt(v) => {
                assert!(!v.negative, "expected a non-negative constant, got {v:?}");
                let mut acc: u128 = 0;
                for &b in &v.magnitude {
                    acc = (acc << 8) | (b as u128);
                }
                acc
            }
            other => panic!("expected the body to fold to a ConstInt, got {other:?}"),
        }
    }

    #[test]
    fn let_binding_in_scope_in_body() {
        // 02-binding-and-control: (let ((x 10)) x) = 10.
        assert_eq!(run_main("(let ((x 10)) x)"), 10);
    }

    #[test]
    fn name_resolves_to_nearest_enclosing_binding() {
        // (let ((x 1)) (let ((x 2)) x)) = 2 — inner shadows outer.
        assert_eq!(run_main("(let ((x 1)) (let ((x 2)) x))"), 2);
    }

    #[test]
    fn a_later_let_binding_sees_an_earlier_one() {
        // (let ((x 1) (y x)) y) = 1 — the second initializer sees the first binding.
        assert_eq!(run_main("(let ((x 1) (y x)) y)"), 1);
    }

    #[test]
    fn a_repeated_binding_shadows_the_earlier_one() {
        // (let ((x 1) (x 2)) x) = 2 — a repeat shadows for what follows.
        assert_eq!(run_main("(let ((x 1) (x 2)) x)"), 2);
    }

    #[test]
    fn a_do_sequencing_block_yields_its_last_form() {
        // 02-binding-and-control: `(do f1 … fn)` in EXPRESSION position is a sequencing block — its value
        // is the LAST form, the non-final (pure) forms evaluated for their discarded value. `(do 1 2 3)` =
        // 3; a discarded compound intermediate `(do (tuple 1 2) 42)` = 42; nested in a `let` body works.
        assert_eq!(run_main("(do 1 2 3)"), 3);
        assert_eq!(run_main("(do (tuple 1 2) 42)"), 42);
        assert_eq!(run_main("(let ((x 4)) (do (+ x 1) x))"), 4);
    }

    #[test]
    fn a_do_block_with_an_ill_typed_intermediate_is_still_caught() {
        // A non-final `do` form is EVALUATED (value discarded), so an ill-typed intermediate must still be
        // rejected — the fault walk descends into EVERY form, not only the last. `(if 5 1 2)` (a non-Bool
        // condition) in a non-final position is caught even though its value is discarded.
        let msg = expect_decline("(do (if 5 1 2) 42)");
        assert!(
            msg.contains("condition must be Bool"),
            "an ill-typed do intermediate must be caught though discarded; got: {msg}"
        );
    }

    #[test]
    fn a_do_local_declaration_binds_the_following_forms() {
        // 02-binding-and-control §A Declaration In A Sequencing Block Is Scoped To The Forms That Follow
        // It: a `(def …)` form of a `do` binds its name for the LATER forms — a VALUE declaration
        // `(def x 5)` and a FUNCTION declaration `(def (f n) …)` alike, each resolved like a top-level
        // def (a `Ref` / a lambda). A later declaration sees an earlier one (`y` = `(+ x 1)`).
        assert_eq!(run_main("(do (def x 5) (+ x 1))"), 6);
        assert_eq!(run_main("(do (def (f n) (+ n 1)) (f 9))"), 10);
        assert_eq!(run_main("(do (def x 5) (def y (+ x 1)) y)"), 6);
    }

    #[test]
    fn a_do_local_declaration_shadows_last_wins_and_an_outer_binding() {
        // A repeated do-local declaration shadows the earlier one for what follows (last-wins, like a
        // repeated `let` binding): `(def x 5) (def x (+ x 10))` → the second `x` sees the first → 15.
        assert_eq!(run_main("(do (def x 5) (def x (+ x 10)) x)"), 15);
        // A do-local declaration shadows an OUTER lexical binding for the forms that follow it: the inner
        // `(def x 99)` shadows the enclosing `let ((x 1))`, so the block yields 99.
        assert_eq!(run_main("(let ((x 1)) (do (def x 99) x))"), 99);
    }

    #[test]
    fn a_top_level_value_definition_binds_a_name() {
        // 11-modules "a top-level value definition binds a name usable by the program's functions": a
        // bare-name `(def NAME VALUE)` at the top level (signature is a NAME atom, not a `(sig param…)`
        // list) is a VALUE definition — `answer` binds to `42`, resolvable by name in a sibling function,
        // the same shape a nullary `(def (answer) 42)` denotes but written as name-plus-value.
        let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m (def answer 42) (def (main) answer) (export main))",
        )))
        .expect("a top-level value def binds its name");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 42);
        // A value def may bind a COMPOUND (a record), projected by a sibling function — the value def's
        // body is an arbitrary expression, not only a scalar.
        let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m (def tbl (record (a 7) (b 8))) (def (main) (. tbl b)) (export main))",
        )))
        .expect("a top-level value def may bind a record");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 8);
        // A value def may bind a SUM value, matched by a sibling function — the self-hosting compiler's
        // top-level constant AST/config-node shape. `chosen = (C.G unit)` dispatches to arm 2.
        let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m (type C (R unit) (G unit) (B unit)) (def chosen (C.G unit)) \
               (def (main) (match chosen ((C.R _) 1) ((C.G _) 2) ((C.B _) 3))) (export main))",
        )))
        .expect("a top-level value def may bind a sum value");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 2);
        // A sum-valued def may FORWARD-reference a later sum def and transform it (order-independent
        // module scope): `derived = (match base …)` reads `base` defined after it. `derived` = (Some 20).
        let bytes = compile_component(&crate::codec::encode(&parse(
            "(module m (def derived (match base ((Some x) (Some (* x 2))) (None None))) \
               (def base (Some 10)) \
               (def (main) (match derived ((Some v) v) (None 0))) (export main))",
        )))
        .expect("a sum value def may forward-reference a later sum def");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 20);
    }

    #[test]
    fn a_do_local_declaration_scope_is_backward_only() {
        // Sequential scope: a form sees only the declarations BEFORE it. A FORWARD reference (`y`'s value
        // `(+ x 1)` references `x` declared AFTER it) is unbound — a declaration does not see later ones.
        let msg = expect_decline("(do (def y (+ x 1)) (def x 5) y)");
        assert!(
            msg.contains("unbound") && msg.contains('x'),
            "a forward reference to a later do-local declaration must be unbound; got: {msg}"
        );
    }

    #[test]
    fn a_do_block_ending_in_a_declaration_is_malformed() {
        // A sequencing block YIELDS its last form's value, so it must end in a value form: a TRAILING
        // declaration `(do (def x 5))` leaves the block valueless.
        let msg = expect_decline("(do (def x 5))");
        assert!(
            msg.contains("must end in a value form"),
            "a do block ending in a declaration must be malformed; got: {msg}"
        );
    }

    #[test]
    fn an_unused_do_local_declaration_with_an_ill_typed_value_is_still_caught() {
        // A VALUE declaration's value is type-checked EAGERLY (like a `let` binding value) — a fault
        // whether or not the name is later used. `(def x (if 5 1 2))` is ill-typed (non-Bool condition)
        // and caught even though the block yields the unrelated `42`.
        let msg = expect_decline("(do (def x (if 5 1 2)) 42)");
        assert!(
            msg.contains("condition must be Bool"),
            "an ill-typed do-local value declaration must be caught though unused; got: {msg}"
        );
    }

    #[test]
    fn a_local_binding_shadows_a_same_named_top_level_def() {
        // A `let` binding named `f` shadows a top-level `(def (f) …)` of the same name for the extent
        // of its scope: `resolve_name` consults the lexical scope FIRST and the top-level def index
        // (`def_by_name`) only on a scope miss. So the body's `f` is the local `7`, not the def's `99`.
        // This pins that resolution keys on the OCCURRENCE + its scope, never on the flat name index
        // alone — the invariant the O(1) `def_name_index` accelerator must not disturb (and the property
        // a future nested-module / import rework must preserve: same-named bindings at different scopes
        // resolve independently). Runs end-to-end so it checks the emitted component, not just resolve.
        let src = "(module m (def (f) 99) (def (main) (let ((f 7)) f)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 7);
    }

    #[test]
    fn same_named_defs_are_distinct_bindings() {
        // The flat top-level namespace keys defs by name (first-wins) — but a def's IDENTITY is its
        // occurrence, and the `def_name_index` is only a lookup accelerator over `defs`, never the
        // source of truth. Two defs are still distinct `Db::defs` entries at distinct occurrences even
        // when they share a name; the index simply resolves the bare NAME to the first. This is the
        // seam the future module rework re-keys (by enclosing module) so sibling submodules' same-named
        // defs stop colliding — see the `def_name_index` field doc. Here we assert the property the
        // rework must uphold: distinct occurrences, and a deterministic first-wins name resolution.
        use crate::db::Db;
        let src = "(module m (def (a) 1) (def (b) 2) (def (main) (+ (a) (b))) (export main))";
        let db = Db::load(parse(src));
        // Every def is its own entry (distinct occurrences), regardless of name.
        let occs: std::collections::HashSet<_> = db.defs.iter().filter_map(|d| d.body).collect();
        assert_eq!(
            occs.len(),
            db.defs.iter().filter(|d| d.body.is_some()).count()
        );
        // The name index resolves each name to a real, distinct def; `main` sums them → 3.
        assert!(db.def_by_name("a").is_some());
        assert!(db.def_by_name("b").is_some());
        assert_ne!(db.def_by_name("a"), db.def_by_name("b"));
        assert_eq!(run_main("(+ (let ((z 1)) z) (let ((z 2)) z))"), 3);
    }

    #[test]
    fn record_field_read_folds_to_the_field() {
        // 05-compound-types: (let ((p (record (x 1) (y 2)))) p.x) = 1 (written in canonical dot form).
        assert_eq!(run_main("(let ((p (record (x 1) (y 2)))) (. p x))"), 1);
    }

    #[test]
    fn projection_is_order_independent() {
        // (. (record (b 2) (a 1)) a) = 1 — projection resolves by NAME, not written position.
        assert_eq!(run_main("(. (record (b 2) (a 1)) a)"), 1);
    }

    #[test]
    fn member_access_chains_through_a_nested_record() {
        // (. (. (record (outer (record (inner 7)))) outer) inner) = 7.
        assert_eq!(
            run_main("(. (. (record (outer (record (inner 7)))) outer) inner)"),
            7
        );
    }

    #[test]
    fn unbound_name_is_rejected() {
        // 02-binding-and-control: a reference to an unbound name is rejected before running.
        assert!(expect_decline("nope").contains("unbound name"));
    }

    // ── "did you mean?" — the rustc-gold-standard fix suggestion for an unbound name ─────────────────

    /// The full error `Diagnostic` for `(module m (def (main) BODY) (export main))` — like
    /// `expect_decline`, but keeps the whole record (code + message + the structural fix) so a test can
    /// assert on the proposed repair, not just the message text.
    fn expect_error(body: &str) -> crate::abi::Diagnostic {
        let src = format!("(module m (def (main) {body}) (export main))");
        compile_component(&crate::codec::encode(&parse(&src))).expect_err("must decline/reject")
    }

    #[test]
    fn an_unbound_name_close_to_a_def_suggests_it_with_a_heuristic_fix() {
        // A typo of a top-level def (`compute` → `computee`) is CDZ0101, but the diagnostic now NAMES
        // the near candidate AND carries a structural fix an agent applies directly
        // (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix).
        let src = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))";
        let d = compile_component(&crate::codec::encode(&parse(src))).expect_err("must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
        assert!(
            d.message.contains("did you mean `compute`?"),
            "message should name the candidate: {}",
            d.message
        );
        let fix = d.fix.expect("a fix is carried");
        assert_eq!(fix.replacement, "compute");
        assert!(
            !fix.verified,
            "a nearest-name guess is heuristic, not verified"
        );
        // The fix's node is the faulting reference itself — the diagnostic anchors there too, so an
        // editor highlights and rewrites the same span.
        assert_eq!(Some(fix.node), d.node, "fix targets the faulting node");
    }

    #[test]
    fn an_unbound_name_close_to_a_let_binding_suggests_the_binding() {
        // A lexical binder in scope is a suggestion candidate too — `counter` bound by an enclosing
        // `let`, referenced as `countr`.
        let d = expect_error("(let ((counter 5)) (+ countr 1))");
        assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("counter"),
            "message: {}",
            d.message
        );
    }

    #[test]
    fn an_unbound_name_close_to_a_prelude_builtin_suggests_it() {
        // The prelude's names are candidates — `List` misspelled `Lst` (a module head).
        let d = expect_error("(Lst.push (list 1 2) 3)");
        assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("List"),
            "message: {}",
            d.message
        );
    }

    #[test]
    fn a_genuinely_unknown_name_carries_no_misleading_suggestion() {
        // No in-scope name is within the edit-distance cutoff of `frobnicate`, so the diagnostic stays
        // the plain "unbound name" — no fix, no misleading "did you mean". (A false suggestion is worse
        // than none: an agent would apply the wrong edit.)
        let d = expect_error("frobnicate");
        assert_eq!(d.code.as_deref(), Some("CDZ0101"), "got: {}", d.message);
        assert!(
            !d.message.contains("did you mean"),
            "no suggestion: {}",
            d.message
        );
        assert!(d.fix.is_none(), "no fix carried: {:?}", d.fix);
    }

    #[test]
    fn the_suggestion_is_deterministic_across_equidistant_candidates() {
        // Two defs equidistant (1 edit) from the reference `ab`: `ac` and `xb`. The tie breaks on the
        // lexicographically-smaller candidate, so the suggestion is a pure function of the source
        // (`spec/capabilities/diagnostics.md` §A Fix Is A Deterministic Function Of The Source), never
        // dependent on def/hash iteration order.
        let src = "(module m (def (ac) 1) (def (xb) 2) (def (main) (+ (ab) 0)) (export main))";
        let d = compile_component(&crate::codec::encode(&parse(src))).expect_err("must reject");
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("ac"),
            "lexicographically-smaller candidate wins the tie: {}",
            d.message
        );
    }

    #[test]
    fn a_malformed_digit_led_token_is_a_malformed_literal_not_an_unbound_name() {
        // A digit-led token that fails numeric parsing (`0o17` octal, `0x`/`0b` empty radix body,
        // `0xGG` bad hex digit, `123abc` digits-then-letters) is a MALFORMED LITERAL (CDZ0201), NOT an
        // "unbound name" (CDZ0101) — a digit-led token is a number, never an identifier
        // (01-literals.sexp). The reader classifies it as a bare name; the well-formedness call is made
        // at resolution.
        for tok in ["0o17", "0x", "0xGG", "0b12", "123abc"] {
            let msg = expect_decline(tok);
            assert!(
                msg.contains("malformed numeric literal"),
                "digit-led `{tok}` must be a malformed literal, got: {msg}"
            );
        }
        // A real (non-digit-led) unbound name is STILL an unbound name, not misclassified.
        assert!(expect_decline("frobnicate").contains("unbound name"));
        // A well-formed literal is unaffected.
        assert_eq!(run_main("0x2A"), 42);
    }

    #[test]
    fn if_condition_must_be_bool() {
        // A non-boolean condition is a type error (CDZ0203) — `(if 5 1 2)` has an Int64 condition.
        let msg = expect_decline("(if 5 1 2)");
        assert!(msg.contains("condition must be Bool"), "got: {msg}");
    }

    #[test]
    fn if_branches_must_agree() {
        // Branches of differing type are a type error (CDZ0203) — Int64 `then` vs Bool `else`.
        let msg = expect_decline("(if true 1 true)");
        assert!(msg.contains("branches differ"), "got: {msg}");
    }

    #[test]
    fn a_type_fault_inside_a_let_body_is_still_caught() {
        // The check descends through a `let` — a bad condition in the body is still reported.
        let msg = expect_decline("(let ((x 5)) (if x 1 2))");
        assert!(msg.contains("condition must be Bool"), "got: {msg}");
    }

    #[test]
    fn duplicate_field_name_is_rejected() {
        // 05-compound-types: (record (a 1) (a 2)) — a record's field names are a SET → CDZ0201. Read a
        // field so the record is reached (a bare record value would decline for the heap first).
        assert!(expect_decline("(. (record (a 1) (a 2)) a)").contains("more than once"));
    }

    #[test]
    fn non_adjacent_duplicate_field_name_is_rejected() {
        // (record (a 1) (b 2) (a 3)) — the check is over the whole field list, not adjacent pairs.
        assert!(expect_decline("(. (record (a 1) (b 2) (a 3)) b)").contains("more than once"));
    }

    #[test]
    fn member_access_on_a_non_record_is_a_type_error() {
        // 05-compound-types: (. 5 x) — member access requires a record → CDZ0201.
        assert!(expect_decline("(. 5 x)").contains("requires a record"));
    }

    #[test]
    fn member_access_of_a_missing_field_is_a_type_error() {
        // (. (record (x 1)) y) — the user record is CLOSED; a missing field rejects (CDZ0201).
        assert!(expect_decline("(. (record (x 1)) y)").contains("no field"));
    }

    #[test]
    fn a_misspelled_field_on_a_visible_record_suggests_the_nearest_field() {
        // The record analogue of the unbound-name "did you mean?" — a field typo (`height` → `heigth`)
        // on a compile-time-visible record names the near field AND carries a heuristic fix on the KEY
        // token (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix).
        let d = expect_error("(. (record (width 10) (height 20)) heigth)");
        assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
        assert!(
            d.message.contains("did you mean `height`?"),
            "names the near field: {}",
            d.message
        );
        let fix = d.fix.expect("a fix is carried");
        assert_eq!(fix.replacement, "height");
        assert!(!fix.verified, "a nearest-field guess is heuristic");
    }

    #[test]
    fn a_misspelled_field_on_a_runtime_record_type_suggests_the_nearest_field() {
        // A RUNTIME record (reached through a def parameter that inlines a record argument) carries a
        // record TYPE; the field typo is caught on that type and suggested the same way. `get-h`'s body
        // `(. r heigth)` faults once `r`'s record argument flows in.
        let src = "(module m (def (get-h r) (. r heigth)) \
                   (def (main) (get-h (record (width 10) (height 20)))) (export main))";
        let d = compile_component(&crate::codec::encode(&parse(src))).expect_err("must reject");
        assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
        assert_eq!(
            d.fix.as_ref().map(|f| f.replacement.as_str()),
            Some("height"),
            "message: {}",
            d.message
        );
    }

    #[test]
    fn a_field_with_no_close_match_carries_no_suggestion() {
        // No field is within the edit-distance cutoff of `zzzzzz` — the plain "no field" message, no
        // misleading fix.
        let d = expect_error("(. (record (width 10) (height 20)) zzzzzz)");
        assert_eq!(d.code.as_deref(), Some("CDZ0201"), "got: {}", d.message);
        assert!(
            !d.message.contains("did you mean"),
            "no suggestion: {}",
            d.message
        );
        assert!(d.fix.is_none(), "no fix carried: {:?}", d.fix);
    }

    #[test]
    fn returning_a_constant_record_compiles_via_the_resource_escape() {
        // A CONSTANT record returned as the program result now crosses the host boundary as a
        // component-model resource whose `encode()` yields the canonical value form (the escape path,
        // §3a) — it no longer declines. (The end-to-end value assertion, decoding to `(: (record …)
        // (Record …))`, is `constant_resource_escape::a_constant_record_return_decodes`.) A record
        // consumed INTERNALLY still folds/declines per its use; this pins that a constant record RESULT
        // compiles to a component.
        let src = "(module m (def (main) (record (x 1))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a constant record return must compile via the resource escape"
        );
    }

    // ── Value-heap H2a: tuple surface + fold (static-correct first) ───────────────────────────────
    //
    // A tuple projected by a constant index over a compile-time-visible tuple FOLDS to the element (no
    // heap) — the same static reduction a record member fold takes, so these run to a scalar. A runtime
    // tuple (one that escapes) DECLINES pending H2b's heap emission. An out-of-arity index and a non-
    // tuple projection are COMPILE-TIME rejects (CDZ0201), never runtime traps.

    #[test]
    fn a_constant_tuple_projection_folds_to_its_element() {
        // `(. (tuple 10 20 30) 1)` — a projection of a visible tuple by a constant index folds to the
        // element `20`, with no heap: the component runs to 20.
        assert_eq!(run_main("(. (tuple 10 20 30) 1)"), 20);
        // Position 0 and the last position, too.
        assert_eq!(run_main("(. (tuple 10 20 30) 0)"), 10);
        assert_eq!(run_main("(. (tuple 10 20 30) 2)"), 30);
    }

    #[test]
    fn a_let_bound_constant_tuple_projection_folds() {
        // A `let`-bound tuple projected ONCE is copy-propagated (single use) and folds — the binding is
        // not kept, so the projection sees the literal `(tuple 1 2)` and reduces to `2`.
        assert_eq!(run_main("(let ((t (tuple 1 2))) (. t 1))"), 2);
    }

    #[test]
    fn positional_projection_is_member_access_with_an_integer_key() {
        // Positional tuple projection is the one member-access form with an INTEGER key — `(. operand N)`
        // (a NAME key reads a record field, an integer key a tuple element). `(. (tuple 10 20) 1)` folds
        // to 20. (There is no `tuple.N` sigil — the single `.` form serves both.)
        assert_eq!(run_main("(. (tuple 10 20) 1)"), 20);
        assert_eq!(run_main("(. (tuple 10 20) 0)"), 10);
        // A multi-digit index is one integer key (not per-digit): position 10 of an 11-tuple.
        assert_eq!(run_main("(. (tuple 0 1 2 3 4 5 6 7 8 9 99) 10)"), 99);
    }

    #[test]
    fn a_nested_tuple_projection_folds() {
        // `(. (. (tuple (tuple 1 2) (tuple 3 4)) 1) 0)` — project the second inner tuple, then its
        // first element: folds through both levels to `3`.
        assert_eq!(run_main("(. (. (tuple (tuple 1 2) (tuple 3 4)) 1) 0)"), 3);
    }

    #[test]
    fn an_out_of_arity_tuple_index_is_a_compile_time_error() {
        // `(. (tuple 10 20 30) 3)` — position 3 of a 3-element tuple (valid 0..2) is out of range. It
        // is REJECTED at compile time (CDZ0201), NOT deferred to a runtime trap (the arity-trap
        // learning: an out-of-arity index on a statically-known tuple is a type error).
        let msg = expect_decline("(. (tuple 10 20 30) 3)");
        assert!(msg.contains("out of range"), "got: {msg}");
    }

    #[test]
    fn a_tuple_projection_on_a_non_tuple_is_a_compile_time_error() {
        // `(. 5 0)` — projecting a position of a non-tuple has no defined result → CDZ0201 at compile
        // time. (An integer key distinguishes this from record member access, which says "requires a
        // record"; a tuple projection says "requires a tuple".)
        let msg = expect_decline("(. 5 0)");
        assert!(msg.contains("requires a tuple"), "got: {msg}");
    }

    #[test]
    fn returning_a_constant_tuple_compiles_via_the_resource_escape() {
        // A CONSTANT tuple returned across the host boundary now crosses as a monomorphized component
        // RESOURCE whose `encode() -> list<u8>` yields the canonical binary value form; the host decodes
        // + prints `(: (tuple …) (Tuple …))` (the escape path, §3a). It no longer declines. (The
        // end-to-end value assertion is `constant_resource_escape::a_constant_tuple_return_decodes…`.)
        // A RUNTIME tuple (elements computed at run time) still declines here pending R2 (the real
        // handle-walking encoder) — see `a_runtime_tuple_return_still_declines_pending_r2`.
        let src = "(module m (def (main) (tuple 1 2)) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a constant tuple return must compile via the resource escape"
        );
    }

    #[test]
    fn a_parameterized_compound_return_export_declines() {
        // A tuple returned from an export that TAKES A PARAMETER cannot cross as the resource escape:
        // the resource's constructor is `make : () -> own<t>` (NULLARY), so the escaping export must be
        // nullary. A parameterized compound-returning export therefore still DECLINES here (its
        // component functype would need a compound RESULT with parameters, which the boundary does not
        // carry — reject-don't-miscompile). The runtime-compound escape (R2) covers the NULLARY case: a
        // recursive/heap-built tuple returned from a nullary export now crosses (see
        // `a_recursive_runtime_tuple_escapes_to_the_host`). Pins the nullary-only escape boundary.
        let src = "(module m (def (pair (: n Int64)) (tuple n 1)) (export pair))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a parameterized compound-return export declines");
        // The message names the ACTUAL trigger — a NULLARY-only resource escape, and this export takes a
        // parameter — NOT the misleading "multi-export boundary" (there is one export). See the emit-side
        // diagnosis in `backend::wasm::emit`.
        assert!(
            err.message.contains("NULLARY export") && err.message.contains("takes a parameter"),
            "the parameterized-compound decline must name the nullary-export trigger, got: {}",
            err.message
        );
    }

    #[test]
    fn a_nullary_recursive_sum_return_renders_via_the_value_encode_walker() {
        // A NULLARY `main` returning a RECURSIVE sum built at runtime (`count 3` → an `IntList` whose
        // spine length is decided at run time) now COMPILES: its `encode()` bakes the shape descriptor and
        // calls the runtime `value-encode` op to render the unbounded-depth value form
        // (DESIGN-recursive-sum-escape-walker.md). Previously this declined "needs a value-form walker
        // that loops to a runtime-determined depth". The escaping component IMPORTS the runtime (the
        // walker calls `value-encode`/`bytes-*`); the end-to-end render is corpus-gated
        // (05-compound-types "renders its full runtime spine"). Here we pin that it compiles + imports the
        // runtime rather than declining.
        use crate::testkit::parse;
        let src = "(module m (type IntList (Cons (Tuple Int64 IntList)) Nil) \
                     (def (count (: n Int64)) (if (< n 1) (IntList.Nil ()) \
                        (IntList.Cons (tuple n (count (- n 1)))))) \
                     (def (main) (count 3)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("a nullary recursive-sum return compiles via the value-encode walker");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "the recursive-sum escape imports the runtime (its encode() calls value-encode)"
        );
    }

    #[test]
    fn a_value_eq_on_a_sum_payload_string_compiles() {
        // Comparing a variant's PAYLOAD (a `SumPayload`/tuple-element read) to a constant string —
        // `(= h "+")` where `h` is bound from a `(NPrim (tuple h a b))` payload — is the shape a recursive
        // resolver dispatches on. The `value-eq` operand-ownership analysis must classify a payload READ
        // as Borrowed (the enclosing compound owns it) and a constant-string literal as Owned (it
        // materializes a fresh byte-leaf that the compare drops); previously the `ConstStr` operand
        // declined "an ownership this backend cannot yet prove", blocking the whole resolver.
        let src = "(module m (type N (NI Int64) (NP (Tuple String N))) \
                     (def (f n) (match n ((NI v) v) ((NP (tuple h t)) (if (= h \"+\") (f t) 0)))) \
                     (def (main) (f (NP (tuple \"+\" (NI 5))))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a value-eq comparing a sum-payload string to a constant compiles (payload=Borrowed, \
             const string=Owned)"
        );
    }

    #[test]
    fn a_multi_export_compound_return_declines_with_the_multi_export_diagnosis() {
        // The OTHER compound-return trigger: a program with MULTIPLE exports, one returning a compound.
        // The resource-escape path takes only a SINGLE nullary compound export, so a multi-export program
        // declines — and here the message DOES name "multiple exports" (the trigger that actually
        // applies), distinct from the single-parameterized-export diagnosis above. Pins the two triggers
        // are diagnosed separately, not conflated into one misleading phrase.
        let src =
            "(module m (def (main) (tuple 5 6)) (def (other) 7) (export main) (export other))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a multi-export compound return declines");
        assert!(
            err.message.contains("multiple exports"),
            "a multi-export compound return must name the multi-export trigger, got: {}",
            err.message
        );
    }

    #[test]
    fn an_escaped_value_with_an_unresolved_type_reports_an_ambiguity_not_an_export_shape_error() {
        // A bare `(None)` returned as the program result has type `(Option ?0)` — the payload is a free
        // variable nothing constrains, so the escaped value has no defined serialization. This is a
        // SINGLE NULLARY sum export (the escape path's shape IS satisfied), so the reject must name the
        // AMBIGUOUS TYPE and the annotation fix (CDZ0203), NOT the export-shape message (which would
        // misdiagnose — the shape is fine, the type is undetermined). The prior message wrongly said the
        // sum "crosses only as a single nullary export's result" — which it already is.
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse("(module m (def (main) (None)) (export main))")),
            )],
            &[crate::backend::Target::Wasm],
        );
        let err = out
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .expect("a bare escaped None with an unresolved payload declines");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0203"),
            "an ambiguous escaped type is a type fault"
        );
        assert!(
            err.message.contains("not fully determined") && err.message.contains("annotate"),
            "the message must name the unresolved type + the annotation fix, got: {}",
            err.message
        );
        // CONTROLS: an ANNOTATED None escapes (the type is resolved), and a `(Some 5)` (payload
        // constrained by its argument) escapes — the ambiguity bites ONLY an unannotated free-var escape.
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (main) (: (None) (Option Int64))) (export main))"
            )))
            .is_ok(),
            "an annotated None must escape"
        );
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (main) (Some 5)) (export main))"
            )))
            .is_ok(),
            "a Some with a constrained payload must escape"
        );
    }

    #[test]
    fn tuple_branches_of_different_arity_are_a_type_error() {
        // 02-binding-and-control: `(if true (tuple 1 2) (tuple 3 4 5))` pairs a 2-tuple with a 3-tuple —
        // different types (a tuple's arity is part of its type), so the whole `if` has no single type
        // and is rejected (CDZ0201/0203). A check comparing only branch KIND (both "a tuple") would
        // wrongly accept it. Projected to keep the whole thing a value the compiler must type.
        let msg = expect_decline("(. (if true (tuple 1 2) (tuple 3 4 5)) 0)");
        assert!(msg.contains("branches differ"), "got: {msg}");
    }

    #[test]
    fn tuple_branches_of_different_element_type_are_a_type_error() {
        // `(if true (tuple 1 2) (tuple 1 true))` pairs `(Tuple Int64 Int64)` with `(Tuple Int64 Bool)`
        // — the second position's types differ, so the branches disagree (checked STRUCTURALLY, not at
        // coarse kind). Rejected.
        let msg = expect_decline("(. (if true (tuple 1 2) (tuple 1 true)) 0)");
        assert!(msg.contains("branches differ"), "got: {msg}");
    }

    // ── the prelude: a built-in module is an arena record, reached by the same projection ──────

    #[test]
    fn int64_max_is_a_folding_constant() {
        // `Int64.max` — a built-in reached by the SAME member-access-and-fold path as `p.x`. Folds to
        // the constant, runs with no value heap. (The reader desugars `Int64.max` to `(. Int64 max)`.)
        assert_eq!(run_main("(. Int64 max)"), i64::MAX);
    }

    #[test]
    fn int64_min_is_a_folding_constant() {
        assert_eq!(run_main("(. Int64 min)"), i64::MIN);
    }

    #[test]
    fn a_program_binding_shadows_a_builtin() {
        // The scope-first lookup means a `let`-bound `Int64` HIDES the built-in module — no special
        // case (`prelude-and-resolution.md` §Name Resolution Is One Ordered Lookup).
        assert_eq!(run_main("(let ((Int64 5)) Int64)"), 5);
    }

    #[test]
    fn the_bare_name_unit_is_the_unit_value() {
        // `unit` is a prelude value — an alias for the empty list `()`, the other spelling of the unit
        // value (core-semantics.md #Unit And The Empty Tuple Are The Same Value). It must RESOLVE (not
        // reject "unbound name `unit`") and produce the unit value exactly as `()` does. Compiling a
        // unit-returning `main` from each spelling must succeed and yield the SAME core.
        let core_of_body = |body: &str| {
            let src = format!("(module m (def (main) {body}) (export main))");
            // A unit `main` has no scalar result to run; assert it COMPILES (resolves) — before the fix
            // `unit` rejected CDZ0101 and no component was produced.
            compile_component(&crate::codec::encode(&crate::testkit::parse(&src)))
                .unwrap_or_else(|e| panic!("`{body}` must compile: {}", e.message))
        };
        // Both spellings compile to a valid component (the unit value crosses the boundary as an empty
        // result). `unit` no longer rejects as an unbound name.
        let from_unit = core_of_body("unit");
        let from_parens = core_of_body("()");
        // The two produce byte-identical components — `unit` and `()` are the same value.
        assert_eq!(
            from_unit, from_parens,
            "`unit` and `()` must compile to the same component (the same unit value)"
        );
    }

    #[test]
    fn two_unit_values_compare_equal() {
        // There is exactly ONE unit value, so any two units compare EQUAL — `(= unit ())` is true,
        // regardless of which spelling (`unit` / `()`) each operand uses. Folded at compile time: unit
        // carries no data and has no machine slot, so this is a trivial constant, NOT a "compound needs
        // a heap walk" decline. `<`/`>` are false (Equal, not Less/Greater); `<=`/`>=` are true.
        assert!(run_main_as::<bool>("(= unit ())"));
        assert!(run_main_as::<bool>("(= unit unit)"));
        assert!(run_main_as::<bool>("(= () ())"));
        assert!(!run_main_as::<bool>("(< unit ())"));
        assert!(run_main_as::<bool>("(<= unit ())"));
        assert!(run_main_as::<bool>("(>= () unit)"));
        // A unit compared against a non-unit is a TYPE error — the operator is `∀a. a → a → Bool`, so
        // both operands must be the SAME type; `(= unit 5)` cannot unify Unit with Int64 (CDZ0203).
        let e = compile_component(&crate::codec::encode(&crate::testkit::parse(
            "(module m (def (main) (= unit 5)) (export main))",
        )))
        .expect_err("comparing unit with an int must reject");
        assert_eq!(e.code.as_deref(), Some("CDZ0203"), "got: {}", e.message);
    }

    #[test]
    fn a_binding_shadows_the_tuple_and_record_constructor_aliases() {
        // `tuple`/`record` are SHADOWABLE prelude aliases for the symbol primitives `(,)`/`{}`. A local
        // binding of the name wins in application-head position via the ordinary scope-first lookup —
        // `(tuple 3 4)` applies the bound function (7), NOT the built-in tuple value. Earlier the
        // structural grammar dispatch on the head name won over the binding (wrong value / spurious
        // reject) — the head-vs-value resolution split #Binding Is Lexical forbids.
        assert_eq!(
            run_main("(let ((tuple (fn (a b) (+ a b)))) (tuple 3 4))"),
            7
        );
        assert_eq!(
            run_main("(let ((record (fn (a b) (+ a b)))) (record 3 4))"),
            7
        );
        // A PARAMETER named `tuple` shadows it just the same (applies the argument function): 3*4 = 12.
        assert_eq!(
            run_main("((fn (tuple) (tuple 3 4)) (fn (a b) (* a b)))"),
            12
        );
        // TYPE soundness: the shadowed result types at the binding's Int64 return, so `(+ … 1)` = 8
        // (earlier CDZ0203 'cannot unify Int64 with (Tuple Int64 Int64)' — the type-level split).
        assert_eq!(
            run_main("(+ (let ((tuple (fn (a b) (+ a b)))) (tuple 3 4)) 1)"),
            8
        );
    }

    #[test]
    fn the_unshadowed_tuple_and_record_aliases_build_the_compound() {
        // With no shadowing binding, the alias names build the compound exactly as the string
        // primitives do — a projection of a constant compound FOLDS to its element (no heap).
        assert_eq!(run_main("(. (tuple 7 8) 0)"), 7);
        assert_eq!(run_main("(. (record (x 1) (y 2)) y)"), 2);
        // The STRING primitives are equivalent — a string head is unspellable as an identifier (so
        // unshadowable) yet writable directly, and builds the same compound as the alias. ("The strings
        // are the symbols.")
        assert_eq!(run_main("(. (\"tuple\" 7 8) 1)"), 8);
        assert_eq!(run_main("(. (\"record\" (x 1) (y 2)) x)"), 1);
    }

    #[test]
    fn a_string_primitive_head_is_not_shadowable_by_a_same_named_binding() {
        // The primitive is a STRING, the alias a NAME — distinct leaf kinds. So even when the NAME
        // `tuple` is shadowed by a binding, the STRING primitive `"tuple"` still builds a tuple: the
        // binding shadows the alias name, never the unspellable string. `(. ("tuple" 7 8) 0)` = 7 even
        // under `(let ((tuple …)) …)`.
        assert_eq!(
            run_main("(let ((tuple (fn (a b) (+ a b)))) (. (\"tuple\" 7 8) 0))"),
            7
        );
    }

    #[test]
    fn an_unrealized_builtin_field_declines() {
        // `(. Int64 of)` — the field EXISTS (present as a poison), so projecting it declines "not yet
        // realized" rather than rejecting as absent. No open-module rule: it is filled with a poison.
        let msg = expect_decline("(. Int64 of)");
        assert!(msg.contains("not yet realized"), "got: {msg}");
    }

    #[test]
    fn an_absent_builtin_field_rejects_like_a_closed_record() {
        // `(. Int64 bogus)` — a field the module does NOT carry rejects (CDZ0201), the same closed
        // projection a user record takes. Realized/unrealized/absent are one uniform projection.
        let msg = expect_decline("(. Int64 bogus)");
        assert!(msg.contains("no field"), "got: {msg}");
    }

    // ── arithmetic intrinsics: application of a built-in operation, generic over the integer type ──

    #[test]
    fn addition_folds() {
        // `(+ 2 3)` = 5 — application of the built-in `+`, folded at compile time (no runtime op).
        assert_eq!(run_main("(+ 2 3)"), 5);
    }

    #[test]
    fn nested_arithmetic_folds() {
        // `(* 2 (+ 3 4))` = 14 — the fold composes through the tree.
        assert_eq!(run_main("(* 2 (+ 3 4))"), 14);
    }

    #[test]
    fn subtraction_folds() {
        assert_eq!(run_main("(- 10 3)"), 7);
    }

    #[test]
    fn arithmetic_is_generic_over_the_integer_type() {
        // A built-in bound (`Int64.max`) and a literal share the operation without a hard-coded width:
        // `(- Int64.max Int64.max)` = 0. Both operands are the signed-64 instance; the op unifies them.
        assert_eq!(run_main("(- (. Int64 max) (. Int64 max))"), 0);
    }

    // ── the full binary-integer operator set (all fold at width 64) ──────────────────────────────

    #[test]
    fn division_truncates_toward_zero() {
        // 06-numeric-model: (/ 7 2) = 3, (/ -7 2) = -3 — truncation toward zero, not floor.
        assert_eq!(run_main("(/ 7 2)"), 3);
        assert_eq!(run_main("(/ -7 2)"), -3);
    }

    #[test]
    fn remainder_takes_sign_of_dividend() {
        // (% -7 2) = -1 — the remainder's sign follows the dividend.
        assert_eq!(run_main("(% -7 2)"), -1);
    }

    #[test]
    fn remainder_by_minus_one_is_zero_even_at_min() {
        // (% MIN -1) = 0 — the one case Rust's `%` panics on; must fold to 0, not trap.
        assert_eq!(run_main("(% -9223372036854775808 -1)"), 0);
    }

    #[test]
    fn division_by_zero_fails_the_build() {
        // (/ 5 0) — a compile-provable trap fails the build (CDZ0304), not a shipped runtime trap.
        assert!(expect_decline("(/ 5 0)").contains("defined value"));
    }

    #[test]
    fn division_of_min_by_minus_one_overflows() {
        // (/ MIN -1) overflows Int64 (the result 2^63 doesn't fit) — CDZ0304.
        assert!(expect_decline("(/ -9223372036854775808 -1)").contains("defined value"));
    }

    #[test]
    fn bitwise_ops_fold() {
        // 06-numeric-model: & masks, | combines, ^ toggles.
        assert_eq!(run_main("(& 255 127)"), 127);
        assert_eq!(run_main("(| 12 3)"), 15);
        assert_eq!(run_main("(^ 12 10)"), 6);
    }

    #[test]
    fn left_shift_is_multiplication_and_folds() {
        // (<< 1 7) = 128 — a left shift is exact multiplication by 2^count.
        assert_eq!(run_main("(<< 1 7)"), 128);
    }

    #[test]
    fn arithmetic_right_shift_folds() {
        // (>> 256 7) = 2, and a NEGATIVE value sign-extends: (>> -256 7) = -2 (arithmetic, not logical).
        assert_eq!(run_main("(>> 256 7)"), 2);
        assert_eq!(run_main("(>> -256 7)"), -2);
    }

    #[test]
    fn left_shift_that_overflows_fails_the_build() {
        // (<< 4611686018427387904 1) overflows Int64 — traps like `*`, so CDZ0304, not a silent wrap.
        assert!(expect_decline("(<< 4611686018427387904 1)").contains("defined value"));
    }

    #[test]
    fn shift_by_the_width_or_more_fails_the_build() {
        // (<< 1 64) — a shift count ≥ width traps rather than masking (CDZ0304).
        assert!(expect_decline("(<< 1 64)").contains("defined value"));
    }

    #[test]
    fn negative_shift_count_fails_the_build() {
        // (<< 1 -1) — a negative shift count traps rather than masking (CDZ0304).
        assert!(expect_decline("(<< 1 -1)").contains("defined value"));
    }

    // ── comparisons: ∀a. a → a → Bool, folded to a boolean, generic over the operand type ────────

    #[test]
    fn integer_comparisons_fold_to_bool() {
        // Each relational operator folds two constant integers to a boolean. The entrypoint returns a
        // Bool at the boundary (03-equality-and-observation).
        assert!(run_main_bool("(< 1 2)"));
        assert!(!run_main_bool("(> 1 2)"));
        assert!(run_main_bool("(<= 2 2)"));
        assert!(!run_main_bool("(>= 1 2)"));
        assert!(run_main_bool("(= 3 3)"));
        assert!(!run_main_bool("(= 3 4)"));
    }

    #[test]
    fn comparison_is_generic_over_bool_operands() {
        // 03-equality-and-observation: (< false true) = true, (<= false false) = true. The SAME
        // operator orders booleans — its type is ∀a. a → a → Bool, a bare operand variable, so `a`
        // unifies with Bool exactly as it does with an integer. No integer-specific comparison.
        assert!(run_main_bool("(< false true)"));
        assert!(run_main_bool("(<= false false)"));
        assert!(run_main_bool("(> true false)"));
        assert!(run_main_bool("(= true true)"));
    }

    #[test]
    fn constant_compound_equality_folds_and_a_runtime_one_emits_a_heap_walk() {
        // Equality of two CONSTANT compounds folds STRUCTURALLY (core-semantics.md §Equality Is
        // Structural: same type + component-wise equal). `(= (Some 1) (Some 1))` → true; `(= (Some 1)
        // (Some 2))` → false; `(= None None)` → true; `(= (tuple 1 2) (tuple 1 2))` → true; a nested
        // compound recurses. Was declined "comparison of a compound value needs a heap walk".
        for (body, want) in [
            (
                "(module m (def (main) (if (= (Some 1) (Some 1)) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (Some 1) (Some 2)) 1 0)) (export main))",
                0,
            ),
            (
                "(module m (def (main) (if (= None None) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (tuple 1 2) (tuple 1 2)) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (tuple 1 2) (tuple 1 3)) 1 0)) (export main))",
                0,
            ),
            (
                "(module m (def (main) (if (= (Some (Some 1)) (Some (Some 1))) 1 0)) (export main))",
                1,
            ),
            // Bytes structural equality (10-bytes.sexp): two constant byte sequences compare equal iff
            // the same bytes in the same order — the byte companion of the tuple/list arm. A `Bytes.concat`
            // and a `Bytes.compact` already fold to a constant `Core::BytesOf`, so their `=` witnesses fold
            // here too.
            (
                "(module m (def (main) (if (= (Bytes.of (list 10 20 30)) (Bytes.of (list 10 20 30))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (Bytes.of (list 10 20 30)) (Bytes.of (list 10 20 99))) 1 0)) (export main))",
                0,
            ),
            (
                "(module m (def (main) (if (= (Bytes.of (list 1 2)) (Bytes.of (list 1 2 3))) 1 0)) (export main))",
                0,
            ),
            (
                "(module m (def (main) (if (= (Bytes.of (list)) (Bytes.of (list))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))) (Bytes.of (list 1 2 3 4))) 1 0)) (export main))",
                1,
            ),
            (
                "(module m (def (main) (if (= (Bytes.compact (Bytes.of (list 1 2 3))) (Bytes.of (list 1 2 3))) 1 0)) (export main))",
                1,
            ),
        ] {
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(body))).expect("compile"),
                    "main"
                ),
                want,
                "constant compound equality folds: {body}"
            );
        }
        // A genuinely-RUNTIME compound comparison (a recursive result, not constant-foldable) now
        // COMPILES to a `value-eq` heap walk (a scalar-leaf compound is canonical by construction, so the
        // tagless `champ_eq` walk is exact). `dn 3` recurses (cannot fold), so `(= (dn 3) (Some 0))`
        // reaches the runtime structural-equality path — it must build a component (not decline) and
        // IMPORT the runtime (the walk is a runtime call). The RESULT VALUE (true → 1) is verified end to
        // end by the corpus heap-walk cases (03-equality-and-observation), which run against the composed
        // runtime; here we assert the compile succeeds AND pulls in the runtime import.
        let runtime = "(module m (def (dn n) (if (< n 1) (Some 0) (dn (- n 1)))) \
                        (def (main) (if (= (dn 3) (Some 0)) 1 0)) (export main))";
        let program = compile_component(&crate::codec::encode(&parse(runtime)))
            .expect("a runtime scalar-leaf compound equality now compiles to a value-eq heap walk");
        assert!(
            cdz_run::required_runtime(&program)
                .expect("valid component")
                .is_some(),
            "a runtime compound equality imports the value-heap runtime (the value-eq walk is a runtime call)"
        );
    }

    #[test]
    fn a_runtime_eq_of_a_multiparam_sum_with_a_phantom_parameter_walks() {
        // A `Result` value has TWO type parameters (`Ok a`, `Err b`); `(Ok n)` fixes only `a`, leaving
        // `b` a PHANTOM no value instantiates. A runtime `(= (Ok x) (Ok 6))` (both operands built from
        // recursion, so unfoldable) must emit the `value-eq` heap walk — the free `Err` parameter `b`
        // does NOT make the sum non-walkable (a phantom parameter carries no runtime structure). Before
        // the fix `ty_heap_walkable` walked every variant's payload type, hit the free `b` (a `Ty::Var`),
        // and declined "comparison of a compound value needs a heap walk" though the compared `Ok` values
        // are exactly walkable. Run through the composed runtime to pin the value (equal → 1).
        use crate::testkit::parse;
        let src = "(module m (def (sumto (: n Int64)) (if (< n 1) 0 (+ n (sumto (- n 1))))) \
                     (def (eq3) (if (= (Ok (sumto 3)) (Ok 6)) 1 0)) (export eq3))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect(
            "a phantom-parameter multi-param sum equality compiles to a value-eq heap walk",
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping phantom-param value-eq run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("eq3".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run phantom-param value-eq") {
            cdz_run::Outcome::Value(s) => assert_eq!(s, "1", "(Ok (sumto 3)) equals (Ok 6)"),
            cdz_run::Outcome::Trap(t) => panic!("phantom-param value-eq trapped: {t}"),
        }
    }

    #[test]
    fn a_comparison_of_mismatched_operand_types_rejects_via_unification() {
        // (< 1 true) — `<` types at ∀a. a → a → Bool; unifying the second operand `true` (Bool) against
        // the first operand's `a` (fixed to Int by `1`) FAILS. The one generic rule catches it.
        let msg = expect_decline("(< 1 true)");
        assert!(
            msg.contains("unify")
                || msg.contains("Bool")
                || msg.contains("Int")
                || msg.contains("differ"),
            "expected a unification mismatch, got: {msg}"
        );
    }

    #[test]
    fn provable_overflow_fails_the_build() {
        // `(+ Int64.max 1)` overflows the checked Int64 — the compiler PROVES it via the fold and
        // fails the build (CDZ0304) rather than shipping a component that traps (numeric-model.md).
        let msg = expect_decline("(+ (. Int64 max) 1)");
        assert!(msg.contains("overflow"), "got: {msg}");
    }

    #[test]
    fn a_non_integer_operand_rejects_via_unification() {
        // `(+ true 1)` — `+` types at `∀a. (Int a) → (Int a) → (Int a)`; unifying the operand `true`
        // (Bool) against `(Int a)` FAILS, so the application is rejected. This is the one generic rule
        // (instantiate the operator's Meta.t scheme, unify each arg) catching the fault — no
        // arithmetic-specific check.
        let msg = expect_decline("(+ true 1)");
        assert!(
            msg.contains("unify") || msg.contains("Bool") || msg.contains("Int"),
            "expected a unification mismatch naming the types, got: {msg}"
        );
    }

    // ── type annotations `(: e T)`: transparent to the value, constrains the type ────────────────

    #[test]
    fn an_annotation_matching_the_value_is_transparent() {
        // `(: 5 Int64)` runs exactly as `5` — the annotation erases; 5 is already Int64.
        assert_eq!(run_main("(: 5 Int64)"), 5);
    }

    #[test]
    fn an_annotation_is_transparent_around_an_expression() {
        // The annotation wraps any expression, not just a literal — `(: (+ 2 3) Int64)` = 5.
        assert_eq!(run_main("(: (+ 2 3) Int64)"), 5);
    }

    #[test]
    fn an_annotation_grounds_a_width_via_int_ctor() {
        // The type side is a full type EXPRESSION reduced by the evaluator: `(: 5 (Int 64))` uses the
        // `(Int 64)` constructor application as the annotation type. Runs as 5.
        assert_eq!(run_main("(: 5 (Int 64))"), 5);
    }

    #[test]
    fn a_bool_annotation_on_a_bool_is_transparent() {
        // `(: true Bool)` — the annotation matches; the comparison-style Bool boundary returns it.
        assert!(run_main_bool("(: true Bool)"));
    }

    #[test]
    fn an_annotation_conflicting_with_the_value_rejects() {
        // `(: true Int64)` — Bool asserted as Int64. Unifying the annotation type against the value's
        // type FAILS → CDZ0203 (the disambiguation force turned against a genuine conflict).
        let msg = expect_decline("(: true Int64)");
        assert!(
            msg.contains("does not match") || msg.contains("Int") || msg.contains("Bool"),
            "got: {msg}"
        );
    }

    #[test]
    fn an_annotation_inside_arithmetic_is_transparent() {
        // `(+ (: 2 Int64) 3)` — the annotation on an operand erases; the arithmetic folds to 5.
        assert_eq!(run_main("(+ (: 2 Int64) 3)"), 5);
    }

    #[test]
    fn a_non_type_in_a_type_annotation_position_is_rejected() {
        // The TYPE OPERAND of `(: expr T)` must denote a TYPE — a non-type (an unbound name, a value, a
        // malformed type application) is REJECTED, not silently dropped-and-ignored (the drop-instead-of-
        // reject gap: `typeval_of` → None was "no constraint"). An UNBOUND NAME rejects CDZ0101 (its
        // value-position code); a well-formed non-type rejects CDZ0203 ("expected a type"). BOTH the value
        // annotation `(: expr T)` and a PARAMETER annotation `(: name T)` are validated.
        let code = |src: &str| -> Option<String> {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None; // compiled — no rejection
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // Value-annotation form: an unbound name → CDZ0101 (as in value position); a literal/compound/
        // malformed application → CDZ0203.
        assert_eq!(
            code("(module m (def (main) (: 5 foo)) (export main))").as_deref(),
            Some("CDZ0101")
        );
        assert_eq!(
            code("(module m (def (main) (: 5 42)) (export main))").as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            code("(module m (def (main) (: 5 (tuple 1 2))) (export main))").as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            code("(module m (def (main) (: true (Int64 Int64))) (export main))").as_deref(),
            Some("CDZ0203")
        );
        // Parameter-annotation form (the signature-side companion): same validation.
        assert_eq!(
            code("(module m (def (f (: x foo)) x) (def (main) (f 7)) (export main))").as_deref(),
            Some("CDZ0101")
        );
        assert_eq!(
            code("(module m (def (f (: x 42)) x) (def (main) (f 7)) (export main))").as_deref(),
            Some("CDZ0203")
        );
        // NO OVER-REJECTION: a real type annotation (value AND parameter), a matching width, a generic,
        // and an arrow all still compile; a genuine type mismatch still rejects CDZ0203.
        assert_eq!(
            code("(module m (def (main) (: 5 Int64)) (export main))"),
            None
        );
        assert_eq!(
            code("(module m (def (main) (: 200 UInt8)) (export main))"),
            None
        );
        assert_eq!(
            code("(module m (def (f (: n Int64)) (+ n 1)) (def (main) (f 5)) (export main))"),
            None
        );
        assert_eq!(
            code(
                "(module m (def (f (: g (-> Int64 Int64))) (g 1)) (def (main) (f (fn (x) (+ x 1)))) (export main))"
            ),
            None
        );
        assert_eq!(
            code("(module m (def (main) (: 5 Bool)) (export main))").as_deref(),
            Some("CDZ0203")
        );
    }

    // ── integer widths (I3, fold): named widths, per-width bounds, annotations, odd widths ────────

    #[test]
    fn signed_8bit_bounds_project() {
        // Int8 bounds are the width-8 signed range: max 127, min -128. Int8 crosses the boundary as its
        // FAITHFUL component primitive `s8` (NOT s32 or s64) — the width flows through to the wire repr.
        let run_i8 = |body: &str| {
            let src = format!("(module m (def (main) {body}) (export main))");
            let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
            run_returns::<i8>(&bytes, "main")
        };
        assert_eq!(run_i8("(. Int8 max)"), 127);
        assert_eq!(run_i8("(. Int8 min)"), -128);
    }

    #[test]
    fn unsigned_8bit_max_is_255() {
        // UInt8.max = 2^8 - 1 = 255. A UInt8 crosses the boundary as its faithful `u8`.
        let src = "(module m (def (main) (. UInt8 max)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u8>(&bytes, "main"), 255);
    }

    #[test]
    fn unsigned_64bit_max_exceeds_i64_and_renders() {
        // UInt64.max = 2^64 - 1 = 18446744073709551615 — a value ABOVE i64::MAX, so it exercises the
        // arbitrary-precision bound + the unsigned bit-pattern emit (-1 as i64) + the u64 boundary lift.
        let src = "(module m (def (main) (. UInt64 max)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u64>(&bytes, "main"), u64::MAX);
    }

    #[test]
    fn an_annotation_takes_a_narrower_width() {
        // 06-numeric-model `(: 200 UInt8)` = 200 : UInt8 — the annotation grounds the literal to the
        // narrower width; 200 fits UInt8 (0..=255). Crosses the boundary as its faithful `u8`.
        let src = "(module m (def (main) (: 200 UInt8)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u8>(&bytes, "main"), 200);
    }

    #[test]
    fn a_literal_outside_the_annotated_width_is_rejected() {
        // `(: 256 UInt8)` — 256 does not fit UInt8 (max 255): rejected (CDZ0302), never truncated to 0.
        assert!(expect_decline("(: 256 UInt8)").contains("does not fit"));
        // A negative into an unsigned width also fails.
        assert!(expect_decline("(: -1 UInt8)").contains("does not fit"));
        // And a signed-8 overflow: 128 does not fit Int8 (max 127).
        assert!(expect_decline("(: 128 Int8)").contains("does not fit"));
    }

    #[test]
    fn a_malformed_integer_width_is_rejected_not_dropped() {
        // A NON-NATURAL width — negative, or a bool/float/type-value in width position — must be
        // rejected (CDZ0302), NOT silently dropped so the literal keeps its default Int64. The width
        // reader used to narrow with `u32::try_from`, whose `None` was ignored: `(: 5 (Int -8))` then
        // ran to 5. Each of these now reduces to the invalid sentinel width 0 and the fit-check
        // rejects it, exactly as an explicit `(UInt 0)` is rejected.
        for body in [
            "(: 5 (Int -8))",
            "(: 5 (UInt -1))",
            "(: 300 (Int -8))", // if the width were honored 300 would overflow; if dropped it fits — neither
            "(: 300 (Int true))", // a bool in width position
            "(: 300 (Int 8.0))", // a float in width position
            "(: 300 (Int Int64))", // a type-value in width position
        ] {
            assert!(
                expect_decline(body).contains("does not fit"),
                "malformed width must be rejected CDZ0302: {body}"
            );
        }
        // A valid width is unaffected — `(Int 64)` still builds and `5` fits (crosses as s64).
        assert_eq!(run_main("(: 5 (Int 64))"), 5);
    }

    #[test]
    fn a_runtime_integer_width_is_rejected_not_dropped() {
        // A `(UInt n)`/`(Int n)` whose width is a RUNTIME value (a function parameter) puts a runtime
        // value in a type-determining position, which the type system forbids: an integer type MUST be
        // indexed by a COMPILE-TIME width (numeric-model.md §An Integer Type Is Indexed By A Compile-Time
        // Width). The width reader resolves `n` to no compile-time natural, so — like a negative or
        // non-natural width — the annotation reduces to the sentinel width 0 and the fit-check rejects
        // it (CDZ0302), rather than DROPPING the annotation so the literal keeps its default Int64 (which
        // is what made `(mk 8)` run to 5). The runtime branch of the drop-instead-of-reject family.
        let compile =
            |src: &str| compile_component(&crate::codec::encode(&crate::testkit::parse(src)));
        // Runtime width via an inlined call — `(mk 8)` substitutes 8 but `n` is not a constant at the
        // annotation site; the width is non-constant → reject, not run-to-5.
        let e = compile("(module m (def (mk n) (: 5 (UInt n))) (def (main) (mk 8)) (export main))")
            .expect_err("a runtime-valued width must be rejected");
        assert_eq!(e.code.as_deref(), Some("CDZ0302"), "got: {}", e.message);
        // The signed variant likewise.
        let e = compile("(module m (def (mk n) (: 5 (Int n))) (def (main) (mk 16)) (export main))")
            .expect_err("a runtime-valued signed width must be rejected");
        assert_eq!(e.code.as_deref(), Some("CDZ0302"), "got: {}", e.message);
        // A width that IS a compile-time constant reached through a `let` is still fine (it folds to the
        // constant): `(let ((w 8)) (: 5 (UInt w)))` builds and 5 fits UInt8 (crosses as u8, not dropped
        // to the default i64 — the constant width is honored).
        assert_eq!(run_main_as::<u8>("(let ((w 8)) (: 5 (UInt w)))"), 5);
    }

    #[test]
    fn arbitrary_odd_widths_compute_their_bounds() {
        // The bounds are computed FROM THE WIDTH PARAMETER, not a per-named-type table — so an ODD,
        // non-machine width works: `(UInt 7)` max = 2^7-1 = 127, `(UInt 24)` max = 2^24-1 = 16777215,
        // `(UInt 48)` max = 2^48-1 = 281474976710655. These pin that nothing assumes a power-of-two
        // machine width. A non-aliased width is INTERNAL-ONLY (no boundary representation — R2), so the
        // bound is asserted at the FOLD (the arbitrary-precision `ConstInt`), not by running it across
        // the component edge. (`arbitrary-width value cannot be exported` is pinned separately below.)
        assert_eq!(fold_const_u128("(. (UInt 7) max)"), 127);
        assert_eq!(fold_const_u128("(. (UInt 24) max)"), 16777215);
        assert_eq!(fold_const_u128("(. (UInt 48) max)"), 281474976710655);
    }

    #[test]
    fn an_odd_width_annotation_range_checks() {
        // `(: 127 (UInt 7))` fits (2^7-1=127); `(: 128 (UInt 7))` does NOT — the range check is
        // width-exact, not rounded to a machine width. The fit case is checked at the FOLD (a `(UInt 7)`
        // is internal-only, no boundary form); the overflow case rejects (CDZ0302) before any boundary.
        assert_eq!(fold_const_u128("(: 127 (UInt 7))"), 127);
        assert!(expect_decline("(: 128 (UInt 7))").contains("does not fit"));
    }

    #[test]
    fn a_non_aliased_width_cannot_cross_the_boundary() {
        // A `(UInt 48)` is a first-class INTERNAL type — its bounds fold and its arithmetic is correct —
        // but only the aliased widths (8/16/32/64) have a component boundary representation. Exporting a
        // value of a non-aliased width DECLINES (naming the width), rather than crossing as a misreported
        // wider primitive. This is the safety property: to expose 48-bit data you take a `u64` at the
        // boundary and convert explicitly, so the host never sees an ambiguous "48 bits in a u64".
        let src = "(module m (def (main) (. (UInt 48) max)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a non-aliased width cannot be exported")
            .message;
        assert!(
            msg.contains("boundary") || msg.contains("aliased"),
            "got: {msg}"
        );
    }

    // ── truncating conversion `T.wrap` (R3): constant FOLD, no sums, never traps ──────────────────
    //
    // `(UInt8.wrap n)` = `((. UInt8 wrap) n)` — projecting the module's `wrap` operator and applying it.
    // On a constant it FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width. The
    // target width comes from the TYPE (`UInt8`), not a magic op name — one truncating conversion, width-
    // indexed. The result crosses the boundary as the target's faithful primitive (u8/s8/…).

    #[test]
    fn wrap_truncates_an_out_of_range_constant() {
        // `(UInt8.wrap 256)` = 0 — keep the low 8 bits (0x100 → 0x00). NEVER traps (contrast the checked
        // `.of`, which will return None). Crosses as u8.
        let src = "(module m (def (main) (UInt8.wrap 256)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u8>(&bytes, "main"), 0);
    }

    #[test]
    fn wrap_of_a_negative_uses_twos_complement() {
        // `(UInt8.wrap -1)` = 255 — the low 8 bits of -1's two's-complement (all ones); the target width
        // (UInt8) comes from the type.
        let src = "(module m (def (main) (UInt8.wrap -1)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u8>(&bytes, "main"), 255);
    }

    #[test]
    fn wrap_into_a_signed_width_reinterprets_the_sign_bit() {
        // `(Int8.wrap 200)` = -56 — 200 = 0xC8, bit 7 set, so as a signed Int8 it is -56. Crosses as s8.
        let src = "(module m (def (main) (Int8.wrap 200)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<i8>(&bytes, "main"), -56);
    }

    #[test]
    fn wrap_in_range_is_the_value() {
        // `(UInt8.wrap 200)` = 200 — an in-range value is unchanged. Pins that wrap is the identity when
        // the value already fits.
        let src = "(module m (def (main) (UInt8.wrap 200)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<u8>(&bytes, "main"), 200);
    }

    #[test]
    fn wrap_via_the_constructor_denotes_the_same_op() {
        // `((Int 8).wrap 200)` = `(Int8.wrap 200)` = -56 — the constructor-built module and the named
        // width carry the SAME `wrap` (built by the twin `wrap_field`s), so both must produce -56 (not
        // merely agree). `(Int 8).wrap` is the postfix-member sugar (reads to `(. (Int 8) wrap)`), the
        // paren sibling of the `Int8.wrap` token form.
        let run_i8 = |body: &str| {
            let src = format!("(module m (def (main) {body}) (export main))");
            let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
            run_returns::<i8>(&bytes, "main")
        };
        assert_eq!(run_i8("((Int 8).wrap 200)"), -56);
        assert_eq!(run_i8("(Int8.wrap 200)"), -56);
    }

    #[test]
    fn wrap_to_a_nonaliased_width_folds_but_cannot_cross() {
        // `((UInt 48).wrap -1)` = 2^48-1 at the FOLD (the low 48 bits of -1) — a non-aliased target has
        // no boundary form, so it can't be exported, but the truncation itself is correct. Asserted at
        // the fold; exporting it declines (the non-aliased-boundary rule from R2). `(UInt 48).wrap` is
        // the postfix-member sugar (reads to `(. (UInt 48) wrap)`).
        assert_eq!(fold_const_u128("((UInt 48).wrap -1)"), (1u128 << 48) - 1);
        assert!(
            expect_decline("((UInt 48).wrap -1)").contains("boundary"),
            "a non-aliased wrap result must decline at the boundary"
        );
    }

    #[test]
    fn wrap_of_a_non_integer_source_is_rejected() {
        // `(UInt8.wrap true)` — `wrap`'s type is `∀(w,s). Int^s_w → UInt8`; unifying the source `true`
        // (Bool) against `(Int a)` FAILS. The one generic application rule catches it (CDZ0203), no
        // conversion-specific check.
        let msg = expect_decline("(UInt8.wrap true)");
        assert!(
            msg.contains("unify")
                || msg.contains("Bool")
                || msg.contains("Int")
                || msg.contains("match"),
            "got: {msg}"
        );
    }

    #[test]
    fn mixing_two_widths_without_conversion_is_rejected() {
        // 06-numeric-model: `(+ (: 1 UInt8) (: 2 Int32))` — two different integer types with no explicit
        // conversion → CDZ0301 (no silent promotion). The one generic rule (unify the operands) catches
        // it: UInt8 and Int32 differ in BOTH signedness and width.
        let msg = expect_decline("(+ (: 1 UInt8) (: 2 Int32))");
        assert!(
            msg.contains("unify")
                || msg.contains("width")
                || msg.contains("Int")
                || msg.contains("UInt"),
            "got: {msg}"
        );
    }

    #[test]
    fn mixing_an_integer_and_a_float_is_rejected_no_silent_promotion() {
        // 06-numeric-model: `(+ 2 2.0)` mixes an Int64 and a Float64, which do NOT silently promote —
        // rejected (numeric-model.md §Numeric Types Do Not Silently Promote). The float literal gets
        // `Ty::Float`, distinct from the operator's `(Int a)`, so unification FAILS (coded CDZ0301, both
        // numeric-but-different). Every integer operator (arith/bitwise/shift) mixed with a float rejects
        // the same way — the one generic operand-unification rule, no float special case. The message
        // names the two conflicting types.
        for op in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
            let msg = expect_decline(&format!("({op} 2 2.0)"));
            assert!(
                msg.contains("Float64") && (msg.contains("Int") || msg.contains("unify")),
                "int↔float mix under `{op}` should cite the numeric mismatch; got: {msg}"
            );
        }
    }

    #[test]
    fn a_non_admitted_float_width_is_rejected_cdz0302() {
        // 06-numeric-model: `(Float N)` for N ∉ {32,64} — the admitted IEEE set — is rejected CDZ0302
        // (numeric-model.md §A Floating-Point Type Is Indexed By A Compile-Time Width), the float
        // analogue of an out-of-range integer width. A non-admitted `(Float N)` reduces to the sentinel
        // width 0 (via `reduce_ctor`), which the annotation check rejects. Set-MEMBERSHIP, not a range:
        // 16 and 48 both fail even though 48 is within 1..=64 (an integer width would accept it).
        for body in [
            "(: 1.5 (Float 16))",
            "(: 1.5 (Float 48))",
            "(: 1.5 (Float 0))",
            "(: 1.5 (Float 128))",
        ] {
            let msg = expect_decline(body);
            assert!(
                msg.contains("admitted IEEE widths"),
                "a non-admitted float width must be rejected CDZ0302: {body}; got: {msg}"
            );
        }
        // An ADMITTED width (`Float64`) types cleanly and CROSSES as f64 — the value runs to 1.5, not a
        // CDZ0302. (A `Float32` value declines at emit until the f32 path lands — F3/F4 — but is
        // WELL-TYPED, no CDZ0302; that decline is covered where the emit path is exercised.)
        assert_eq!(run_main_as::<f64>("(: 1.5 (Float 64))"), 1.5);
        assert_eq!(run_main_as::<f64>("(: 1.5 Float64)"), 1.5);
    }

    #[test]
    fn a_float_operator_rejects_an_integer_operand_no_silent_promotion() {
        // 06-numeric-model: the FLOAT operators `+.`/`-.`/`*.`/`/.` are float-only — an integer operand
        // fails to unify with `(Float a)` → CDZ0301, the dual of an int operator rejecting a float
        // (numeric-model.md §A Floating-Point Operation Uses A Floating-Point Operator). Neither operator
        // coerces: `(+. 2 2.0)` is as rejected as `(+ 2 2.0)`.
        for op in ["+.", "-.", "*.", "/."] {
            let msg = expect_decline(&format!("({op} 2 2.0)"));
            assert!(
                msg.contains("Float") || msg.contains("Int") || msg.contains("unify"),
                "an integer operand to `{op}` should cite the numeric mismatch; got: {msg}"
            );
        }
    }

    #[test]
    fn signed_and_unsigned_of_the_same_width_do_not_promote() {
        // `(+ (: 1 Int8) (: 2 UInt8))` — same width (8), different SIGNEDNESS → still rejected (no
        // implicit promotion). Pins that signedness alone is a mismatch, not just width.
        let msg = expect_decline("(+ (: 1 Int8) (: 2 UInt8))");
        assert!(
            msg.contains("unify")
                || msg.contains("Int")
                || msg.contains("UInt")
                || msg.contains("differ"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_named_width_and_the_constructor_denote_the_same_module() {
        // `Int8` and `(Int 8)` are the SAME module — both project a bound of the same value and type.
        // (No per-name special case; both go through the width-generic builder.) Both cross as s8.
        let run_i8 = |body: &str| {
            let src = format!("(module m (def (main) {body}) (export main))");
            let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
            run_returns::<i8>(&bytes, "main")
        };
        assert_eq!(run_i8("(. Int8 max)"), run_i8("(. (Int 8) max)"));
        assert_eq!(run_i8("(. Int8 min)"), run_i8("(. (Int 8) min)"));
    }

    #[test]
    fn an_annotated_def_parameter_binds_and_folds() {
        // `(def (w (: a Int64) b) (+ a b))` — the annotated binder `(: a Int64)` binds `a` (the body's
        // `a` resolves through the annotation, not UNBOUND), and the call folds. 09-functions witness.
        let src = "(module m (def (w (: a Int64) b) (+ a b)) (def (main) (w 20 22)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<i64>(&bytes, "main"), 42);
    }

    #[test]
    fn an_annotated_fn_parameter_binds() {
        // The same through a `fn` lambda: `((fn ((: x Int64)) (+ x 1)) 5)` = 6.
        assert_eq!(run_main("((fn ((: x Int64)) (+ x 1)) 5)"), 6);
    }

    #[test]
    fn a_contradicting_parameter_annotation_rejects() {
        // `(def (bad (: a Bool)) (+ a 1))` — `a` annotated Bool but used as an integer operand of `+`;
        // the annotation contradicts the use → CDZ0203 (type-system.md Annotations Constrain, Never
        // Contradict). 09-functions witness.
        let src = "(module m (def (bad (: a Bool)) (+ a 1)) (def (main) (bad true)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("must reject")
            .message;
        assert!(
            msg.contains("match")
                || msg.contains("Bool")
                || msg.contains("Int")
                || msg.contains("unify"),
            "got: {msg}"
        );
    }

    #[test]
    fn an_argument_conflicting_with_its_parameter_type_is_rejected_not_miscompiled() {
        // The ARGUMENT side of parameter typing: β-reduction erases the parameter↔argument relationship
        // (the arg is substituted into the body), so a mistyped argument must be checked at the CALL —
        // else it is silently accepted (case A) or, once the mis-accepted value is USED at its claimed
        // type, MISCOMPILED to an invalid component (cases B/C). Each of these MUST reject, never run to
        // a value and never emit invalid wasm.
        let must_reject = |src: &str| {
            compile_component(&crate::codec::encode(&parse(src)))
                .expect_err("mistyped argument must reject")
                .message
        };
        // A: Int argument to a Bool-annotated identity parameter (would otherwise return 5).
        must_reject("(module m (def (f (: x Bool)) x) (def (main) (f 5)) (export main))");
        // B: Int argument to a bare parameter used as a Bool condition (would otherwise miscompile).
        must_reject("(module m (def (f x) (if x 1 2)) (def (main) (f 5)) (export main))");
        // C: Bool argument to a bare parameter used in integer addition (would otherwise miscompile).
        must_reject("(module m (def (f x) (+ x x)) (def (main) (f true)) (export main))");
        // The correctly-typed calls of the SAME functions still compile and run — no over-rejection.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (f x) (if x 1 2)) (def (main) (f true)) (export main))"
                )))
                .expect("compile"),
                "main"
            ),
            1
        );
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (f x) (+ x x)) (def (main) (f 5)) (export main))"
                )))
                .expect("compile"),
                "main"
            ),
            10
        );
    }

    #[test]
    fn a_function_typed_parameter_annotation_is_checked_against_the_argument() {
        // The HIGHER-ORDER analogue of the scalar arg-vs-param check: a parameter annotated with a
        // FUNCTION type `(-> A B)` must be checked against the passed function's type — RESULT included.
        // A lambda argument used to type as `Any` (so unifying `Fn(Int,Bool)` against it trivially
        // succeeded and the annotation was silently dropped); now a lambda types as its own arrow type,
        // so a result mismatch is caught. Each of these MUST reject (CDZ0203), never run to a value.
        let must_reject = |src: &str| {
            compile_component(&crate::codec::encode(&parse(src)))
                .expect_err("a mismatched function-type annotation must reject")
                .code
        };
        // Outermost result mismatch: (-> Int64 Bool) annotation, an Int64 -> Int64 argument.
        assert_eq!(
            must_reject(
                "(module m (def (f (: g (-> Int64 Bool))) (g 41)) \
                   (def (main) (f (fn (x) (+ x 1)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
        // The annotation constrains the BODY too: (+ (g 41) 1) with g's result fixed to Bool rejects.
        assert_eq!(
            must_reject(
                "(module m (def (f (: g (-> Int64 Bool))) (+ (g 41) 1)) \
                   (def (main) (f (fn (x) (+ x 1)))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
        // NESTED arrow: the inner result of a curried (-> Int64 (-> Int64 Bool)) is checked too.
        assert_eq!(
            must_reject(
                "(module m (def (f (: g (-> Int64 (-> Int64 Bool)))) ((g 1) 2)) \
                   (def (main) (f (fn (a) (fn (b) (+ a b))))) (export main))"
            )
            .as_deref(),
            Some("CDZ0203")
        );
        // No over-rejection: a MATCHING function-type annotation compiles and runs.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (f (: g (-> Int64 Int64))) (g 41)) \
                       (def (main) (f (fn (x) (+ x 1)))) (export main))"
                )))
                .expect("a matching function-type annotation compiles"),
                "main"
            ),
            42
        );
        // A matching Bool-returning argument is accepted and runs: (< 41 5) = false.
        assert!(!run_returns::<bool>(
            &compile_component(&crate::codec::encode(&parse(
                "(module m (def (f (: g (-> Int64 Bool))) (g 41)) \
                   (def (main) (f (fn (x) (< x 5)))) (export main))"
            )))
            .expect("a matching Bool-returning fn argument compiles"),
            "main"
        ));
    }

    // ── first-class types: `(Int W)` builds a width-specialized MODULE via the one Meta.apply path ──

    #[test]
    fn int_ctor_builds_a_module_projected_for_max() {
        // `(. (Int 64) max)` — `Int` is a type-constructor record; applying it to 64 REDUCES (via its
        // `(meta apply)` builder) to a fresh width-64 integer module, off which `max` projects. Folds
        // to the constant with no runtime trace, exactly like the pre-built `Int64.max`.
        assert_eq!(run_main("(. (Int 64) max)"), i64::MAX);
    }

    #[test]
    fn int_ctor_min_of_a_smaller_width() {
        // `(. (Int 8) min)` = -128 — the module `Int` builds is SPECIALIZED to the width argument, so a
        // different width yields that width's bounds. An Int8 value crosses the boundary as s8.
        let src = "(module m (def (main) (. (Int 8) min)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<i8>(&bytes, "main"), -128);
    }

    #[test]
    fn int_ctor_max_of_a_smaller_width() {
        // `(. (Int 8) max)` = 127 (crosses as s8).
        let src = "(module m (def (main) (. (Int 8) max)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(run_returns::<i8>(&bytes, "main"), 127);
    }

    // ── functions: a lambda application β-reduces (folds/monomorphizes) ──────────────────────────

    #[test]
    fn a_lambda_applied_folds() {
        // `((fn (x) (+ x 1)) 5)` = 6 — the lambda β-reduces (substitute 5 for x → `(+ 5 1)`), then the
        // arithmetic folds. No function value emitted (monomorphized away).
        assert_eq!(run_main("((fn (x) (+ x 1)) 5)"), 6);
    }

    #[test]
    fn a_let_bound_function_applied() {
        // `(let ((inc (fn (x) (+ x 1)))) (inc 10))` = 11 — a let-bound function, applied through the
        // ref to its lambda.
        assert_eq!(run_main("(let ((inc (fn (x) (+ x 1)))) (inc 10))"), 11);
    }

    // ── binding patterns: a `let` binder may be an irrefutable pattern ───────────────────────────

    #[test]
    fn a_let_binder_may_be_a_tuple_pattern() {
        // core-semantics.md §A Binding Position Accepts An Irrefutable Pattern: a `let` binder MAY be a
        // tuple pattern that destructures the value, binding each element — the ergonomic form of the
        // bind-then-`match` idiom. A binder reference resolves to a `SumPayload` reading the element out
        // of the bound value (the same machinery a `(match v ((tuple a b) …))` arm uses), so this is a
        // pure resolve-side lift with no new IR. Nested to any depth; a later binding sees an earlier
        // pattern's binders (`let*` scoping).
        assert_eq!(run_main("(let (((tuple a b) (tuple 3 4))) (+ a b))"), 7);
        assert_eq!(
            run_main("(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) (+ a (+ b c)))"),
            6
        );
        assert_eq!(
            run_main("(let (((tuple a b) (tuple 3 4)) (c (+ a b))) c)"),
            7
        );
    }

    #[test]
    fn an_ill_formed_let_binding_pattern_is_rejected_not_miscompiled() {
        // A binding position has no alternative arm, so its pattern MUST be irrefutable and its shape MUST
        // match the value's type (core-semantics.md §A Binding Position Accepts An Irrefutable Pattern).
        // Without the validation hook these silently MISCOMPILED (a wrong-arity / non-linear pattern
        // resolved its binders and ran to a value); the hook faults them with the code the equivalent
        // single-arm match would raise. `code` compiles a `main`-body and reports its rejection code (or
        // None if it compiled) — the same shape the linearity test above uses.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m (def (main) {body}) (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None; // compiled — no rejection
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // Refutable: a multi-variant constructor / a literal → CDZ0210 (non-exhaustive).
        assert_eq!(
            code("(let (((Some x) (Some 5))) x)").as_deref(),
            Some("CDZ0210")
        );
        assert_eq!(code("(let ((0 5)) 42)").as_deref(), Some("CDZ0210"));
        // Shape-incompatible: a wrong-arity tuple / a tuple pattern vs a non-tuple value → CDZ0201.
        assert_eq!(
            code("(let (((tuple a b c) (tuple 1 2))) a)").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            code("(let (((tuple a b) 5)) a)").as_deref(),
            Some("CDZ0201")
        );
        // Non-linear: a binder repeated across the pattern → CDZ0102.
        assert_eq!(
            code("(let (((tuple x x) (tuple 1 2))) x)").as_deref(),
            Some("CDZ0102")
        );
        // Refutability is checked RECURSIVELY: a refutable sub-pattern NESTED in a tuple binding element
        // is CDZ0210, exactly as the top-level one — the check does not stop at the top level. A literal
        // element, a multi-variant constructor element, a deeply-nested literal, and a nested string
        // literal all fault (the last was previously a codeless "malformed sum match pattern" decline).
        assert_eq!(
            code("(let (((tuple 0 b) (tuple 0 9))) b)").as_deref(),
            Some("CDZ0210")
        );
        assert_eq!(
            code("(let (((tuple (Some x) b) (tuple (Some 5) 9))) x)").as_deref(),
            Some("CDZ0210")
        );
        assert_eq!(
            code("(let (((tuple a (tuple 0 b)) (tuple 1 (tuple 0 3)))) (+ a b))").as_deref(),
            Some("CDZ0210")
        );
        assert_eq!(
            code("(let (((tuple \"hi\" b) (tuple \"hi\" 9))) b)").as_deref(),
            Some("CDZ0210")
        );
        // NO OVER-REJECTION: a well-formed destructuring binding compiles — flat, and nested to any depth
        // (every element irrefutable), and with a wildcard element.
        assert_eq!(code("(let (((tuple a b) (tuple 3 4))) (+ a b))"), None);
        assert_eq!(
            code("(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) (+ a (+ b c)))"),
            None
        );
        assert_eq!(code("(let (((tuple a _) (tuple 3 9))) a)"), None);
    }

    #[test]
    fn an_annotated_let_binder_constrains_the_value() {
        // A `let` binder MAY carry a `(: <pat> <Type>)` annotation (core-semantics.md §A Binding Position
        // Accepts An Irrefutable Pattern / type-system.md §Annotations Constrain, Never Contradict): the
        // annotation constrains the value's type while the inner pattern binds. `check_binding_pattern` +
        // `last_binder_named` peel it — a contradiction is CDZ0203, else the inner pattern binds normally.
        let run = |body: &str| -> i64 {
            let src = format!("(module m (def (main) {body}) (export main))");
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(&src))).expect("compile"),
                "main",
            )
        };
        // An annotated name binder, and an annotated DESTRUCTURING binder (annotation before the pattern).
        assert_eq!(run("(let (((: x Int64) 5)) x)"), 5);
        assert_eq!(
            run("(let (((: (tuple a b) (Tuple Int64 Int64)) (tuple 3 4))) (+ a b))"),
            7
        );
        // A CONTRADICTING annotation is CDZ0203.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m (def (main) {body}) (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None;
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        assert_eq!(code("(let (((: x Bool) 5)) x)").as_deref(), Some("CDZ0203"));
        assert_eq!(
            code("(let (((: (tuple a b) (Tuple Int64 Bool)) (tuple 3 4))) a)").as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn a_def_parameter_may_be_a_tuple_pattern() {
        // core-semantics.md §A Binding Position Accepts An Irrefutable Pattern: a `def` parameter MAY be a
        // tuple pattern naming the pair's parts, keeping ARITY ONE. `binding_params::lower` rewrites
        // `(def (f (tuple a b)) BODY)` → `(def (f p$0) (let (((tuple a b) p$0)) BODY))` at load, so it
        // reuses the (proven) destructuring-`let` path. A body reference to `a`/`b` resolves through the
        // synthesized `let`; the fresh `p$0` binds the whole argument.
        let run = |src: &str| -> i64 {
            let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
            run_returns::<i64>(&bytes, "main")
        };
        // A single tuple param, a nested one, multiple tuple params, and a mixed name+tuple signature.
        assert_eq!(
            run(
                "(module m (def (fst (tuple a b)) a) (def (main) (fst (tuple 7 8))) (export main))"
            ),
            7
        );
        assert_eq!(
            run("(module m (def (f (tuple a (tuple b c))) (+ a (+ b c))) \
                   (def (main) (f (tuple 1 (tuple 2 3)))) (export main))"),
            6
        );
        assert_eq!(
            run(
                "(module m (def (f (tuple a b) (tuple c d)) (+ (+ a b) (+ c d))) \
                   (def (main) (f (tuple 1 2) (tuple 3 4))) (export main))"
            ),
            10
        );
        assert_eq!(
            run("(module m (def (f x (tuple a b)) (+ x (+ a b))) \
                   (def (main) (f 10 (tuple 2 4))) (export main))"),
            16
        );
    }

    #[test]
    fn an_ill_formed_def_parameter_pattern_is_rejected() {
        // A parameter position is a binding position — no alternative arm — so its pattern must be
        // irrefutable and linear (core-semantics.md §A Binding Position Accepts An Irrefutable Pattern).
        // The rewrite carries the pattern to the same `let`-validation the binder case uses, so a refutable
        // / non-linear parameter faults with the binding-position code rather than miscompiling.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m {body} (def (main) 0) (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None;
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // Refutable multi-variant constructor parameter → CDZ0210.
        assert_eq!(code("(def (f (Some x)) x)").as_deref(), Some("CDZ0210"));
        // Non-linear tuple parameter → CDZ0102.
        assert_eq!(code("(def (f (tuple x x)) x)").as_deref(), Some("CDZ0102"));
        // NO OVER-REJECTION: a well-formed tuple parameter compiles (used by `main`).
        assert_eq!(
            code("(def (f (tuple a b)) (+ a b)) (def (g) (f (tuple 1 2)))"),
            None
        );
    }

    #[test]
    fn a_sum_type_may_be_declared_inside_a_do_block() {
        // A `(type …)` declaration nested in a `do` block — a LOCAL sum, not a top-level item. `top_items`
        // sees only the root's direct children, so `db::collect_nested_decls` descends def bodies + nested
        // `do`s to gather these and synthesize their sum records; the do-form walks (resolve/infer/compile/
        // eval) skip a `(type …)` form as a declaration (like a `def`), so its `type` head is not resolved
        // as an unbound value. The type name + variant names then resolve program-wide (nominal identity).
        let run = |src: &str| -> i64 {
            let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
            run_returns::<i64>(&bytes, "main")
        };
        // A nullary enum and a payload variant, both declared inside `main`'s `do`, with CONSTANT
        // scrutinees that fold at compile time (so the run needs no value-heap runtime — the recursive-
        // heap case is covered by the corpus, which links the runtime). The names `C`/`R`/`G`/`Bx`
        // resolve although the type is declared locally, not at top level.
        assert_eq!(
            run(
                "(module m (def (main) (do (type C R G B) (match R (R 1) (G 2) (B 3)))) (export main))"
            ),
            1
        );
        assert_eq!(
            run(
                "(module m (def (main) (do (type Box (Bx Int64)) (match (Bx 5) ((Bx x) x)))) (export main))"
            ),
            5
        );
        // A trailing `(type …)` (nothing to yield) is malformed, like a trailing `def`.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m {body} (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None;
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        assert_eq!(
            code("(def (main) (do (type C R G)))").as_deref(),
            Some("CDZ0201")
        );
    }

    #[test]
    fn a_wrong_type_variant_payload_is_a_malformed_construction() {
        // A variant constructor is a single-arity function whose argument is checked against its DECLARED
        // payload type (core-semantics.md §A Sum Type Constructor Is A Single-Arity Function). A wrong-type
        // / wrong-arity payload is a MALFORMED construction (CDZ0201, a structural sum-shape violation),
        // the code the corpus assigns — NOT the generic unify mismatch (CDZ0203) an ordinary function's
        // argument mismatch gets. The reclassification is on the SAME instantiated-and-substituted unify
        // the generic application takes, so a GENERIC/nested construction is not over-rejected.
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m {body} (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None;
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // Wrong-type scalar payload (String where Int64 declared), wrong-arity tuple payload → CDZ0201.
        assert_eq!(
            code("(type T (Mk Int64)) (def (main) (T.Mk \"x\"))").as_deref(),
            Some("CDZ0201")
        );
        assert_eq!(
            code("(type W (Wt (Tuple Int64 Int64))) (def (main) (W.Wt (tuple 1 2 3)))").as_deref(),
            Some("CDZ0201")
        );
        // NO OVER-REJECTION: a GENERIC variant accepts any payload type; a nested construction is fine.
        assert_eq!(
            code("(def (main) (match (Some true) ((Some x) (if x 1 0)) (None 0)))"),
            None
        );
        assert_eq!(
            code("(def (main) (match (Ok (Err 9)) ((Ok (Ok n)) n) ((Ok (Err e)) e) ((Err _) -2)))"),
            None
        );
        // An ordinary (non-constructor) function's argument mismatch stays CDZ0203, not reclassified.
        assert_eq!(
            code("(def (main) (if (< 1 true) 1 0))").as_deref(),
            Some("CDZ0203")
        );
    }

    #[test]
    fn comparing_distinct_same_shape_nominal_sums_is_a_nominal_boundary_error() {
        // Comparing two values whose types are distinct NOMINAL sums of the SAME structural shape —
        // `(= (A.Mk 1) (B.Mk 1))` for `(type A (Mk Int64))` / `(type B (Mk Int64))` — is a comparison
        // across the nominal boundary (CDZ0202), NOT the `false` an untagged structural comparison gives
        // (type-system.md §Nominal Types Are Not Comparable Across Their Boundary). Two sums of DIFFERENT
        // shape (disjoint variants — `Option` vs `Result`) are unrelated types → the plain CDZ0203; the
        // same sum compared to itself is well-typed (→ false).
        let code = |body: &str| -> Option<String> {
            let src = format!("(module m {body} (def (main) 0) (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // Same-shape distinct sums → CDZ0202.
        assert_eq!(
            code("(type A (Mk Int64)) (type B (Mk Int64)) (def (c) (= (A.Mk 1) (B.Mk 1)))")
                .as_deref(),
            Some("CDZ0202")
        );
        // Disjoint-variant sums (Option vs Result) → CDZ0203 (unrelated types, not a nominal boundary).
        assert_eq!(
            code("(def (c) (= (Some 1) (Ok 1)))").as_deref(),
            Some("CDZ0203")
        );
        // A scalar-vs-Bool mismatch stays CDZ0203.
        assert_eq!(code("(def (c) (= 1 true))").as_deref(), Some("CDZ0203"));
    }

    #[test]
    fn a_recursive_function_over_a_sum_infers_its_unannotated_parameters() {
        // A recursive function that MATCHES an unannotated parameter against a user sum's constructors —
        // `(def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t)))))` — infers `c : Code` from
        // the pattern SHAPE (`pattern_implied_ty` threads the variant/tuple pattern onto the param var),
        // rather than leaving it a free var that grounds to `Any` and declines "annotate its parameters".
        // A PASS-THROUGH parameter returned by one arm (`ys` in `cat`) is inferred from the sibling arm's
        // determined result. Both were previously an unannotated-recursive-param decline. `compiles`
        // means the recursive signatures were determined (no decline); the runtime run is corpus-gated.
        let compiles = |body: &str| -> bool {
            let src = format!("(module m {body} (export main))");
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(&src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            out.artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
        };
        // `len` — a single sum parameter matched against its constructors, inferred from the pattern.
        assert!(compiles(
            "(type Code CNil (CCons (Tuple Int64 Code))) \
             (def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t))))) \
             (def (main) (len (CCons (tuple 1 (CCons (tuple 2 CNil))))))"
        ));
        // `cat` — TWO sum parameters, one (`xs`) inferred from the pattern, the other (`ys`) inferred as
        // the pass-through returned by the `CNil` arm (its type borrowed from the `CCons` arm's result).
        assert!(compiles(
            "(type Code CNil (CCons (Tuple Int64 Code))) \
             (def (cat xs ys) (match xs (CNil ys) ((CCons (tuple h t)) (CCons (tuple h (cat t ys)))))) \
             (def (len c) (match c (CNil 0) ((CCons (tuple h t)) (+ 1 (len t))))) \
             (def (main) (len (cat (CCons (tuple 1 CNil)) CNil)))"
        ));
    }

    #[test]
    fn sum_shape_descriptor_closes_recursion_with_a_ref() {
        // The compiler's shape descriptor for a RECURSIVE sum `(type IL (Cons (Tuple Int64 IL)) Nil)` is
        // a TABLE with a self-`Ref` (the `Cons` payload tuple's second element points back at the sum),
        // so the descriptor is FINITE. This must be BYTE-IDENTICAL to the descriptor the runtime's
        // `value_encode_form_matches_the_codec` test hard-codes (the compiler/runtime contract). We build
        // it from the solved `Ty::Sum` and assert the bytes.
        let src = "(module m (type IL (Cons (Tuple Int64 IL)) Nil) (def (main) Nil) (export main))";
        let mut db =
            crate::db::Db::load(crate::codec::decode(&crate::codec::encode(&parse(src))).unwrap());
        let body = db
            .defs
            .iter()
            .find(|d| d.name == "main")
            .unwrap()
            .body
            .unwrap();
        let ty = crate::infer::type_of(&mut db, body);
        let desc = crate::lower::sum_shape_descriptor(&mut db, &ty).expect("descriptor");
        // The value walk must terminate + close the recursion: decode the table, confirm some entry is a
        // `Ref` (tag 11) — the recursion-closing back-edge — and that the descriptor is non-trivial.
        assert!(
            desc.len() > 8,
            "a recursive-sum descriptor is more than a trivial table"
        );
        assert!(
            desc.contains(&11u8),
            "the recursive payload closes with a Ref (tag 11)"
        );
        // The descriptor round-trips through a bounded decode (no infinite expansion): count the table
        // entries via the leading LEB and confirm it is small + finite (the IL sum has a handful).
        let table_len = desc[0] as usize; // small table → single LEB byte
        assert!(
            (1..=16).contains(&table_len),
            "IntList's shape table is small + finite, got {table_len}"
        );
    }

    #[test]
    fn a_nullary_function_call_invokes_it() {
        // `(def (g) 7)` is a NULLARY function; `(g)` — a zero-argument application — invokes it and
        // yields 7. A nullary def resolves its name to its body value (a bare `g` IS 7), so the call
        // form `(g)` must be recognized as "apply to no arguments = the head value", not misread as
        // applying the scalar 7 to zero arguments ("value is not applyable"). Composes in arithmetic
        // and inside another function's body.
        let run = |src: &str| {
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main",
            )
        };
        assert_eq!(
            run("(module m (def (mk) 42) (def (main) (mk)) (export main))"),
            42
        );
        assert_eq!(
            run("(module m (def (g) 7) (def (main) (+ (g) 5)) (export main))"),
            12
        );
        assert_eq!(
            run("(module m (def (g) 7) (def (f x) (+ x (g))) (def (main) (f 5)) (export main))"),
            12
        );
        // A nullary LAMBDA applied still works (the β-reduce path, unchanged), and a bare nullary-def
        // reference still denotes its body value.
        assert_eq!(run("(module m (def (main) ((fn () 7))) (export main))"), 7);
        assert_eq!(
            run("(module m (def (g) 7) (def (main) g) (export main))"),
            7
        );
    }

    #[test]
    fn a_duplicate_definition_is_rejected() {
        // A module evaluates to a record of its definitions, and a record has a FIXED SET of field
        // names — defining `f` twice is the same ill-formedness `(record (a 1) (a 2))` is rejected for
        // (CDZ0201), not resolved by an implicit first-wins. (Unmasked once nullary calls work: `(f)`
        // used to reject as "not applyable", hiding the duplicate.)
        let src = "(module m (def (f) 1) (def (f) 2) (def (main) (f)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("duplicate definition must reject")
            .message;
        assert!(msg.contains("more than once"), "got: {msg}");
        // Two DISTINCT named defs are fine — not a false duplicate.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (a) 1) (def (b) 2) (def (main) (+ (a) (b))) (export main))"
                )))
                .expect("compile"),
                "main"
            ),
            3
        );
    }

    #[test]
    fn a_duplicate_parameter_name_is_rejected_as_nonlinear() {
        // A function's parameter list is a BINDER POSITION, linear like a pattern (core-semantics.md
        // §Patterns Compose: "A pattern MUST bind each name at most once … rather than silently shadowing
        // an earlier binder"). `(def (f x x) …)` binds `x` twice — accepting it last-wins made the FIRST
        // parameter (and any argument passed to it, including a trap) silently unreachable. Reject
        // CDZ0102, anchored at the repeated binder.
        let code = |src: &str| -> Option<String> {
            let out = crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "m",
                    crate::codec::encode(&parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            );
            if out
                .artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some()
            {
                return None; // compiled — no rejection
            }
            out.diagnostics
                .iter()
                .find(|d| d.severity == crate::abi::Severity::Error)
                .and_then(|d| d.code.clone())
        };
        // A bare, an annotated, and a mid-list duplicate parameter all reject CDZ0102.
        assert_eq!(
            code("(module m (def (f x x) x) (def (main) (f 1 2)) (export main))").as_deref(),
            Some("CDZ0102")
        );
        assert_eq!(
            code(
                "(module m (def (f (: x Int64) (: x Int64)) x) (def (main) (f 1 2)) (export main))"
            )
            .as_deref(),
            Some("CDZ0102")
        );
        assert_eq!(
            code("(module m (def (f a b a) a) (def (main) (f 1 2 3)) (export main))").as_deref(),
            Some("CDZ0102")
        );
        // NO OVER-REJECTION: distinct params compile, and the SAME name in DIFFERENT defs is not a dup.
        assert_eq!(
            code("(module m (def (f x y) (+ x y)) (def (main) (f 3 4)) (export main))"),
            None
        );
        assert_eq!(
            code(
                "(module m (def (f x) x) (def (g x) x) (def (main) (+ (f 1) (g 2))) (export main))"
            ),
            None
        );
    }

    #[test]
    fn a_duplicate_export_is_rejected_not_miscompiled() {
        // A module's exports are a record whose fields are the exported names — exporting `a` twice is
        // the same CDZ0201 duplicate-field ill-formedness as a duplicate definition. It MUST reject
        // BEFORE emitting: two export entries of one name are forbidden by the component binary format,
        // so emitting them writes an invalid component that fails to parse (decline-don't-miscompile).
        let src = "(module m (def (a) 42) (export a) (export a))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("duplicate export must reject, not emit an invalid component")
            .message;
        assert!(msg.contains("exported more than once"), "got: {msg}");
        // A single export is unaffected — the control compiles and runs.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (a) 42) (export a))"
                )))
                .expect("compile"),
                "a"
            ),
            42
        );
    }

    #[test]
    fn an_unmodeled_top_level_form_declines() {
        // A top-level declaration the compiler does not model — `(pragma …)` and any other unrecognized
        // declaration-shaped head — makes the whole program DECLINE (decline-don't-miscompile), NOT
        // silently ignore it and run `main` as if it were absent. `(pragma …)` is unmodeled, so the
        // program declines rather than compiling `(def (main) 1)` alone. (`(effect …)` USED to be the
        // example here; it is now a modeled top-level form — see `a_bare_effect_declaration_compiles`.)
        let src = "(do (pragma strict) (def (main) 1) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a program with an unmodeled top-level form must decline"
        );
    }

    #[test]
    fn a_bare_effect_declaration_compiles() {
        // An `(effect …)` declaration is a routing-agnostic contract; declaring it grants nothing and
        // performs nothing (`capabilities-and-effects.md` §An Effect Declaration Names The Effect). So a
        // program that declares an effect but never performs it is well-formed — `main` returns its
        // value, the effect decl contributing no behavior (E0: the surface is recognized, not lowered).
        let src = "(do (effect E (op f (-> Int64 Int64))) (def (main) 1) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a bare effect declaration compiles"),
                "main"
            ),
            1
        );
    }

    #[test]
    fn a_duplicate_effect_operation_is_rejected() {
        // An effect's operations are a closed name-set (like a sum's variants), so declaring `f` twice is
        // CDZ0201 — the fifth closed-name-set duplicate-member check (`capabilities-and-effects.md` §An
        // Effect Declaration Names The Effect And Types Its Operations).
        let src = "(do (effect E (op f (-> Int64 Int64)) (op f (-> Int64 Int64))) \
                   (def (main) 1) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a duplicate effect operation must be rejected"
        );
    }

    #[test]
    fn performing_an_operation_with_a_wrong_type_argument_is_rejected() {
        // `E.op` is declared `(-> Int64 Int64)`; performing `(E.op true)` supplies a Bool where an Int64
        // is required. E0 synthesizes `E.op` as a typed operation value, so a perform is an ordinary
        // application checked against that arrow (`capabilities-and-effects.md` §Performing An Operation
        // Is Typed) — a wrong-type argument is a type error even before the perform can be lowered
        // (which E0 does not do; the point is the ARGUMENT check fires at the surface). The perform is
        // written with the qualified projection `(. E op)` (the desugaring of `E.op`); `main` returns the
        // op's result, so the whole program still declines to run (no handler yet) — but the mistyped
        // argument is caught first.
        let src = "(do (effect E (op op (-> Int64 Int64))) \
                   (def (main) ((. E op) true)) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a wrong-type perform argument must be rejected"
        );
    }

    #[test]
    fn two_effects_may_declare_a_same_named_operation() {
        // An operation is reached through its declaring effect, so two effects may each declare an `op`
        // of the same name without collision (`capabilities-and-effects.md` §An Operation Is Reached
        // Through Its Declaring Effect). Both `A.op` and `B.op` synthesize as distinct operation values
        // (identity = the declaring effect's occurrence), so neither the declarations nor the projections
        // collide — the program is well-formed (it declines to RUN only for want of a handler, an E1
        // concern; here we assert the two same-named ops do not clash at the surface, so a projection of
        // each resolves rather than erroring unbound/duplicate).
        let src = "(do (effect A (op op (-> Int64 Int64))) (effect B (op op (-> Bool Bool))) \
                   (def (main) 1) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("two effects with a same-named op are well-formed"),
                "main"
            ),
            1
        );
    }

    #[test]
    fn a_tail_resumptive_handler_runs() {
        // E1c: a tail-resumptive handler is REDUCED AWAY at lowering — the perform `(Choose.pick)` is
        // resolved to its arm and rewritten to the arm's resume value `5`, so `(+ (Choose.pick) 1)` = 6.
        // The handler is stateless (seed unit, thread s unchanged); the whole `handle` becomes plain
        // arithmetic, so `select` sees only ordinary `Core` (no effect node, no runtime handler search).
        let src = "(do (effect Choose (op pick (-> Unit Int64))) \
                   (def (main) (handle unit (((. Choose pick) () s (resume 5 s))) \
                   (+ ((. Choose pick)) 1))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a tail-resumptive handler compiles and runs"),
                "main"
            ),
            6
        );
    }

    #[test]
    fn an_effect_reached_with_no_handler_or_delegation_is_cdz0401() {
        // E1d: an effect operation performed with NEITHER an enclosing handler NOR a host delegation has
        // no home — CDZ0401 (`capabilities-and-effects.md` §An Ungranted Effect Is A Compile-Time Error).
        // A handled perform is reduced away before lowering, so any perform reaching lowering directly is
        // ungranted. Reported cleanly (not as a leaked "unknown intrinsic" / unbound `effect-op`).
        let src = "(do (effect Ask (op ask (-> Unit Int64))) \
                   (def (main) (+ ((. Ask ask)) 1)) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("an ungranted effect must be rejected");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0401"),
            "expected CDZ0401 (no home for a reached effect), got: {}",
            err.message
        );
    }

    #[test]
    fn a_handler_arm_for_an_undeclared_operation_is_cdz0403() {
        // E2a: a handler arm naming an operation its effect does not declare is a closed-set violation —
        // CDZ0403 (`capabilities-and-effects.md` §A Handler Arm Names An Operation Its Effect Declares).
        // `Choose` declares only `pick`; an arm `((. Choose guess) …)` names `guess` — undeclared. This
        // is CDZ0403, not the generic "record has no field" CDZ0201 the member projection alone gives.
        let src = "(do (effect Choose (op pick (-> Unit Int64))) \
                   (def (main) (handle unit (((. Choose guess) () s (resume 5 s))) ((. Choose pick)))) \
                   (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a handler arm for an undeclared op must be rejected");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0403"),
            "expected CDZ0403 (handler arm names an undeclared op), got: {}",
            err.message
        );
    }

    #[test]
    fn an_undeclared_handler_op_close_to_a_declared_one_suggests_it() {
        // The effect-op "did you mean?" (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route
        // To A Fix): a handler arm names `emitt`, a typo of the effect's declared `emit` → CDZ0403 names
        // the near op AND carries a replace fix on the op KEY.
        let src = "(do (effect Log (op emit (-> Int64 Unit))) \
                   (def (main) (handle unit (((. Log emitt) (v) s (resume unit s))) ((. Log emit) 5))) \
                   (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("undeclared op must reject");
        assert_eq!(err.code.as_deref(), Some("CDZ0403"), "got: {}", err.message);
        assert!(
            err.message.contains("did you mean `emit`?"),
            "names the near op: {}",
            err.message
        );
        let fix = err.fix.expect("a fix is carried");
        assert_eq!(fix.replacement, "emit");
        assert!(!fix.verified, "a nearest-name guess is heuristic");
    }

    #[test]
    fn a_non_exhaustive_handler_is_cdz0405() {
        // A `handle E` names ONE effect and its arms ARE that effect's operations; an effect's operations
        // are a closed set, so a handler must discharge the WHOLE set (CDZ0405 —
        // `capabilities-and-effects.md` §A Handler Discharges Its Effect, the effect analogue of match
        // exhaustiveness). `Diag` declares `emit` + `collect`; a `handle Diag` binding only `emit` leaves
        // `collect` undischarged — rejected. Written in the CANONICAL shape so the load-time desugar
        // (effect + seed promoted, bare arm op) is exercised on the way to the check.
        let src = "(do (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64)))) \
                   (def (main) (handle Diag (list) ((emit (code) s (resume unit (List.push s code)))) \
                                 (do (Diag.emit 1) 0))) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a handler missing an operation must be rejected");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0405"),
            "expected CDZ0405 (non-exhaustive handler), got: {}",
            err.message
        );
    }

    #[test]
    fn a_delegation_of_an_unreached_effect_is_cdz0404() {
        // E2a: a `host` delegation naming an effect the body never reaches is latent authority — CDZ0404
        // (`capabilities-and-effects.md` §Host Delegation Is An Entrypoint's Prerogative). `main`
        // delegates `log` but its body is `42` (never performs `log.emit`), so the manifest would carry a
        // granted-but-unexercised capability — rejected.
        let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) 42)) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a delegation of an unreached effect must be rejected");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0404"),
            "expected CDZ0404 (latent authority), got: {}",
            err.message
        );
    }

    #[test]
    fn a_latent_authority_delegation_offers_a_delete_fix() {
        // The CDZ0404 repair is to DROP the unreached effect from the manifest — a DELETE fix on the
        // effect-name occurrence (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
        // Fix), the first use of `Edit::Delete`. `main` delegates `log` but never performs it.
        let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) 42)) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("latent authority must reject");
        assert_eq!(err.code.as_deref(), Some("CDZ0404"), "got: {}", err.message);
        let fix = err.fix.expect("a delete fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::Delete);
        assert!(!fix.verified, "a delete is heuristic (intent guess)");
        // The fix targets the delegated effect-name occurrence (the same node the diagnostic anchors).
        assert_eq!(
            Some(fix.node),
            err.node,
            "delete targets the effect-name node"
        );
    }

    #[test]
    fn a_stateful_handler_threads_its_state_across_performs() {
        // E1c-2: a handler that FOLDS state — `(resume s (+ s 1))` hands back the current state and
        // threads `s+1` forward — is reduced by the evaluation-order fold. `(Fresh.next)` reads state
        // `0`; three performs in a `do` see 0, 1, 2, and the `do` yields the last (2). The state binder
        // `s` (bound in scope by E1b) is substituted with the threaded state at each perform. The whole
        // handle becomes plain arithmetic, so it runs to 2 (`capabilities-and-effects.md` §A Handler
        // Threads State Across The Operations It Discharges).
        let src = "(do (effect Fresh (op next (-> Unit Int64))) \
                   (def (main) (handle 0 (((. Fresh next) (u) s (resume s (+ s 1)))) \
                   (do ((. Fresh next)) ((. Fresh next)) ((. Fresh next))))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a stateful tail-resumptive handler compiles and runs"),
                "main"
            ),
            2
        );
    }

    #[test]
    fn a_cross_function_perform_is_discharged_by_the_callers_handler() {
        // E1c-3 (the inline trigger): a perform in a CALLEE `gen` is discharged by the handler enclosing
        // `gen`'s CALL — `(handle … (gen))`. The fold inlines `gen` into the handled region (β-reduces
        // the call) so its perform `(Bump.by 41)` resolves to the arm, which resumes `(+ n 1)` = 42.
        // `gen` performing an effect is well-formed (its home is its caller's handler, not itself), so it
        // is not independently faulted CDZ0401. (`capabilities-and-effects.md` §Handler Resolution Is
        // Dynamic In Extent — a function may perform an operation its caller discharges.)
        let src = "(do (effect Bump (op by (-> Int64 Int64))) \
                   (def (gen) ((. Bump by) 41)) \
                   (def (main) (handle unit (((. Bump by) (n) s (resume (+ n 1) s))) (gen))) \
                   (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a cross-function perform is discharged by the caller's handler"),
                "main"
            ),
            42
        );
    }

    #[test]
    fn a_recursive_effectful_function_is_specialized_per_context() {
        // E3: a recursive function that performs a discharged op is SPECIALIZED under its handler context
        // — the handler's state threads as a trailing parameter (`DESIGN-effects-rcdzc.md` §4.3).
        // `loop` performs `(Countdown.tick)` and recurses; the arm resumes the current counter and
        // threads `s-1`, so ticks read 3,2,1,0 and `loop` returns `1+1+1+0` = 3. `loop#ctx(s)` becomes
        // `(if (= s 0) 0 (+ 1 (loop#ctx (- s 1))))` — a single-return recursive fn, no multi-value.
        let src = "(do (effect Countdown (op tick (-> Unit Int64))) \
                   (def (loop) (if (= (Countdown.tick) 0) 0 (+ 1 (loop)))) \
                   (def (main) (handle 3 ((Countdown.tick (u) s (resume s (- s 1)))) (loop))) \
                   (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a recursive effectful function is specialized and runs"),
                "main"
            ),
            3
        );
    }

    #[test]
    fn a_recursive_fn_under_a_two_arm_single_effect_handler_specializes() {
        // E3-multi-arm: a handler with SEVERAL arms of ONE effect threads ONE logical state, so a
        // recursive fn under it specializes with a single trailing state param (each perform substitutes
        // its OWN arm's state binder). `St` has two ops — `get` (reads the counter) and `tick` (returns 1,
        // threads `s-1`); `loop` recurses summing a `tick` per non-zero `get`. Seeded 3: `get` reads 3,2,1,0
        // and `tick` returns 1 three times → `1+1+1+0` = 3. (The 1-arm case is countdown above; this pins
        // that the arm-count no longer gates specialization — only the single-EFFECT property does.)
        let src = "(do (effect St (op get (-> Unit Int64)) (op tick (-> Unit Int64))) \
                   (def (loop) (if (= (St.get) 0) 0 (+ (St.tick) (loop)))) \
                   (def (main) (handle 3 ((St.get (u) s (resume s s)) (St.tick (u) s (resume 1 (- s 1)))) \
                     (loop))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect(
                    "a recursive fn under a 2-arm single-effect handler specializes and runs"
                ),
                "main"
            ),
            3
        );
    }

    #[test]
    fn recursion_is_detected_through_a_nested_do() {
        // A self-call inside a nested `(do …)` is a real recursion edge. `resolve_do` collapses a `do` to
        // `Ref{last}` (intermediates discarded as pure), which would hide a self-call in a `do` item from
        // `is_recursive`'s callee walk — so `collect_callees` reads a `do` by raw AST and descends every
        // item. Without this, `sum-to` here would read as NON-recursive and inline without end (a hang) or
        // miscompile. `(do 0 (sum-to (- n 1)))` puts the self-call as the last item, and `(do (dummy) …)`
        // shape (a discarded intermediate then the recursive tail) is exactly the effect-walk shape. This
        // pins recursion detection through `do` at the plain (non-effect) level. `sum-to 3` = 3+2+1+0 = 6.
        let src = "(module m (def (sum-to n) (if (= n 0) 0 (do 7 (+ n (sum-to (- n 1)))))) \
                   (def (main) (sum-to 3)) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("recursion through a nested do is detected and compiles"),
                "main"
            ),
            6
        );
    }

    #[test]
    fn a_parameterized_recursive_walk_threads_a_list_state() {
        // E3: a recursive PARAMETERIZED effectful walk `(walk n)` that accumulates into a LIST state,
        // performing inside a nested `(do …)`. `walk` emits `n` at each step (threading `(List.push s n)`)
        // and reads the list back at the base via `collect`. Specialized as `walk#ctx(n: Int64, s: List
        // Int64)` — original param `n` threaded + annotated with its solved type, state a trailing list
        // param, recursion via the memoized self-call. Seeded `(list 0)` (a DETERMINED element type — an
        // empty `(list)` seed declines, its element type being `Any`), `(walk 3)` accumulates
        // `(list 0 3 2 1)` whose length is 4. Exercises parameterized spec + list-state-through-recursion
        // + do-intermediate perform + recursion-through-do all at once.
        let src = "(do (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64)))) \
                   (def (walk n) (if (< n 1) (Diag.collect unit) (do (Diag.emit n) (walk (- n 1))))) \
                   (def (main) (handle (list 0) \
                     ((Diag.emit (v) s (resume unit (List.push s v))) (Diag.collect (u) s (resume s s))) \
                     (List.len (walk 3)))) (export main))";
        // COMPILES (the specialization + list-state threading succeed) — asserting compilation, not a run,
        // because a list-returning body needs the value-heap runtime composed from the store (the
        // `#[ignore]`d heap tests do that); the store-driven CLI run yields 4 (`(list 0 3 2 1)` length).
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a parameterized recursive walk threading a determined list state must compile"
        );
    }

    #[test]
    fn an_abortive_handler_arm_abandons_the_computation() {
        // E4: an ABORTIVE arm `(Bail.bail (n) s n)` never resumes — performing `(Bail.bail 7)` inside
        // `(+ 1 (Bail.bail 7))` ABANDONS the surrounding `+ 1` (control never returns) and the handle
        // yields the arm value 7, NOT 8. The fold classifies the arm abortive (no `resume` in its body)
        // and, when the perform fires in a strict position, records the arm value as the whole handle's
        // value (the surrounding computation is dead). This is the "bail" / typed early-exit class.
        let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 0 ((Bail.bail (n) s n)) (+ 1 (Bail.bail 7)))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("an abortive handler compiles"),
                "main"
            ),
            7
        );
    }

    #[test]
    fn an_abortive_perform_in_a_tail_if_branch_folds_per_branch() {
        // E4 branch-tail fold: an abortive perform in the TAIL of a tail-position `if` branch is LOCAL to
        // that branch — the `if` IS the handle body's value, so per-branch the abort just yields the arm
        // value for that branch and the sibling branch survives. `(if true (Bail.bail 7) 99)` folds to
        // `(if true 7 99)` → 7; `(if false (Bail.bail 7) 99)` → 99. A constant condition here keeps the
        // test to the fold (a runtime param in a handle body is a separate, not-yet-supported case); the
        // guard is STRUCTURAL — it does not rely on the constant folding away.
        let aborts = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 0 ((Bail.bail (n) s n)) (if true (Bail.bail 7) 99))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(aborts)))
                    .expect("a tail-if-branch abort compiles"),
                "main"
            ),
            7
        );
        let survives = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 0 ((Bail.bail (n) s n)) (if false (Bail.bail 7) 99))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(survives)))
                    .expect("the non-aborting branch survives"),
                "main"
            ),
            99
        );
    }

    #[test]
    fn a_non_tail_conditional_abortive_perform_declines_rather_than_miscompiles() {
        // E4 soundness guard: an abortive perform under a NON-tail conditional cannot be realized by either
        // sound shape — the unconditional collapse (whole handle → arm value) would abort the other path
        // too, and the per-branch fold needs the `if` to BE the handle value. `(+ 1 (if true (Bail.bail 7)
        // 0))` would have to abort OUT of the enclosing `+ 1`, which needs a real `block`/`br` control node
        // (a later increment). `body_has_unsound_abortive_perform` flags it (`under_cond && !tail`) and
        // `reduce_handle` DECLINES rather than miscompile. The guard is STRUCTURAL — a constant condition
        // does not let the abort slip past as an unconditional one. Regression guard for the E4-a miscompile.
        let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 99 ((Bail.bail (n) s n)) (+ 1 (if true (Bail.bail 7) 0)))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a non-tail conditional abortive perform must decline, not miscompile"
        );
    }

    #[test]
    fn an_abortive_perform_in_a_short_circuit_operand_declines() {
        // E4 soundness guard, the short-circuit variant: `(or true (Bail.bail 7))` evaluates its right
        // operand only when `true` is false — a CONDITIONAL abort. Unlike an `if`, the threading path folds
        // `and`/`or` as a STRICT `Apply` (both operands, no per-branch abort capture), so an abort in the
        // right operand would set the abort cell and collapse the whole handle — the wrong value. The guard
        // marks a short-circuit right operand `under_cond`, so the abort is flagged and `reduce_handle`
        // DECLINES (a short-circuit conditional abort needs the `if`-style per-branch fold, a later step).
        // `Bail.bail` returns `Bool` here so it can sit in an `or`.
        let src = "(do (effect Bail (op bail (-> Int64 Bool))) \
                   (def (main) (handle false ((Bail.bail (n) s n)) (or true (Bail.bail 7)))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "an abortive perform in a short-circuit right operand must decline, not miscompile"
        );
    }

    #[test]
    fn an_abortive_perform_in_a_tail_if_branch_under_a_let_folds_per_branch() {
        // E4 branch-tail fold, the `let`-body case: a `let`'s VALUE is its BODY's value, so a `let` body is
        // in the same tail position as the `let`. An abortive perform in the tail of an `if` branch inside a
        // tail-position `let` body folds per-branch exactly like a bare `if`. `(let ((k 5)) (if true
        // (Bail.bail 7) k))` → 7 (abort); the `false`-branch sibling would yield `k`=5 (survives). The FOLD
        // already threaded the let body correctly; this pins that the guard's `let` arm carries tail-ness
        // into the body (a generic descent would have marked it non-tail and wrongly DECLINED).
        let aborts = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 0 ((Bail.bail (n) s n)) (let ((k 5)) (if true (Bail.bail 7) k)))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(aborts)))
                    .expect("a let-body tail-if-branch abort compiles"),
                "main"
            ),
            7
        );
        let survives = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 0 ((Bail.bail (n) s n)) (let ((k 5)) (if false (Bail.bail 7) k)))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(survives)))
                    .expect("the non-aborting let-body branch survives"),
                "main"
            ),
            5
        );
    }

    #[test]
    fn an_abortive_perform_in_a_conditional_let_init_declines() {
        // E4 soundness guard, the `let`-init case: a `let` binding's INIT is a strict operand (NON-tail — its
        // value feeds the binder), so an abortive perform in an init UNDER A CONDITIONAL cannot be realized
        // by the per-branch fold (the branch value must become `k`, not abandon the whole handle). `(let ((k
        // (if true (Bail.bail 7) 0))) (+ 1 k))` must DECLINE — the guard's `let` arm descends each init at
        // `tail=false`, so the abort is flagged (`under_cond && !tail`). Contrast an UNCONDITIONAL init abort
        // (`(let ((k (Bail.bail 7))) …)`) which is the sound E4-a collapse and still folds.
        let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main) (handle 99 ((Bail.bail (n) s n)) (let ((k (if true (Bail.bail 7) 0))) (+ 1 k)))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "an abortive perform in a conditional let-init must decline, not miscompile"
        );
    }

    #[test]
    fn a_handle_body_reads_an_enclosing_function_parameter() {
        // The fold's rewritten body must resolve a FREE variable up the ORIGINAL lexical chain — a handle
        // body is not closed, it may read an enclosing function's parameter. `(+ x (Get.get 0))` under a
        // handler that resumes 5 is `x + 5`; called with x=10 → 15. `reduce_handle` synthesizes a fresh
        // body subtree (root parent `None`); lowering re-anchors it UNDER the original `handle` node so `x`
        // still reaches `main`'s parameter binder instead of a spurious CDZ0101. Regression guard for the
        // reparent fix — before it, ANY handle body referencing a function parameter failed to compile.
        use wasmtime::component::Val;
        let src = "(do (effect Get (op get (-> Int64 Int64))) \
                   (def (main (: x Int64)) (handle 0 ((Get.get (n) s (resume 5 s))) (+ x (Get.get 0)))) (export main))";
        assert_eq!(
            run_returns_with::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("a handle body reading a parameter compiles"),
                "main",
                &[Val::S64(10)],
            ),
            15
        );
    }

    #[test]
    fn a_runtime_condition_selects_an_abortive_branch_reading_a_parameter() {
        // Composes the branch-tail abortive fold with a RUNTIME condition over an enclosing parameter — the
        // "validate and bail" shape. `(if (< x 5) (Bail.bail 7) x)`: the true branch aborts (arm value 7),
        // the false branch reads the parameter `x`. Called with x=3 (< 5) → 7 (abort); x=9 → 9 (fall
        // through). Exercises the reparent fix (free `x`) together with `thread_branch_local_abort`.
        use wasmtime::component::Val;
        let src = "(do (effect Bail (op bail (-> Int64 Int64))) \
                   (def (main (: x Int64)) (handle 0 ((Bail.bail (n) s n)) (if (< x 5) (Bail.bail 7) x))) (export main))";
        let component = compile_component(&crate::codec::encode(&parse(src)))
            .expect("a runtime-conditioned branch-tail abort reading a parameter compiles");
        assert_eq!(
            run_returns_with::<i64>(&component, "main", &[Val::S64(3)]),
            7,
            "x=3 (< 5) aborts to the arm value 7"
        );
        assert_eq!(
            run_returns_with::<i64>(&component, "main", &[Val::S64(9)]),
            9,
            "x=9 falls through to the parameter"
        );
    }

    #[test]
    fn nested_intra_program_handlers_compose_inside_out() {
        // E3-nested: two nested `handle`s compose — the fold reduces the INNER handle first (discharging
        // its effect), leaving the OUTER effect's performs for the outer fold. `(A.a)` resumes 22 (inner),
        // `(B.b)` resumes 20 (outer), so `(+ (A.a) (B.b))` = 42. The inner handle in the outer's body is
        // recursively `reduce_handle`d, then threaded under the outer context.
        let src = "(do (effect A (op a (-> Unit Int64))) (effect B (op b (-> Unit Int64))) \
                   (def (main) (handle 0 ((B.b (u) s (resume 20 s))) \
                     (handle 0 ((A.a (u) s (resume 22 s))) (+ (A.a) (B.b))))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("nested intra-program handlers compose"),
                "main"
            ),
            42
        );
    }

    #[test]
    fn a_delegated_effect_performed_inside_an_intra_program_handler_compiles() {
        // E2h: a host-delegated `ask.ask` performed INSIDE an intra-program `handle` (over a DIFFERENT
        // effect `Scale`). The fold reduces the `Scale` handler away (its arm doubles the perform value),
        // leaving `(ask.ask)` — a host call — in the SYNTHESIZED rewritten body. That synthesized node has
        // no `host` ANCESTOR, so `perform_host_target` falls back to the program-wide delegation set (the
        // manifest is the union of the entrypoints' `host` clauses) to recognize it as host-bound. It
        // compiles to a component importing `ask` (verified via the gate → 42 with `ask.ask=21`). The host
        // import is unbound in-process, so this asserts it COMPILES; the corpus gate runs the value.
        let src = "(do (effect ask (op ask (-> Unit Int64))) (effect Scale (op by (-> Int64 Int64))) \
                   (def (main) (host (ask) (handle unit ((Scale.by (n) s (resume (* n 2) s))) \
                     (Scale.by (ask.ask))))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a host-delegated perform inside an intra-program handler must compile"
        );
    }

    #[test]
    fn a_host_op_with_a_string_argument_compiles() {
        // E2h-string: a host op `log.emit : String -> Unit` performed on a CONSTANT string. The string
        // crosses the boundary as the component `string` (core `(ptr,len)`): the compiler bakes "ready"
        // into a data segment, imports a shared memory, and the op's canon-lower carries the Memory option
        // (the shared-memory 2-instance envelope shape). Compiles to a component importing
        // `log: interface { emit: func(p0: string) }` (verified via the gate → unit + observed log.emit).
        let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (log.emit \"ready\"))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a host op with a constant string argument must compile"
        );
    }

    #[test]
    fn a_do_block_of_host_calls_sequences_them() {
        // E2h-seq: a `(do (log.emit "first") (log.emit "second"))` — two side-effecting host-call
        // statements — lowers to a `Core::Seq` that EMITS each statement in order (their calls both cross
        // the boundary), then the tail is the block's value. Was a clean decline (the block would else
        // drop the non-final call); now it compiles + BOTH calls fire (verified in order via the corpus
        // gate → observed [log.emit, log.emit]). The host imports are unbound in-process, so this asserts
        // it COMPILES; the gate runs the observed sequence.
        let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (main) (host (log) (do (log.emit \"first\") (log.emit \"second\")))) \
                   (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a do block of host-call statements must compile (sequencing both calls)"
        );
    }

    #[test]
    fn a_delegated_effect_reached_through_a_recursive_callee_compiles() {
        // E2h-rec: `main` delegates `log` and calls a RECURSIVE `go` that performs `log.emit` on each
        // step. Two coupled fixes: (1) `body_reaches_effect` (the CDZ0404 latent-authority check) now
        // FOLLOWS a recursive callee (visited-set guarded), so `log` is seen as reached — no false
        // "latent authority"; (2) `go`'s `log.emit` lowers to a `Core::HostCall` via
        // `perform_host_target`'s program-delegation fallback (the enclosing entrypoint delegates `log`),
        // and the `(do (log.emit "x") (go …))` body sequences it via `Core::Seq` (E2h-seq) so the call is
        // EMITTED, not dropped. Compiles to a component importing `log` (gate → unit + observed log.emit).
        let src = "(do (effect log (op emit (-> String Unit))) \
                   (def (go n) (if (= n 0) unit (do (log.emit \"x\") (go (- n 1))))) \
                   (def (main) (host (log) (go 1))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a host effect reached through a recursive callee must compile"
        );
    }

    #[test]
    fn an_intra_program_handler_interposes_on_a_delegated_effect_and_forwards() {
        // E2h-interpose: the entrypoint delegates `ask` to the host, but an inner handler INTERCEPTS every
        // `ask.ask`, records it via the intra-program `Count.tick`, and RE-PERFORMS `(ask.ask)` in tail
        // position (forwarding). The arm body `(do (Count.tick) (resume (ask.ask) s))` is a
        // do-wrapped resume — the perform arm peels the trailing resume, keeps the `Count.tick` statement
        // (folded to the OUTER Count handler), and forwards the re-performed `ask.ask` to the host (a
        // `Core::HostCall`, since no nearer handler discharges it). `(+ (ask.ask) (ask.ask))` with host
        // responses 3,4 → 7 (2 observed ask.ask calls). The host is unbound in-process, so assert it
        // COMPILES; the corpus gate runs the value + host-call sequence.
        let src = "(do (effect ask (op ask (-> Unit Int64))) (effect Count (op tick (-> Unit Unit))) \
                   (def (main) (host (ask) \
                     (handle unit ((Count.tick (u) s (resume unit s))) \
                       (handle unit ((ask.ask () s (do (Count.tick) (resume (ask.ask) s)))) \
                         (+ (ask.ask) (ask.ask)))))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "an interposing handler that forwards a delegated effect must compile"
        );
    }

    #[test]
    fn a_recursive_fn_threads_two_nested_handlers_states_at_once() {
        // E3h-B: a recursive `loop` runs under TWO nested stateful handlers — `A` (countdown seeded 3,
        // `tick` threads `s-1`) governs recursion depth, `B` (accumulator seeded 0, `bump` threads `s+10`)
        // folds across the steps. The ticks read 3,2,1,0 (three non-zero), the bumps read 0,10,20, so the
        // sum is 0+10+20+0 = 30. Both states are live simultaneously — neither handler alone can specialize
        // `loop` (it performs BOTH effects), so the fold MERGES the two contexts into one 2-slot context
        // and specializes `loop#ctx(s_B, s_A)` once, threading each effect's state as its own trailing
        // param (`DESIGN-effects-rcdzc.md` §4.3: "two nested handlers → two trailing params").
        let src = "(do (effect A (op tick (-> Unit Int64))) (effect B (op bump (-> Unit Int64))) \
                   (def (loop) (if (= (A.tick) 0) 0 (+ (B.bump) (loop)))) \
                   (def (main) (handle 0 ((B.bump (u) s (resume s (+ s 10)))) \
                     (handle 3 ((A.tick (u) s (resume s (- s 1)))) (loop)))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src)))
                    .expect("two nested handler states specialize"),
                "main"
            ),
            30
        );
    }

    #[test]
    fn a_recursive_effectful_walk_accumulates_into_a_list_state_handler() {
        // E3 list-state: a recursive effectful walk over a 2-arm handler whose state is an empty-list
        // seed `(list)`. `walk` performs `(Diag.emit n)` at each descent step and reads the accumulator
        // back with `(Diag.collect)` at the base; the handler seeds `(list)` and threads `(List.push s v)`,
        // so `(walk 3)` accumulates `(list 3 2 1)`, whose length is 3. The empty-list seed's element type
        // is fixed by JOINING the seed type with each tail arm's next-state type — the growing arm's
        // `(List.push s v)` reveals `List Int64` because the arm op param `v` types from the op's declared
        // `(-> Int64 Unit)` signature (`handle_arm_param_ty`). Was the hang/decline boundary; now served.
        // Also the regression guard for the unbounded-inline loop (`THREAD_INLINE_LIMIT`) — a served value
        // still proves the fold TERMINATES.
        let src = "(do (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64)))) \
                   (def (walk n) (if (< n 1) (Diag.collect unit) (do (Diag.emit n) (walk (- n 1))))) \
                   (def (main) (handle (list) \
                     ((Diag.emit (v) s (resume unit (List.push s v))) (Diag.collect (u) s (resume s s))) \
                     (List.len (walk 3)))) (export main))";
        // The state lives on the value heap (`List.push`/`List.len`), so this needs the store runtime the
        // in-process linker doesn't supply — assert it COMPILES (a well-formed component); the corpus gate
        // (`14-effects-and-handlers.sexp`) verifies the runtime value is 3 against the real heap.
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "recursive effectful list-state walk must compile"
        );
    }

    #[test]
    fn resuming_with_a_wrong_type_value_is_cdz0201() {
        // E1c-2: the value a handler resumes with is returned to the perform site, so it must have the
        // operation's declared RESULT type (`capabilities-and-effects.md` §Performing An Operation Is
        // Typed). `(resume true s)` for an `(-> Int64 Int64)` op resumes with a Bool — CDZ0201 (the
        // result-type companion of the perform-argument check). Without the check the fold would silently
        // substitute `true` as `(E.op 1)`'s value, a type-confusion miscompile.
        let src = "(do (effect E (op op (-> Int64 Int64))) \
                   (def (main) (handle unit ((E.op (n) s (resume true s))) (E.op 1))) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a wrong-type resume value must be rejected");
        assert_eq!(
            err.code.as_deref(),
            Some("CDZ0201"),
            "expected CDZ0201 (resume value vs result type), got: {}",
            err.message
        );
    }

    #[test]
    fn a_runtime_integer_width_is_rejected() {
        // `(def (mk n) (: 5 (UInt n)))` puts a RUNTIME value `n` (a parameter) in a width position — a
        // width must be a compile-time natural (`numeric-model.md §An Integer Type Is Indexed By A
        // Compile-Time Width`), so it is rejected (CDZ0302) EVEN THOUGH a constant call site `(mk 8)`
        // would fold `n` — the width is non-dependent by declaration. (The check fires on the un-inlined
        // `mk` body during well-formedness, which covers every def reachable or not.)
        let src = "(module m (def (mk n) (: 5 (UInt n))) (def (main) (mk 8)) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a width from runtime data must reject, not fold at a constant call site"
        );
        // Sanity: a CONSTANT width still works — `(UInt 8)` is fine, crossing as a `u8`.
        assert_eq!(
            run_returns::<u8>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (main) (: 5 (UInt 8))) (export main))"
                )))
                .expect("constant width compiles"),
                "main"
            ),
            5
        );
    }

    #[test]
    fn a_duplicate_sum_variant_is_rejected() {
        // 05-compound-types §a sum declaring a variant name twice: `(type T (A Int64) (A Bool))` names
        // the variant `A` twice — a sum's variant names are a fixed SET (the fourth closed name-set
        // beside record fields, module definitions, and exports), so it is CDZ0201, the same
        // duplicate-member ill-formedness. Rejected, not silently registered as two `A`s.
        let src = "(module m (type T (A Int64) (A Bool)) (def (main) 1) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a duplicate sum variant must reject")
            .message;
        assert!(msg.contains("more than once"), "got: {msg}");
        // DISTINCT variant names in one sum are fine — the type decl is scanned + validated but does not
        // block an otherwise-valid program (the sum's construction/use is a later increment).
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (type T (A Int64) (B Bool)) (def (main) 1) (export main))"
                )))
                .expect("compile"),
                "main"
            ),
            1
        );
        // Two DIFFERENT types may REUSE a variant name — the set is per-declaration, not global.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (type T (A Int64)) (type U (A Bool)) (def (main) 1) (export main))"
                )))
                .expect("compile"),
                "main"
            ),
            1
        );
    }

    #[test]
    fn a_sum_type_name_resolves_to_its_sum_type_in_type_position() {
        // The records-everywhere realization: `(type Option (Some Int64) None)` binds `Option` to a
        // synthesized RECORD whose `(meta t)` is the sum type-value. So `Option` used in a type
        // annotation reduces to `Ty::Sum` through the ORDINARY `(meta t)` projection — the same path
        // `(: e UInt8)` takes to `Ty::Int`, no sum special case. Its identity is the DECLARATION
        // occurrence (`type-system.md §158`), and its render name is the declared name.
        use crate::db::Db;
        use crate::eval::typeval_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        let ast = parse("(module m (type Option (Some Int64) None) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = db.type_decl_by_name("Option").expect("Option bound");
        let ty = typeval_of(&mut db, occ).expect("Option is a type");
        match ty {
            Ty::Sum { name, decl, args } => {
                assert_eq!(name, "Option");
                assert!(
                    args.is_empty(),
                    "a bare monomorphic-shaped sum has no type args"
                );
                // The identity is the declaration occurrence recorded in the scan.
                let decl_occ = db
                    .type_decls
                    .iter()
                    .find(|t| t.name == "Option")
                    .unwrap()
                    .occ;
                assert_eq!(decl, decl_occ);
            }
            other => panic!("expected Ty::Sum, got {}", other.render_name()),
        }
    }

    #[test]
    fn newtype_underlying_reads_the_erased_structural_type() {
        // `Db::newtype_underlying` reports the underlying structural type of an erasable single-variant
        // sum (a nominal newtype), and declines (None) for everything that must stay boxed. This is the
        // predicate the erasure (N3/N4) keys off — the realization of `§Nominal Is An Orthogonal
        // Modifier` (the tag adds nothing to the runtime representation).
        use crate::db::Db;
        use crate::infer::newtype_underlying;
        use crate::testkit::parse;
        use crate::ty::{IntTy, Ty};

        let decl_of = |db: &Db, n: &str| db.type_decls.iter().find(|t| t.name == n).unwrap().occ;

        // A newtype over a scalar → the scalar's type.
        let ast = parse("(module m (type UserId (Mk Int64)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "UserId");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            Some(Ty::Int(IntTy::i64())),
            "a newtype over Int64 erases to Int64"
        );

        // A multi-payload single variant (a struct) → a tuple of the payload types.
        let ast = parse("(module m (type Point (Mk Int64 Int64)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "Point");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            Some(Ty::Tuple(
                vec![Ty::Int(IntTy::i64()), Ty::Int(IntTy::i64())].into()
            )),
            "a two-payload single variant erases to a 2-tuple"
        );

        // A nullary single variant (a unit tag) → Unit.
        let ast = parse("(module m (type Marker (The)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "Marker");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            Some(Ty::Unit),
            "a nullary single variant erases to Unit"
        );

        // A MULTI-variant sum is NOT a newtype (it needs the discriminant).
        let ast = parse("(module m (type E (A Int64) (B Int64)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "E");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            None,
            "a two-variant sum stays boxed"
        );

        // A RECURSIVE single-variant sum is NOT erased (its inner would be infinite / inconsistently
        // boxed) — the recursion guard declines.
        let ast = parse(
            "(module m (type Stream (More (Tuple Int64 Stream))) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "Stream");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            None,
            "a recursive single-variant sum stays boxed"
        );

        // A GENERIC single-variant sum stays boxed for now (erasure is monomorphic-only this increment).
        let ast = parse("(module m (type Box (Mk a)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let occ = decl_of(&db, "Box");
        assert_eq!(
            newtype_underlying(&mut db, occ),
            None,
            "a generic single-variant sum stays boxed (monomorphic-only erasure)"
        );
    }

    #[test]
    fn a_generic_sum_monomorphizes_at_a_concrete_instantiation() {
        // A GENERIC sum `(type Option (Some a) None)` has an implicit type parameter `a` (a free
        // lowercase name in a payload). `(Option Int64)` in type position APPLIES the sum constructor,
        // monomorphizing to `Ty::Sum { args: [Int64] }`; `(Option Bool)` to `[Bool]`. The two are
        // DISTINCT types (`type-system.md §the head Option agrees but the payload does not`), and each
        // renders as `(Option Int64)` / `(Option Bool)`. This is the "generics are type-valued
        // parameters" model — no new mechanism, the ctor's `(meta apply)` builds the type.
        use crate::db::Db;
        use crate::eval::typeval_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        // Parse a program that USES `(Option Int64)` and `(Option Bool)` in annotations, and reach those
        // type occurrences to reduce them.
        let ast = parse(
            "(module m (type Option (Some a) None) \
               (def (i (: x (Option Int64))) x) \
               (def (b (: y (Option Bool))) y) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        // The declaration recorded its implicit param `a`.
        let decl = db.type_decls.iter().find(|t| t.name == "Option").unwrap();
        assert_eq!(
            decl.params,
            vec!["a".to_string()],
            "implicit param a scanned"
        );
        // Locate the `(Option Int64)` type occurrence — the annotation type of `i`'s parameter. Rather
        // than dig the AST, build the type expression directly: `Option` applied to `Int64`.
        // (Reaching it via the annotation is fiddly; instead confirm the two instantiations differ by
        // reducing `Option` bare then comparing rendered instantiations produced by the escape/infer
        // path in the run tests below. Here we assert the SCAN + the type identity via encode/decode.)
        let opt_int = Ty::Sum {
            decl: decl.occ,
            name: "Option".to_string(),
            args: vec![Ty::int64()],
        };
        let opt_bool = Ty::Sum {
            decl: decl.occ,
            name: "Option".to_string(),
            args: vec![Ty::Bool],
        };
        assert!(
            !opt_int.agrees_with(&opt_bool),
            "Option Int64 ≠ Option Bool"
        );
        assert!(opt_int.agrees_with(&opt_int));
        assert_eq!(opt_int.render_name(), "(Option Int64)");
        assert_eq!(opt_bool.render_name(), "(Option Bool)");
        // The generic sum record IS applyable in type position (has `(meta apply)` = sum-ctor), so
        // `typeval_of` of a `(Option Int64)` application reduces to the monomorphized `Ty::Sum`.
        let option_rec = db.type_decl_by_name("Option").expect("Option bound");
        let int64 = db.push_name("Int64");
        let app = db.push_list(vec![option_rec, int64]);
        match typeval_of(&mut db, app) {
            Some(Ty::Sum { args, .. }) => {
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].render_name(), "Int64", "Option applied to Int64");
            }
            other => panic!(
                "expected (Option Int64) to reduce to Ty::Sum, got {:?}",
                other.map(|t| t.render_name())
            ),
        }
        // NESTED: `(Option (Option Int64))` reduces to a `Ty::Sum` whose arg is ITSELF a `Ty::Sum`.
        // `typeval_of` on the SumCtor application returns the `Ty` DIRECTLY (no arena encode→decode
        // round-trip — that round-trip made a deeply-nested generic annotation O(N²); see
        // `eval::reduce_sum_ctor`). The nested structure and rendering must survive that direct path.
        let option_rec2 = db.type_decl_by_name("Option").expect("Option bound");
        let inner = db.push_list(vec![option_rec2, int64]); // (Option Int64)
        let option_rec3 = db.type_decl_by_name("Option").expect("Option bound");
        let outer = db.push_list(vec![option_rec3, inner]); // (Option (Option Int64))
        match typeval_of(&mut db, outer) {
            Some(t @ Ty::Sum { .. }) => {
                assert_eq!(
                    t.render_name(),
                    "(Option (Option Int64))",
                    "nested generic sum resolves through the direct (round-trip-free) path"
                );
                let Ty::Sum { args, .. } = &t else {
                    unreachable!()
                };
                assert!(
                    matches!(&args[0], Ty::Sum { .. }),
                    "the outer arg is itself a Ty::Sum"
                );
            }
            other => panic!(
                "expected (Option (Option Int64)) to reduce to a nested Ty::Sum, got {:?}",
                other.map(|t| t.render_name())
            ),
        }
    }

    #[test]
    fn a_generic_variant_constructor_infers_its_instantiation() {
        // G2: a GENERIC variant ctor `Some` of `(type Option (Some a) None)` has `(meta t)` =
        // `(fn (a) (-> a (Option a)))` — a scheme `∀a. a → Option a`. So `(Option.Some 5)` INFERS the
        // instantiation `Ty::Sum { args: [Int64] }` through the ordinary `apply_type` (instantiate +
        // unify a = Int64), not a monomorphic `(-> a Sum)`. `(Option.Some true)` infers `Option Bool`.
        use crate::db::Db;
        use crate::infer::type_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        let ast = parse(
            "(module m (type Option (Some a) None) \
               (def (i) (Option.Some 5)) \
               (def (b) (Option.Some true)) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let i_body = db
            .defs
            .iter()
            .find(|d| d.name == "i")
            .and_then(|d| d.body)
            .expect("i");
        match type_of(&mut db, i_body) {
            Ty::Sum { name, args, .. } => {
                assert_eq!(name, "Option");
                assert_eq!(args.len(), 1);
                assert_eq!(
                    args[0].render_name(),
                    "Int64",
                    "(Some 5) infers Option Int64"
                );
            }
            other => panic!("expected Ty::Sum, got {}", other.render_name()),
        }
        let b_body = db
            .defs
            .iter()
            .find(|d| d.name == "b")
            .and_then(|d| d.body)
            .expect("b");
        match type_of(&mut db, b_body) {
            Ty::Sum { args, .. } => assert_eq!(
                args[0].render_name(),
                "Bool",
                "(Some true) infers Option Bool"
            ),
            other => panic!("expected Ty::Sum, got {}", other.render_name()),
        }
    }

    #[test]
    fn a_generic_sum_matches_and_runs_at_a_concrete_instantiation() {
        // The G2 end-to-end: a GENERIC `Option` built + matched at a concrete payload, composed + run.
        // `(pick n)` builds `(Option.Some n)` / `Option.None` (an `Option Int64` by inference) and
        // matches it — `(Some x) → x`, `None → -1`. So `pick 7` → 7 (reads the Int64 payload via
        // `sum-payload`), `pick 0` → -1. Exercises a generic sum's construction + disc dispatch + payload
        // binding at a concrete instantiation, all through the ordinary machinery.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some a) None) \
                     (def (pick (: n Int64)) \
                        (match (if (> n 0) (Option.Some n) Option.None) \
                          ((Option.Some x) x) (Option.None -1))) \
                     (export pick))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile generic sum match");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime generic-sum match imports the value-heap runtime"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed generic-sum-match run");
            return;
        };
        for (arg, want) in [("7", "7"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("pick".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "generic pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed generic-sum-match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_generic_sum_escapes_to_the_host_at_a_concrete_instantiation() {
        // A GENERIC `Option` built at `Int64` and RETURNED across the host boundary renders the
        // parameterized type surface `(Option Int64)` (§158, the corpus form), driven by the sum's solved
        // `Ty::Sum{args:[Int64]}`. Here the program DECLARES its own `(type Option (Some a) None)`; its
        // variant renders as its BARE name `Some` — uniformly with a built-in sum (the value form does
        // not depend on built-in-vs-user). The escape walker's per-variant template reads the concrete
        // payload type (`Int64`) from the instantiation, so the `Some` arm holds a real Int64 hole.
        // Composed + run through `cdz-run` (the resource shape, `export: None`).
        use crate::testkit::parse;
        let src = "(module m (type Option (Some a) None) \
                     (def (main) (Option.Some 5)) (export main))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile generic escape");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed generic-escape run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (Some 5) (Option Int64))",
                    "a user-declared generic sum escape renders the instantiation with the bare variant name"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("composed generic-escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_generic_recursive_sum_bare_self_reference_carries_its_params() {
        // A GENERIC recursive sum whose self-reference is written BARE — `(type Tree (Leaf a) (Branch
        // (Tuple Tree Tree)))` — must treat the bare `Tree` in the payload as the sum applied to its OWN
        // params (`(Tree a)`), the same convention the constructor's RESULT type uses. Before the fix a
        // bare self-reference reduced to the args-LESS `Ty::Sum{args:[]}`, which did not unify with the
        // `(Tree Int64)` a `Branch` value carries → `cannot unify Tree with (Tree Int64)`. The param is
        // annotated (recursive-sum-parameter instantiation inference is a separate increment). `sm` folds
        // the tree to 12. Also covers a self-reference NESTED in a `List` payload (rose tree).
        use crate::testkit::parse;
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping generic-recursive-sum run");
            return;
        };
        for (label, src, want) in [
            (
                "bare self-ref in a tuple payload",
                "(module m (type Tree (Leaf a) (Branch (Tuple Tree Tree))) \
                   (def (sm (: t (Tree Int64))) \
                     (match t ((Tree.Leaf n) n) ((Tree.Branch (tuple l r)) (+ (sm l) (sm r))))) \
                   (def (main) \
                     (sm (Tree.Branch (tuple (Tree.Leaf 3) \
                                             (Tree.Branch (tuple (Tree.Leaf 4) (Tree.Leaf 5))))))) \
                   (export main))",
                "12",
            ),
            (
                "bare self-ref nested in a list payload",
                "(module m (type Rose (RLeaf a) (RNode (List Rose))) \
                   (def (sz (: t (Rose Int64))) \
                     (match t ((Rose.RLeaf n) n) ((Rose.RNode xs) (List.len xs)))) \
                   (def (main) (sz (Rose.RNode (list (Rose.RLeaf 1) (Rose.RLeaf 2) (Rose.RLeaf 3))))) \
                   (export main))",
                "3",
            ),
        ] {
            let bytes = compile_component(&crate::codec::encode(&parse(src)))
                .unwrap_or_else(|e| panic!("compile generic-recursive-sum ({label}): {e:?}"));
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).unwrap_or_else(|e| panic!("run ({label}): {e:?}")) {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "{label}"),
                cdz_run::Outcome::Trap(t) => panic!("generic-recursive-sum trapped ({label}): {t}"),
            }
        }
    }

    #[test]
    fn bare_prelude_option_needs_no_declaration() {
        // `Option`/`Some`/`None` are BUILT IN (prelude sums), so a program uses bare `Some`/`None` with
        // NO `(type Option …)` — the corpus surface. `(Some 5)` infers `Option Int64`; a bare `None` is
        // an `Option` at an unconstrained payload (a var until something fixes it).
        use crate::db::Db;
        use crate::infer::type_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        let ast = parse("(module m (def (s) (Some 5)) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let s_body = db
            .defs
            .iter()
            .find(|d| d.name == "s")
            .and_then(|d| d.body)
            .expect("s");
        match type_of(&mut db, s_body) {
            Ty::Sum { name, args, .. } => {
                assert_eq!(name, "Option", "bare Some builds the prelude Option");
                assert_eq!(
                    args[0].render_name(),
                    "Int64",
                    "(Some 5) infers Option Int64"
                );
            }
            other => panic!("expected Ty::Sum Option, got {}", other.render_name()),
        }
    }

    #[test]
    fn bare_prelude_option_matches_and_runs() {
        // The prelude Option built + matched with BARE `Some`/`None` (no declaration), composed + run.
        // `(pick n)` → `(Some n)` / `None`, matched `(Some x) → x`, `None → -1`. `pick 7`→7, `pick 0`→-1.
        use crate::testkit::parse;
        let src = "(module m \
                     (def (pick (: n Int64)) \
                        (match (if (> n 0) (Some n) None) ((Some x) x) (None -1))) \
                     (export pick))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile prelude-option match");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed prelude-option run");
            return;
        };
        for (arg, want) in [("7", "7"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("pick".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "prelude pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed prelude-option run trapped: {t}"),
            }
        }
    }

    #[test]
    fn the_prelude_sign_sum_is_built_in() {
        // `Sign` is a BUILT-IN monomorphic prelude sum `(type Sign Neg Zero Pos)` — a program uses
        // `Sign.Pos`/`Sign.Zero`/`Sign.Neg` with NO declaration, and an exhaustive three-way match over
        // it folds through the constant scrutinee. `(Sign.Zero unit)` matched → 0; the other two → 1/-1.
        use crate::testkit::parse;
        for (ctor, want) in [("Sign.Neg", -1), ("Sign.Zero", 0), ("Sign.Pos", 1)] {
            let src = format!(
                "(module m (def (main) (match ({ctor} unit) \
                   ((Sign.Neg _) -1) ((Sign.Zero _) 0) ((Sign.Pos _) 1))) (export main))"
            );
            let bytes = compile_component(&crate::codec::encode(&parse(&src)))
                .expect("compile built-in Sign match");
            let Some(runtime) = super::find_runtime_wasm() else {
                eprintln!("runtime wasm not found; skipping Sign run");
                return;
            };
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => {
                    assert_eq!(s, want.to_string(), "built-in Sign match: {ctor}")
                }
                cdz_run::Outcome::Trap(t) => panic!("Sign match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn compare_yields_the_three_way_ordering() {
        // `compare : ∀a. a → a → Ordering` — the three-way comparison (core-semantics.md §A Total Order
        // Is Observed Through A Three-Way Comparison). A constant scalar/string pair FOLDS to the matching
        // `Ordering` variant; deconstructed by a three-arm match → -1/0/1. Covers int, string, and the
        // agreement with `<` (all three relations across two operand types).
        for (prog, want) in [
            ("(compare 1 2)", -1),         // Less
            ("(compare 2 2)", 0),          // Equal
            ("(compare 3 2)", 1),          // Greater
            ("(compare \"a\" \"b\")", -1), // strings order lexicographically
            ("(compare \"b\" \"b\")", 0),
        ] {
            let src = format!(
                "(module m (def (main) (match {prog} \
                   ((Ordering.Less _) -1) ((Ordering.Equal _) 0) ((Ordering.Greater _) 1))) (export main))"
            );
            let bytes = compile_component(&crate::codec::encode(&parse(&src)))
                .expect("compile compare match");
            let Some(runtime) = super::find_runtime_wasm() else {
                eprintln!("runtime wasm not found; skipping compare run");
                return;
            };
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => {
                    assert_eq!(s, want.to_string(), "compare fold: {prog}")
                }
                cdz_run::Outcome::Trap(t) => panic!("compare run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_user_type_shadows_a_prelude_sum_name() {
        // A user `(type Option …)` SHADOWS the built-in Option — top-level `type_decls` resolve before
        // the prelude. So `Option` in a program that declares it is the USER sum (its own declaration
        // occurrence), distinct from the prelude one. Pins that the built-ins do not privilege the name.
        use crate::db::Db;
        use crate::eval::typeval_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        let ast = parse("(module m (type Option (Only Int64) Nope) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        // The USER Option is found first (first-wins), and it has the user's variants (Only/Nope), not
        // the prelude's (Some/None).
        let occ = db.type_decl_by_name("Option").expect("Option");
        let user_occ = db
            .type_decls
            .iter()
            .find(|t| t.name == "Option")
            .unwrap()
            .occ;
        assert_eq!(
            occ,
            db.type_decls
                .iter()
                .find(|t| t.name == "Option")
                .unwrap()
                .synth
                .unwrap()
        );
        let ty = typeval_of(&mut db, occ).expect("Option is a type");
        match ty {
            Ty::Sum { decl, .. } => {
                assert_eq!(decl, user_occ, "the user Option shadows the prelude one")
            }
            other => panic!("expected Ty::Sum, got {}", other.render_name()),
        }
    }

    #[test]
    fn a_variant_constructor_is_a_member_typed_as_a_function_to_the_sum() {
        // `Option.Some` is ORDINARY member access on the sum record (the `Int64.max` path), reaching a
        // variant-constructor record whose `(meta t)` is `(-> Int64 Option)`. So `Some` types as a
        // FUNCTION from its payload to the sum — read by the same `scheme_of`/`apply_type` machinery
        // every operator uses, no per-variant rule. (Applying it to actually CONSTRUCT is a later tick;
        // this pins that the constructor's TYPE is right.)
        use crate::db::Db;
        use crate::eval::{project_field, scheme_of};
        use crate::resolved::Symbol;
        use crate::testkit::parse;
        use crate::ty::Ty;
        use crate::unify::Fresh;
        let ast = parse("(module m (type Option (Some Int64) None) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        let option = db.type_decl_by_name("Option").expect("Option bound");
        // Project the `Some` field off the sum record (the member-access reduction).
        let some = project_field(&mut db, option, &Symbol::plain("Some")).expect("Some field");
        // Its `(meta t)` scheme is `(-> Int64 Option)`.
        let mut fresh = Fresh::new();
        let scheme = scheme_of(&mut db, some, &mut fresh).expect("Some has a type");
        match scheme.ty {
            Ty::Fn(param, result) => {
                assert_eq!(param.render_name(), "Int64");
                assert_eq!(result.render_name(), "Option");
                assert!(matches!(*result, Ty::Sum { .. }));
            }
            other => panic!("expected (-> Int64 Option), got {}", other.render_name()),
        }
        // A NULLARY variant `None` types as the sum directly (no arrow).
        let none = project_field(&mut db, option, &Symbol::plain("None")).expect("None field");
        let none_scheme = scheme_of(&mut db, none, &mut fresh).expect("None has a type");
        assert!(matches!(none_scheme.ty, Ty::Sum { .. }));
        assert_eq!(none_scheme.ty.render_name(), "Option");
    }

    #[test]
    fn two_same_named_sums_from_different_declarations_are_distinct_types() {
        // A nominal sum's identity is its DECLARATION occurrence, NOT its local name (`type-system.md
        // §160`: two nominal types are distinct whenever their FQNs differ, even with identical name +
        // structure). Two `(type Foo …)` declarations — as two modules would produce, or an import
        // would splice — carry distinct `TypeDecl.occ`, so their `Ty::Sum` do NOT agree. This is the
        // property no name-keyed map could hold (the second would clobber the first); we get it for free
        // by keying identity on the occurrence and resolving through the occurrence-keyed `type_decls`.
        use crate::db::Db;
        use crate::eval::typeval_of;
        use crate::testkit::parse;
        // Two identically-shaped, identically-NAMED sums declared separately (the cross-module case,
        // simulated in one flat program). We reach each by its DECLARATION, not the shadowed name.
        let ast = parse(
            "(module m (type Foo (A Int64)) (type Foo (A Int64)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        // Both declarations are present (nothing was nuked) — the Vec holds two `Foo` entries.
        let foos: Vec<_> = db.type_decls.iter().filter(|t| t.name == "Foo").collect();
        assert_eq!(foos.len(), 2, "both same-named declarations are retained");
        let synth0 = foos[0].synth.expect("synth 0");
        let synth1 = foos[1].synth.expect("synth 1");
        assert_ne!(synth0, synth1, "distinct declarations ⇒ distinct records");
        let ty0 = typeval_of(&mut db, synth0).expect("Foo #0 is a type");
        let ty1 = typeval_of(&mut db, synth1).expect("Foo #1 is a type");
        // Different declarations ⇒ different identity ⇒ do NOT agree, despite identical name + shape.
        assert!(
            !ty0.agrees_with(&ty1),
            "distinct declarations must be distinct types even when name + shape match"
        );
        // A sum agrees with ITSELF (same declaration).
        assert!(ty0.agrees_with(&ty0));
    }

    #[test]
    fn a_variant_construction_lowers_to_sum_new() {
        // `(Some a)` with a RUNTIME payload `a` (a parameter, so it cannot fold) lowers to
        // `Core::SumNew { disc: 0, payloads: [a] }` — the variant application dispatched by the ctor's
        // `(meta apply)` = `sum-new`, its discriminant read off `(meta variant)`. A NULLARY `None` used
        // bare lowers to `Core::SumNew { disc: 1, payloads: [] }` (the sum's second variant). This pins
        // that construction routes to the heap builder without needing escape/match (later ticks).
        use crate::core::Core;
        use crate::db::Db;
        use crate::lower::core_of;
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                   (def (mk (: a Int64)) (Option.Some a)) \
                   (def (none) Option.None) \
                   (def (main) 0) (export main))";
        let mut db = Db::load(parse(src));
        let mk_body = db
            .defs
            .iter()
            .find(|d| d.name == "mk")
            .and_then(|d| d.body)
            .expect("mk");
        match core_of(&mut db, mk_body) {
            Core::SumNew { disc, payloads } => {
                assert_eq!(disc, 0, "Some is variant 0");
                assert_eq!(payloads.len(), 1, "Some carries one payload");
            }
            other => panic!("expected Core::SumNew, got {other:?}"),
        }
        let none_body = db
            .defs
            .iter()
            .find(|d| d.name == "none")
            .and_then(|d| d.body)
            .expect("none");
        match core_of(&mut db, none_body) {
            Core::SumNew { disc, payloads } => {
                assert_eq!(disc, 1, "None is variant 1");
                assert!(payloads.is_empty(), "None is nullary");
            }
            other => panic!("expected Core::SumNew for None, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_user_variant_name_resolves_like_the_qualified_form() {
        // A USER `(type …)` sum's variant may be referenced BARE — `NLit`/`NNil`, not only the qualified
        // `(. Node NLit)` (`core-semantics.md` §A Sum Type Constructor Is A Single-Arity Function). A bare
        // reference resolves to the SAME ctor field the qualified member access projects, so it lowers to
        // the same `Core::SumNew`. This is the user-declaration analog of the built-in sums binding bare
        // `Some`/`None` — pinned by comparing the bare form's disc against the qualified form's.
        use crate::core::Core;
        use crate::db::Db;
        use crate::lower::core_of;
        use crate::testkit::parse;
        let src = "(module m (type Node (NLit Int64) NNil) \
                   (def (bare-lit (: a Int64)) (NLit a)) \
                   (def (qual-lit (: a Int64)) (Node.NLit a)) \
                   (def (bare-nil) NNil) \
                   (def (qual-nil) Node.NNil) \
                   (def (main) 0) (export main))";
        let mut db = Db::load(parse(src));
        let disc_of = |db: &mut Db, name: &str| -> u32 {
            let body = db
                .defs
                .iter()
                .find(|d| d.name == name)
                .and_then(|d| d.body)
                .unwrap_or_else(|| panic!("def {name}"));
            match core_of(db, body) {
                Core::SumNew { disc, .. } => disc,
                other => panic!("expected Core::SumNew for {name}, got {other:?}"),
            }
        };
        // The BARE payload variant and the QUALIFIED one lower to the same discriminant (0).
        assert_eq!(disc_of(&mut db, "bare-lit"), disc_of(&mut db, "qual-lit"));
        assert_eq!(disc_of(&mut db, "bare-lit"), 0, "NLit is variant 0");
        // The BARE nullary variant and the QUALIFIED one likewise (disc 1).
        assert_eq!(disc_of(&mut db, "bare-nil"), disc_of(&mut db, "qual-nil"));
        assert_eq!(disc_of(&mut db, "bare-nil"), 1, "NNil is variant 1");
    }

    #[test]
    fn a_bare_nullary_variant_constructs_and_matches_as_a_value() {
        // 05-compound-types "a bare nullary constructor is the nullary sum value": a nullary variant used
        // as a VALUE may be written bare (`NNil`), equivalent to `(Node.NNil unit)`. `(classify 0)` builds
        // `NNil` bare and `(classify 7)` builds `(Node.NLit 7)`; `val` matches both → 1 + 7 = 8. Pins that
        // the bare nullary form both CONSTRUCTS and MATCHES for a user sum, end-to-end through a run.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (type Node (NLit Int64) NNil) \
                       (def (classify n) (if (= n 0) NNil (Node.NLit n))) \
                       (def (val x) (match x ((Node.NLit v) v) ((Node.NNil _) 1))) \
                       (def (main) (+ (val (classify 0)) (val (classify 7)))) (export main))"
                )))
                .expect("a bare nullary user variant must construct + match"),
                "main"
            ),
            8
        );
    }

    #[test]
    fn a_bare_variant_is_shadowed_by_a_same_named_binding() {
        // Scope-first resolution: a bare variant name resolves AFTER the lexical scope + top-level defs
        // (step 3c, before the prelude), so a `def`/`let`/param of the same name SHADOWS the variant. A
        // top-level `(def (NLit x) …)` named like the `Node` variant `NLit` wins over the ctor: `(NLit 5)`
        // calls the def (→ 105), not the constructor. Pins that the bare-variant binding does not privilege
        // a variant name over an ordinary binding.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (type Node (NLit Int64) NNil) \
                       (def (NLit x) (+ x 100)) \
                       (def (main) (NLit 5)) (export main))"
                )))
                .expect("a def named like a variant shadows the bare variant"),
                "main"
            ),
            105
        );
    }

    #[test]
    fn a_variant_construction_emits_the_heap_build_ops() {
        // Construction lowers to the right value-heap OPS — `collect_used_ops` reports exactly what
        // `emit` lays down (they must agree, or the import section omits a called op). A single-payload
        // `(Some a)` uses `sum-new` + `box-int` (box the Int64 payload); a nullary `None` uses `sum-new`
        // ALONE — its unit payload is the inline-unit CONSTANT (`IMM_UNIT`), so it imports no `arr-alloc`.
        // This proves construction reaches the heap builder without needing the sum to escape (next tick)
        // or a composed run (a dead sum folds away — a sum is only observable once it escapes or matched).
        use crate::backend::wasm::select::collect_used_ops;
        use crate::db::Db;
        use crate::testkit::parse;
        // A payload variant: `sum-new` + `box-int`.
        let src = "(module m (type Option (Some Int64) None) \
                     (def (mk (: a Int64)) (Option.Some a)) (export mk))";
        let mut db = Db::load(parse(src));
        let mk = db
            .defs
            .iter()
            .find(|d| d.name == "mk")
            .and_then(|d| d.body)
            .expect("mk");
        let mut ops = std::collections::BTreeSet::new();
        collect_used_ops(&mut db, mk, &mut ops);
        assert!(ops.contains("sum-new"), "Some emits sum-new; got {ops:?}");
        assert!(
            ops.contains("box-int"),
            "Some boxes its Int64 payload; got {ops:?}"
        );
        // A nullary variant: `sum-new` with the INLINE-UNIT CONSTANT payload — so it does NOT import
        // `arr-alloc`. `arr-alloc(0)` returns the inline unit (`runtime_abi::IMM_UNIT`), so the compiler
        // pushes that constant directly instead of calling `arr-alloc(0)` — a nullary construction needs
        // only `sum-new`, no per-payload heap op.
        let src2 = "(module m (type Option (Some Int64) None) \
                      (def (none) Option.None) (export none))";
        let mut db2 = Db::load(parse(src2));
        let none = db2
            .defs
            .iter()
            .find(|d| d.name == "none")
            .and_then(|d| d.body)
            .expect("none");
        let mut ops2 = std::collections::BTreeSet::new();
        collect_used_ops(&mut db2, none, &mut ops2);
        assert!(ops2.contains("sum-new"), "None emits sum-new; got {ops2:?}");
        assert!(
            !ops2.contains("arr-alloc"),
            "None's unit payload is the inline-unit constant, no arr-alloc; got {ops2:?}"
        );
    }

    #[test]
    fn a_nullary_variant_pushes_the_inline_unit_constant_not_an_arr_alloc_call() {
        // The nullary-variant unit payload is emitted as the `IMM_UNIT` constant (derived from the
        // runtime's `cdz-abi` section), NOT a runtime `arr-alloc(0)` call. Asserted at the Lir level: the
        // construction contains a `ConstI32(IMM_UNIT)` and NO `CallImport("arr-alloc")`. And it runs
        // correctly (a `Sign` classify over the three nullary variants) — the constant IS the handle
        // `arr-alloc(0)` would have returned, so `sum-new`/`sum-disc` see the same value.
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::runtime_abi::IMM_UNIT;
        use crate::db::Db;
        let ast = crate::testkit::parse(
            "(module m (type Sign Neg Zero Pos) \
               (def (f (: n Int64)) \
                  (match (if (< n 0) (Sign.Neg unit) (if (= n 0) (Sign.Zero unit) (Sign.Pos unit))) \
                    ((Sign.Neg _) -1) ((Sign.Zero _) 0) ((Sign.Pos _) 1))) \
               (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
        let sig = db.defs[d].params.clone();
        let params: Vec<_> = sig
            .into_iter()
            .map(|p| {
                let b = db
                    .ast
                    .as_form(p, ":")
                    .and_then(|t| t.first().copied())
                    .unwrap_or(p);
                (b, crate::infer::type_of(&mut db, b))
            })
            .collect();
        let body = db.defs[d].body.expect("body");
        let code = crate::backend::wasm::select::select_function(&mut db, body, &params, &layout)
            .expect("select")
            .code;
        assert!(
            code.iter()
                .any(|i| matches!(i, Lir::ConstI32(v) if *v == IMM_UNIT as i32)),
            "a nullary variant pushes the inline-unit constant, got: {code:?}"
        );
        assert!(
            !code
                .iter()
                .any(|i| matches!(i, Lir::CallImport("arr-alloc"))),
            "no arr-alloc call for a nullary variant's unit payload, got: {code:?}"
        );
    }

    #[test]
    fn a_parameterized_sum_returning_export_declines() {
        // A sum crosses the host boundary ONLY as a single NULLARY export's result (the resource escape
        // path, `emit`). A PARAMETERIZED sum-returning export (`mk` takes `a`) is not that shape — it has
        // no scalar boundary form — so it DECLINES cleanly (not a miscompile). The nullary escape is
        // exercised by `a_nullary_sum_export_escapes_to_the_host` below.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (mk (: a Int64)) (Option.Some a)) (export mk))";
        let result = compile_component(&crate::codec::encode(&parse(src)));
        assert!(
            result.is_err(),
            "a parameterized sum-returning export must decline (not the nullary escape shape)"
        );
    }

    #[test]
    fn a_nullary_sum_export_escapes_to_the_host() {
        // A single NULLARY export returning a user sum crosses as a resource. `(Option.Some 5)` over a
        // USER `(type Option (Some Int64) None)` is a COMPILE-TIME CONSTANT, so its canonical bytes are
        // baked (the `const_value_ast` `SumNew` arm) and NO value-heap runtime is imported. The variant
        // renders as its BARE name — `Some` — uniformly with a built-in sum (the value form does not
        // depend on built-in-vs-user). `(Option.Some 5)` → `(: (Some 5) Option)`. The RUNTIME disc-switch
        // encoder (a sum built from a non-constant payload) is exercised separately by
        // `a_runtime_sum_export_escapes_via_the_heap_walk`. Composed + run through `cdz-run`.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (main) (Option.Some 5)) (export main))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile a sum escape");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_none(),
            "a CONSTANT sum escape bakes its bytes — no value-heap runtime import"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!(
                "runtime wasm not found (run `cargo xtask build`); skipping composed sum-escape run"
            );
            return;
        };
        // `export: None` — a sum escape is a RESOURCE component (`make`/`encode`), auto-detected by
        // `cdz_run`, not a bare function export.
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (Some 5) Option)",
                    "sum escape renders the bare variant name"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("composed sum-escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_runtime_sum_export_escapes_via_the_heap_walk() {
        // The R2 runtime sum escape: a single NULLARY export whose returned sum is built from a NON-CONSTANT
        // payload (so it cannot fold to baked bytes) crosses as a resource whose `encode()` SWITCHES on
        // `sum-disc` and WALKS the live handle. Here `pick` recurses (so the fold cannot inline it) and its
        // `Some` payload is a genuine runtime value; the walker builds the sum on the heap (`sum-new`),
        // reads its disc, walks `sum-payload` → `get-int`, and renders `(: (Some 3) (Option Int64))` — the
        // bare built-in `Some` (no user declaration). Verifies the runtime disc-switch encoder, the
        // companion of the constant-bake path in `a_nullary_sum_export_escapes_to_the_host`.
        use crate::testkit::parse;
        let src = "(module m (def (down n) (if (< n 1) (Some 0) (down (- n 1)))) \
                     (def (main) (down 2)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile runtime sum escape");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a RUNTIME sum escape imports the value-heap runtime (the disc-switch heap walk)"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (Some 0) (Option Int64))",
                    "runtime sum escape walks the heap and renders the variant"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("composed runtime-sum-escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_nullary_variant_export_escapes_as_unit_payload() {
        // The nullary arm: `Option.None` over a USER `(type Option …)` renders `(: (None unit) Option)` —
        // a nullary variant carries the unit value, and the variant renders as its BARE name `None`
        // (uniform with a built-in sum). The value is a compile-time constant, so its bytes are baked (no
        // runtime import); the `None` template has NO holes (the `unit` payload is static).
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (main) Option.None) (export main))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile None escape");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed None-escape run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: None,
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (None unit) Option)",
                    "nullary variant escape renders the bare variant name"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("composed None-escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_variant_with_a_record_payload_whose_field_is_lowercase_is_not_generic() {
        // REGRESSION: a variant carrying a `(Record (field Type)…)` payload whose field NAME is lowercase
        // (`(Pt (Record (x Int64) (y Int64)))`) must NOT be mistaken for a generic sum. `collect_type_params`
        // scanned the payload for free lowercase names as implicit type parameters — and wrongly picked up
        // the record FIELD NAMES `x`/`y`, making `P` spuriously generic over them; the ctor arrow then read
        // its payload as an unresolvable variable, so `P.Pt` looked NULLARY and rejected the construction
        // (CDZ0201 "a nullary variant takes the unit value"). A field name is a LABEL, not a type expr — the
        // fix descends only into each field pair's TYPE. The bug was a compile-time REJECT, so COMPILING the
        // construction (it no longer faults) is the precise guard; the run is exercised by the corpus case
        // "a variant carrying a RECORD payload constructs and matches" (which links the value-heap runtime).
        let src = "(module m (type P (Pt (Record (x Int64) (y Int64))) O) \
                     (def (sum r) (+ (. r x) (. r y))) \
                     (def (main) (match (P.Pt (record (x 3) (y 4))) ((P.Pt r) (sum r)) (P.O 0))) \
                     (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_ok(),
            "a record-payload variant with lowercase field names must COMPILE — not reject P.Pt as \
             nullary (the field names must not be collected as type parameters)"
        );
    }

    #[test]
    fn a_constant_sum_match_folds_to_the_selected_arm() {
        // A match over a CONSTANT sum folds at compile time to the arm whose variant it is — no runtime
        // dispatch, like a constant scalar match. `(match (Some 5) ((Some x) x) (None 0))` selects the
        // `Some` arm and yields its payload binding `x` = 5; `(match None …)` selects the `None` arm → 0.
        let some = "(module m (type Option (Some Int64) None) \
                     (def (main) (match (Option.Some 5) ((Option.Some x) x) (Option.None 0))) \
                     (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(some))).expect("compile"),
                "main"
            ),
            5
        );
        let none = "(module m (type Option (Some Int64) None) \
                     (def (main) (match Option.None ((Option.Some x) x) (Option.None 0))) \
                     (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(none))).expect("compile"),
                "main"
            ),
            0
        );
    }

    #[test]
    fn a_record_scrutinee_is_bound_whole_by_a_match_binder() {
        // A `match` on a RECORD scrutinee binds the WHOLE record with a bare-binder arm — a record is not
        // pattern-destructured field-by-field (core-semantics.md §Patterns Compose lists tuple + ctor
        // patterns, not record patterns; a record is read by `(. r field)` projection), so its only
        // patterns are a whole-value binder or a wildcard. Was declined "matching a compound value needs a
        // heap walk"; now a `Ty::Record` scrutinee routes through the decision-tree matcher like a
        // tuple/sum. `(match (record (x 3) (y 4)) (r (+ (. r x) (. r y))))` binds `r` and projects → 7. A
        // wildcard arm ignores it → 99. A CONSTANT record folds, so this runs via run_returns.
        let bind = "(module m (def (main) \
                      (match (record (x 3) (y 4)) (r (+ (. r x) (. r y))))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(bind))).expect("compile"),
                "main"
            ),
            7,
            "a record scrutinee bound whole projects its fields"
        );
        let wild = "(module m (def (main) (match (record (x 3) (y 4)) (_ 99))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(wild))).expect("compile"),
                "main"
            ),
            99,
            "a wildcard arm over a record scrutinee ignores it"
        );
    }

    #[test]
    fn a_tuple_scrutinee_is_matched_by_a_tuple_pattern() {
        // Matching directly on a TUPLE scrutinee — `(match (tuple a b) ((tuple x y) …))` — was declined "a
        // match pattern that is not a scalar literal or `_`"; now a `Ty::Tuple` scrutinee routes through the
        // decision-tree matcher (`Elem`-path binders + literal tests, no discriminant). A CONSTANT tuple
        // folds: `(tuple 3 4)` destructures to x=3,y=4 → 7. A tuple of SUMS matches with nested constructor
        // patterns (the structural-editing idiom `(match (tuple a b) ((tuple (E.Lit x) (E.Lit y)) …))`).
        let sum = "(module m (def (main) (match (tuple 3 4) ((tuple x y) (+ x y)))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(sum))).expect("compile"),
                "main"
            ),
            7,
            "a tuple scrutinee destructures by a tuple pattern"
        );
        // A LITERAL tuple element refines the match; a non-match falls to the binder arm.
        let lit_hit = "(module m (def (main) \
                        (match (tuple 0 9) ((tuple 0 y) 100) ((tuple x y) x))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(lit_hit))).expect("compile"),
                "main"
            ),
            100,
            "a matching tuple-element literal selects its arm"
        );
        let lit_miss = "(module m (def (main) \
                         (match (tuple 5 9) ((tuple 0 y) 100) ((tuple x y) x))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(lit_miss))).expect("compile"),
                "main"
            ),
            5,
            "a non-matching tuple-element literal falls through to the binder arm"
        );
        // A wrong-TYPE literal element (`true` where the element is Int64) is CDZ0201; a repeated binder
        // (`(tuple x x)`) is CDZ0102 (linearity) — both checked in the tuple/nested pattern path, not only
        // at the top level.
        let wrong_ty = "(module m (def (main) \
                         (match (tuple 1 2) ((tuple true b) 9) (_ 0))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(wrong_ty)))
                .expect_err("wrong-type literal element must reject")
                .message
                .contains("does not match"),
            "a wrong-type tuple-element literal is a type error (CDZ0201)"
        );
        let dup = "(module m (def (main) (match (tuple 1 2) ((tuple x x) x))) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(dup)))
                .expect_err("a repeated binder must reject")
                .message
                .contains("more than once"),
            "a tuple pattern binding the same name twice is non-linear (CDZ0102)"
        );
    }

    #[test]
    fn a_literal_payload_pattern_refines_a_sum_match() {
        // A variant pattern whose payload is a LITERAL — `(Some 0)` — matches the variant carrying EXACTLY
        // that value, falling through to a same-variant binder `(Some k)` otherwise (core-semantics.md
        // §Pattern Matching: "the literal refines the match"). Was declined "a malformed sum match
        // pattern"; now a `SumCont::LitTest` at the payload path tests the scalar and threads the
        // non-matching case to the fall-through. CONSTANT-scrutinee folds (checked here via run_returns):
        // `(Some 0)` selects the literal arm → 100; `(Some 5)` folds past it to the binder → 5. (The
        // runtime-payload path is exercised by corpus 02 "a literal inside a constructor pattern matches a
        // runtime payload"; a nested + Result + bool literal by the sibling corpus cases.)
        let hit = "(module m (def (main) \
                     (match (Some 0) ((Some 0) 100) ((Some k) k) ((None _) -1))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(hit))).expect("compile"),
                "main"
            ),
            100,
            "a matching literal payload selects its arm"
        );
        let miss = "(module m (def (main) \
                      (match (Some 5) ((Some 0) 100) ((Some k) k) ((None _) -1))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(miss))).expect("compile"),
                "main"
            ),
            5,
            "a non-matching literal payload falls through to the same-variant binder"
        );
        // A literal-only arm with no same-variant fall-through is NON-EXHAUSTIVE (the literal does not
        // cover the variant — a `Some` other than 0 is unmatched), CDZ0210 — the same rule a guard follows.
        let non_exhaustive = "(module m (def (f n) \
                               (match (Some n) ((Some 0) 100) ((None _) -1))) \
                             (def (main) (f 5)) (export main))";
        let err = compile_component(&crate::codec::encode(&parse(non_exhaustive)))
            .expect_err("a literal-only Some arm with no binder fall-through must reject");
        assert!(
            err.message.contains("non-exhaustive") || err.message.contains("cover every variant"),
            "a literal payload arm with no same-variant binder fall-through is non-exhaustive; got: {}",
            err.message
        );
    }

    #[test]
    fn a_non_exhaustive_sum_match_is_rejected() {
        // A sum match must cover every variant or end in a wildcard (§190). `(match s ((Some x) x))` —
        // missing the `None` arm, no wildcard — is CDZ0210 (non-exhaustive), not a runtime fallthrough.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (f (: s Int64)) (match (Option.Some s) ((Option.Some x) x))) (export f))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a non-exhaustive sum match must reject");
        assert!(
            err.message.contains("non-exhaustive") || err.message.contains("cover every variant"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn a_non_exhaustive_sum_match_names_the_missing_variants_and_offers_an_add_arms_fix() {
        // The rustc-gold structural suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries
        // A Route To A Fix): a non-exhaustive sum match NAMES the uncovered variant AND carries an
        // INSERT fix that appends a covering arm to the `(match …)` form.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (f (: s Int64)) (match (Option.Some s) ((Option.Some x) x))) (export f))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("non-exhaustive must reject");
        assert_eq!(err.code.as_deref(), Some("CDZ0210"), "got: {}", err.message);
        assert!(
            err.message.contains("`None`") && err.message.contains("not covered"),
            "names the missing variant: {}",
            err.message
        );
        let fix = err.fix.expect("an add-arms fix is carried");
        assert_eq!(fix.kind, crate::abi::FixKind::InsertInto, "it INSERTS arms");
        // `None` is nullary → the synthesized arm is `(None unit)`.
        assert_eq!(fix.replacement, "(None unit)", "the covering arm");
        assert!(!fix.verified, "the arm body is a placeholder → heuristic");
    }

    #[test]
    fn a_non_exhaustive_match_synthesizes_payload_binders_and_lists_multiple_missing() {
        // Two missing variants, one with a PAYLOAD: the fix synthesizes a `_`-prefixed binder per payload
        // (`(Some _p0) unit`) so the arm is well-formed and does not itself warn unused, and the message
        // lists both missing variants.
        use crate::testkit::parse;
        let src = "(module m (type T (A Int64) B C) \
                     (def (f (: t T)) (match t ((A x) x))) (export f))";
        let err = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("non-exhaustive must reject");
        assert_eq!(err.code.as_deref(), Some("CDZ0210"), "got: {}", err.message);
        assert!(
            err.message.contains("`B`") && err.message.contains("`C`"),
            "lists both missing: {}",
            err.message
        );
        let fix = err.fix.expect("an add-arms fix is carried");
        // B and C are nullary; the fix appends both arms, space-joined.
        assert_eq!(fix.replacement, "(B unit) (C unit)", "both covering arms");
    }

    #[test]
    fn a_runtime_sum_match_dispatches_on_the_discriminant() {
        // The RUNTIME sum match, composed + run: a function builds a sum from a runtime param, matches
        // it, and returns a scalar. `(pick n)` builds `(Some n)` when n>0 else `None`, then matches:
        // `(Some x) → x`, `None → -1`. So `pick 5` → 5 (the `Some` arm reads the payload via
        // `sum-payload`), `pick 0` → -1 (the `None` arm). Exercises `sum-disc` dispatch + `sum-payload`
        // binding end-to-end through the composed runtime.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (pick (: n Int64)) \
                        (match (if (> n 0) (Option.Some n) Option.None) \
                          ((Option.Some x) x) (Option.None -1))) \
                     (export pick))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile sum match");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime sum match imports the value-heap runtime"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed sum-match run");
            return;
        };
        for (arg, want) in [("5", "5"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("pick".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed sum-match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_sum_match_disc_zero_probe_uses_eqz_and_dispatches() {
        // A sum match dispatches on `sum-disc(scrutinee) == disc`; when the tested discriminant is 0 —
        // the FIRST declared variant (`Some`), the common first-arm probe — that is `i32.eqz` (opcode
        // 0x45), not `const 0 ; i32.eq`. Verify BOTH the emitted core module carries an `i32.eqz` AND the
        // match still dispatches correctly across the present/absent variants (composed run). `Some`
        // first so its disc-0 probe is the eqz.
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (pick (: n Int64)) \
                        (match (if (> n 0) (Option.Some n) Option.None) \
                          ((Option.Some x) x) (Option.None -1))) \
                     (export pick))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        // The emitted component's embedded core module carries an `i32.eqz` (opcode 0x45) — the disc-0
        // probe. Without the eqz special case the probe would be `i32.const 0` (0x41 0x00) + `i32.eq`
        // (0x46) and no 0x45 would appear (this program has no other eqz — no scalar `== 0`, no bool
        // negation). A whole-component byte scan suffices: the core module is embedded verbatim.
        assert!(
            bytes.contains(&0x45),
            "the disc-0 (Some) probe must emit an i32.eqz (0x45)"
        );
        // And the dispatch is correct: `pick 5` → 5 (Some arm reads the payload), `pick 0` → -1 (None).
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed run");
            return;
        };
        for (arg, want) in [("5", "5"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("pick".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("disc-0-eqz sum-match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_nested_sum_match_is_a_decision_tree() {
        // The Maranget decision tree over MONOMORPHIC nested sums: `Box` wraps an `Inner` (itself a sum),
        // so `(match b ((Full (Pos x)) x) ((Full (Neg y)) y) (Empty -1))` shares the OUTER `Full` probe
        // and only then splits on the INNER `Pos`/`Neg` discriminant — three arms, ONE outer `sum-disc`
        // switch whose `Full` arm recurses into an inner switch, not three re-probes. `(build n)` makes
        // `(Full (Pos n))` for n>0, `(Full (Neg n))` for n<0, `Empty` for 0. Composed + run.
        use crate::testkit::parse;
        let src = "(module m \
                     (type Inner (Pos Int64) (Neg Int64)) \
                     (type Box (Full Inner) Empty) \
                     (def (classify (: b Box)) \
                        (match b \
                          ((Box.Full (Inner.Pos x)) x) \
                          ((Box.Full (Inner.Neg y)) y) \
                          (Box.Empty -1))) \
                     (def (build (: n Int64)) \
                        (classify (if (> n 0) (Box.Full (Inner.Pos n)) \
                                     (if (< n 0) (Box.Full (Inner.Neg n)) Box.Empty)))) \
                     (export build))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile a nested sum match");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a nested runtime sum match imports the value-heap runtime"
        );
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed nested-match run");
            return;
        };
        for (arg, want) in [("9", "9"), ("-3", "-3"), ("0", "-1")] {
            let opts = cdz_run::RunOpts {
                export: Some("build".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "classify {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed nested-match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_three_variant_sum_match_dispatches_via_br_table() {
        // A runtime match over a 3-variant sum dispatches through a BR_TABLE (O(1) jump on the
        // discriminant) instead of a linear `if (disc == k)` chain. `(code n)` picks Red/Green/Blue from
        // a runtime `n` and matches each to a distinct scalar; every variant must dispatch to the right
        // arm (a wrong br_table index or block depth would return the wrong value — hence a run). The
        // Lir-level br_table SHAPE is pinned separately by the `select` unit tests; here we prove the
        // end-to-end dispatch value is correct through the composed runtime.
        use crate::testkit::parse;
        let src = "(module m \
                     (type Color Red Green Blue) \
                     (def (pick (: n Int64)) \
                        (if (= n 0) (Color.Red unit) \
                          (if (= n 1) (Color.Green unit) (Color.Blue unit)))) \
                     (def (code (: n Int64)) \
                        (match (pick n) \
                          ((Color.Red _) 10) ((Color.Green _) 20) ((Color.Blue _) 30))) \
                     (export code))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile 3-variant match");
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed 3-variant run");
            return;
        };
        for (arg, want) in [("0", "10"), ("1", "20"), ("2", "30")] {
            let opts = cdz_run::RunOpts {
                export: Some("code".to_string()),
                args: vec![arg.to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "code {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed 3-variant run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_runtime_value_eq_in_a_loop_condition_does_not_clash_scratch() {
        // REGRESSION: a runtime `value-eq` (or any i32 heap-handle-producing op) in the CONDITION of a
        // tail-recursive function compiled as a wasm LOOP must not reuse a scratch slot the sibling
        // branch's i64 arithmetic uses. `find` compares `(N.I n)` against `(N.I 3)` each iteration and
        // `(find (+ n 1))` iterates; before the fix the compare's i32 handle slot and the `(+ n 1)` i64
        // slot were the SAME number (both allocated from `base`), forcing one wasm local to two types →
        // `func failed to validate: expected i64, found i32`. The fix advances the branches' scratch
        // floor past the condition's high-water. `find(0)` = 3.
        use crate::testkit::parse;
        let src = "(module m \
                     (type N (I Int64) (J Int64)) \
                     (def (mk (: n Int64)) (N.I n)) \
                     (def (find (: n Int64)) (if (= (mk n) (mk 3)) n (find (+ n 1)))) \
                     (export find))";
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .expect("compile value-eq-in-loop-condition");
        // Must be a valid module — the whole point of the regression (a bad module fails HERE at
        // instantiation, before any run).
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping composed value-eq-in-loop run");
            return;
        };
        let opts = cdz_run::RunOpts {
            export: Some("find".to_string()),
            args: vec!["0".to_string()],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run value-eq-in-loop") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(s, "3", "find(0) searches up to n where N.I n = N.I 3")
            }
            cdz_run::Outcome::Trap(t) => panic!("value-eq-in-loop run trapped: {t}"),
        }
    }

    #[test]
    fn a_guarded_wildcard_arm_falling_through_to_a_tail_call_iterates_the_loop() {
        // REGRESSION (two composed fixes): a `match` whose first arm is a GUARDED WILDCARD and whose
        // fall-through arm SELF-TAIL-CALLS, compiled as a wasm LOOP. Two independent pre-existing bugs
        // both broke this:
        //  (1) TAIL DEPTH — a guarded wildcard emits `if <guard> body else <fall-through>` with NO probe
        //      `if` (a wildcard needs no test), yet the emitter counted one, so the fall-through's
        //      self-tail-call `br`'d one level too far, PAST the loop → `expected i64 but nothing on
        //      stack`. Hit even by a SCALAR guard (no heap handle).
        //  (2) BRANCH SCRATCH — when the guard produces an i32 heap handle (`value-eq`/`MatchSum`), its
        //      scratch slot must sit above the fall-through's i64 iteration arithmetic.
        // A scalar guard exercises (1); the value-eq guard exercises (1)+(2). This also covers the two
        // sibling scratch seams a heap-op-in-a-loop hit: a value-eq as the match SCRUTINEE (its handle
        // scratch vs the probe chain's arm-body arith), and a value-eq guard on a LITERAL-probe arm (its
        // handle scratch in the THEN vs the probe-else's iteration arith). All must be valid modules.
        use crate::testkit::parse;
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found; skipping guarded-wildcard-loop run");
            return;
        };
        for (label, src, want) in [
            (
                "scalar guard (tail depth)",
                "(module m (def (find (: n Int64)) \
                   (match n ((guard x (> x 2)) x) (_ (find (+ n 1))))) (export find))",
                "3",
            ),
            (
                "value-eq guard on wildcard (tail depth + scratch)",
                "(module m (type N (I Int64) (J Int64)) (def (mk (: n Int64)) (N.I n)) \
                   (def (find (: n Int64)) \
                     (match n ((guard x (= (mk x) (mk 3))) x) (_ (find (+ n 1))))) (export find))",
                "3",
            ),
            (
                "value-eq as match scrutinee (scrutinee scratch)",
                "(module m (type N (I Int64) (J Int64)) (def (mk (: n Int64)) (N.I n)) \
                   (def (find (: n Int64)) \
                     (match (= (mk n) (mk 3)) (true n) (false (find (+ n 1))))) (export find))",
                "3",
            ),
            (
                "value-eq guard on literal-probe arm (probe-else scratch)",
                "(module m (type N (I Int64) (J Int64)) (def (mk (: n Int64)) (N.I n)) \
                   (def (find (: n Int64)) \
                     (match n ((guard 3 (= (mk n) (mk 3))) 300) (_ (find (+ n 1))))) (export find))",
                "300",
            ),
            (
                "value-eq guard on SUM-match arm, call scrutinee (sum-cont scratch)",
                "(module m (type N (I Int64) (J Int64)) \
                   (def (bump (: n Int64)) (if (< n 0) (N.J n) (N.I n))) \
                   (def (mk (: n Int64)) (N.I n)) \
                   (def (find (: n Int64)) \
                     (match (bump n) ((guard (N.I x) (= (mk x) (mk 3))) x) (_ (find (+ n 1))))) \
                   (export find))",
                "3",
            ),
        ] {
            let bytes = compile_component(&crate::codec::encode(&parse(src)))
                .unwrap_or_else(|e| panic!("compile guarded-wildcard-loop ({label}): {e:?}"));
            let opts = cdz_run::RunOpts {
                export: Some("find".to_string()),
                args: vec!["0".to_string()],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).unwrap_or_else(|e| panic!("run ({label}): {e:?}")) {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "find(0) = {want} ({label})"),
                cdz_run::Outcome::Trap(t) => panic!("guarded-wildcard-loop trapped ({label}): {t}"),
            }
        }
    }

    #[test]
    fn a_variant_carrying_a_bare_nullary_variant_type_checks() {
        // A value carrying a bare nullary variant with an UNCONSTRAINED payload — `(Some (None))`,
        // `(Ok (None))` — must type-check, not trip a spurious occurs-check. Each `apply_type` uses a
        // private `Fresh` from 0, so `Some`'s scheme `(-> ?0 (Option ?0))` and the inner `(None)`'s
        // independently-inferred `(Option ?0)` SHARE variable numbers; unifying the parameter `?0`
        // against the argument `(Option ?0)` gave `?0 = Option ?0` and rejected CDZ0203 "infinite type".
        // Freshening the argument's free vars past the head's counter (`?0` → `?1`) breaks the alias.
        // Consumed by a nested match to a scalar so the value need not cross the host boundary.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (main) \
                       (match (Some (None)) ((Some (Some x)) x) ((Some (None)) 1) ((None) 2))) \
                       (export main))"
                )))
                .expect("(Some (None)) must type-check, not reject as an infinite type"),
                "main"
            ),
            1
        );
        // Result too — the bug spans the generic-sum constructors, not only Option.
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(
                    "(module m (def (main) \
                       (match (Ok (None)) ((Ok (Some x)) x) ((Ok (None)) 1) ((Err e) e))) \
                       (export main))"
                )))
                .expect("(Ok (None)) must type-check"),
                "main"
            ),
            1
        );
        // NO OVER-ACCEPTANCE: the payload is still typed (a nested `Some 5` binds x : Int64, and using it
        // as a Bool condition is still a fault) — freshening only removes the spurious cycle, it does not
        // drop the real constraint.
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (main) \
                   (match (Some (Some 5)) ((Some (Some x)) (if x 1 0)) ((Some (None)) 0) ((None) -1))) \
                   (export main))"
            )))
            .is_err(),
            "an inner payload used as a Bool must still be a type fault"
        );
    }

    #[test]
    fn an_if_over_a_sum_type_checks_regardless_of_branch_order() {
        // A runtime `if` whose branches build the SAME sum two ways — a nullary variant `(None)` :
        // `Option ?0` and a payload variant `(Some n)` : `Option Int64` — must COMPILE in EITHER order.
        // The `if`'s type is the JOIN of its branches; `Ty::join` now joins two agreeing sums' type ARGS
        // pairwise, so the payload-carrying branch fixes the parameter whether it is first or second.
        // Before, a leading `None` took the join's fallthrough and kept `(Option ?0)` with `?0` free, so
        // the value-heap layout declined "projecting a tuple element of type ?0 needs the value heap".
        for branches in ["(None) (Some n)", "(Some n) (None)"] {
            let src = format!(
                "(module m (def (g (: n Int64)) \
                   (match (if (= n 0) {branches}) ((Some k) k) ((None) 0))) \
                 (def (main (: n Int64)) (g n)) (export main))"
            );
            assert!(
                compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
                "an if-built Option must compile with branches in order [{branches}]"
            );
        }
        // The Result companion — two type parameters, resolved from either branch (the sibling finding).
        for branches in ["(Err n) (Ok n)", "(Ok n) (Err n)"] {
            let src = format!(
                "(module m (def (g (: n Int64)) \
                   (match (if (= n 0) {branches}) ((Ok k) k) ((Err e) 0))) \
                 (def (main (: n Int64)) (g n)) (export main))"
            );
            assert!(
                compile_component(&crate::codec::encode(&parse(&src))).is_ok(),
                "an if-built Result must compile with branches in order [{branches}]"
            );
        }
        // NO OVER-ACCEPTANCE: genuinely mismatched branches (a sum vs a bare integer) still reject.
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (g (: n Int64)) (if (> n 0) (Some n) n)) \
                 (def (main (: n Int64)) (g n)) (export main))"
            )))
            .is_err(),
            "an if whose branches are a sum and a scalar must still be a type mismatch"
        );
    }

    #[test]
    fn a_non_exhaustive_nested_sum_match_is_rejected() {
        // A nested match missing the inner `Neg` case with no wildcard is NON-EXHAUSTIVE at the INNER
        // switch (the outer `Full` is reached, but its inner `Inner` leaves `Neg` uncovered). The
        // decision tree checks exhaustiveness at EACH switch, so this is CDZ0210 — not a silent
        // fallthrough. `(Full (Pos x))` + `Empty` leaves the inner `Neg` uncovered.
        use crate::testkit::parse;
        let src = "(module m \
                     (type Inner (Pos Int64) (Neg Int64)) \
                     (type Box (Full Inner) Empty) \
                     (def (classify (: b Box)) \
                        (match b ((Box.Full (Inner.Pos x)) x) (Box.Empty -1))) \
                     (export classify))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("a non-exhaustive nested match must be rejected")
            .message;
        assert!(
            msg.contains("non-exhaustive") || msg.contains("cover every variant"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_nested_sum_match_folds_over_a_constant() {
        // A constant nested scrutinee FOLDS to the selected arm through both switch levels — no runtime
        // match. Over monomorphic `Box`/`Inner`, `(Full (Pos 7))` folds outer `Full` then inner `Pos`,
        // binds `x = 7`, and the whole match reduces to the constant 7 (a plain scalar, no heap sum).
        let src = "(module m \
                     (type Inner (Pos Int64) (Neg Int64)) \
                     (type Box (Full Inner) Empty) \
                     (def (main) \
                        (match (Box.Full (Inner.Pos 7)) \
                          ((Box.Full (Inner.Pos x)) x) \
                          ((Box.Full (Inner.Neg y)) y) \
                          (Box.Empty -1))) \
                     (export main))";
        use crate::testkit::parse;
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main"
            ),
            7
        );
    }

    #[test]
    fn a_wide_sum_match_with_a_default_partitions_by_disc_in_source_order() {
        // A wide sum (6 variants) matched with arms out of declaration order, a DEFAULT (`_`) arm in the
        // MIDDLE, and a SHADOWED later arm — exercises `build_tree`'s single-pass disc partition +
        // `merge_rows` (which fold the per-arm O(V) `specialize`/`variant_ctor_field` scans that made a
        // V-variant match O(V²)). Arm priority is SOURCE order: the first `(T.C …)` arm wins over the
        // later `(T.C z)`, and — critically — a `(T.F …)` arm listed AFTER the `_` default is shadowed by
        // it (the default row has the smaller source index, so `merge_rows` places it first and it wins).
        // So only `C`/`A` (listed before the default) reach their own arm; `B`/`E`/`F` fall to the default.
        // Constant scrutinees fold to the selected arm through the partitioned tree.
        use crate::testkit::parse;
        let ty = "(type T (A Int64) (B Int64) (C Int64) (D Int64) (E Int64) (F Int64))";
        let arms = "((T.C x) x) ((T.A p) (+ p 100)) (_ 999) ((T.C z) (+ z 1)) ((T.F w) w)";
        for (scrut, want) in [
            ("(T.C 5)", 5),   // first C arm wins (not the shadowed later `+z 1`)
            ("(T.A 7)", 107), // A arm, listed before the default
            ("(T.F 3)", 999), // F arm is AFTER the default → shadowed by it (source-order priority)
            ("(T.B 8)", 999), // B untested → default
            ("(T.E 2)", 999), // E untested → default
        ] {
            let src = format!("(module m {ty} (def (main) (match {scrut} {arms})) (export main))");
            assert_eq!(
                run_returns::<i64>(
                    &compile_component(&crate::codec::encode(&parse(&src))).expect("compile"),
                    "main"
                ),
                want,
                "match {scrut}"
            );
        }
    }

    #[test]
    fn a_multi_param_function() {
        // `((fn (a b) (+ a b)) 3 4)` = 7 — both params substituted.
        assert_eq!(run_main("((fn (a b) (+ a b)) 3 4)"), 7);
    }

    #[test]
    fn partial_application_curries_at_compile_time() {
        // `core-semantics.md` §Functions Are Single-Arity: applying a curried function to fewer args
        // than its arity returns a closure awaiting the rest. Under-application β-reduces the leading
        // params into a RESIDUAL lambda, so the whole chain folds when fully applied — no runtime
        // function survives.
        // A lambda applied to ONE of two args, then the residual applied to the second → `(+ 3 4)` = 7.
        assert_eq!(run_main("(((fn (x y) (+ x y)) 3) 4)"), 7);
        // The residual bound and applied later: `(let ((add3 ((fn (x y) (+ x y)) 3))) (add3 7))` = 10.
        assert_eq!(
            run_main("(let ((add3 ((fn (x y) (+ x y)) 3))) (add3 7))"),
            10
        );
        // Through a NAMED def, both the explicit-curried `((add 3) 4)` and the let-bound `inc` shapes.
        let curried = "(module m (def (add x y) (+ x y)) (def (main) ((add 3) 4)) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(curried))).expect("compile"),
                "main"
            ),
            7
        );
        let letbound = "(module m (def (add x y) (+ x y)) (def (main) (let ((inc (add 3))) (inc 4))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(letbound))).expect("compile"),
                "main"
            ),
            7
        );
        // THREE params, applied one-then-two: the residual itself partially applies again.
        assert_eq!(
            run_main("(let ((f (fn (a b c) (+ (+ a b) c)))) (let ((g (f 1))) (g 2 3)))"),
            6
        );
    }

    #[test]
    fn over_applying_a_function_is_a_coded_type_error() {
        // OVER-application (more args than arity) is a TYPE ERROR (CDZ0203), not a silent argument drop
        // nor a to-do decline — the SAME code an over-applied CONSTRUCTOR uses. `((fn (x) (+ x 1)) 5 9)`
        // desugars to `(((fn (x)…) 5) 9)`: `(fn x) 5` = 6 (not a function), applied to 9 → CDZ0203.
        // `core-semantics.md` §Functions Are Single-Arity (closes SPEC-BACKLOG-21 for user functions).
        for src in [
            "(module m (def (main) ((fn ((: x Int64)) (+ x 1)) 5 9)) (export main))",
            "(module m (def (f (: x Int64)) (+ x 1)) (def (main) (f 5 9)) (export main))",
            // Three args to a 2-param lambda.
            "(module m (def (main) ((fn ((: a Int64) (: b Int64)) (+ a b)) 1 2 3)) (export main))",
        ] {
            let d = compile_component(&crate::codec::encode(&parse(src)))
                .expect_err("over-application must be rejected");
            assert_eq!(
                d.code.as_deref(),
                Some("CDZ0203"),
                "over-application should be CDZ0203, got {:?} for `{src}`",
                d.code
            );
        }
        // A correctly-applied curried chain is NOT over-application (it fully consumes each level).
        assert_eq!(
            run_main("((((fn ((: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c))) 1) 2) 3)"),
            6
        );
    }

    #[test]
    fn partial_application_captures_a_runtime_variable_in_the_residual() {
        use wasmtime::component::Val;
        // Partially applying to a VARIABLE reference (a runtime param / let-bound) must CAPTURE it in the
        // residual lambda: `((sub n) 3)` curries to `(fn (b) (- n b))`, and `n` — a caller-pinned free
        // variable — must keep its binding across the residual's later full application. The currying copy
        // re-copied the `n` name atom and re-resolved it against the residual's scope (where it is
        // unbound) → CDZ0101. Fixed by SHARING a resolve-pinned name (a captured free var) rather than
        // re-copying it. A CONSTANT capture always worked (no free var); this pins the variable case.
        let param =
            "(module m (def (sub a b) (- a b)) (def (main (: n Int64)) ((sub n) 3)) (export main))";
        let b =
            compile_component(&crate::codec::encode(&parse(param))).expect("compile param capture");
        assert_eq!(run_returns_with::<i64>(&b, "main", &[Val::S64(10)]), 7);
        // The let-bound companion: `(let ((m 10)) ((sub m) 3))` = 7 — the captured value is any in-scope
        // binding, not only a parameter.
        let letcap = "(module m (def (sub a b) (- a b)) (def (main) (let ((m 10)) ((sub m) 3))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(letcap)))
                    .expect("compile let capture"),
                "main"
            ),
            7
        );
        // A body-internal `let`-local is UNAFFECTED (never resolve-pinned, so it still re-resolves against
        // the copied scope): `(inc n)` with `(let ((k 10)) (+ n k))`, n=5 → 15.
        let letlocal = "(module m (def (inc n) (let ((k 10)) (+ n k))) (def (main (: v Int64)) (inc v)) (export main))";
        let l =
            compile_component(&crate::codec::encode(&parse(letlocal))).expect("compile let-local");
        assert_eq!(run_returns_with::<i64>(&l, "main", &[Val::S64(5)]), 15);
    }

    #[test]
    fn a_let_bound_variable_passed_as_a_call_argument_resolves_at_the_call_site() {
        // `(let ((k 10)) (inc k))` = 11: β-reduction splices the CALL-SITE argument `k` into a copy of
        // `inc`'s body, and `push_list` re-parents the splice — so without pinning the argument's scope
        // at the call site, `k` would resolve in the CALLEE's scope (where it is unbound) and the
        // program would be spuriously rejected CDZ0101. The argument must keep the caller's `let`.
        let src =
            "(module m (def (inc x) (+ x 1)) (def (main) (let ((k 10)) (inc k))) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main"
            ),
            11
        );
        // The lambda sibling and the nested-application sibling exercise the same splice: a let-bound
        // name and a call's own result, each passed as an argument to another user call.
        assert_eq!(run_main("(let ((k 10) (f (fn (x) (+ x 1)))) (f k))"), 11);
        assert_eq!(run_main("(let ((f (fn (x) (+ x 1)))) (f (f 0)))"), 2);
    }

    #[test]
    fn a_stored_or_captured_lambda_applies_through_the_fold() {
        // `core-semantics.md` §A Function Is A First-Class Value — a function stored in a data
        // structure is extractable and callable, and a function value captures its CREATION scope.
        // The fold reduces THROUGH a projection / a `let` body to reach the lambda, so these all fold
        // to a scalar with no runtime function surviving.
        // Stored in a tuple element, projected, applied: `((. (tuple (fn (x) (+ x 1)) 9) 0) 5)` = 6.
        assert_eq!(run_main("((. (tuple (fn (x) (+ x 1)) 9) 0) 5)"), 6);
        // Stored in a record field, projected, applied.
        assert_eq!(run_main("((. (record (f (fn (x) (+ x 1)))) f) 5)"), 6);
        // Captured over a `let` binding, applied immediately: `((let ((y 3)) (fn (x) (+ x y))) 4)` = 7.
        assert_eq!(run_main("((let ((y 3)) (fn (x) (+ x y))) 4)"), 7);
        // Capture is by the CREATION scope, NOT the application scope — a `y=100` shadow at the call
        // site must NOT be observed; the captured `y=3` wins (`core-semantics.md` §A Function Value
        // Captures The Bindings In Scope Where It Is Created).
        assert_eq!(
            run_main("(let ((add-y (let ((y 3)) (fn (x) (+ x y))))) (let ((y 100)) (add-y 4)))"),
            7
        );
    }

    #[test]
    fn a_lambda_captures_an_enclosing_binding_across_reduction() {
        // A lambda that references an ENCLOSING binding (a free variable bound further out, NOT inside
        // its own body) and is applied INSIDE that binding's scope. β-reducing the application must
        // PRESERVE the free variable's resolution to the enclosing binding — `pin_free_vars` pins it so
        // `beta_reduce` shares the occurrence rather than copying it into the orphan reduced node (where
        // it would re-resolve unbound → a spurious CDZ0101, the bug this fixes). `core-semantics.md`
        // §A Function Value Captures The Bindings In Scope Where It Is Created.
        // Over an enclosing `let`: `(let ((k 10)) ((fn (x) (+ x k)) 5))` = 15.
        assert_eq!(
            run_main("(let ((k 10)) ((fn ((: x Int64)) (+ x k)) 5))"),
            15
        );
        // Two enclosing captures + a nested let: `(+ x a b)` reads `a`,`b` from the enclosing lets.
        assert_eq!(
            run_main("(let ((a 2)) (let ((b 3)) ((fn ((: x Int64)) (+ (+ x a) b)) 10)))"),
            15
        );
        // Over an enclosing DEF PARAMETER: `(def (f k) ((fn (x) (+ x k)) 5))`, `f(10)` = 15.
        let src = "(module m (def (f (: k Int64)) ((fn ((: x Int64)) (+ x k)) 5)) \
             (def (main (: k Int64)) (f k)) (export main))";
        assert_eq!(
            run_returns_with::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main",
                &[wasmtime::component::Val::S64(10)]
            ),
            15
        );
    }

    #[test]
    fn a_named_capturing_closure_applied_directly_folds() {
        // A capturing lambda BOUND to a name (`let ((g (fn (x) (+ x k))))`) and applied `(g 5)` where `g`
        // closes over an enclosing `k`. `g` is copy-propagated (a lambda value is never KEPT as a runtime
        // `let` slot — `should_keep_binding` short-circuits it), so `(g 5)` β-reduces to `(+ 5 k)` and `k`
        // folds through its enclosing binding. Regression guard: a prior version LIFTED `g` speculatively
        // during the keep check, polluting `db.captured_ref` with `k`'s occurrence → the FOLDED `k` then
        // lowered to a `Core::Captured` env-read in the enclosing scope (an uninitialized-local
        // miscompile). The lambda-valued-binding short-circuit fixes it.
        assert_eq!(
            run_main("(let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (g 5)))"),
            15
        );
        // Applied more than once — each use folds independently: (5+10)+(6+10) = 31.
        assert_eq!(
            run_main("(let ((k 10)) (let ((g (fn ((: x Int64)) (+ x k)))) (+ (g 5) (g 6))))"),
            31
        );
        // Capturing an enclosing PARAMETER through a named binding (not just a `let`).
        let src = "(module m \
            (def (f (: k Int64)) (let ((g (fn ((: x Int64)) (+ x k)))) (g 5))) \
            (def (main (: k Int64)) (f k)) (export main))";
        assert_eq!(
            run_returns_with::<i64>(
                &compile_component(&crate::codec::encode(&parse(src))).expect("compile"),
                "main",
                &[wasmtime::component::Val::S64(10)]
            ),
            15
        );
    }

    #[test]
    fn a_capturing_closure_composes_with_return_storage_and_multi_capture() {
        // A closure FACTORY: `(def (mk k) (fn (x) (+ x k)))` returns a closure over `k`; `((mk 10) 5)`
        // applies the returned closure = 15. A returned closure composed with a capture, both folded.
        let factory = "(module m (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k))) \
            (def (main) ((mk 10) 5)) (export main))";
        assert_eq!(
            run_returns::<i64>(
                &compile_component(&crate::codec::encode(&parse(factory))).expect("compile"),
                "main"
            ),
            15
        );
        // A capturing closure STORED in a tuple, projected, applied — capture survives the storage.
        assert_eq!(
            run_main("(let ((k 7)) ((. (tuple (fn ((: x Int64)) (+ x k)) 9) 0) 5))"),
            12
        );
        // MULTIPLE distinct captures from different enclosing `let`s, through a nested-arith body.
        assert_eq!(
            run_main("(let ((a 2) (b 3)) ((fn ((: x Int64)) (+ (* x a) b)) 5))"),
            13
        );
    }

    #[test]
    fn a_returned_capturing_closure_bound_by_let_and_applied_folds() {
        use wasmtime::component::Val;
        // A capturing closure RETURNED by a function, BOUND with `let`, then APPLIED — the discriminator
        // that MISCOMPILED (invalid wasm, silently written at exit 0). `(mk n)` returns `(fn (x) (+ n x))`
        // capturing the parameter `n`; `(let ((f (mk n))) (f 3))` binds that closure to `f` and applies it.
        // Applying the SAME closure INLINE `((mk n) 3)` or through a higher-order function `(ap (mk n) 3)`
        // always folded; only binding it with `let` mis-typed the local. Root cause: `should_keep_binding`
        // short-circuits a syntactic `Resolved::Lambda` init to avoid a speculative `core_of` LIFT that
        // pollutes `db.captured_ref`, but `(mk n)` is an `Apply` that REDUCES to a capturing lambda — it
        // slipped past, was lifted, and recorded `n`'s occurrence as a capture. The copy-propagated
        // `((mk n) 3)` then β-reduced to `(+ n 3)`, and the SHARED `n` occurrence lowered to a
        // `Core::Captured` env-read in `main` (no env) → the i32/i64 slot mismatch. The fix extends the
        // short-circuit to a binding whose value `lambda_body`-reduces to a lambda: it copy-propagates and
        // folds inline exactly like the working `((mk n) 3)` form. With n = 10 → 10 + 3 = 13 (a pure fold,
        // no runtime import).
        let src = "(module m (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x))) \
            (def (main (: n Int64)) (let ((f (mk n))) (f 3))) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_none(),
            "the let-bound closure folds inline — no heap round-trip, no runtime import"
        );
        assert_eq!(run_returns_with::<i64>(&bytes, "main", &[Val::S64(10)]), 13);
        // Applied MORE THAN ONCE through the one binding — each application folds independently:
        // (10+3) + (10+4) = 27.
        let twice = "(module m (def (mk (: n Int64)) (fn ((: x Int64)) (+ n x))) \
            (def (main (: n Int64)) (let ((f (mk n))) (+ (f 3) (f 4)))) (export main))";
        let b2 = compile_component(&crate::codec::encode(&parse(twice))).expect("compile");
        assert_eq!(run_returns_with::<i64>(&b2, "main", &[Val::S64(10)]), 27);
        // A MULTI-PARAM returned closure applied at full arity, both flat `(f 3 4)` and curried `((f 3) 4)`:
        // 10 + 3 + 4 = 17.
        for shape in [
            "(module m (def (mk (: n Int64)) (fn (x y) (+ n (+ x y)))) \
             (def (main (: n Int64)) (let ((f (mk n))) (f 3 4))) (export main))",
            "(module m (def (mk (: n Int64)) (fn (x y) (+ n (+ x y)))) \
             (def (main (: n Int64)) (let ((f (mk n))) ((f 3) 4))) (export main))",
        ] {
            let b = compile_component(&crate::codec::encode(&parse(shape))).expect("compile");
            assert_eq!(run_returns_with::<i64>(&b, "main", &[Val::S64(10)]), 17);
        }
    }

    #[test]
    fn a_closure_captures_another_closure_and_composes() {
        // A HIGHER-ORDER capture: `twice = (fn (y) (inc (inc y)))` closes over `inc` (itself a closure)
        // and applies it twice; `(twice 5)` = inc(inc(5)) = 7. A function value captured by another
        // closure folds at each application.
        assert_eq!(
            run_main(
                "(let ((inc (fn ((: x Int64)) (+ x 1)))) \
                       (let ((twice (fn ((: y Int64)) (inc (inc y))))) (twice 5)))"
            ),
            7
        );
        // A closure's argument is another closure's RESULT: `((fn (x) (+ x k)) ((fn (y) (* y 2)) 3))`
        // with k=10 → (+ 6 10) = 16. Composing two closure applications.
        assert_eq!(
            run_main("(let ((k 10)) ((fn ((: x Int64)) (+ x k)) ((fn ((: y Int64)) (* y 2)) 3)))"),
            16
        );
        // A closure that itself references an enclosing closure through a further nesting.
        assert_eq!(
            run_main(
                "(let ((a 1)) (let ((f (fn ((: x Int64)) (+ x a)))) \
                       (let ((g (fn ((: y Int64)) (f (+ y 1))))) (g 5))))"
            ),
            7
        );
    }

    #[test]
    fn a_runtime_selected_function_applies_via_case_of_case() {
        use wasmtime::component::Val;
        // `((if c f g) x)` with a RUNTIME condition — the function is chosen at run time. The
        // application is pushed into each branch (case-of-case: `(if c (f x) (g x))`), where each
        // branch's lambda β-reduces, so the whole thing computes with no closure surviving. Driven
        // through a Bool parameter so the condition is genuinely runtime (a constant would fold the
        // `if` upstream). `core-semantics.md` §A Function Is A First-Class Value (an `if` returns a fn).
        let src = "(module m \
            (def (choose (: b Bool)) (if b (fn (x) (+ x 1)) (fn (x) (+ x 10)))) \
            (def (main (: b Bool)) ((choose b) 5)) (export main))";
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&bytes, "main", &[Val::Bool(true)]),
            6
        );
        assert_eq!(
            run_returns_with::<i64>(&bytes, "main", &[Val::Bool(false)]),
            15
        );
        // The `if` directly in head position (no intervening def).
        let src2 = "(module m \
            (def (main (: b Bool)) ((if b (fn (x) (+ x 1)) (fn (x) (- x 1))) 10)) (export main))";
        let bytes2 = compile_component(&crate::codec::encode(&parse(src2))).expect("compile");
        assert_eq!(
            run_returns_with::<i64>(&bytes2, "main", &[Val::Bool(true)]),
            11
        );
        assert_eq!(
            run_returns_with::<i64>(&bytes2, "main", &[Val::Bool(false)]),
            9
        );
    }

    /// Run `src`'s `main` with a single integer `arg` through the COMPOSED value-heap runtime (a closure
    /// is a heap cell, so instantiation needs the runtime linked), returning the rendered result string.
    /// Skips (returns `None`) if the runtime wasm is not built.
    fn run_closure(src: &str, arg: i64) -> Option<String> {
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a runtime closure imports the value-heap runtime (a heap cell, not a fold)"
        );
        let runtime = find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![arg.to_string()],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("closure run trapped: {t}"),
        }
    }

    /// `run_closure`'s sibling for a program whose `main` takes a single BOOL argument — used to exercise
    /// a closure that captures a boolean (its lifted body unboxes with `get-bool`). Same composed-runtime
    /// path; `arg` is rendered as the s-expr boolean literal the entrypoint expects.
    fn run_closure_bool(src: &str, arg: bool) -> Option<String> {
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        let runtime = find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![arg.to_string()],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("closure run trapped: {t}"),
        }
    }

    /// `run_closure`'s sibling for a program whose `main` is NULLARY (a closure captures a compound built
    /// in `main`, so no runtime entry argument is needed). Same composed-runtime path.
    fn run_closure_nullary(src: &str) -> Option<String> {
        let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        let runtime = find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: Some("main".to_string()),
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("closure run trapped: {t}"),
        }
    }

    #[test]
    fn a_function_crosses_a_recursive_boundary_as_a_runtime_closure() {
        // The genuine runtime-closure case (`call_indirect`): a function argument passed to a RECURSIVE
        // higher-order function, applied inside the recursion. `apply-sum` cannot inline (it recurses),
        // so its function parameter `g` is a real runtime CLOSURE VALUE — the lambda `(fn (x) (* x 2))`
        // is LAMBDA-LIFTED to a standalone function, passed as a heap-cell handle, and applied via
        // `call_indirect`. `apply-sum g n = g(n) + g(n-1) + … + g(1)`, so with `g = (*2)` the result is
        // `2·(n + (n-1) + … + 1) = n·(n+1)`. `core-semantics.md` §A Function Is A First-Class Value.
        let src = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (* x 2)) n)) (export main))";
        let Some(r0) = run_closure(src, 0) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r0, "0");
        assert_eq!(run_closure(src, 1).unwrap(), "2");
        assert_eq!(run_closure(src, 3).unwrap(), "12");
        assert_eq!(run_closure(src, 5).unwrap(), "30");
        // A DIFFERENT lifted lambda through the same recursive HOF — `(+ x 100)` — so the closure must
        // carry the RIGHT code (the table slot selects the applied function). n=3: 3·100 + 6 = 306.
        let src2 = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: n Int64)) (apply-sum (fn ((: x Int64)) (+ x 100)) n)) (export main))";
        assert_eq!(run_closure(src2, 3).unwrap(), "306");
    }

    #[test]
    fn a_capturing_closure_crosses_a_recursive_boundary() {
        // A CAPTURING closure: `(fn (x) (+ x k))` closes over the free variable `k` from `main`'s scope.
        // It is a genuine runtime closure with an ENVIRONMENT — a heap cell holding the code pointer AND
        // the captured `k` — passed to the recursive `apply-sum` and applied at each step, each
        // application reading `k` back from the cell. `core-semantics.md` §A Function Value Captures The
        // Bindings In Scope Where It Is Created. `apply-sum (fn (x) (+ x k)) 3 = (3+k)+(2+k)+(1+k) = 6+3k`.
        let src = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: k Int64)) (apply-sum (fn ((: x Int64)) (+ x k)) 3)) (export main))";
        let Some(r0) = run_closure(src, 0) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r0, "6");
        assert_eq!(run_closure(src, 10).unwrap(), "36");
        assert_eq!(run_closure(src, 100).unwrap(), "306");
    }

    #[test]
    fn a_closure_captures_a_function_value_and_applies_it_through_a_recursive_hof() {
        // HIGHER-ORDER CAPTURE: a closure whose captured free variable is ITSELF A FUNCTION. The inner
        // `(fn (b) (g b))` closes over `g` — a fn-typed parameter of the recursive `rec` — so the closure
        // cell must store `g`'s closure HANDLE (a u32 cell) as a capture and, in the lifted body, read it
        // back and apply it via `call_indirect`. Because `rec` is recursive, `g` stays a genuine runtime
        // value (not inlined), and it threads through the recursive specialization as a fresh param
        // binder — so `collect_captures` must classify a ref whose target is a SYNTHESIZED param as a
        // capture (not a global), and `box_op`/`get_op` must store/read a `Ty::Fn` handle as-is (like any
        // compound handle), NOT box it as a scalar. Both were bugs: the capture was skipped (a bare
        // `Core::Param` with no local slot → invalid module) and the fn handle was boxed as an i64 (an
        // i32/i64 type mismatch → invalid module). `rec` builds `(fn (b) (g b))`, hands it to the
        // recursive `sumapply` (applied at 2 and 1), and sums over its own recursion: each `rec` level
        // contributes `g(2)+g(1)`, repeated n times. With `g = (+1)`: (2+1)+(1+1) = 5 per level, ×3 = 15.
        let src = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (rec (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1))))) \
            (def (main (: n Int64)) (rec (fn ((: x Int64)) (+ x 1)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "15"); // 3 levels × (g(2)+g(1)) = 3 × ((2+1)+(1+1)) = 3×5
        // A DIFFERENT captured function proves the handle dispatches the right code: `g = (*3)` →
        // (2·3)+(1·3) = 9 per level, ×3 = 27.
        let src2 = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (rec (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1))))) \
            (def (main (: n Int64)) (rec (fn ((: x Int64)) (* x 3)) n)) (export main))";
        assert_eq!(run_closure(src2, 3).unwrap(), "27");
    }

    #[test]
    fn a_closure_captures_a_capturing_closure_and_calls_it() {
        // NESTED CAPTURING CLOSURES: `g = (fn (x) (f x))` captures `f`, and `f = (fn (y) (+ y k))` ITSELF
        // captures `k`. Inside `g`'s lifted body, `f` is a runtime closure HANDLE read from `g`'s env cell
        // — NOT the compile-time lambda it was defined from — so `(f x)` must apply via `call_indirect`
        // (which threads `f`'s OWN env, carrying `k`), not β-reduce. The bug: `head_is_runtime_fn_value`
        // followed the captured `f` REF through to its original `(fn …)` definition and reported
        // not-runtime, so `(f x)` mis-lowered — `f`'s handle was read as a scalar and ADDED to `x` instead
        // of called (a silent MISCOMPILE, wrong value). Fixed by recognizing a captured-ref head as a
        // runtime fn value. `ap (fn (x) (f (+ x 1))) 2` with `f = (+100)`: g(2)+g(1) = (2+1+100)+(1+1+100)
        // = 205.
        let src = "(module m \
            (def (ap (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (ap g (- n 1))))) \
            (def (main (: k Int64)) \
              (let ((f (fn ((: y Int64)) (+ y k)))) \
                (ap (fn ((: x Int64)) (f (+ x 1))) 2))) (export main))";
        let Some(r) = run_closure(src, 100) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "205"); // (2+1+100)+(1+1+100)
        // The simplest form — `g` just forwards to the captured capturing `f` (no wrapping arithmetic):
        // `ap (fn (x) (f x)) 2` with `f = (+100)` = (2+100)+(1+100) = 203. Pins that the captured closure's
        // OWN environment (k) is threaded through the nested indirect call.
        let plain = "(module m \
            (def (ap (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (ap g (- n 1))))) \
            (def (main (: k Int64)) \
              (let ((f (fn ((: y Int64)) (+ y k)))) \
                (ap (fn ((: x Int64)) (f x)) 2))) (export main))";
        assert_eq!(run_closure(plain, 100).unwrap(), "203"); // (2+100)+(1+100)
    }

    #[test]
    fn a_three_parameter_closure_applies_at_full_arity() {
        // A THREE-parameter runtime closure applied at full arity through a recursive HOF — the multi-param
        // lift generalizes past two params. `(fn (a b c) (+ (+ a b) c))` lifts to `(env, a, b, c) ->
        // result` and applies via one `call_indirect` with all three args. `ap3 g n` sums `(g i i i) =
        // 3·i` for i = n..1, so with n=3 the total is 3·(3+2+1) = 18.
        let src = "(module m \
            (def (ap3 (: g (-> Int64 (-> Int64 (-> Int64 Int64)))) (: n Int64)) \
              (if (= n 0) 0 (+ (g n n n) (ap3 g (- n 1))))) \
            (def (main (: n Int64)) (ap3 (fn ((: a Int64) (: b Int64) (: c Int64)) (+ (+ a b) c)) n)) \
            (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "18"); // 3·(3+2+1)
        assert_eq!(run_closure(src, 4).unwrap(), "30"); // 3·(4+3+2+1)
    }

    #[test]
    fn a_runtime_closure_body_calls_a_recursive_top_level_function() {
        // A lifted closure whose body drives an ordinary RECURSIVE CALL — `(fn (x) (fact x))` (passed to
        // the recursive `ap`, so it cannot fold) invokes the recursive `fact`. The lifted body holds a
        // `Core::Call` to `fact` nested inside a `call_indirect`ed closure — the canonical "map a recursive
        // function over a structure" shape. `ap g 3` = fact(3)+fact(2)+fact(1) = 6+2+1 = 9.
        let src = "(module m \
            (def (fact (: m Int64)) (if (= m 0) 1 (* m (fact (- m 1))))) \
            (def (ap (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (ap g (- n 1))))) \
            (def (main (: n Int64)) (ap (fn ((: x Int64)) (fact x)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "9"); // fact(3)+fact(2)+fact(1)
        assert_eq!(run_closure(src, 5).unwrap(), "153"); // 120+24+6+2+1
    }

    #[test]
    fn a_runtime_closure_compares_its_argument_to_a_captured_value() {
        // A captured value drives a COMPARISON + BRANCH inside the lifted body: `(fn (x) (if (= x k) 1 0))`
        // captures `k` and branches on `x == k`. Through the recursive `ap` with k=2 over 3,2,1, only x=2
        // matches, so the sum is 1. (A different k selects a different element — k=3 → 1 at x=3.)
        let src = "(module m \
            (def (ap (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (ap g (- n 1))))) \
            (def (main (: k Int64)) (ap (fn ((: x Int64)) (if (= x k) 1 0)) 3)) (export main))";
        let Some(r) = run_closure(src, 2) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "1"); // only x=2 matches k=2
        assert_eq!(run_closure(src, 3).unwrap(), "1"); // only x=3 matches k=3
        assert_eq!(run_closure(src, 9).unwrap(), "0"); // none of 3,2,1 equal 9
    }

    #[test]
    fn a_runtime_fn_value_is_manually_eta_wrapped_and_applied() {
        // A genuinely-RUNTIME fn value partially applied via a MANUAL eta-wrap. `g` is a runtime
        // two-parameter fn parameter (no compile-time lambda to partially apply); `(fn (b) (g n b))`
        // captures `g` (a runtime closure handle) AND `n`, and applies `g` at full arity inside — an
        // ordinary capturing closure whose body is a full-arity `call_indirect` on the captured `g`. Both
        // `ap` and `sumapply` recurse, so nothing folds: TWO nested indirect calls (ap→wrapper, wrapper→g).
        // This composes the captured-closure-call path (a closure captures a runtime fn and calls it) with
        // a two-level HOF. `ap g n` = sum over i=n..1 of (g(i,2)+g(i,1)) = (i+2)+(i+1) = 2i+3; n=3 → 21.
        let src = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) \
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g n b)) 2) (ap g (- n 1))))) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "21"); // (9)+(7)+(5)
        // The NON-recursive-outer form folds `ap` but the eta-wrapper still runs through recursive
        // `sumapply`: `sumapply (fn (b) (g 5 b)) 2` = (5+2)+(5+1) = 13.
        let flat = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) (sumapply (fn ((: b Int64)) (g n b)) 2)) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n)) (export main))";
        assert_eq!(run_closure(flat, 5).unwrap(), "13"); // (5+2)+(5+1)
    }

    #[test]
    fn a_predicate_closure_returning_bool_drives_an_early_exit_hof() {
        // A closure whose RESULT type is Bool. `(fn (x) (= x k))` is a `(-> Int64 Bool)` value threaded
        // through the recursive `anyp` ("does any i in n..1 satisfy it?"), which short-circuits on the
        // first `true`. The closure's boolean result crosses the `call_indirect` (the lifted signature
        // returns an i32) and drives `anyp`'s `if`. With k=2 over 3,2,1 the predicate holds at x=2 → true →
        // 100; a k absent from the range → false → 0.
        let src = "(module m \
            (def (anyp (: g (-> Int64 Bool)) (: n Int64)) \
              (if (= n 0) false (if (g n) true (anyp g (- n 1))))) \
            (def (main (: k Int64)) (if (anyp (fn ((: x Int64)) (= x k)) 3) 100 0)) (export main))";
        let Some(r) = run_closure(src, 2) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "100"); // x=2 satisfies the predicate
        assert_eq!(run_closure(src, 1).unwrap(), "100"); // x=1 satisfies at the last step
        assert_eq!(run_closure(src, 5).unwrap(), "0"); // none of 3,2,1 equal 5
    }

    #[test]
    fn a_closure_that_captures_a_boolean_imports_the_ops_its_lifted_body_uses() {
        // A runtime op used ONLY inside a LIFTED closure body must still be imported. The used-op set that
        // fixes the module's import layout was walked over the top-level defs ONLY, NOT the lambda-lifted
        // bodies — so an op no top-level def happens to use, but a closure body does (`get-bool` unboxing a
        // captured boolean; `box-bool` storing it), resolved to a bogus import index and the module was
        // INVALID (`call 4294967295`). `collect_module_used_ops` now walks the reached lifted bodies too.
        // `(fn (x) (if flag (* x 2) x))` captures the boolean `flag`; the lifted body reads it back with
        // `get-bool`. With flag=true it doubles: apply-sum over 3,2,1 = 6+4+2 = 12. (Before the fix this
        // compiled to a module wasmtime rejects — `run_closure` would panic on the invalid component.)
        let src = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: t Bool)) (apply-sum (fn ((: x Int64)) (if t (* x 2) x)) 3)) \
            (export main))";
        let Some(r) = run_closure_bool(src, true) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "12"); // flag=true → 2·(3+2+1) = 12
        assert_eq!(run_closure_bool(src, false).unwrap(), "6"); // flag=false → 3+2+1 = 6
    }

    #[test]
    fn a_closure_captures_a_compound_and_reads_it_back_inside_its_lifted_body() {
        // A capture slot holds a COMPOUND heap HANDLE (a tuple / a sum), not a boxed scalar — stored into
        // the env cell as-is and read back as-is (`box_op`/`get_op` return None for a compound). Two
        // shapes: (a) a captured TUPLE projected in the body; (b) a captured SUM matched in the body (the
        // scrutinee of a `match` is a CAPTURED free variable, read from the env cell). Both survive the
        // recursive `apply-sum` indirect-call boundary.
        // (a) tuple capture: each application adds (. p 0)+(. p 1) = 30, so over 3,2,1 = 96.
        let tup = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main) (let ((p (tuple 10 20))) \
              (apply-sum (fn ((: x Int64)) (+ (+ x (. p 0)) (. p 1))) 3))) (export main))";
        let Some(r) = run_closure_nullary(tup) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "96");
        // (b) sum capture matched: each application takes the Some arm and adds 100, so over 3,2,1 = 306.
        let sum = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main) (let ((o (Some 100))) \
              (apply-sum (fn ((: x Int64)) (match o ((Some v) (+ x v)) (None x))) 3))) (export main))";
        assert_eq!(run_closure_nullary(sum).unwrap(), "306");
    }

    #[test]
    fn a_lambda_forwarding_to_a_substituted_fn_param_runs_through_a_recursive_hof() {
        // The outer HOF `twice` is NON-recursive, so it INLINES and its fn parameter `g` is SUBSTITUTED by
        // `main`'s concrete lambda. The inner `(fn (b) (g b))` is passed to the recursive `sumapply`, so it
        // must survive as a runtime closure. This case DECLINED for several ticks (a spurious "self-capture"
        // — `collect_captures` tripped its `binder == node` guard on the inner lambda's OWN param binder,
        // reached while descending a nested `(fn …)` in the lifted body). Now `collect_captures` handles a
        // nested lambda explicitly (descends its body with the inner params EXCLUDED, skipping the param
        // list), so the inner applied lambda β-reduces during lowering and the program RUNS. `sumapply
        // (fn (b) (g b)) 3` with `g = (+1)`: g(3)+g(2)+g(1) = 4+3+2 = 9.
        let src = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (twice (: g (-> Int64 Int64))) (sumapply (fn ((: b Int64)) (g b)) 3)) \
            (def (main) (twice (fn ((: x Int64)) (+ x 1)))) (export main))";
        let Some(r) = run_closure_nullary(src) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "9"); // (3+1)+(2+1)+(1+1)
        // The `(g (g b))` double-apply form runs too: `h(b) = g(g(b)) = b+2`, so h(3)+h(2)+h(1) = 5+4+3 = 12.
        let src2 = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (twice (: g (-> Int64 Int64))) (sumapply (fn ((: b Int64)) (g (g b))) 3)) \
            (def (main) (twice (fn ((: x Int64)) (+ x 1)))) (export main))";
        assert_eq!(run_closure_nullary(src2).unwrap(), "12"); // (3+2)+(2+2)+(1+2)
    }

    #[test]
    fn an_unannotated_closure_param_is_grounded_from_its_body() {
        // A bare `(fn (x) (* x 2))` — no `(: x T)`. `x` types `Any` at its own occurrence (inference does
        // not thread the use-site arrow back), but the body `(* x 2)` uses it as an integer operand, so
        // `solve_lambda_param_ty` grounds `x : Int64` from that use (the lambda analogue of the recursive
        // -def A2 solve). The closure lifts with that machine type and runs — no annotation needed.
        // `apply-sum (fn (x) (* x 2)) 3 = 6+4+2 = 12`.
        let src = "(module m \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: n Int64)) (apply-sum (fn (x) (* x 2)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "12");
        assert_eq!(run_closure(src, 5).unwrap(), "30");
    }

    #[test]
    fn a_closure_carried_in_a_sum_payload_applies_through_call_indirect() {
        // A closure stored in a SUM variant's payload, extracted by a match binder, and applied — the
        // callback-in-a-variant shape. `(Some (fn (n) (* n 2)))` carries a closure; `(match … ((Some f)
        // (f 5)) …)` binds `f` to the payload (a `sum-payload` heap read) and applies it. The closure is
        // reached through the PAYLOAD, not a `let`/tuple projection the fold reduces through, so `f`'s
        // application is a runtime `call_indirect` — a payload/element binder is a runtime function-value
        // source like a `Param`. Before the fix `(f 5)` declined "value is not applyable" (the closure
        // path was gated on a `Param` head only). Run through the composed runtime (result 10).
        use crate::testkit::parse;
        let cases = [
            (
                "Some payload, single arg",
                "(module m (def (main) \
                   (match (Some (fn ((: n Int64)) (* n 2))) ((Some f) (f 5)) ((None _) 0))) (export main))",
                "10",
            ),
            (
                "Ok payload, single arg",
                "(module m (def (main) \
                   (match (Ok (fn ((: n Int64)) (+ n 100))) ((Ok f) (f 5)) ((Err e) 0))) (export main))",
                "105",
            ),
            (
                "Some payload, TWO args (multi-param closure)",
                "(module m (def (main) \
                   (match (Some (fn ((: a Int64) (: b Int64)) (+ a b))) ((Some f) (f 3 4)) ((None _) 0))) \
                 (export main))",
                "7",
            ),
            (
                "USER-SUM payload, single arg",
                "(module m (type T (Mk (-> Int64 Int64))) \
                   (def (main) (match (T.Mk (fn ((: n Int64)) (* n 2))) ((T.Mk f) (f 5)))) (export main))",
                "10",
            ),
            (
                "USER-SUM payload, curried TWO args",
                "(module m (type T (Mk (-> Int64 (-> Int64 Int64)))) \
                   (def (main) (match (T.Mk (fn ((: a Int64) (: b Int64)) (+ a b))) ((T.Mk f) (f 3 4)))) \
                 (export main))",
                "7",
            ),
        ];
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        for (label, src, want) in cases {
            let bytes = compile_component(&crate::codec::encode(&parse(src)))
                .unwrap_or_else(|e| panic!("compile closure-in-sum-payload ({label}): {e:?}"));
            assert!(
                cdz_run::required_runtime(&bytes).expect("valid").is_some(),
                "a payload-bound closure application imports the runtime ({label})"
            );
            let opts = cdz_run::RunOpts {
                export: Some("main".to_string()),
                args: vec![],
                runtime: Some(runtime.clone()),
                runtime_cache_dir: None,
                host_responses: Vec::new(),
            };
            match cdz_run::run(&bytes, &opts).unwrap_or_else(|e| panic!("run ({label}): {e:?}")) {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "{label}"),
                cdz_run::Outcome::Trap(t) => {
                    panic!("closure-in-sum-payload trapped ({label}): {t}")
                }
            }
        }
    }

    #[test]
    fn a_multi_param_closure_applies_at_full_arity() {
        // A TWO-parameter lambda VALUE `(fn (a b) (+ a b))` passed to a recursive HOF and applied at
        // full arity `(g i i)`. It lifts to a `(env, a, b) -> result` function and applies via ONE
        // `call_indirect` with both args (no intermediate closure). `ap2 g n = sum(i=n..1) (i+i) =
        // 2·n(n+1)/2 = n(n+1)`. `core-semantics.md` §Functions Are Single-Arity (full-arity application).
        let src = "(module m \
            (def (ap2 (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) \
              (if (= n 0) 0 (+ (g n n) (ap2 g (- n 1))))) \
            (def (main (: n Int64)) (ap2 (fn ((: a Int64) (: b Int64)) (+ a b)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "12"); // (3+3)+(2+2)+(1+1)
        assert_eq!(run_closure(src, 4).unwrap(), "20"); // 4·5
        // A multi-param closure that also CAPTURES: `(fn (a b) (+ (+ a b) k))` captures `k`. n=2, k=100:
        // (2+2+100)+(1+1+100) = 104+102 = 206.
        let src2 = "(module m \
            (def (ap2 (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) \
              (if (= n 0) 0 (+ (g n n) (ap2 g (- n 1))))) \
            (def (main (: k Int64)) (ap2 (fn ((: a Int64) (: b Int64)) (+ (+ a b) k)) 2)) (export main))";
        assert_eq!(run_closure(src2, 100).unwrap(), "206");
    }

    #[test]
    fn a_curried_application_spine_flattens_to_a_full_arity_indirect_call() {
        // RUNTIME CURRYING reaching full arity. `((g n) 1)` is `(fn (a b) …)` single-arity curried sugar
        // applied with nested parens — the SAME full-arity application as `(g n 1)`. When `g` is a
        // recursive HOF's runtime parameter it cannot β-reduce; the nested `Apply` spine must FLATTEN so
        // `g` applies to both `n` and `1` in ONE `call_indirect` (not decline as an unbuilt intermediate
        // closure). `ap g n = sum(i=n..1) (i+1)`; with `g = (+)`, n=3 → (3+1)+(2+1)+(1+1) = 9.
        let src = "(module m \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) \
              (if (= n 0) 0 (+ ((g n) 1) (ap g (- n 1))))) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "9"); // (3+1)+(2+1)+(1+1)
        assert_eq!(run_closure(src, 1).unwrap(), "2"); // (1+1)
        assert_eq!(run_closure(src, 5).unwrap(), "20"); // sum(i=1..5)(i+1) = 15+5
        // A DIFFERENT lifted lambda through the same curried recursive HOF — `g = (*)` — proving the
        // flattened spine's `call_indirect` carries the right code (the table slot selects the applied
        // function): `ap g n = sum(i=n..1) (i*1) = sum(i=n..1) i`; n=4 → 4+3+2+1 = 10.
        let src2 = "(module m \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) \
              (if (= n 0) 0 (+ ((g n) 1) (ap g (- n 1))))) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (* a b)) n)) (export main))";
        assert_eq!(run_closure(src2, 4).unwrap(), "10"); // 4+3+2+1
    }

    #[test]
    fn a_partially_applied_function_escapes_as_a_value_and_runs_through_a_recursive_hof() {
        // A PARTIAL APPLICATION escaping short of full arity, run as a runtime closure. `(g n)` where `g`
        // is `main`'s statically-known two-parameter lambda applied to ONE arg PARTIALLY APPLIES at compile
        // time into the residual `(fn (b) (+ 5 b))`; that residual escapes as a VALUE into the recursive
        // `sumapply`, which cannot inline it, so it runs as a runtime closure via `call_indirect`. The
        // partial-application fold + the runtime-closure lift compose: `sumapply (g 5) 2 = (5+2)+(5+1) =
        // 13`. Regression guard for the fix that made the residual's parameter annotation survive the
        // β-copy that carries it into the recursive callee (before it, the awaited param lost its declared
        // type — `is_binder_occurrence` now recognizes a `fn`/`def` param so a `(: p T)` binder copies with
        // its annotation intact).
        let src = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) (sumapply (g n) 2)) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (+ a b)) n)) (export main))";
        let Some(r) = run_closure(src, 5) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "13"); // (5+2)+(5+1)
        // A `(*)` variant proves the residual dispatches the right code: sumapply (g 5) 2 = (5*2)+(5*1) = 15.
        let src2 = "(module m \
            (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
            (def (ap (: g (-> Int64 (-> Int64 Int64))) (: n Int64)) (sumapply (g n) 2)) \
            (def (main (: n Int64)) (ap (fn ((: a Int64) (: b Int64)) (* a b)) n)) (export main))";
        assert_eq!(run_closure(src2, 5).unwrap(), "15"); // (5*2)+(5*1)
    }

    #[test]
    fn a_lambda_with_an_annotated_param_keeps_its_type_across_beta_copy() {
        // Focused regression for the `is_binder_occurrence` fix. β-COPYING a lambda whose parameter is
        // ANNOTATED `(: p T)` must preserve the annotation: the param name is a BINDER, not a value
        // reference, so it copies structurally as part of the `(: …)` binder — before the fix it was
        // treated as a reference and copied detached, so the copy's param lost its declared type. Exercised
        // by a lambda that is copied (a partial application curries `(add 5)` into a residual carrying the
        // annotated remaining param) then applied through a recursive HOF (which forces the copy). If the
        // annotation were lost the residual's param would type `Any` and the closure would decline; that it
        // RUNS pins the annotation survived. `apply-sum (add 5) 3 = (5+3)+(5+2)+(5+1) = 21`.
        let src = "(module m \
            (def (add (: a Int64) (: b Int64)) (+ a b)) \
            (def (apply-sum (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (g n) (apply-sum g (- n 1))))) \
            (def (main (: n Int64)) (apply-sum (add 5) n)) (export main))";
        let Some(r) = run_closure(src, 3) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping");
            return;
        };
        assert_eq!(r, "21"); // (5+3)+(5+2)+(5+1)
    }

    #[test]
    fn a_foldable_captured_closure_does_not_emit_a_dead_lift() {
        // A constant closure that CAPTURES but folds away entirely — `((let ((y 3)) (fn (x) (+ x y))) 4)`
        // reduces to 7 at compile time via the reduce-through fold, so no runtime closure survives. The
        // lambda may be lowered speculatively (registering a lift), but it is UNREACHED by any emitted
        // `Core::Closure`, so it must be emitted as an inert stub (not a dead, ill-formed lifted body)
        // and the program must still fold to a scalar. Regression guard for the reached-lift filter.
        assert_eq!(
            run_main("(let ((add-y (let ((y 3)) (fn (x) (+ x y))))) (let ((y 100)) (add-y 4)))"),
            7
        );
    }

    #[test]
    fn a_higher_order_function_folds() {
        // `(let ((twice (fn (f v) (f (f v))))) (twice (fn (x) (+ x 1)) 5))` = 7 — a function passed as
        // an argument, applied twice; the whole thing β-reduces to `(+ (+ 5 1) 1)` = 7.
        assert_eq!(
            run_main("(let ((twice (fn (f v) (f (f v))))) (twice (fn (x) (+ x 1)) 5))"),
            7
        );
    }

    #[test]
    fn a_directly_recursive_function_declines_quickly() {
        // `(def (loop n) (loop n))` applied — the callee reaches its own body, so the STATIC recursion
        // check declines it before any β-reduction (a recursive function needs runtime specialization,
        // not yet built). It must decline, not hang: the check is structural, so this returns at once.
        let src = "(module m (def (loop n) (loop n)) (def (main) (loop 0)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("recursion must decline")
            .message;
        assert!(
            msg.contains("recursive") || msg.contains("runtime"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_branching_recursive_function_declines_and_does_not_explode() {
        // A recursive body with TWO self-calls (the shape a CBOR tree-reader takes) would explode
        // exponentially in appended nodes if inlined; the static check declines it up front, so this
        // returns immediately rather than hanging. Regression for the 10-bytes.sexp gate timeout.
        let src = "(module m \
            (def (rec n) (+ (rec n) (rec n))) \
            (def (main) (rec 3)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("branching recursion must decline")
            .message;
        assert!(
            msg.contains("recursive") || msg.contains("runtime"),
            "got: {msg}"
        );
    }

    #[test]
    fn an_unproductive_nullary_recursion_declines_not_crashes() {
        // `(def (f) (f))` — a NULLARY self-call with no base case. A nullary def resolves its name to a
        // `Ref` at its body, so the head/record-reduction helpers (`lambda_of`, `reduce_to_record_id`)
        // would re-enter the same body and overflow the native stack. It must DECLINE (a recursive
        // function it cannot specialize), never abort. Regression for the compile-time-recursion crash.
        let src = "(module m (def (f) (f)) (def (main) (f)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("nullary self-recursion must decline")
            .message;
        assert!(
            msg.contains("recursive") || msg.contains("runtime") || msg.contains("deeply"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_pathologically_deep_expression_declines_not_crashes() {
        // A `(+ 1 (+ 1 …))` nest far past the recursive-descent depth bound. The compiler must DECLINE
        // (a resource-limit rejection) rather than overflow its own stack and abort — a compiler
        // completes or declines on well-formed input, never crashes. A shallow nest still folds (the
        // 64-deep corpus case folds to 65); this pins the deep end declines instead of aborting.
        //
        // Run through the SAME host-stack helper the `rcdzc` bin uses: the decline is enforced by the
        // recursive-descent depth guard (`DESCENT_DEPTH_LIMIT`), and reaching that guard needs a stack
        // deeper than a default `cargo test` worker thread's (≈2 MB, which overflows at ~depth 179 in a
        // debug build). `run_with_compiler_stack` sizes the stack FROM the guard, so the semantic guard
        // is what fires here just as it does at the bin — no `RUST_MIN_STACK` to remember. See
        // `rcdzc::host`.
        let msg = crate::host::run_with_compiler_stack(|| {
            let mut body = "1".to_string();
            for _ in 0..4000 {
                body = format!("(+ 1 {body})");
            }
            let src = format!("(module m (def (main) {body}) (export main))");
            compile_component(&crate::codec::encode(&parse(&src)))
                .expect_err("a pathologically deep expression must decline")
                .message
        });
        assert!(
            msg.contains("deeply") || msg.contains("recursion") || msg.contains("resource"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_nested_call_chain_compiles_without_exponential_blowup() {
        // A nested call chain `(f (f (f … 0)))` β-inlines each level; a generation that did not MEMOIZE
        // the reduction and the fault walk re-analyzed each cached-but-shared reduced term per enclosing
        // level — EXPONENTIAL (×2/level; 2^depth when the callee duplicates its parameter). At depth 25 a
        // single-use chain took MINUTES unmemoized; this test would then never finish. Memoized, it folds
        // in milliseconds. The value (25 increments of 0 = 25) triangulates the fold is correct; the
        // POINT is that it TERMINATES quickly — a regression here reappears as a hang, not a wrong value.
        let mut body = "0".to_string();
        for _ in 0..25 {
            body = format!("(f {body})");
        }
        let src = format!("(module m (def (f n) (+ n 1)) (def (main) {body}) (export main))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        assert_eq!(
            run_returns::<i64>(&bytes, "main"),
            25,
            "a depth-25 single-use call chain must fold to 25 (and compile in linear, not exponential, time)"
        );
        // The worse shape: a callee that DUPLICATES its parameter doubles the substituted term per level
        // (2^depth nodes). At depth 12 the value is 1·2^12 = 4096; unmemoized this was already ~0.6s and
        // depth 18 never finished. Memoized it is instant.
        let mut body = "1".to_string();
        for _ in 0..12 {
            body = format!("(g {body})");
        }
        let src = format!("(module m (def (g n) (+ n n)) (def (main) {body}) (export main))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        assert_eq!(
            run_returns::<i64>(&bytes, "main"),
            4096,
            "a depth-12 parameter-duplicating call chain must fold to 4096 without exponential blowup"
        );
    }

    #[test]
    fn mutually_recursive_functions_decline() {
        // `f` calls `g`, `g` calls `f` — neither reaches a normal form. The transitive call-graph walk
        // finds the cycle f→g→f, so applying `f` declines.
        let src = "(module m \
            (def (f n) (g n)) \
            (def (g n) (f n)) \
            (def (main) (f 0)) (export main))";
        let msg = compile_component(&crate::codec::encode(&parse(src)))
            .expect_err("mutual recursion must decline")
            .message;
        assert!(
            msg.contains("recursive") || msg.contains("runtime"),
            "got: {msg}"
        );
    }

    #[test]
    fn a_self_referential_shape_that_is_not_actually_recursive_still_folds() {
        // `(f (f v))` nests the SAME non-recursive function twice — a legitimate terminating fold, NOT
        // recursion. The static check must NOT false-positive on it (the old body-on-stack set did):
        // this still folds to `(+ (+ 5 1) 1)` = 7.
        assert_eq!(
            run_main("(let ((twice (fn (f v) (f (f v))))) (twice (fn (x) (+ x 1)) 5))"),
            7
        );
    }

    #[test]
    fn a_type_value_has_type_type() {
        // A type is a first-class VALUE, so it has a type — `Type`. `Bool` (a ground-type record) and
        // `(Int 64)` (a built module) both type as `Type`; a type used as a value doesn't fall to Any.
        use crate::db::Db;
        use crate::infer::type_of;
        use crate::testkit::parse;
        use crate::ty::Ty;
        // The `Bool` reference in `(def (t) Bool)` — find it and check its type.
        let ast = parse("(module m (def (t) Bool) (def (main) 0) (export main))");
        let mut db = Db::load(ast);
        // Locate the `Bool` occurrence: the body of def `t`.
        let bool_occ = db
            .defs
            .iter()
            .find(|d| d.name == "t")
            .and_then(|d| d.body)
            .expect("def t");
        assert_eq!(type_of(&mut db, bool_occ), Ty::Type);
    }

    #[test]
    fn int_module_is_built_once_and_shared() {
        // `(Int 64)` reduces to the SAME module however many times it is demanded — the build cache
        // makes the reduction idempotent, so the arena does not grow per demand. Demand the same
        // occurrence's module twice and a second `(Int 64)`; the cache holds ONE entry per width.
        use crate::db::Db;
        use crate::eval::{meta_apply_of, reduce_ctor};
        use crate::testkit::parse;
        // Two separate `(Int 64)` applications, plus one used twice.
        let ast = parse(
            "(module m (def (main) (. (Int 64) max)) (def (other) (. (Int 64) min)) (export main))",
        );
        let mut db = Db::load(ast);
        // Find the two `(Int 64)` applications by resolving them; force each module build several times.
        // (We reduce directly via the evaluator to exercise the cache without threading occurrences.)
        let int_prim = db.prelude.get("Int").copied().expect("Int in prelude");
        let p = meta_apply_of(&mut db, int_prim).expect("Int is applyable");
        // Build width 64 three times — all must return the same node, and the cache must have 1 entry.
        let w64 = db.push_atom(crate::ast::Leaf::Int {
            value: crate::ast::IntValue::from_i64(64),
            radix: crate::ast::Radix::Dec,
        });
        // `Int`/`UInt` key their build-once cache on the WIDTH value, not the `origin` occurrence, so
        // any StructId serves as origin here (the width atom itself).
        let a = reduce_ctor(&mut db, p, w64, &[w64]).expect("build");
        let b = reduce_ctor(&mut db, p, w64, &[w64]).expect("build");
        assert_eq!(a, b, "the same width must reduce to the same module node");
        // A different width is a different module.
        let w8 = db.push_atom(crate::ast::Leaf::Int {
            value: crate::ast::IntValue::from_i64(8),
            radix: crate::ast::Radix::Dec,
        });
        let c = reduce_ctor(&mut db, p, w8, &[w8]).expect("build");
        assert_ne!(a, c, "different widths are different modules");
    }
}

// ── value-heap R0: the canonical `list<u8>` ABI (memory + cabi_realloc + list<u8> lift) ────────────
//
// The escape path (a compound crossing to the host) returns the value's CANONICAL BINARY VALUE FORM as
// `list<u8>` (`contracts/deterministic-value-form.md`; the resource `encode()` method's return — R1+).
// R0 is the ABI substrate beneath it: a core module with a linear memory + `cabi_realloc`, and a
// `-> list<u8>` component lift that reads the canonical-ABI `(ptr, len)` return area out of that
// memory. Proven two ways, exactly as the H1b import envelope was: the hand-emitted envelope is
// byte-identical to the `ComponentBuilder` oracle, AND it RUNS under wasmtime, returning the bytes.
//
// The core module here is a hand-built `wasm-encoder` fixture (a nullary entry returning a retptr to
// constant bytes) — R0 proves the ENVELOPE's list-lift byte layer independently of the serializer that
// will emit the real renderer core (R1/R2). This mirrors H1b, whose oracle used `import_oracle_core`.
mod r0 {
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

    /// The hand-emitted `() -> list<u8>` component RUNS under wasmtime and returns the bytes — R0's
    /// behavior gate (the byte-identity check alone can't prove the canonical-ABI `(ptr, len)` return
    /// area is read correctly). Observed by execution: `main()` returns `[1, 2, 3]`.
    #[test]
    fn list_u8_envelope_runs_and_returns_the_bytes() {
        use wasmtime::component::{Component, Linker, Val};
        use wasmtime::{Engine, Store};

        let core = working_list_core();
        let exports = vec![BoundaryExport {
            name: "main".to_string(),
            params: Vec::new(),
            result: BoundaryResult::Bytes,
        }];
        let comp = assemble(&core, &exports, &[], "");

        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid component");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        let func = instance
            .get_func(&mut store, "main")
            .expect("export present");
        let mut results = [Val::Bool(false)];
        func.call(&mut store, &[], &mut results).expect("call");
        func.post_return(&mut store).expect("post_return");
        match &results[0] {
            Val::List(items) => {
                let got: Vec<u8> = items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => *b,
                        other => panic!("list element not u8: {other:?}"),
                    })
                    .collect();
                assert_eq!(got, vec![1, 2, 3], "list<u8> round-trip");
            }
            other => panic!("expected a list result, got {other:?}"),
        }
    }
}

// ── value-heap R1: monomorphized resource + encode() -> list<u8> (the ComponentBuilder reference) ──
//
// The escape path exports a compound as a monomorphized component-model RESOURCE whose `encode() ->
// list<u8>` returns the canonical binary value form. This module builds that shape with the
// authoritative `ComponentBuilder` encoder + RUNS it under wasmtime (make → resource handle → encode
// → bytes), the reference the hand-emitted `envelope.rs` R1 path mirrors — the H1b method (validate a
// ComponentBuilder oracle first, then hand-emit byte-identical). It exercises: the core module imports
// + calls `resource.new` (a raw rep is NOT auto-wrapped by the lift), the resource carries a dtor (the
// rc-handle release, stubbed for the constant-bytes proof), and an inner re-export component publishes
// the abstracted resource + funcs as the `cadenza:run/run` instance.
//
// A LEANER shape than wit-bindgen's: the dtor lives in its OWN core module (importing nothing), so it
// can be instantiated first — the resource type gets a real dtor core-func before `resource.new` (and
// the main module) need the resource type. This dissolves the resource↔dtor↔`resource.new` circular
// dependency WITHOUT the shim/fixup trampoline+table+elem dance wit-bindgen needs (wit-bindgen only
// needs the shim because it puts the dtor in the same module that imports `resource.new`). Simpler to
// hand-emit AND a cleaner integration story (a self-contained dtor unit).
mod r1_reference {
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

    #[test]
    fn resource_encode_oracle_runs() {
        let core = resource_core();
        let comp = oracle_resource_component(&core);

        // Validate with wasmparser (localizes any byte error) then wasmtime.
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&comp)
            .expect("resource component validates under wasmparser");
        // Run it: make() -> resource handle -> encode(handle) -> list<u8> == [1,2,3].
        use wasmtime::component::{Component, Linker, Val};
        use wasmtime::{Engine, Store};
        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid component");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        // The resource + funcs are exported inside the `cadenza:run/run` instance.
        let iface = instance
            .get_export_index(&mut store, None, "cadenza:run/run")
            .expect("run interface");
        let make_idx = instance
            .get_export_index(&mut store, Some(&iface), "make")
            .expect("make export");
        let encode_idx = instance
            .get_export_index(&mut store, Some(&iface), "encode")
            .expect("encode export");
        let make = instance.get_func(&mut store, make_idx).expect("make func");
        let encode = instance
            .get_func(&mut store, encode_idx)
            .expect("encode func");
        // make() → a strongly-typed resource handle (a Val::Resource, NOT a bare u32).
        let mut handle = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle).expect("make call");
        make.post_return(&mut store).expect("make post_return");
        assert!(
            matches!(handle[0], Val::Resource(_)),
            "make must return a resource handle, got {:?}",
            handle[0]
        );
        // encode(handle) → the canonical binary value form as list<u8>.
        let mut out = [Val::Bool(false)];
        encode
            .call(&mut store, &handle, &mut out)
            .expect("encode call");
        encode.post_return(&mut store).expect("encode post_return");
        match &out[0] {
            Val::List(items) => {
                let got: Vec<u8> = items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => *b,
                        o => panic!("not u8: {o:?}"),
                    })
                    .collect();
                assert_eq!(got, vec![1, 2, 3], "encode round-trip");
            }
            o => panic!("expected list, got {o:?}"),
        }
    }

    /// MINIMAL BORROW ISOLATION (R2-dtor diagnosis): the SAME constant-encode R1 shape, but `encode`
    /// takes `self: borrow<t>` instead of `own<t>`. No runtime import, no heap walk — `encode` ignores
    /// its self and returns the constant `[1,2,3]`. This isolates whether a `borrow<t>` self + a host
    /// `resource_drop` afterward is fundamentally sound in this wasmtime, apart from the runtime-walk
    /// machinery. If this RUNS (make → encode(borrow) → [1,2,3] → drop → ok), borrow is sound at the
    /// envelope level and the runtime-walk trap is elsewhere (resource.rep-on-borrow in the walker, or
    /// the borrow-lend scope across the composed heap instance). If it TRAPS, borrow+this resource shape
    /// is the problem.
    fn oracle_resource_component_borrow(core: &[u8]) -> Vec<u8> {
        use wasm_encoder::*;
        let mut c = ComponentBuilder::default();
        let dtor_idx = c.core_module_raw(&dtor_module());
        let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
        let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
        let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
        let rnew_core = c.resource_new(res_ty);
        let heap_inst = c.core_instantiate_exports([("resource-new", ExportKind::Func, rnew_core)]);
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
        // encode BORROWS self.
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

    /// The inner re-export component with `encode` re-typed against a `borrow<t>` self.
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

    /// Like [`resource_core`] but `encode` CALLS `resource.rep(self)` (imported as `heap.resource-rep`)
    /// before returning the constant retptr — isolating whether `resource.rep` on a BORROWED self traps
    /// (the walker's very first encode instruction). `make` uses a dummy rep 7. Imports: resource-new
    /// (func 0), resource-rep (func 1); defined make=2, encode=3, realloc=4.
    fn resource_core_rep() -> Vec<u8> {
        use wasm_encoder::*;
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0: (i32)->i32
        types.ty().function(vec![], vec![ValType::I32]); // 1: make ()->i32
        types.ty().function(
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
        ); // 2: cabi_realloc
        m.section(&types);
        let mut imports = ImportSection::new();
        imports.import("heap", "resource-new", EntityType::Function(0));
        imports.import("heap", "resource-rep", EntityType::Function(0));
        m.section(&imports);
        // import funcs: resource-new=0, resource-rep=1; defined: make=2, encode=3, realloc=4.
        let mut funcs = FunctionSection::new();
        funcs.function(1); // make ()->i32
        funcs.function(0); // encode (i32)->i32
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
        exports.export("make", ExportKind::Func, 2);
        exports.export("t-encode", ExportKind::Func, 3);
        exports.export("cabi_realloc", ExportKind::Func, 4);
        m.section(&exports);
        let mut data = DataSection::new();
        let bytes = [1u8, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0];
        data.active(0, &ConstExpr::i32_const(0), bytes.iter().copied());
        let mut code = CodeSection::new();
        let mut make = Function::new(vec![]);
        make.instruction(&Instruction::I32Const(7));
        make.instruction(&Instruction::Call(0)); // resource-new
        make.instruction(&Instruction::End);
        code.function(&make);
        // encode(self): call resource.rep(self), DROP the result, return the constant retptr.
        let mut encode = Function::new(vec![]);
        encode.instruction(&Instruction::LocalGet(0));
        encode.instruction(&Instruction::Call(1)); // resource-rep(self) → rep
        encode.instruction(&Instruction::Drop); // ignore the rep value
        encode.instruction(&Instruction::I32Const(8)); // retptr
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

    /// Borrow oracle whose program core CALLS `resource.rep(self)` in encode — threads `resource.rep`
    /// into the `heap` instance too. Isolates `resource.rep`-on-borrow.
    fn oracle_resource_component_borrow_rep(core: &[u8]) -> Vec<u8> {
        use wasm_encoder::*;
        let mut c = ComponentBuilder::default();
        let dtor_idx = c.core_module_raw(&dtor_module());
        let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
        let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
        let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
        let rnew_core = c.resource_new(res_ty);
        let rrep_core = c.resource_rep(res_ty);
        let heap_inst = c.core_instantiate_exports([
            ("resource-new", ExportKind::Func, rnew_core),
            ("resource-rep", ExportKind::Func, rrep_core),
        ]);
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

    #[test]
    fn borrow_encode_calls_resource_rep_isolation() {
        // encode calls resource.rep(borrow self) then returns [1,2,3]. If this TRAPS, resource.rep on a
        // borrow is the culprit; if it RUNS, the trap is in the arr-get heap walk (or its borrow-lend
        // scope), not resource.rep.
        let core = resource_core_rep();
        let comp = oracle_resource_component_borrow_rep(&core);
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator.validate_all(&comp).expect("validates");
        use wasmtime::component::{Component, Linker, Val};
        use wasmtime::{Engine, Store};
        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker.instantiate(&mut store, &component).expect("inst");
        let iface = instance
            .get_export_index(&mut store, None, "cadenza:run/run")
            .expect("iface");
        let make_idx = instance
            .get_export_index(&mut store, Some(&iface), "make")
            .expect("make");
        let encode_idx = instance
            .get_export_index(&mut store, Some(&iface), "encode")
            .expect("encode");
        let make = instance.get_func(&mut store, make_idx).expect("make fn");
        let encode = instance.get_func(&mut store, encode_idx).expect("enc fn");
        let mut handle = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle).expect("make");
        make.post_return(&mut store).expect("make post");
        let mut out = [Val::Bool(false)];
        let r = encode.call(&mut store, &handle, &mut out);
        match r {
            Ok(()) => {
                eprintln!("PROBE resource.rep-on-borrow → OK (rep call fine; trap is in the walk)")
            }
            Err(e) => eprintln!("PROBE resource.rep-on-borrow → TRAP: {e}"),
        }
    }

    #[test]
    fn borrow_encode_minimal_isolation_runs() {
        let core = resource_core();
        let comp = oracle_resource_component_borrow(&core);
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&comp)
            .expect("borrow resource component validates");
        use wasmtime::component::{Component, Linker, ResourceAny, Val};
        use wasmtime::{Engine, Store};
        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid component");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        let iface = instance
            .get_export_index(&mut store, None, "cadenza:run/run")
            .expect("run interface");
        let make_idx = instance
            .get_export_index(&mut store, Some(&iface), "make")
            .expect("make export");
        let encode_idx = instance
            .get_export_index(&mut store, Some(&iface), "encode")
            .expect("encode export");
        let make = instance.get_func(&mut store, make_idx).expect("make func");
        let encode = instance
            .get_func(&mut store, encode_idx)
            .expect("encode func");
        let mut handle = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle).expect("make call");
        make.post_return(&mut store).expect("make post_return");
        let mut out = [Val::Bool(false)];
        // encode BORROWS — the host keeps the handle.
        encode
            .call(&mut store, &handle, &mut out)
            .expect("encode(borrow) call");
        encode.post_return(&mut store).expect("encode post_return");
        match &out[0] {
            Val::List(items) => {
                let got: Vec<u8> = items
                    .iter()
                    .map(|v| match v {
                        Val::U8(b) => *b,
                        o => panic!("not u8: {o:?}"),
                    })
                    .collect();
                assert_eq!(got, vec![1, 2, 3], "borrow encode returns the constant");
            }
            o => panic!("expected list, got {o:?}"),
        }
        // Now DROP the still-owned handle → the dtor fires.
        if let Val::Resource(r) = handle[0] {
            let r: ResourceAny = r;
            r.resource_drop(&mut store)
                .expect("resource drop after borrow");
        } else {
            panic!("make did not return a resource");
        }
    }
}

// ── value-heap R1c/R3: a COMPILER-GENERATED constant-compound resource escape, run + decoded ──────
//
// The compiler routes a nullary export returning a fully-CONSTANT compound through the resource escape
// path (`emit` → `serialize::resource_core_module` with the baked `constant_value_form` bytes →
// `envelope::assemble_resource`). This exercises the WHOLE pipeline the corpus's constant tuple/record
// returns need: the value is serialized to its canonical binary form AT COMPILE TIME, crosses as the
// resource `encode()`'s `list<u8>`, and the host DECODES it with the same codec + prints the recorded
// `(: value type)` text. Constant-first (R1): no runtime heap construction, so it runs under a bare
// linker (no value-heap runtime composed) — the runtime path (R2) reuses this envelope + host wiring.
mod constant_resource_escape {
    use crate::compile::compile_component;
    use crate::testkit::parse;

    /// Compile `src`, run its resource export under wasmtime, and DECODE the `encode()` bytes back to
    /// canonical text — the value the host would print. Returns the decoded `(: value type)` string.
    fn run_and_decode(src: &str) -> String {
        use wasmtime::component::{Component, Linker, Val};
        use wasmtime::{Engine, Store};
        let comp = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        // Validate then run under a bare linker — a constant escape composes no runtime.
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&comp)
            .expect("resource component validates");
        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid component");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        let iface = instance
            .get_export_index(&mut store, None, "cadenza:run/run")
            .expect("run interface");
        let make_idx = instance
            .get_export_index(&mut store, Some(&iface), "make")
            .expect("make export");
        let encode_idx = instance
            .get_export_index(&mut store, Some(&iface), "encode")
            .expect("encode export");
        let make = instance.get_func(&mut store, make_idx).expect("make func");
        let encode = instance
            .get_func(&mut store, encode_idx)
            .expect("encode func");
        let mut handle = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle).expect("make call");
        make.post_return(&mut store).expect("make post_return");
        let mut out = [Val::Bool(false)];
        encode
            .call(&mut store, &handle, &mut out)
            .expect("encode call");
        encode.post_return(&mut store).expect("encode post_return");
        let bytes: Vec<u8> = match &out[0] {
            Val::List(items) => items
                .iter()
                .map(|v| match v {
                    Val::U8(b) => *b,
                    o => panic!("not u8: {o:?}"),
                })
                .collect(),
            o => panic!("expected list<u8>, got {o:?}"),
        };
        // Decode with `cadenza-syntax`'s codec — the SAME codec `cdz-run` (the host) uses; rcdzc's own
        // `codec` is a byte-identical COPY (the copy-don't-depend rule), so the bytes the compiler baked
        // decode here through the front-end crate exactly as the host will. Then print canonical text.
        let arenas = cadenza_syntax::codec::decode(&bytes).expect("decode canonical value form");
        cadenza_syntax::sexpr::print(&arenas).trim().to_string()
    }

    /// A constant tuple returned as the program result crosses as a resource and decodes to the exact
    /// corpus text — the `(def (main) (tuple 3 1))` case (`spec/semantics/05-compound-types.sexp`).
    #[test]
    fn a_constant_tuple_return_decodes_to_its_value_form() {
        let src = "(module m (def (main) (tuple 3 1)) (export main))";
        assert_eq!(run_and_decode(src), "(: (tuple 3 1) (Tuple Int64 Int64))");
    }

    /// A constant tuple with a Bool element — the heap layout carries a Bool beside an Int64.
    #[test]
    fn a_constant_mixed_tuple_return_decodes() {
        let src = "(module m (def (main) (tuple 0 true)) (export main))";
        assert_eq!(run_and_decode(src), "(: (tuple 0 true) (Tuple Int64 Bool))");
    }

    /// A nested constant tuple — a compound element recurses in both the value and the type.
    #[test]
    fn a_constant_nested_tuple_return_decodes() {
        let src = "(module m (def (main) (tuple 2 (tuple 2 2))) (export main))";
        assert_eq!(
            run_and_decode(src),
            "(: (tuple 2 (tuple 2 2)) (Tuple Int64 (Tuple Int64 Int64)))"
        );
    }

    /// A constant record — the value head is lowercase `record`, the type head capital `Record`, fields
    /// in canonical (key-sorted) order.
    #[test]
    fn a_constant_record_return_decodes() {
        let src = "(module m (def (main) (record (a 3) (b 1))) (export main))";
        assert_eq!(
            run_and_decode(src),
            "(: (record (a 3) (b 1)) (Record (a Int64) (b Int64)))"
        );
    }
}

// ── value-heap R2: a RUNTIME compound escapes — the handle-walking encode() (ComponentBuilder ref) ──
//
// R1 proved a CONSTANT compound crosses (its bytes baked at compile time). R2 is the real thing: a
// compound BUILT ON THE HEAP at run time (`arr-alloc`/`box-int`/`arr-set`) crosses as a resource whose
// `encode()` WALKS the live handle (`arr-get`/`get-int`/`get-bool`) and writes the canonical value
// bytes. This module builds that shape with `ComponentBuilder` (the authoritative encoder) + RUNS it
// COMPOSED WITH THE REAL RUNTIME — the reference the hand-emitted combined envelope + walker will
// mirror (the H1b method). It is the FIRST end-to-end exercise of the genuine heap path: every corpus
// runtime-compound case so far passes by CONSTANT-FOLDING (they run with no runtime present, because a
// nullary `main` inlines its call), so the alloc→escape→walk pipeline has never actually run until now.
//
// The encode() strategy is R2 step 1's template made concrete: the value-form byte TEMPLATE is a data
// segment doubling as the OUTPUT buffer (every leaf is a runtime hole that `encode` fully overwrites,
// and the non-hole framing bytes are constant, so no copy is needed). CRUCIAL (empirically, R2): the
// canonical ABI hands `encode` the resource-table HANDLE, NOT the heap rep — so `encode` FIRST calls
// `resource.rep(handle) -> rep` to recover the walkable heap rep (a guest resource's `own<t>` param
// crosses as a table index; `arr-get` on it traps because the small index is misread as an inline
// value). It then walks each hole's `arr-get` path from `rep`, reads the leaf (`get-int`/`get-bool`),
// and writes its bytes into the template at the hole's offset (an Int as 8 big-endian magnitude bytes
// + the NEG kind byte for a negative; a Bool as the 8/9 kind byte).
mod r2_runtime_resource {
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
        })
    }

    /// The core functype `(params)->(result?)` of a runtime op — its CORE valtypes (a u32 handle is i32,
    /// s64 is i64), the shape the program core module imports it under.
    fn op_core_functype(op: &RtOp) -> (Vec<wasm_encoder::ValType>, Vec<wasm_encoder::ValType>) {
        use wasm_encoder::ValType;
        let core = |p: AbiValType| match p {
            AbiValType::U32 | AbiValType::Bool => ValType::I32,
            AbiValType::S64 => ValType::I64,
            AbiValType::F64 => ValType::F64,
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
    /// exports the resource directly + the funcs against it). Reused so the combined shape publishes
    /// `cadenza:run/run` the same way.
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

        // (5) lift make/encode against the internal resource type (both `own<t>` self for now — the
        // `borrow<t>` migration is the R2-dtor follow-up).
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

    /// Drive the combined component through the HOST (`cdz_run::run`), composing the real value-heap
    /// runtime — the same path `cdz-run` takes for a resource-escape program. Returns the decoded
    /// `(: value type)` text (or skips if the runtime wasm is not built). The import name carries the
    /// runtime's content hash so the host composes it.
    fn run_composed(core: &[u8]) -> Option<String> {
        use crate::backend::wasm::runtime_abi::{REQUIRED_RUNTIME_HASH, RUNTIME_IFACE};
        let import_name = format!("{RUNTIME_IFACE}@0.0.0+{REQUIRED_RUNTIME_HASH}");
        let comp = oracle_runtime_resource_component(core, &import_name);
        // Validate structurally first (localizes any byte/index error before wasmtime).
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&comp)
            .expect("combined runtime-resource component validates");
        let runtime = super::find_runtime_wasm()?;
        let opts = cdz_run::RunOpts {
            export: None, // no bare func export → the host takes the resource-escape path
            args: vec![],
            runtime: Some(runtime),
            runtime_cache_dir: None,
            host_responses: Vec::new(),
        };
        match cdz_run::run(&comp, &opts).expect("run composed") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("composed runtime-resource run trapped: {t}"),
        }
    }

    #[test]
    fn a_flat_runtime_tuple_walks_and_crosses() {
        // Build `(tuple 3 1)` on the value heap, escape it as a resource, walk it in encode(), decode →
        // the exact corpus value form. The FIRST genuine heap-alloc→escape→walk round-trip.
        let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
        let tpl = runtime_value_form_template(&ty).expect("template");
        let core = walker_core(&tpl, &[3, 1]);
        if let Some(text) = run_composed(&core) {
            assert_eq!(text, "(: (tuple 3 1) (Tuple Int64 Int64))");
        } else {
            eprintln!("[R2] runtime wasm not found; skipping composed walk");
        }
    }

    #[test]
    fn a_runtime_tuple_with_a_negative_element_walks() {
        // A negative element exercises the NEG kind-byte flip + absolute-magnitude write in the walker.
        let ty = Ty::Tuple(vec![Ty::int64(), Ty::int64()].into());
        let tpl = runtime_value_form_template(&ty).expect("template");
        let core = walker_core(&tpl, &[-5, 7]);
        if let Some(text) = run_composed(&core) {
            assert_eq!(text, "(: (tuple -5 7) (Tuple Int64 Int64))");
        } else {
            eprintln!("[R2] runtime wasm not found; skipping composed walk");
        }
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
        let tpl = runtime_value_form_template(&ty).expect("template");
        let core = walker_core(&tpl, &[3, 1]);
        let dtor = dtor_module();
        let import_name = "cadenza:runtime/heap@0.0.0+deadbeef";
        let ops: Vec<&RtOp> = walker_ops().to_vec();
        let ours = assemble_runtime_resource(&core, &dtor, &ops, import_name);
        let oracle = oracle_runtime_resource_component(&core, import_name);
        assert_eq!(
            ours, oracle,
            "combined envelope mismatch vs ComponentBuilder"
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
}

/// Micro-benchmarks for compiler THROUGHPUT — not correctness gates, so every one is `#[ignore]`d
/// (they never run in the normal `cargo test` suite or the behavior gate). Run explicitly with
/// `cargo test -p rcdzc --profile release-debug scope_resolution -- --ignored --nocapture`.
///
/// The subject is scope resolution on a deep sequential `let`: `(let ((a0 0)(a1 a0)(a2 a1)…) aN)`.
/// Each initializer reference ascends into the bindings-list (`resolve::binder_in` Case 2) and the
/// body reference sees the whole list (Case 1). The pre-optimization resolver materialized a
/// `Vec<(StructId,StructId)>` of ALL pairs on every one of the N lookups and re-scanned the sibling
/// list with `position()` to find the ascent window — O(N²) with N allocations. The optimization
/// (an O(1) `child_ix` window bound + an allocation-free reverse in-place scan) makes each lookup
/// O(distance-to-binder), so this chain — where each reference's binder is the immediately preceding
/// pair — resolves in O(N). Printing wall-time and time-per-binding across sizes makes the change
/// from super-linear to linear scaling visible.
#[cfg(test)]
mod bench {
    use crate::compile::compile_component;
    use crate::testkit::parse;
    use std::time::Instant;

    /// A `(module m (def (main) (let ((a0 0)(a1 a0)…(a{n-1} a{n-2})) a{n-1})) (export main))` program:
    /// a chain of `n` bindings each aliasing the previous, all constant so the whole thing folds and
    /// compiles end-to-end (the resolver is fully exercised — `n` distinct reference nodes, each
    /// ascending into the growing bindings-list — with no decline short-circuiting the run).
    fn deep_let_chain(n: usize) -> String {
        let mut binds = String::from("(a0 0)");
        for i in 1..n {
            binds.push_str(&format!("(a{i} a{})", i - 1));
        }
        format!(
            "(module m (def (main) (let ({binds}) a{})) (export main))",
            n - 1
        )
    }

    #[test]
    #[ignore = "benchmark, not a correctness gate — run with --ignored --nocapture"]
    fn scope_resolution_deep_let_chain() {
        // A few reads per size, keep the best (min) to shed scheduler noise. The absolute numbers are
        // machine-dependent; the SHAPE across N is the signal — a per-binding time that stays flat is
        // O(N) resolution, one that climbs with N is the old O(N²).
        // Sizes stay under the compiler's recursive-descent depth bound (a deeper aliasing chain
        // DECLINES at the fold/inference depth backstop — orthogonal to scope resolution); a size that
        // declines is SKIPPED, not counted, so the bench never conflates that ceiling with the timing.
        println!("\n  deep sequential let-chain — full compile_component wall time");
        println!(
            "  {:>7}  {:>12}  {:>14}",
            "N", "total (ms)", "per-bind (µs)"
        );
        for &n in &[100usize, 250, 500, 750, 1000] {
            let src = deep_let_chain(n);
            let bytes = crate::codec::encode(&parse(&src));
            // Warm once (build the arena caches); skip this size if the depth bound declines it.
            if compile_component(&bytes).is_err() {
                println!("  {n:>7}  {:>12}  {:>14}", "(declined)", "-");
                continue;
            }
            let mut best = f64::INFINITY;
            for _ in 0..5 {
                let t0 = Instant::now();
                let out = compile_component(&bytes);
                let ms = t0.elapsed().as_secs_f64() * 1e3;
                out.expect("bench program compiles");
                best = best.min(ms);
            }
            println!(
                "  {:>7}  {:>12.3}  {:>14.3}",
                n,
                best,
                best * 1e3 / n as f64
            );
        }
        println!();
    }
}

// ── the sidecar request list: driving a compilation as Emit + Query requests ─────────────────────
//
// The sidecar generalizes `compile`'s `targets` into a request list crossing as one more kinded INPUT
// artifact. These drive the WHOLE `compile()` path (not the unit codec tests in `sidecar.rs`): a
// request list in, artifacts + diagnostics out, selected by kind. They pin the load-bearing claims of
// `DESIGN-sidecar-api.md`: a Query is a total, pure fact read (answers even when emit fails, never
// denies a component), an Emit request is today's `Target` reached through the list, and the no-sidecar
// path is unchanged.
mod sidecar_driven {

    use crate::abi::{Artifact, Severity};
    use crate::backend::Target;
    use crate::compile::compile;
    use crate::sidecar::{
        self, KIND_DIAGNOSTICS, KIND_EXPORTS, KIND_RESOLVE, KIND_SCOPE, KIND_TYPE_AT,
        KIND_TYPE_INFO, KIND_USES, Query, Request,
    };
    use crate::testkit::parse;

    /// Build the two input artifacts (the AST + a sidecar request list) for `src` and `requests`.
    fn inputs(src: &str, requests: &[Request]) -> Vec<Artifact> {
        vec![
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
            Artifact::new(sidecar::KIND_SIDECAR, "drive", sidecar::encode(requests)),
        ]
    }

    /// The text bytes of the first artifact of `kind`, as a `String`.
    fn artifact_text(out: &crate::abi::CompileOutput, kind: &str) -> Option<String> {
        out.artifacts
            .iter()
            .find(|a| a.kind == kind)
            .map(|a| String::from_utf8(a.bytes.clone()).unwrap())
    }

    #[test]
    fn a_type_of_query_reads_the_type_column() {
        // A `TypeOf` request for a nullary def answers with its rendered type — the same canonical text
        // an annotation carries — read straight from the type column.
        let src = "(module m (def (main) (: 42 Int64)) (export main))";
        let out = compile(
            &inputs(
                src,
                &[Request::Query(Query::TypeOf {
                    name: "main".into(),
                })],
            ),
            &[],
        );
        assert!(
            !out.has_error(),
            "a query does not fail: {:?}",
            out.diagnostics
        );
        assert_eq!(
            artifact_text(&out, KIND_TYPE_INFO).as_deref(),
            Some("Int64")
        );
    }

    #[test]
    fn a_type_of_query_for_a_function_renders_its_arrow_type() {
        // A def with a parameter denotes a function; its type is the arrow the annotation fixes.
        let src = "(module m (def (f (: x Int64)) x) (def (main) (f 1)) (export main))";
        let out = compile(
            &inputs(src, &[Request::Query(Query::TypeOf { name: "f".into() })]),
            &[],
        );
        assert_eq!(
            artifact_text(&out, KIND_TYPE_INFO).as_deref(),
            Some("(-> Int64 Int64)")
        );
    }

    #[test]
    fn a_type_of_query_for_an_unknown_name_is_total() {
        // Querying a name that names no definition yields a DEFINED result, never an error — the
        // oracle contract (a query is total over every input).
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &inputs(
                src,
                &[Request::Query(Query::TypeOf {
                    name: "ghost".into(),
                })],
            ),
            &[],
        );
        assert!(!out.has_error());
        assert_eq!(
            artifact_text(&out, KIND_TYPE_INFO).as_deref(),
            Some("no such definition `ghost`")
        );
    }

    #[test]
    fn a_query_answers_even_when_the_program_fails_to_emit() {
        // The program is ill-typed (`(if 5 1 2)` — a non-Bool condition, CDZ0203), so no component is
        // produced. But a `TypeOf` query is a PURE fact read that never denies an artifact: the answer
        // rides ALONGSIDE the error diagnostic. This is the "branch on a fact even for a broken program"
        // affordance — the whole reason a query is not gated on a clean emit.
        let src = "(module m (def (g) (: 7 Int64)) (def (main) (if 5 1 2)) (export main))";
        let out = compile(
            &inputs(
                src,
                &[
                    Request::Query(Query::TypeOf { name: "g".into() }),
                    Request::Emit(Target::Wasm),
                ],
            ),
            &[],
        );
        // Emit failed: an error diagnostic, and NO component artifact.
        assert!(out.has_error());
        assert!(out.artifact("component").is_none());
        // …but the query still answered, carried past the failure.
        assert_eq!(
            artifact_text(&out, KIND_TYPE_INFO).as_deref(),
            Some("Int64")
        );
    }

    #[test]
    fn a_uses_of_query_finds_every_reference_and_excludes_the_definition() {
        // `helper` is referenced twice (in `main` and in `other`); the query returns those occurrences
        // as node indices in ascending order, and the definition itself is not a use.
        let src = "(module m \
                   (def (helper) 1) \
                   (def (main) (+ helper helper)) \
                   (def (other) helper) \
                   (export main))";
        let out = compile(
            &inputs(
                src,
                &[Request::Query(Query::UsesOf {
                    name: "helper".into(),
                })],
            ),
            &[],
        );
        assert!(!out.has_error());
        let text = artifact_text(&out, KIND_USES).expect("a uses artifact");
        let ids: Vec<u32> = text.lines().map(|l| l.parse().unwrap()).collect();
        // Three references to `helper`, none of them the def's body.
        assert_eq!(ids.len(), 3, "uses = {ids:?}");
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "ascending order: {ids:?}"
        );
        // Each reported node resolves to `helper` — spot-check via a fresh resolve.
        let mut db = crate::db::Db::load(parse(src));
        let helper_body = db.defs[db.def_by_name("helper").unwrap()].body.unwrap();
        for &id in &ids {
            match crate::resolve::resolved_of(&mut db, crate::ast::StructId(id)) {
                crate::resolved::Resolved::Ref { value } => {
                    assert_eq!(value, helper_body, "node {id} must reference helper")
                }
                other => panic!("node {id} resolved to {other:?}, not a Ref to helper"),
            }
        }
    }

    #[test]
    fn a_uses_of_query_for_an_unused_or_unknown_name_is_empty() {
        // A name with no references (or no such definition) yields an empty list, not an error.
        let src = "(module m (def (lonely) 1) (def (main) 42) (export main))";
        let out = compile(
            &inputs(
                src,
                &[Request::Query(Query::UsesOf {
                    name: "lonely".into(),
                })],
            ),
            &[],
        );
        assert!(!out.has_error());
        assert_eq!(artifact_text(&out, KIND_USES).as_deref(), Some(""));
    }

    #[test]
    fn an_emit_request_is_a_target_reached_through_the_list() {
        // An `Emit(Wasm)` request produces the component exactly as `targets: [Wasm]` does — Emit IS
        // the generalization of a Target. Here `targets` is EMPTY; the component comes only from the
        // sidecar request, proving the two paths are one.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(&inputs(src, &[Request::Emit(Target::Wasm)]), &[]);
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        assert!(
            out.artifact("component").is_some(),
            "a component is produced"
        );
    }

    #[test]
    fn emit_and_query_compose_in_one_run() {
        // A realistic driver: build the component AND ask a fact in one invocation. Both artifacts come
        // back, selected by kind — one kinded-artifact list, not two calls.
        let src = "(module m (def (main) (: 42 Int64)) (export main))";
        let out = compile(
            &inputs(
                src,
                &[
                    Request::Emit(Target::Wasm),
                    Request::Query(Query::TypeOf {
                        name: "main".into(),
                    }),
                ],
            ),
            &[],
        );
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        assert!(out.artifact("component").is_some());
        assert_eq!(
            artifact_text(&out, KIND_TYPE_INFO).as_deref(),
            Some("Int64")
        );
    }

    #[test]
    fn no_sidecar_input_is_todays_behavior() {
        // The common path: no `sidecar` artifact at all. Behavior is exactly today's — `targets` drives
        // emission, one component out, no query artifacts.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &[Artifact::new(
                Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::Wasm],
        );
        assert!(!out.has_error());
        assert_eq!(out.artifacts.len(), 1, "exactly the component");
        assert_eq!(out.artifacts[0].kind, "component");
    }

    #[test]
    fn a_malformed_sidecar_list_declines() {
        // A `sidecar` artifact whose bytes are not a valid request list is a DECLINE (a diagnostic),
        // never a panic or a silent drop — reject-don't-miscompile at the tool edge.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
                Artifact::new(sidecar::KIND_SIDECAR, "drive", vec![0xff, 0xff, 0xff]),
            ],
            &[Target::Wasm],
        );
        assert!(out.has_error());
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error)
            .unwrap();
        assert!(
            d.message.contains("malformed `sidecar`"),
            "message: {}",
            d.message
        );
        // No component: the request list could not be understood, so nothing was driven.
        assert!(out.artifact("component").is_none());
    }

    #[test]
    fn a_type_at_query_types_the_node_at_a_source_offset() {
        // The "type at cursor" query: the CONSUMER resolves a source offset to the innermost node id
        // (via the span table it holds), then asks `TypeAt { node }`. This proves the split — offset→node
        // at the boundary (span-owning), node→type in the compiler (span-free). Here the literal `42` is
        // annotated Int64; hovering it yields `Int64`.
        let src = "(module m (def (main) (: 42 Int64)) (export main))";
        // The consumer parses WITH spans and maps the offset of `42` to its node. Use `read_spanned`
        // (a SINGLE top-level form stays bare) — the same root convention the real `cdz` CLI uses; the
        // whole-program `read_all_spanned` would wrap a lone `(module …)` in `(do …)`, and the
        // compiler's top-level scan would then miss the module's defs. The AST crosses to the compiler
        // as BYTES (the copy-don't-depend bridge): `cadenza_syntax`'s codec produces the byte-identical
        // form `rcdzc::codec::decode` reads, so the `StructId`s line up — which is exactly what lets the
        // span-resolved node id name the same node inside the compiler.
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse with spans");
        // Hover the `42` literal — the innermost node there is the literal itself, whose width the
        // annotation `(: 42 Int64)` pins to Int64 (the type column carries the SOLVED type, not the
        // bare-literal deferred one). `node_at_offset` maps the offset to that literal node.
        let off = src.find("42").expect("the literal is in the source");
        let node = spans
            .node_at_offset(off)
            .expect("a node at the literal offset");
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
                ),
            ],
            &[],
        );
        assert!(
            !out.has_error(),
            "a query does not fail: {:?}",
            out.diagnostics
        );
        assert_eq!(artifact_text(&out, KIND_TYPE_AT).as_deref(), Some("Int64"));
    }

    #[test]
    fn a_type_at_query_for_a_non_user_node_is_total() {
        // A node id past the program is not a user node — a DEFINED "unknown", never a crash (the query
        // is total, guarding a malformed request the span table would never actually produce).
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &inputs(src, &[Request::Query(Query::TypeAt { node: 100_000 })]),
            &[],
        );
        assert!(!out.has_error());
        assert_eq!(
            artifact_text(&out, KIND_TYPE_AT).as_deref(),
            Some("unknown")
        );
    }

    /// Hover text for the node at `substr`'s first occurrence in `src` (the TypeAt query answer).
    fn hover_at(src: &str, substr: &str) -> String {
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
        let off = src.find(substr).expect("substr in source");
        let node = spans.node_at_offset(off).expect("a node at the offset");
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
                ),
            ],
            &[],
        );
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        artifact_text(&out, KIND_TYPE_AT).expect("a type-at artifact")
    }

    #[test]
    fn hover_on_a_grammar_keyword_names_the_keyword_not_any() {
        // A grammar keyword (`def`/`export`/`module`/`:`) is syntax, not an expression — hover names it
        // rather than returning the misleading `Any` fallback.
        let src = "(module m (def (main) (: 42 Int64)) (export main))";
        assert_eq!(hover_at(src, "def"), "keyword def");
        assert_eq!(hover_at(src, "export"), "keyword export");
        assert_eq!(hover_at(src, "module"), "keyword module");
        assert_eq!(hover_at(src, ": 42"), "keyword :");
    }

    #[test]
    fn hover_on_a_definition_shows_its_signature() {
        // Hovering a function's NAME or its `(def …)` form shows the SIGNATURE (the full arrow), not the
        // body's return type alone. A nullary def shows `name : T`.
        let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (g) (: 5 Int64)) (export main))";
        // The function name and the whole def form both read as the signature.
        assert_eq!(hover_at(src, "inc"), "inc : (-> Int64 Int64)");
        assert_eq!(hover_at(src, "(def (inc"), "inc : (-> Int64 Int64)");
        // A nullary def reads `name : T`.
        assert_eq!(hover_at(src, "(g)"), "g : Int64");
    }

    #[test]
    fn hover_on_a_reference_shows_the_value_type_not_the_signature() {
        // A USE of a name is a value — it hovers as the value's type. (Only the DEFINITION shows the
        // `name : sig` form; a reference to a nullary def shows the value it denotes.)
        let src = "(module m (def (v) (: 7 Int64)) (def (main) v) (export main))";
        // The `v` reference in `(def (main) v)` — the LAST occurrence.
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
        let off = src.rfind('v').expect("the v reference");
        let node = spans.node_at_offset(off).unwrap();
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::TypeAt { node: node.0 })]),
                ),
            ],
            &[],
        );
        assert_eq!(artifact_text(&out, KIND_TYPE_AT).as_deref(), Some("Int64"));
    }

    #[test]
    fn hover_on_an_operator_does_not_leak_a_record() {
        // A prelude operator name (`+`) resolves to an internal record; hover must not leak
        // `(record (apply …) (t …))` — it surfaces the callable arrow instead.
        let src = "(module m (def (main) (+ 1 2)) (export main))";
        let h = hover_at(src, "+");
        assert!(!h.starts_with("(record"), "no record leak: {h}");
        assert!(h.starts_with("(->"), "an arrow/callable: {h}");
    }

    #[test]
    fn a_diagnostics_query_reports_faults_without_an_export() {
        // The "diagnostics as you type" primitive: an ill-typed program with NO export still yields its
        // faults (the query is not gated on layout/export). `(if 5 1 2)` — a non-Bool condition — is a
        // CDZ0203, reported as `severity<TAB>code<TAB>node-id<TAB>message`.
        let src = "(module m (def (main) (if 5 1 2)))"; // note: NO (export …)
        let out = compile(&inputs(src, &[Request::Query(Query::Diagnostics)]), &[]);
        // A query never fails the compile; the diagnostics ride in the artifact, not the error channel.
        assert!(
            !out.has_error(),
            "a query does not fail: {:?}",
            out.diagnostics
        );
        let text = artifact_text(&out, KIND_DIAGNOSTICS).expect("a diagnostics artifact");
        let line = text
            .lines()
            .find(|l| l.contains("CDZ0203"))
            .unwrap_or_else(|| panic!("expected a CDZ0203 fault, got:\n{text}"));
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols[0], "error", "severity column");
        assert_eq!(cols[1], "CDZ0203", "code column");
        assert!(
            cols[2].parse::<u32>().is_ok(),
            "node-id column is a number: {:?}",
            cols[2]
        );
        assert!(!cols[3].is_empty(), "message column is non-empty");
    }

    #[test]
    fn a_diagnostics_query_on_a_clean_program_is_empty() {
        // A well-formed program yields the empty diagnostics result (total — no faults, no error).
        let src = "(module m (def (main) (: 42 Int64)) (export main))";
        let out = compile(&inputs(src, &[Request::Query(Query::Diagnostics)]), &[]);
        assert!(!out.has_error());
        assert_eq!(artifact_text(&out, KIND_DIAGNOSTICS).as_deref(), Some(""));
    }

    #[test]
    fn a_resolve_of_query_finds_the_defining_occurrence() {
        // Go-to-definition: a reference to `helper` resolves to `helper`'s def body. The consumer maps
        // an offset to the reference node; ResolveOf answers the DEFINING occurrence's node id.
        let src = "(module m (def (helper) 1) (def (main) helper) (export main))";
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse with spans");
        // The reference is the `helper` in `(def (main) helper)` — the LAST occurrence of "helper".
        let ref_off = src.rfind("helper").expect("a reference to helper");
        let ref_node = spans
            .node_at_offset(ref_off)
            .expect("a node at the reference");
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::ResolveOf { node: ref_node.0 })]),
                ),
            ],
            &[],
        );
        assert!(!out.has_error());
        let target: u32 = artifact_text(&out, KIND_RESOLVE)
            .expect("a resolve artifact")
            .trim()
            .parse()
            .expect("a node id");
        // The target is helper's def body — spot-check via a fresh resolve (the same occurrence).
        let db = crate::db::Db::load(parse(src));
        let helper_body = db.defs[db.def_by_name("helper").unwrap()].body.unwrap();
        assert_eq!(
            crate::ast::StructId(target),
            helper_body,
            "ResolveOf points at helper's def body"
        );
    }

    #[test]
    fn a_resolve_of_query_for_a_non_reference_is_empty() {
        // A literal (or any non-navigable node) resolves to nothing — the empty result, total.
        let src = "(module m (def (main) 42) (export main))";
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
        let lit = spans
            .node_at_offset(src.find("42").unwrap())
            .expect("the literal node");
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::ResolveOf { node: lit.0 })]),
                ),
            ],
            &[],
        );
        assert!(!out.has_error());
        assert_eq!(artifact_text(&out, KIND_RESOLVE).as_deref(), Some(""));
    }

    /// Parse `src`, resolve `offset` to a node, run `ScopeAt`, and return the `(name, type)` bindings.
    fn scope_bindings(src: &str, offset: usize) -> Vec<(String, String)> {
        let (arenas, spans) = cadenza_syntax::sexpr::read_spanned(src).expect("parse");
        let node = spans.node_at_offset(offset).expect("a node at the offset");
        let ast = cadenza_syntax::codec::encode(&arenas);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", ast),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Query(Query::ScopeAt { node: node.0 })]),
                ),
            ],
            &[],
        );
        assert!(
            !out.has_error(),
            "a query does not fail: {:?}",
            out.diagnostics
        );
        artifact_text(&out, KIND_SCOPE)
            .expect("a scope artifact")
            .lines()
            .map(|l| {
                let mut c = l.split('\t');
                (c.next().unwrap().to_string(), c.next().unwrap().to_string())
            })
            .collect()
    }

    #[test]
    fn a_scope_at_query_lists_let_bindings_and_params_with_types() {
        // Inside `body`, both the parameter `p` (Int64) and the let-binding `q` (Int64) are visible.
        let src = "(module m (def (f (: p Int64)) (let ((q (: 5 Int64))) (+ p q))) (export main))";
        // Offset at the `(+ p q)` body.
        let off = src.find("(+ p q)").expect("the body");
        let scope = scope_bindings(src, off);
        // `p` and `q` are both in scope, both Int64.
        assert!(
            scope.iter().any(|(n, t)| n == "p" && t == "Int64"),
            "param p:Int64 in scope: {scope:?}"
        );
        assert!(
            scope.iter().any(|(n, t)| n == "q" && t == "Int64"),
            "let-binding q:Int64 in scope: {scope:?}"
        );
    }

    #[test]
    fn a_scope_at_query_at_the_top_level_is_empty() {
        // At a top-level def body with no enclosing binder, no local bindings are in scope.
        let src = "(module m (def (main) 42) (export main))";
        let off = src.find("42").expect("the literal");
        assert!(
            scope_bindings(src, off).is_empty(),
            "top level has no local scope"
        );
    }

    #[test]
    fn a_scope_at_query_respects_sequential_let_scope() {
        // In `(let ((a 1) (b (+ a 1))) …)`, the initializer of `b` sees `a` but NOT `b` itself.
        let src = "(module m (def (main) (let ((a (: 1 Int64)) (b (+ a 1))) b)) (export main))";
        // Offset inside `b`'s initializer `(+ a 1)`.
        let off = src.find("(+ a 1)").expect("b's initializer");
        let scope = scope_bindings(src, off);
        assert!(
            scope.iter().any(|(n, _)| n == "a"),
            "a is visible in b's init: {scope:?}"
        );
        assert!(
            !scope.iter().any(|(n, _)| n == "b"),
            "b is NOT visible in its own init: {scope:?}"
        );
    }

    #[test]
    fn an_exports_query_lists_each_export_with_its_type() {
        // The module interface: every `(export …)` clause paired with the named def's signature.
        let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (v) (: 5 Int64)) \
                   (export inc) (export v))";
        let out = compile(&inputs(src, &[Request::Query(Query::Exports)]), &[]);
        assert!(!out.has_error(), "{:?}", out.diagnostics);
        let text = artifact_text(&out, KIND_EXPORTS).expect("an exports artifact");
        let rows: Vec<(&str, &str)> = text
            .lines()
            .map(|l| {
                let mut c = l.split('\t');
                (c.next().unwrap(), c.next().unwrap())
            })
            .collect();
        // Both exports appear with their types (a function's arrow, a value's type), in export order.
        assert_eq!(rows[0], ("inc", "(-> Int64 Int64)"), "rows: {rows:?}");
        assert_eq!(rows[1], ("v", "Int64"), "rows: {rows:?}");
    }

    #[test]
    fn an_exports_query_with_no_exports_is_empty() {
        // A module with no `(export …)` yields the empty interface (total, not an error).
        let src = "(module m (def (main) 42))";
        let out = compile(&inputs(src, &[Request::Query(Query::Exports)]), &[]);
        assert!(!out.has_error());
        assert_eq!(artifact_text(&out, KIND_EXPORTS).as_deref(), Some(""));
    }
}

/// Debug information (D0 of `DESIGN-debug-info-rcdzc.md`) — the wasm `name` custom section, enabled by
/// a Mode-E `EMIT_WASM_DEBUG` sidecar request (`Target::WasmDebug`). Two load-bearing properties: the
/// `name` section actually carries the source function names (readable frames), and it is INERT +
/// STRIPPABLE — the debug component stripped of custom sections is byte-identical to the plain one (the
/// §5 reproducibility anchor). The strip is done with `wasm-tools` (the same section-remover the spec
/// mandates); the test skips gracefully if `wasm-tools` is not on PATH.
#[cfg(test)]
mod debug_info {
    use crate::abi::Artifact;
    use crate::backend::Target;
    use crate::compile::compile;
    use crate::sidecar::{self, Request};
    use crate::spans;
    use crate::testkit::{parse, parse_spanned};

    /// The `ast` + `spans` input artifacts for `src` — the pair a debug-enabled driver supplies.
    fn debug_inputs(src: &str) -> Vec<Artifact> {
        let (arenas, span_data) = parse_spanned(src);
        vec![
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
            Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
        ]
    }

    /// Compile `src` to a `component` under `target`, returning the component bytes. A debug target
    /// (which requires the `spans` input, §9.4) is given both artifacts; a plain target just the `ast`.
    fn component_of(src: &str, target: Target) -> Vec<u8> {
        let inputs = if target.needs_spans() {
            debug_inputs(src)
        } else {
            vec![Artifact::new(
                Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )]
        };
        let out = compile(&inputs, &[target]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        out.artifact("component")
            .expect("a component artifact")
            .to_vec()
    }

    /// Every embedded core module's bytes, in order — the resource envelope embeds several (a dtor plus
    /// the main walker core), and the `.debug_*` sections ride in a LATER one, not the first `dtor`. A
    /// raw section-framing scan (a component's core-module is section id 1) rather than `wasmparser`,
    /// whose stateful reader chokes when a nested module's own `\0asm` magic follows the section header.
    fn core_modules_of(component: &[u8]) -> Vec<Vec<u8>> {
        fn uleb(b: &[u8], i: &mut usize) -> u64 {
            let (mut r, mut s) = (0u64, 0u32);
            loop {
                let x = b[*i];
                *i += 1;
                r |= u64::from(x & 0x7f) << s;
                s += 7;
                if x & 0x80 == 0 {
                    return r;
                }
            }
        }
        let mut mods = Vec::new();
        if component.get(0..4) != Some(b"\0asm") {
            return mods;
        }
        let mut i = 8; // past `\0asm` + the 4-byte version/layer word
        while i < component.len() {
            let sid = component[i];
            i += 1;
            let len = uleb(component, &mut i) as usize;
            // In a COMPONENT, section id 1 is an embedded core module (its body IS a `\0asm…` module).
            if sid == 1 {
                mods.push(component[i..i + len].to_vec());
            }
            i += len;
        }
        mods
    }

    /// The bytes of a named custom section inside a CORE module (`\0asm`…), or `None`. A dev-only
    /// scanner used to byte-compare a sidecar's `.debug_*` against the embedded component's.
    fn custom_section_of(core: &[u8], want: &str) -> Option<Vec<u8>> {
        fn uleb(b: &[u8], i: &mut usize) -> u64 {
            let (mut r, mut s) = (0u64, 0u32);
            loop {
                let x = b[*i];
                *i += 1;
                r |= u64::from(x & 0x7f) << s;
                s += 7;
                if x & 0x80 == 0 {
                    return r;
                }
            }
        }
        if core.get(0..4) != Some(b"\0asm") {
            return None;
        }
        let mut i = 8;
        while i < core.len() {
            let sid = core[i];
            i += 1;
            let len = uleb(core, &mut i) as usize;
            let body = &core[i..i + len];
            if sid == 0 {
                let mut j = 0;
                let nl = uleb(body, &mut j) as usize;
                if std::str::from_utf8(&body[j..j + nl]) == Ok(want) {
                    return Some(body[j + nl..].to_vec());
                }
            }
            i += len;
        }
        None
    }

    /// The parsed `name` section: the optional module name and the `(func_index, name)` pairs.
    type NameSection = (Option<String>, Vec<(u32, String)>);

    /// The embedded core module's bytes, extracted from a component wrapper via `wasmparser` (a dev-only
    /// validator, never in the compile path) — the blob that carries the `name` + `.debug_*` sections.
    fn core_module_of(component: &[u8]) -> Option<Vec<u8>> {
        use wasmparser::{Chunk, Parser, Payload};
        let mut parser = Parser::new(0);
        let mut offset = 0usize;
        let mut buf = component;
        loop {
            match parser.parse(buf, true).expect("parse component") {
                Chunk::NeedMoreData(_) => return None,
                Chunk::Parsed { consumed, payload } => {
                    if let Payload::ModuleSection {
                        unchecked_range, ..
                    } = &payload
                    {
                        return Some(
                            component[unchecked_range.start..unchecked_range.end].to_vec(),
                        );
                    }
                    offset += consumed;
                    buf = &component[offset..];
                    if let Payload::End(_) = payload {
                        return None;
                    }
                }
            }
        }
    }

    /// Extract the embedded core module's `name`-section function names from a component's bytes.
    /// Returns `(module_name, [(func_index, name)])`; `None` if there is no `name` section.
    fn name_section_of(component: &[u8]) -> Option<NameSection> {
        use wasmparser::{Parser, Payload};
        let core = core_module_of(component)?;
        let core = core.as_slice();
        // Now parse the core module for its `name` custom section.
        let mut module_name = None;
        let mut func_names = Vec::new();
        let mut found = false;
        for payload in Parser::new(0).parse_all(core) {
            if let Payload::CustomSection(reader) = payload.expect("parse core")
                && reader.name() == "name"
            {
                found = true;
                let name_reader = wasmparser::NameSectionReader::new(
                    wasmparser::BinaryReader::new(reader.data(), reader.data_offset()),
                );
                for subsection in name_reader {
                    match subsection.expect("name subsection") {
                        wasmparser::Name::Module { name, .. } => {
                            module_name = Some(name.to_string())
                        }
                        wasmparser::Name::Function(map) => {
                            for naming in map {
                                let naming = naming.expect("naming");
                                func_names.push((naming.index, naming.name.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        found.then_some((module_name, func_names))
    }

    #[test]
    fn a_plain_component_carries_no_name_section() {
        // Baseline: without a debug request, there is no `name` section — today's exact bytes.
        let src = "(module m (def (main) 42) (export main))";
        let plain = component_of(src, Target::Wasm);
        assert!(
            name_section_of(&plain).is_none(),
            "a plain component must carry no `name` section (byte-identical to today)"
        );
    }

    #[test]
    fn a_debug_component_names_its_functions() {
        // A program with an exported `main` calling a RECURSIVE internal callee `countdown`. The callee
        // is recursive so it cannot be inlined away — it survives as a real emitted function, so the
        // `name` section names both `main` and `countdown` (a non-recursive helper would β-reduce into
        // its caller and no longer be a function to name). The module name is the first export, `main`.
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        let debug = component_of(src, Target::WasmDebug);
        let (module_name, func_names) =
            name_section_of(&debug).expect("a debug component carries a `name` section");
        assert_eq!(module_name.as_deref(), Some("main"));
        // The source names appear in the function-name map (order/index is emission-determined; assert
        // membership so the test is robust to which index each lands at).
        let names: Vec<&str> = func_names.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"main"), "names = {names:?}");
        assert!(names.contains(&"countdown"), "names = {names:?}");
    }

    #[test]
    fn a_debug_component_only_adds_custom_sections() {
        // The debug component must differ from the plain one ONLY by added custom sections — i.e. the
        // debug bytes are strictly longer (the `name` section is appended, moving no executed byte).
        let src = "(module m (def (main) 42) (export main))";
        let plain = component_of(src, Target::Wasm);
        let debug = component_of(src, Target::WasmDebug);
        assert!(
            debug.len() > plain.len(),
            "the debug component must be larger (it carries an extra `name` section): plain {} vs debug {}",
            plain.len(),
            debug.len()
        );
        assert!(
            name_section_of(&debug).is_some() && name_section_of(&plain).is_none(),
            "only the debug component carries the `name` section"
        );
    }

    #[test]
    fn strip_recovers_the_undecorated_component_byte_for_byte() {
        // The reproducibility anchor (§5): `strip(emit(debug)) == emit(no-debug)`, byte-for-byte, where
        // `strip` is a SECTION REMOVER (`wasm-tools strip`), never a re-serializer. Proves inertness,
        // strippability, and reproducibility in one cheap check. Skips if `wasm-tools` is not installed.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m (def (main) 42) (export main))";
        let plain = component_of(src, Target::Wasm);
        let debug = component_of(src, Target::WasmDebug);

        // Write the debug component to a temp file and strip its custom sections.
        let dir = std::env::temp_dir();
        let in_path = dir.join(format!("cdz-dbg-{}.wasm", std::process::id()));
        let out_path = dir.join(format!("cdz-dbg-{}-stripped.wasm", std::process::id()));
        std::fs::File::create(&in_path)
            .and_then(|mut f| f.write_all(&debug))
            .expect("write debug component");

        // `wasm-tools strip --all` removes every custom section (including `name`).
        let status = Command::new("wasm-tools")
            .args(["strip", "--all"])
            .arg(&in_path)
            .arg("-o")
            .arg(&out_path)
            .status();
        let status = match status {
            Ok(s) => s,
            Err(_) => {
                eprintln!("wasm-tools not found on PATH; skipping strip round-trip");
                let _ = std::fs::remove_file(&in_path);
                return;
            }
        };
        assert!(status.success(), "wasm-tools strip failed");
        let stripped = std::fs::read(&out_path).expect("read stripped");
        let _ = std::fs::remove_file(&in_path);
        let _ = std::fs::remove_file(&out_path);

        assert_eq!(
            stripped, plain,
            "stripping a debug component's custom sections must recover the undecorated bytes exactly"
        );
    }

    #[test]
    fn a_sidecar_emit_wasm_debug_request_drives_the_debug_component() {
        // Mode-E enablement end-to-end: an `EMIT_WASM_DEBUG` request in the sidecar list, with the
        // `spans` input supplied, drives a debug-carrying `component` — the debug directive is a
        // request, not a build flag.
        let src = "(module m (def (main) 42) (export main))";
        let (arenas, span_data) = parse_spanned(src);
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
                Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
                Artifact::new(
                    sidecar::KIND_SIDECAR,
                    "drive",
                    sidecar::encode(&[Request::Emit(Target::WasmDebug)]),
                ),
            ],
            &[],
        );
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let component = out.artifact("component").expect("a component artifact");
        assert!(
            name_section_of(component).is_some(),
            "an EMIT_WASM_DEBUG request must produce a component carrying the `name` section"
        );
        // The round-trip through the sidecar codec preserves the debug request.
        let reqs = sidecar::decode(&sidecar::encode(&[Request::Emit(Target::WasmDebug)]));
        assert_eq!(reqs, Some(vec![Request::Emit(Target::WasmDebug)]));
    }

    #[test]
    fn a_debug_emit_without_spans_declines() {
        // §9.4 — a debug artifact requested WITHOUT the `spans` input is a decline, not a silent
        // undecorated component. The debug `Emit` is the signal; `spans` is the data — required together.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &[Artifact::new(
                Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::WasmDebug],
        );
        assert!(out.has_error(), "must decline without a spans input");
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .unwrap();
        assert!(
            d.message.contains("`spans`"),
            "the decline must name the missing spans input: {}",
            d.message
        );
        // Crucially, NO component was produced — not an undecorated one.
        assert!(out.artifact("component").is_none());
    }

    #[test]
    fn a_malformed_spans_artifact_declines() {
        // A present-but-malformed `spans` input is a decline (reject-don't-miscompile at the tool edge),
        // exactly like a malformed sidecar list.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
                // A path length claiming 9 bytes but with none present — a truncated table.
                Artifact::new(spans::KIND_SPANS, "m", vec![0x09]),
            ],
            &[Target::WasmDebug],
        );
        assert!(out.has_error());
        let d = out
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .unwrap();
        assert!(
            d.message.contains("malformed `spans`"),
            "message: {}",
            d.message
        );
    }

    #[test]
    fn a_plain_wasm_target_ignores_a_spans_input() {
        // A `spans` input present without a debug request changes nothing — the plain component is
        // byte-identical to one compiled with no spans input at all (spans are inert until a debug
        // target reads them).
        let src = "(module m (def (main) 42) (export main))";
        let (arenas, span_data) = parse_spanned(src);
        let with_spans = compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
                Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
            ],
            &[Target::Wasm],
        );
        let without = component_of(src, Target::Wasm);
        assert_eq!(
            with_spans.artifact("component").expect("component"),
            without.as_slice(),
            "a spans input must not change the plain (non-debug) component's bytes"
        );
    }

    // ── D1b: the offset→StructId line-table primitive (§2.1b) ──────────────────────────────────────

    #[test]
    fn code_ranges_partition_the_code_section_payload_and_carry_src() {
        // The D1b primitive: `code_ranges` gives each function's byte range within the code-section
        // PAYLOAD, paired with its source occurrence. Verify (a) the ranges are contiguous, start past
        // the function-count prefix, and cover exactly the payload; (b) each range's bytes ARE that
        // function's `code_entry` (byte-identical); (c) `src` round-trips. Two functions with distinct
        // source ids exercise the pairing.
        use crate::ast::StructId;
        use crate::backend::wasm::lir::Lir;
        use crate::backend::wasm::runtime_abi::OPS;
        use crate::backend::wasm::select::SelectedFunc;
        use crate::backend::wasm::serialize::{code_entry_bytes, code_ranges, core_module};
        use crate::layout::{ExportPlan, Layout};
        use crate::ty::Ty;

        let funcs = vec![
            SelectedFunc {
                params: vec![],
                ret: Ty::int64(),
                code: vec![Lir::ConstI64(7)],
                declared: vec![],
                src_body: Some(StructId(11)),
                locals: vec![],
                scopes: vec![],
                stmt_lines: vec![],
            },
            SelectedFunc {
                params: vec![],
                ret: Ty::int64(),
                code: vec![Lir::ConstI64(1000), Lir::ConstI64(2), Lir::I64Add],
                declared: vec![],
                src_body: Some(StructId(22)),
                locals: vec![],
                scopes: vec![],
                stmt_lines: vec![],
            },
        ];
        // One import so the layout mirrors the oracle shape; ranges are import-independent.
        let imports = [OPS.arr_alloc];
        let layout = Layout::new(
            vec![ExportPlan {
                name: "main".to_string(),
                def: 0,
                body: StructId(11),
                params: vec![],
                result: Ty::int64(),
            }],
            vec![0, 1],
            1,
        );

        let ranges = code_ranges(&funcs, &imports);
        assert_eq!(ranges.len(), 2);
        // (c) src round-trips, in emission order.
        assert_eq!(ranges[0].src, Some(StructId(11)));
        assert_eq!(ranges[1].src, Some(StructId(22)));

        // (a) contiguous + starts past the count prefix (a 2-function module: count = 1 byte).
        assert_eq!(
            ranges[0].code_start, 1,
            "starts past the 1-byte function count"
        );
        assert_eq!(
            ranges[0].code_end, ranges[1].code_start,
            "ranges are contiguous"
        );

        // (b) each range's bytes ARE that function's code entry.
        let entry0 = code_entry_bytes(&funcs[0], &imports);
        let entry1 = code_entry_bytes(&funcs[1], &imports);
        assert_eq!(
            (ranges[0].code_end - ranges[0].code_start) as usize,
            entry0.len()
        );
        assert_eq!(
            (ranges[1].code_end - ranges[1].code_start) as usize,
            entry1.len()
        );

        // Cross-check the payload total: count prefix + both entries == last range end.
        let count_prefix = 1u32; // uleb(2) is one byte
        assert_eq!(
            ranges[1].code_end,
            count_prefix + entry0.len() as u32 + entry1.len() as u32
        );

        // And the whole thing is consistent with a real `core_module`: the module must contain the two
        // entries' bytes consecutively at the payload region the ranges describe.
        let core = core_module(&funcs, &imports, &layout).expect("core module");
        let mut concat = entry0.clone();
        concat.extend_from_slice(&entry1);
        assert!(
            core.windows(concat.len()).any(|w| w == concat.as_slice()),
            "the emitted core module must contain both code entries consecutively"
        );
    }

    // ── D2: the .debug_* DWARF sections (§2.3) ─────────────────────────────────────────────────────

    /// The names of the custom sections in a component's embedded core module (in order).
    fn custom_section_names(component: &[u8]) -> Vec<String> {
        use wasmparser::{Parser, Payload};
        let core = core_module_of(component).expect("embedded core module");
        let mut names = Vec::new();
        for payload in Parser::new(0).parse_all(&core) {
            if let Payload::CustomSection(reader) = payload.expect("parse core") {
                names.push(reader.name().to_string());
            }
        }
        names
    }

    #[test]
    fn a_debug_component_carries_the_dwarf_sections() {
        // A debug component's embedded core module carries the four `.debug_*` custom sections (plus the
        // D0 `name` section) — the sections a debugger reads to step through source.
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        let debug = component_of(src, Target::WasmDebug);
        let names = custom_section_names(&debug);
        for want in [
            "name",
            ".debug_abbrev",
            ".debug_info",
            ".debug_str",
            ".debug_line",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "missing custom section `{want}`; found {names:?}"
            );
        }
        // A plain component has none of them.
        let plain = component_of(src, Target::Wasm);
        assert!(
            custom_section_names(&plain)
                .iter()
                .all(|n| !n.starts_with(".debug")),
            "a plain component must carry no `.debug_*` sections"
        );
    }

    #[test]
    fn the_dwarf_parses_under_llvm_dwarfdump() {
        // The correctness ORACLE (§6): the hand-rolled DWARF must parse cleanly under `llvm-dwarfdump`.
        // We extract the embedded core module (dwarfdump rejects a component's version number) and dump
        // it; the output must name our compile unit + a subprogram, and report NO parse error. Skips if
        // `llvm-dwarfdump` is not installed.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        let debug = component_of(src, Target::WasmDebug);
        let core = core_module_of(&debug).expect("embedded core module");

        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-dwarf-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&core))
            .expect("write core module");

        let output = match Command::new("llvm-dwarfdump")
            .arg("--all")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found on PATH; skipping DWARF-validity check");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "llvm-dwarfdump failed: {stderr}\n{stdout}"
        );
        // The dump must NOT report a parse error, and must name our compile unit + a subprogram.
        assert!(
            !stdout.to_lowercase().contains("error:") && !stderr.to_lowercase().contains("error:"),
            "llvm-dwarfdump reported an error:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stdout.contains("DW_TAG_compile_unit"),
            "no compile unit in the dump:\n{stdout}"
        );
        assert!(
            stdout.contains("DW_TAG_subprogram"),
            "no subprogram in the dump:\n{stdout}"
        );
        // The producer + a function name must round-trip through .debug_str.
        assert!(
            stdout.contains("cadenza-rcdzc"),
            "producer string missing:\n{stdout}"
        );
        assert!(
            stdout.contains("countdown") || stdout.contains("main"),
            "no source function name in the dump:\n{stdout}"
        );
        // The compile unit declares a source language (DW_LANG_C) — a CU with no language reads as
        // "<not loaded>" in a real debugger, which then declines to format scalar values.
        assert!(
            stdout.contains("DW_AT_language") && stdout.contains("DW_LANG_C"),
            "the CU must declare DW_LANG_C:\n{stdout}"
        );
    }

    // ── Mode S: the detached `dwarf` sidecar artifact (§9.2) ───────────────────────────────────────

    /// Compile `src` with the given request list + the `spans` input, returning the full output.
    fn compile_debug(src: &str, requests: &[Request]) -> crate::abi::CompileOutput {
        let (arenas, span_data) = parse_spanned(src);
        compile(
            &[
                Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
                Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
                Artifact::new(sidecar::KIND_SIDECAR, "d", sidecar::encode(requests)),
            ],
            &[],
        )
    }

    #[test]
    fn an_emit_dwarf_request_produces_a_dwarf_artifact_not_a_component() {
        // A lone `Emit(Dwarf)` yields a `dwarf` artifact and NO component — the detached sidecar mode.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        assert!(
            out.artifact("dwarf").is_some(),
            "an Emit(Dwarf) request must produce a `dwarf` artifact"
        );
        assert!(
            out.artifact("component").is_none(),
            "a lone Emit(Dwarf) must NOT produce a component"
        );
    }

    #[test]
    fn wasm_and_dwarf_compose_into_two_artifacts() {
        // Requesting both a plain component AND a detached dwarf yields both — the "lean component + its
        // detached DWARF" shape.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile_debug(
            src,
            &[Request::Emit(Target::Wasm), Request::Emit(Target::Dwarf)],
        );
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        assert!(
            out.artifact("component").is_some(),
            "the runnable component"
        );
        assert!(out.artifact("dwarf").is_some(), "the detached DWARF");
    }

    #[test]
    fn the_dwarf_sidecar_parses_under_llvm_dwarfdump() {
        // The sidecar module is ALREADY a bare core module (no component wrapper), so llvm-dwarfdump
        // reads it directly. It must show the same compile unit + subprograms as the embedded form.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-sidecar-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write dwarf sidecar");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--all")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "dwarfdump failed: {stderr}\n{stdout}"
        );
        assert!(
            !stdout.to_lowercase().contains("error:") && !stderr.to_lowercase().contains("error:"),
            "dwarfdump reported an error:\n{stdout}\n{stderr}"
        );
        assert!(stdout.contains("DW_TAG_compile_unit"), "no CU:\n{stdout}");
        assert!(
            stdout.contains("DW_TAG_subprogram"),
            "no subprogram:\n{stdout}"
        );
        assert!(
            stdout.contains("countdown") || stdout.contains("main"),
            "no fn name:\n{stdout}"
        );
    }

    #[test]
    fn the_dwarf_sidecar_loads_in_lldb() {
        // A SECOND, INDEPENDENT oracle beyond llvm-dwarfdump (design §6 "wall #2" — does a REAL
        // debugger, not just the lenient dump parser, consume our hand-rolled DWARF?). lldb loads the
        // bare sidecar core module, recognizes it as a `wasm32` target, and parses the compile unit —
        // reporting its source language (which requires the `DW_AT_language` we emit; a CU without it
        // reads as "<not loaded>"). Skips if lldb is absent.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-lldb-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write dwarf sidecar");
        // Dump the compile unit via lldb's Python API — `str(CompileUnit)` includes `language = "…"` and
        // the source file, which is exactly what a debugger reads from our CU DIE + line-program header.
        let output = match Command::new("lldb")
            .arg("--batch")
            .arg("-o")
            .arg(
                "script m=lldb.target.module_iter().__next__(); \
                 print('CU=' + str(m.compile_unit_iter().__next__()))",
            )
            .arg(path.to_str().unwrap())
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("lldb not found; skipping the second-oracle check");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // lldb recognizes the module as a wasm32 target (it accepts our bare core module).
        assert!(
            stdout.contains("wasm32"),
            "lldb did not recognize a wasm32 target:\n{stdout}\n{stderr}"
        );
        // It parsed a compile unit and read our declared source language (proving DW_AT_language landed —
        // an absent language would print `language = "<not loaded>"`).
        assert!(
            stdout.contains("CU=") && stdout.contains("language = \"c\""),
            "lldb did not read the CU's source language:\n{stdout}\n{stderr}"
        );
    }

    #[test]
    fn the_sidecar_code_offsets_match_the_embedded_ones() {
        // The load-bearing invariant: a detached DWARF's code addresses must reference the RUNNABLE
        // component's code section identically to the embedded form — otherwise a debugger would map to
        // the wrong instruction. Compare the subprogram low_pc/high_pc reported by dwarfdump for the
        // embedded component vs the sidecar; they must be identical. Skips if llvm-dwarfdump is absent.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
        // Embedded: extract the core module from the WasmDebug component.
        let embedded_component = component_of(src, Target::WasmDebug);
        let embedded_core = core_module_of(&embedded_component).expect("core module");
        // Sidecar: the standalone dwarf module.
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let sidecar_mod = out.artifact("dwarf").expect("dwarf").to_vec();

        // Dump each and pull the ordered list of (low_pc, high_pc) from the subprogram DIEs.
        let pcs = |bytes: &[u8], tag: &str| -> Option<Vec<(String, String)>> {
            let dir = std::env::temp_dir();
            let path = dir.join(format!("cdz-cmp-{}-{tag}.wasm", std::process::id()));
            std::fs::File::create(&path)
                .and_then(|mut f| f.write_all(bytes))
                .ok()?;
            let o = Command::new("llvm-dwarfdump")
                .arg("--debug-info")
                .arg(&path)
                .output()
                .ok()?;
            let _ = std::fs::remove_file(&path);
            let s = String::from_utf8_lossy(&o.stdout).to_string();
            let mut lows: Vec<String> = Vec::new();
            let mut highs: Vec<String> = Vec::new();
            for line in s.lines() {
                let t = line.trim();
                if let Some(v) = t.strip_prefix("DW_AT_low_pc") {
                    lows.push(v.trim().to_string());
                } else if let Some(v) = t.strip_prefix("DW_AT_high_pc") {
                    highs.push(v.trim().to_string());
                }
            }
            Some(lows.into_iter().zip(highs).collect())
        };

        let (Some(emb), Some(side)) = (pcs(&embedded_core, "emb"), pcs(&sidecar_mod, "side"))
        else {
            eprintln!("llvm-dwarfdump not found; skipping offset-match check");
            return;
        };
        assert!(!emb.is_empty(), "the embedded DWARF had no pc ranges");
        assert_eq!(
            emb, side,
            "the sidecar's code offsets must match the embedded component's exactly"
        );
    }

    // ── D3: scalar variable inspection (§2.4) ──────────────────────────────────────────────────────

    #[test]
    fn scalar_params_get_formal_parameter_dies_with_locations() {
        // D3: an exported function's scalar parameters emit `DW_TAG_formal_parameter` DIEs, each with a
        // `DW_AT_type` (→ a base type) and a `DW_AT_location` naming the wasm local slot — so a debugger
        // can `print` the argument. Verified via llvm-dwarfdump on the detached sidecar (a bare module).
        use std::io::Write;
        use std::process::Command;
        // Two Int64 params so the fn has real scalar locals at slots 0 and 1.
        let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();

        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-d3-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.to_lowercase().contains("error:"),
            "dwarfdump error:\n{stdout}"
        );
        // A base type for Int64.
        assert!(
            stdout.contains("DW_TAG_base_type") && stdout.contains("\"i64\""),
            "missing the i64 base type:\n{stdout}"
        );
        // Both params, as formal parameters, with a type ref and a wasm-local location.
        assert!(
            stdout.contains("DW_TAG_formal_parameter"),
            "no formal_parameter DIE:\n{stdout}"
        );
        assert!(
            stdout.contains("\"a\"") && stdout.contains("\"b\""),
            "param names:\n{stdout}"
        );
        assert!(
            stdout.contains("DW_OP_WASM_location"),
            "no wasm-local location:\n{stdout}"
        );
        // The two params sit at consecutive local slots 0 and 1.
        assert!(
            stdout.contains("DW_OP_WASM_location 0x0 0x0")
                && stdout.contains("DW_OP_WASM_location 0x0 0x1"),
            "params must be at local slots 0 and 1:\n{stdout}"
        );
    }

    #[test]
    fn a_nullary_function_has_no_formal_parameters() {
        // A function with no params emits a childless subprogram — no formal_parameter DIEs.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m (def (main) 42) (export main))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-d3n-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.contains("DW_TAG_formal_parameter"),
            "a nullary function must have no formal parameters:\n{stdout}"
        );
    }

    #[test]
    fn float_params_get_formal_parameter_dies_with_a_float_base_type() {
        // D3 for FLOATS: a `Float32`/`Float64` parameter is a scalar the runtime holds in a wasm
        // `f32`/`f64` local, so it earns a `DW_TAG_formal_parameter` with a `DW_ATE_float` base type +
        // a `DW_OP_WASM_location` local slot — a debugger can `print` a float argument, exactly as for
        // an integer. (Before this, `base_type_of` returned `None` for `Ty::Float`, so a float param got
        // NO DIE.) Both widths are covered: `Float64` → `f64` (8 bytes), `Float32` → `f32` (4 bytes).
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (def (scale (: x Float64) (: k Float32)) (*. x (Float64.of-int 1))) \
                     (export scale))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-d3f-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.to_lowercase().contains("error:"),
            "dwarfdump reported an error:\n{stdout}"
        );
        // Both scalar float widths get a base type with the IEEE-float encoding.
        assert!(
            stdout.contains("DW_ATE_float"),
            "a float param must reference a DW_ATE_float base type:\n{stdout}"
        );
        assert!(
            stdout.contains("\"f64\"") && stdout.contains("\"f32\""),
            "both f64 and f32 base types must be present:\n{stdout}"
        );
        // The params are formal parameters with names + wasm-local locations.
        assert!(
            stdout.contains("DW_TAG_formal_parameter")
                && stdout.contains("\"x\"")
                && stdout.contains("\"k\""),
            "the float params x and k must have formal_parameter DIEs:\n{stdout}"
        );
        assert!(
            stdout.contains("DW_OP_WASM_location 0x0 0x0")
                && stdout.contains("DW_OP_WASM_location 0x0 0x1"),
            "the float params must sit at local slots 0 and 1:\n{stdout}"
        );
    }

    #[test]
    fn a_nominal_newtype_scalar_param_gets_a_formal_parameter_die() {
        // D3 for a NOMINAL NEWTYPE over a scalar: `(type UserId (Mk Int64))` is erased to a runtime i64,
        // so a `UserId` parameter is a scalar the runtime holds in a wasm local — it must earn a
        // `DW_TAG_formal_parameter` with the UNDERLYING scalar's base type (`i64`, the value the runtime
        // actually holds), exactly like a bare `Int64`. (Before this, `base_type_of`/`select`'s scalar
        // guards didn't peel the nominal tag, so a nominal-scalar param got NO DIE.)
        use std::io::Write;
        use std::process::Command;
        let src = "(module m \
                     (type UserId (Mk Int64)) \
                     (def (idof (: u UserId)) (match u ((Mk n) n))) \
                     (export idof))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-d3nom-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.to_lowercase().contains("error:"),
            "dwarfdump reported an error:\n{stdout}"
        );
        // The nominal param `u` is described via its erased i64 base type + a wasm-local location.
        assert!(
            stdout.contains("DW_TAG_formal_parameter") && stdout.contains("\"u\""),
            "the nominal-scalar param u must have a formal_parameter DIE:\n{stdout}"
        );
        assert!(
            stdout.contains("\"i64\"") && stdout.contains("DW_ATE_signed"),
            "the nominal param must reference its underlying i64 base type:\n{stdout}"
        );
        assert!(
            stdout.contains("DW_OP_WASM_location 0x0 0x0"),
            "the nominal param must sit at local slot 0:\n{stdout}"
        );
    }

    #[test]
    fn wasm_plus_dwarf_links_the_component_to_the_sidecar() {
        // When a run emits BOTH a lean component and a detached DWARF sidecar, the component carries an
        // `external_debug_info` custom section naming the sidecar file — so a debugger auto-loads it.
        let src = "(module m (def (main) 42) (export main))";
        let out = compile_debug(
            src,
            &[Request::Emit(Target::Wasm), Request::Emit(Target::Dwarf)],
        );
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let comp = out.artifact("component").expect("component").to_vec();
        let names = custom_section_names(&comp);
        assert!(
            names.iter().any(|n| n == "external_debug_info"),
            "the lean component must point at the sidecar; found {names:?}"
        );
        // The lean component embeds NO DWARF itself (that lives in the sidecar) — only the pointer.
        assert!(
            names.iter().all(|n| !n.starts_with(".debug")),
            "a lean component must not embed DWARF (it points at the sidecar): {names:?}"
        );
        // The pointer's payload names the on-disk sidecar file (`main.dwarf` — the program name).
        assert!(
            comp.windows(b"main.dwarf".len())
                .any(|w| w == b"main.dwarf"),
            "the external_debug_info payload must name the sidecar file"
        );
    }

    #[test]
    fn a_lone_wasm_carries_no_external_debug_info() {
        // Without a paired `Dwarf` target, a plain component has no `external_debug_info` pointer — it
        // stays byte-identical to today's undecorated output.
        let src = "(module m (def (main) 42) (export main))";
        let plain = component_of(src, Target::Wasm);
        assert!(
            !custom_section_names(&plain)
                .iter()
                .any(|n| n == "external_debug_info"),
            "a lone Wasm target must carry no external_debug_info"
        );
    }

    #[test]
    fn a_compound_returning_program_carries_dwarf() {
        // A program returning a RUNTIME compound crosses via the resource-escape path (a different core
        // than the multi-export path). Its user function bodies still lead the escape core's code
        // section, so the `.debug_*` sections attribute correctly — a compound-returning program is
        // debuggable too. `f` recurses (not constant-foldable), so `main` builds the tuple on the heap.
        let src = "(module m \
                     (def (f n) (if (= n 0) (tuple n 7) (f (- n 1)))) \
                     (def (main) (f 3)) \
                     (export main))";
        let debug = component_of(src, Target::WasmDebug);
        // The resource envelope embeds MULTIPLE core modules (a dtor + the main walker core); the DWARF
        // rides in the main one, which is not necessarily first — so scan the raw component bytes for
        // the section names (those bytes are exactly what ships to the debugger) rather than only the
        // first embedded core. A plain (non-debug) build of the same program carries none of them.
        let has = |needle: &[u8]| debug.windows(needle.len()).any(|w| w == needle);
        for want in [b".debug_info".as_slice(), b".debug_line".as_slice()] {
            assert!(
                has(want),
                "a compound-returning program must carry {:?}",
                std::str::from_utf8(want).unwrap()
            );
        }
        // The source function names ride in — the resource core's user bodies get subprogram DIEs.
        assert!(
            has(b"main"),
            "the escape component's DWARF must name the source functions"
        );
        // The wasm `name` section rides in too (uniformly with the ordinary path now) — so a plain
        // profiler/trace shows `f`/`main`, not `func[N]`. Its section-name string is the length-prefixed
        // `\x04name` (the custom-section name), distinct from the incidental substring "name".
        assert!(
            has(b"\x04name"),
            "the escape component must carry the wasm `name` section"
        );
        // The plain build embeds no DWARF (the sections are debug-only).
        let plain = component_of(src, Target::Wasm);
        assert!(
            !plain
                .windows(b".debug_info".len())
                .any(|w| w == b".debug_info"),
            "a plain compound-returning component must carry no DWARF"
        );
    }

    #[test]
    fn a_compound_returning_export_gets_a_dwarf_sidecar_matching_the_embedded_dwarf() {
        // Mode S for the RESOURCE-ESCAPE path: a compound-returning export used to DECLINE a detached
        // `dwarf` sidecar ("not yet supported"). It now emits one, and — the load-bearing invariant —
        // its `.debug_*` sections are BYTE-IDENTICAL to the ones the embedded (Mode E) component carries,
        // so a debugger maps to the same instructions with either artifact. Covers the flat-tuple, sum,
        // and runtime-Bytes escape cores (each a distinct resource core layout).
        for src in [
            // runtime tuple (recursive build → real user bodies, resource-escape core)
            "(module m \
               (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (- n 1)))) \
               (def (main) (f 3)) \
               (export main))",
            // runtime sum (Some payload, disc-switch walker)
            "(module m \
               (def (pick (: n Int64)) (if (= n 0) (Some 5) (pick (- n 1)))) \
               (def (main) (pick 3)) \
               (export main))",
            // runtime Bytes (looping walker)
            "(module m \
               (def (grow (: n Int64) (: acc Bytes)) \
                 (if (= n 0) acc (grow (- n 1) (Bytes.concat acc (Bytes.of (list 65)))))) \
               (def (main) (grow 3 (Bytes.of (list)))) \
               (export main))",
        ] {
            // The sidecar (Mode S).
            let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
            assert!(
                !out.has_error(),
                "a compound-returning export must now emit a dwarf sidecar, not decline: {:?}",
                out.diagnostics
            );
            let sidecar = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

            // The embedded (Mode E) component — find the core module that carries the DWARF (a resource
            // envelope embeds several; the dtor core carries none).
            let embedded = component_of(src, Target::WasmDebug);
            let debug_core = core_modules_of(&embedded)
                .into_iter()
                .find(|m| custom_section_of(m, ".debug_info").is_some())
                .expect("an embedded core module carrying DWARF");

            for sect in [".debug_info", ".debug_line", ".debug_abbrev", ".debug_str"] {
                let s = custom_section_of(&sidecar, sect);
                let e = custom_section_of(&debug_core, sect);
                assert_eq!(
                    s, e,
                    "the sidecar's {sect} must be byte-identical to the embedded component's \
                     (same code offsets); src = {src}"
                );
                assert!(s.is_some(), "the sidecar must carry {sect}");
            }
        }
    }

    #[test]
    fn a_constant_compound_export_gets_a_valid_empty_dwarf_sidecar() {
        // A FULLY-CONSTANT compound bakes its bytes into a resource core with NO user function to
        // attribute — the embedded path emits no `.debug_*` for it. The sidecar must therefore be a
        // VALID module with an empty compile unit (no subprograms), NOT a decline and not malformed.
        let src = "(module m (def (pair) (tuple 42 7)) (export pair))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(
            !out.has_error(),
            "a constant compound must emit an (empty) sidecar, not decline: {:?}",
            out.diagnostics
        );
        let sidecar = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
        // A bare core module carrying a `.debug_info` (the CU) but the CU has no subprogram DIEs — there
        // were no user bodies. We assert the section EXISTS (the standalone module is well-formed) and
        // that the embedded component carried no `.debug_info` (nothing to attribute), the dual property.
        assert!(
            custom_section_of(&sidecar, ".debug_info").is_some(),
            "even an empty CU sidecar carries a .debug_info section"
        );
        let embedded = component_of(src, Target::WasmDebug);
        let has_embedded_dwarf = core_modules_of(&embedded)
            .iter()
            .any(|m| custom_section_of(m, ".debug_info").is_some());
        assert!(
            !has_embedded_dwarf,
            "a constant compound's embedded component has no user body → no embedded DWARF"
        );
    }

    #[test]
    fn a_multi_line_body_gets_a_line_row_per_line() {
        // Per-statement/expression granularity: an `if` whose condition, then-branch, and else-branch
        // sit on DISTINCT source lines produces a `.debug_line` row per line (not one function-entry
        // row) — so a debugger steps line-by-line. Rows are at ascending code offsets. Skips if
        // llvm-dwarfdump is absent.
        use std::io::Write;
        use std::process::Command;
        let src = "(module m\n  (def (f (: a Int64))\n    (if (< a 0)\n      (- a 1)\n      (+ a 1)))\n  (export f))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-lines-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-line")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        // Parse the line-table rows: lines beginning with `0x…` have columns [Address, Line, …]. Collect
        // the distinct source lines (col 2) across the rows.
        let mut lines: Vec<u32> = stdout
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                if !l.starts_with("0x") {
                    return None;
                }
                l.split_whitespace().nth(1)?.parse().ok()
            })
            .collect();
        lines.sort_unstable();
        lines.dedup();
        // The `if` condition (line 3), then-branch (line 4), and else-branch (line 5) each get a row —
        // at least 3 distinct source lines (function granularity would give only 1).
        assert!(
            lines.len() >= 3,
            "expected a row per source line (≥3 distinct), got {lines:?}\n{stdout}"
        );
        assert!(
            lines.contains(&3) && lines.contains(&4) && lines.contains(&5),
            "expected rows for lines 3/4/5, got {lines:?}\n{stdout}"
        );
    }

    #[test]
    fn a_kept_scalar_let_binding_gets_a_variable_die() {
        // D3 locals: a `let` binding whose runtime value is used more than once is KEPT as a named slot
        // (`Core::Let`); a scalar one earns a `DW_TAG_variable` DIE with a type and a wasm-local location,
        // so a debugger can `print` the local. The binder key is the initializer occurrence, so the name
        // is recovered from its `(name init)` pair (`db.let_binding_name`) — this test guards that path.
        use std::io::Write;
        use std::process::Command;
        // `x` is used twice, so it survives A-normalization as a kept `Core::Let` binding.
        let src = "(module m (def (f (: a Int64)) (let ((x (+ a 1))) (+ x x))) (export f))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-letloc-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.to_lowercase().contains("error:"),
            "dwarfdump error:\n{stdout}"
        );
        // The kept binding `x` is a variable (not a formal parameter), with a wasm-local location.
        assert!(
            stdout.contains("DW_TAG_variable"),
            "no variable DIE for the kept let binding:\n{stdout}"
        );
        assert!(
            stdout.contains("\"x\""),
            "the local `x` is unnamed:\n{stdout}"
        );
        // `x` sits ABOVE the single param slot 0 — a variable at a non-zero local slot.
        assert!(
            stdout.contains("DW_OP_WASM_location 0x0 0x1"),
            "the kept local must live at local slot 1:\n{stdout}"
        );
    }

    #[test]
    fn a_scalar_match_binder_gets_a_lexical_block_variable() {
        // D3 match binders: a bare-binder arm over a COMPUTED scrutinee (`(match (+ a b) (x (* x x)))`)
        // binds the scrutinee's spill slot — described by a `DW_TAG_variable` inside a
        // `DW_TAG_lexical_block` whose PC range fences it to the match (its slot is a reused scratch slot,
        // so a function-scoped variable would misreport `x` for the rest of the function). Verified via
        // llvm-dwarfdump on the sidecar. Skips if the tool is absent.
        use std::io::Write;
        use std::process::Command;
        let src =
            "(module m (def (f (: a Int64) (: b Int64)) (match (+ a b) (x (* x x)))) (export f))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-mb-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        assert!(
            !stdout.to_lowercase().contains("error:"),
            "dwarfdump error:\n{stdout}"
        );
        // A lexical block scopes the binder — NOT a function-scoped variable.
        assert!(
            stdout.contains("DW_TAG_lexical_block"),
            "no lexical block for the match binder:\n{stdout}"
        );
        // The binder `x` is a variable inside it, with a wasm-local location (the scrutinee spill slot).
        assert!(
            stdout.contains("DW_TAG_variable") && stdout.contains("\"x\""),
            "the match binder `x` is missing:\n{stdout}"
        );
        // The lexical block's low_pc must be ABOVE the subprogram's (the block covers only the match's
        // arm code, not the whole function) — i.e. two distinct low_pc values appear.
        let low_pcs: Vec<&str> = stdout
            .lines()
            .filter_map(|l| l.trim().strip_prefix("DW_AT_low_pc"))
            .collect();
        assert!(
            low_pcs.len() >= 3,
            "expected a low_pc for the CU, subprogram, AND lexical block:\n{stdout}"
        );
    }

    #[test]
    fn a_scalar_match_binder_dwarf_verifies_under_llvm_dwarfdump() {
        // The lexical-block DIE tree (subprogram → lexical_block → variable, with the nested NULL
        // terminators) must be WELL-FORMED — `llvm-dwarfdump --verify` reports no errors. This guards the
        // delicate DIE-tree/offset math of the match-binder scope. Skips if the tool is absent.
        use std::io::Write;
        use std::process::Command;
        let src =
            "(module m (def (f (: a Int64) (: b Int64)) (match (- a b) (y (+ y 1)))) (export f))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-mbv-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--verify")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("No errors"),
            "the match-binder DWARF failed --verify:\n{stdout}"
        );
    }

    #[test]
    fn single_line_constructs_get_distinct_columns() {
        // COLUMN granularity: an `if` whose condition, then-branch, and else-branch all sit on ONE
        // source line still produces DISTINCT line-table rows — each carrying the sub-expression's
        // COLUMN — so a debugger highlights the exact construct within the line (the payoff for
        // s-expression Cadenza, where a whole `(if c a b)` is one line). Line-only granularity would
        // collapse these to a single row. Skips if llvm-dwarfdump is absent.
        use std::io::Write;
        use std::process::Command;
        // Everything on line 2 (line 1 is the module header) — distinct columns, same line.
        let src = "(module m\n(def (f (: a Int64)) (if (< a 0) (- a 1) (+ a 1)))\n(export f))";
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-cols-{}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(&dwarf))
            .expect("write");
        let output = match Command::new("llvm-dwarfdump")
            .arg("--debug-line")
            .arg(&path)
            .output()
        {
            Ok(o) => o,
            Err(_) => {
                eprintln!("llvm-dwarfdump not found; skipping");
                let _ = std::fs::remove_file(&path);
                return;
            }
        };
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
        // Line-table rows begin with `0x…`; columns are [Address, Line, Column, …]. Collect the
        // (line, column) of the rows on line 2 (the single source line the code lives on).
        let cols: Vec<u32> = stdout
            .lines()
            .filter_map(|l| {
                let l = l.trim_start();
                if !l.starts_with("0x") {
                    return None;
                }
                let mut it = l.split_whitespace();
                let _addr = it.next()?;
                let line: u32 = it.next()?.parse().ok()?;
                let col: u32 = it.next()?.parse().ok()?;
                (line == 2).then_some(col)
            })
            .collect();
        // Several DISTINCT non-zero columns on the one source line — the condition, then, and else each
        // get their own row (line-only granularity would give a single row / a single column).
        let mut distinct: Vec<u32> = cols.iter().copied().filter(|&c| c != 0).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() >= 3,
            "expected ≥3 distinct source columns on line 2 (condition/then/else), got {distinct:?}\n{stdout}"
        );
    }

    #[test]
    fn a_runtime_bytes_returning_program_carries_dwarf() {
        // A program returning a RUNTIME `Bytes` (a recursion + `Bytes.concat` result) crosses via the
        // looping-walker escape path (`emit_runtime_bytes_resource`) — a THIRD resource core beyond the
        // flat/sum ones. Its user bodies lead the code section, so the `.debug_*` + `name` sections
        // attribute correctly; a compound/bytes-returning program is debuggable too.
        let src = "(module m \
                     (def (uleb n) (if (< n 128) \
                        ((. Bytes of) (list ((. UInt8 wrap) n))) \
                        ((. Bytes concat) ((. Bytes of) (list ((. UInt8 wrap) (| (& n 127) 128)))) (uleb (>> n 7))))) \
                     (def (main) (uleb 624485)) \
                     (export main))";
        let debug = component_of(src, Target::WasmDebug);
        let has = |needle: &[u8]| debug.windows(needle.len()).any(|w| w == needle);
        for want in [
            b".debug_info".as_slice(),
            b".debug_line".as_slice(),
            b"\x04name".as_slice(),
        ] {
            assert!(
                has(want),
                "a runtime-Bytes program must carry {:?}",
                std::str::from_utf8(want).unwrap_or("<name>")
            );
        }
        assert!(
            has(b"uleb") && has(b"main"),
            "DWARF must name the source functions"
        );
        // A plain build embeds no DWARF (debug-only sections).
        let plain = component_of(src, Target::Wasm);
        assert!(
            !plain
                .windows(b".debug_info".len())
                .any(|w| w == b".debug_info"),
            "a plain runtime-Bytes component must carry no DWARF"
        );
    }
}

// ── C-HOST-1: a Cadenza CLOSURE exported to the host as a component-model RESOURCE with a `call` method ──
//
// The FIRST end-to-end proof of the closures-across-the-host-boundary feature
// (`DESIGN-closure-host-resource-rcdzc.md`). This is the ORACLE: a `ComponentBuilder`-built reference
// that RUNS under wasmtime, proving a guest-exported resource whose `call` method dispatches through the
// guest's own funcref table (`call_indirect`) is accepted and returns the right value. The compiler then
// hand-emits byte-identical against this shape (C-HOST-1 emit-path increment).
//
// Scope of THIS oracle: a NO-CAPTURE closure `(fn (x) (+ x 1))`, standalone (no value-heap runtime
// composed) — the closure "cell" is modelled directly as the funcref-table SLOT (an i32), which isolates
// the NEW component-model wiring (a resource with a callable method + an internal funcref table +
// resource.new/resource.rep) from the heap-runtime composition. A capturing closure (a real heap cell)
// arrives in C-HOST-2, reusing this envelope + the composed runtime.
#[cfg(test)]
mod closure_host_resource {
    /// A standalone core module that exports a closure resource's `make` + `call` primitives, plus the
    /// lifted closure body, wired through a funcref table. No heap-runtime import — the "closure cell" IS
    /// the table slot (an i32), so `make` registers that slot as the resource rep and `call` dispatches
    /// `call_indirect` on it directly. Imports only `resource-new`/`resource-rep` (threaded by the
    /// envelope). Exports: `make : () -> i32` (rep = the table slot), `call : (i32 rep, i64 x) -> i64`.
    ///
    ///  * lifted closure `lifted(x: i64) -> i64` = `x + 1` (the body of `(fn (x) (+ x 1))`). It takes NO
    ///    env param here (no captures in C-HOST-1); the real compiler prepends an env cell, added in
    ///    C-HOST-2.
    ///  * `make()` → `resource.new(0)`: table slot 0 is the closure's code, so the rep IS 0. (A capturing
    ///    closure's rep will be a heap cell handle instead.)
    ///  * `call(self_handle, x)` → `resource.rep(self_handle)` recovers the rep (the table slot); push
    ///    `x`, then `call_indirect` the recovered slot against the lifted functype.
    fn closure_call_core() -> Vec<u8> {
        use wasm_encoder::*;
        let mut m = Module::new();

        // Types: 0 = resource-new/resource-rep (i32)->i32; 1 = lifted (i64)->i64 (the call_indirect
        // functype); 2 = make ()->i32; 3 = call (i32,i64)->i64.
        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I32], vec![ValType::I32]); // 0
        types.ty().function(vec![ValType::I64], vec![ValType::I64]); // 1 (lifted / indirect)
        types.ty().function(vec![], vec![ValType::I32]); // 2 make
        types
            .ty()
            .function(vec![ValType::I32, ValType::I64], vec![ValType::I64]); // 3 call
        m.section(&types);

        // Imports: resource-new (func 0), resource-rep (func 1) from "heap".
        let mut imports = ImportSection::new();
        imports.import("heap", "resource-new", EntityType::Function(0));
        imports.import("heap", "resource-rep", EntityType::Function(0));
        m.section(&imports);
        let f_rnew = 0u32;
        let f_rrep = 1u32;

        // Defined funcs: lifted = 2 (type 1), make = 3 (type 2), call = 4 (type 3).
        let mut funcs = FunctionSection::new();
        funcs.function(1); // lifted
        funcs.function(2); // make
        funcs.function(3); // call
        m.section(&funcs);
        let f_lifted = 2u32;
        let f_make = 3u32;
        let f_call = 4u32;

        // One funcref table of size 1, slot 0 = the lifted closure.
        let mut tables = TableSection::new();
        tables.table(TableType {
            element_type: RefType::FUNCREF,
            minimum: 1,
            maximum: Some(1),
            table64: false,
            shared: false,
        });
        m.section(&tables);

        let mut exports = ExportSection::new();
        exports.export("make", ExportKind::Func, f_make);
        exports.export("call", ExportKind::Func, f_call);
        m.section(&exports);

        // Active element segment: table 0, offset 0, [lifted].
        let mut elems = ElementSection::new();
        elems.active(
            Some(0),
            &ConstExpr::i32_const(0),
            Elements::Functions(std::borrow::Cow::Borrowed(&[f_lifted])),
        );
        m.section(&elems);

        let mut code = CodeSection::new();
        // lifted(x) = x + 1
        let mut lifted = Function::new(vec![]);
        lifted.instruction(&Instruction::LocalGet(0));
        lifted.instruction(&Instruction::I64Const(1));
        lifted.instruction(&Instruction::I64Add);
        lifted.instruction(&Instruction::End);
        code.function(&lifted);
        // make() = resource.new(0)  — rep is the table slot (0) of the closure's code
        let mut make = Function::new(vec![]);
        make.instruction(&Instruction::I32Const(0));
        make.instruction(&Instruction::Call(f_rnew));
        make.instruction(&Instruction::End);
        code.function(&make);
        // call(self, x) = call_indirect[type 1](x, table_slot = resource.rep(self))
        let mut call = Function::new(vec![]);
        call.instruction(&Instruction::LocalGet(1)); // x (the closure arg)
        call.instruction(&Instruction::LocalGet(0)); // self handle
        call.instruction(&Instruction::Call(f_rrep)); // → the rep (table slot)
        call.instruction(&Instruction::CallIndirect {
            type_index: 1,
            table_index: 0,
        });
        call.instruction(&Instruction::End);
        code.function(&call);
        m.section(&code);
        m.finish()
    }

    /// The inner re-export component that publishes the closure resource WITH its `call` method — the
    /// closure analog of `inner_reexport_component` (which re-exports `make`/`encode`). It imports the
    /// resource abstractly (`SubResource`) plus `make : () -> own<t>` and `call : (self: own<t>, i64) ->
    /// i64`, then re-exports the resource type DIRECTLY (publishing its identity — no `SubResource`
    /// ascription, which would mint a distinct resource) and the two funcs ASCRIBED against the exported
    /// identity. The outer component instantiates this with the real rep-carrying resource + lifted funcs.
    fn inner_reexport_component() -> wasm_encoder::ComponentBuilder {
        use wasm_encoder::*;
        let mut c = ComponentBuilder::default();
        let imp_t = c.import(
            "import-type-t",
            ComponentTypeRef::Type(TypeBounds::SubResource),
        ); // type 0
        // make : () -> own<0>
        let (own_imp, od) = c.type_defined();
        od.own(imp_t);
        let (make_ty, mut mf) = c.type_function();
        mf.params::<[(&str, ComponentValType); 0], _>([])
            .result(Some(ComponentValType::Type(own_imp)));
        let make_fn = c.import("import-func-make", ComponentTypeRef::Func(make_ty)); // func 0
        // call : (self: own<0>, x: s64) -> s64
        let (own_imp2, od2) = c.type_defined();
        od2.own(imp_t);
        let (call_ty, mut cf) = c.type_function();
        cf.params([
            ("self", ComponentValType::Type(own_imp2)),
            ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
        ])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        let call_fn = c.import("import-func-call", ComponentTypeRef::Func(call_ty)); // func 1
        // RE-EXPORT the resource type directly (publish `imp_t`'s identity under `t`).
        let exp_t = c.export("t", ComponentExportKind::Type, imp_t, None);
        // make ascribed against the exported identity.
        let (own_exp, od3) = c.type_defined();
        od3.own(exp_t);
        let (make_exp_ty, mut mf2) = c.type_function();
        mf2.params::<[(&str, ComponentValType); 0], _>([])
            .result(Some(ComponentValType::Type(own_exp)));
        c.export(
            "make",
            ComponentExportKind::Func,
            make_fn,
            Some(ComponentTypeRef::Func(make_exp_ty)),
        );
        // call ascribed against the exported identity.
        let (own_exp2, od4) = c.type_defined();
        od4.own(exp_t);
        let (call_exp_ty, mut cf2) = c.type_function();
        cf2.params([
            ("self", ComponentValType::Type(own_exp2)),
            ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
        ])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        c.export(
            "call",
            ComponentExportKind::Func,
            call_fn,
            Some(ComponentTypeRef::Func(call_exp_ty)),
        );
        c
    }

    /// The outer oracle component: wraps `closure_call_core` in a resource with `make` + `call`, published
    /// as `cadenza:closure/exports`. Standalone (no heap runtime) — a `heap` core-instance exports only
    /// `resource-new`/`resource-rep`, which the core imports. `call` is lifted against `own<t>` for now
    /// (own/no-drop; the `borrow<t>` migration is C-HOST-5, shared with the value-escape's `encode`).
    fn oracle_closure_component(core: &[u8]) -> Vec<u8> {
        use wasm_encoder::*;
        let mut c = ComponentBuilder::default();
        // dtor module (imports nothing) → instantiate first → the resource type has a real dtor core-func.
        let dtor_idx = c.core_module_raw(&dtor_stub_module());
        let dtor_inst = c.core_instantiate(dtor_idx, std::iter::empty::<(&str, ModuleArg)>());
        let dtor_core = c.core_alias_export(dtor_inst, "t-dtor", ExportKind::Func);
        let res_ty = c.type_resource(ValType::I32, Some(dtor_core));
        let rnew_core = c.resource_new(res_ty);
        let rrep_core = c.resource_rep(res_ty);
        // heap core-instance exporting the two resource intrinsics; instantiate the program core.
        let heap_inst = c.core_instantiate_exports([
            ("resource-new", ExportKind::Func, rnew_core),
            ("resource-rep", ExportKind::Func, rrep_core),
        ]);
        let module_idx = c.core_module_raw(core);
        let prog_inst = c.core_instantiate(module_idx, [("heap", ModuleArg::Instance(heap_inst))]);
        let make_core = c.core_alias_export(prog_inst, "make", ExportKind::Func);
        let call_core = c.core_alias_export(prog_inst, "call", ExportKind::Func);
        // lift make : () -> own<t>
        let (own_t, odef) = c.type_defined();
        odef.own(res_ty);
        let (make_ty, mut mf) = c.type_function();
        mf.params::<[(&str, ComponentValType); 0], _>([])
            .result(Some(ComponentValType::Type(own_t)));
        let make_comp = c.lift_func(make_core, make_ty, []);
        // lift call : (self: own<t>, x: s64) -> s64
        let (own_t2, odef2) = c.type_defined();
        odef2.own(res_ty);
        let (call_ty, mut cf) = c.type_function();
        cf.params([
            ("self", ComponentValType::Type(own_t2)),
            ("x", ComponentValType::Primitive(PrimitiveValType::S64)),
        ])
        .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        let call_comp = c.lift_func(call_core, call_ty, []);
        // inner re-export → cadenza:closure/exports.
        let inner_idx = c.component(inner_reexport_component());
        let inst = c.instantiate(
            inner_idx,
            [
                ("import-type-t", ComponentExportKind::Type, res_ty),
                ("import-func-make", ComponentExportKind::Func, make_comp),
                ("import-func-call", ComponentExportKind::Func, call_comp),
            ],
        );
        c.export(
            "cadenza:closure/exports",
            ComponentExportKind::Instance,
            inst,
            None,
        );
        c.finish()
    }

    /// A minimal dtor core module: `t-dtor : (i32 rep) -> ()`, empty body (own/no-drop C-HOST-1 — the rep
    /// is a table slot, nothing to release). Imports nothing → instantiates first. The `borrow<t>` +
    /// real-drop dtor is C-HOST-5.
    fn dtor_stub_module() -> Vec<u8> {
        use wasm_encoder::*;
        let mut m = Module::new();
        let mut types = TypeSection::new();
        types.ty().function(vec![ValType::I32], vec![]);
        m.section(&types);
        let mut funcs = FunctionSection::new();
        funcs.function(0);
        m.section(&funcs);
        let mut exports = ExportSection::new();
        exports.export("t-dtor", ExportKind::Func, 0);
        m.section(&exports);
        let mut code = CodeSection::new();
        let mut f = Function::new(vec![]);
        f.instruction(&Instruction::End);
        code.function(&f);
        m.section(&code);
        m.finish()
    }

    /// C-HOST-1 END-TO-END: the closure-resource oracle RUNS under wasmtime. Build the component, call
    /// `make()` to get the closure resource handle, then `call(handle, 5)` — which dispatches
    /// `(fn (x) (+ x 1))` through the guest's funcref table via `call_indirect` — and expect 6. This is
    /// the proof that a Cadenza closure can cross to the host as a resource the host invokes.
    #[test]
    fn a_closure_crosses_as_a_resource_the_host_calls() {
        use wasmtime::component::{Component, Linker, Val};
        use wasmtime::{Engine, Store};
        let comp = oracle_closure_component(&closure_call_core());
        // Validate structurally first (localize any byte/index error before wasmtime).
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&comp)
            .expect("closure-resource component validates");

        let engine = Engine::default();
        let component = Component::from_binary(&engine, &comp).expect("valid component");
        let linker: Linker<()> = Linker::new(&engine);
        let mut store = Store::new(&engine, ());
        let instance = linker
            .instantiate(&mut store, &component)
            .expect("instantiate");
        let iface = instance
            .get_export_index(&mut store, None, "cadenza:closure/exports")
            .expect("closure interface");
        let make_idx = instance
            .get_export_index(&mut store, Some(&iface), "make")
            .expect("make export");
        let call_idx = instance
            .get_export_index(&mut store, Some(&iface), "call")
            .expect("call export");
        let make = instance.get_func(&mut store, make_idx).expect("make func");
        let call = instance.get_func(&mut store, call_idx).expect("call func");

        // make() → the closure resource handle.
        let mut handle = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle).expect("make call");
        make.post_return(&mut store).expect("make post_return");
        assert!(
            matches!(handle[0], Val::Resource(_)),
            "make must return a resource, got {:?}",
            handle[0]
        );

        // call(handle, 5) → 6  (the closure (fn (x) (+ x 1)) dispatched via the guest's call_indirect).
        // NOTE: `call` takes `own<t>`, so it CONSUMES the handle — the resource is single-use per handle
        // in C-HOST-1 (own/no-drop). The host `make`s a fresh handle for each invocation. Making `call`
        // take `borrow<t>` (so one handle serves repeated calls, the natural callback shape) is C-HOST-5,
        // shared with the value-escape's `encode` borrow migration.
        let mut out = [Val::Bool(false)];
        call.call(&mut store, &[handle[0].clone(), Val::S64(5)], &mut out)
            .expect("call(handle, 5)");
        call.post_return(&mut store).expect("call post_return");
        assert_eq!(out[0], Val::S64(6), "closure (+ x 1) applied to 5 = 6");

        // A SECOND handle from a fresh `make`, called with a different arg — proves the resource+dispatch
        // is reusable across handles (each `own` handle is one-shot). Same closure code, applied to 41.
        let mut handle2 = [Val::Bool(false)];
        make.call(&mut store, &[], &mut handle2).expect("make 2");
        make.post_return(&mut store).expect("make 2 post_return");
        let mut out2 = [Val::Bool(false)];
        call.call(&mut store, &[handle2[0].clone(), Val::S64(41)], &mut out2)
            .expect("call(handle2, 41)");
        call.post_return(&mut store).expect("call post_return 2");
        assert_eq!(out2[0], Val::S64(42), "same closure applied to 41 = 42");
        // Both `own` handles were consumed by `call`; a borrow-based repeated-call handle is C-HOST-5.
    }

    /// C-HOST-1 (compiler serializer): `serialize::closure_resource_core_module` — the PRODUCTION core
    /// module a closure-resource export emits — produces a structurally VALID core module. Unlike the
    /// standalone oracle above (whose "cell" is the bare table slot), this is the real shape: the export
    /// body builds a value-heap CELL (`arr-alloc`/`box-int`/`arr-set`), and `call` recovers it
    /// (`resource.rep`) + reads the code slot (`arr-get`/`get-int`) + `call_indirect`s the lifted body. It
    /// imports the heap ops + `resource-new`/`resource-rep`, and carries the funcref table from
    /// `layout.lifted`. This pins the serializer's byte shape (validated by `wasmparser`) with real,
    /// minimally-constructed `SelectedFunc`s — the runnable end-to-end path (wiring this core into the
    /// production envelope + composing the runtime) is the next increment.
    #[test]
    fn closure_resource_core_module_is_structurally_valid() {
        use crate::backend::wasm::lir::{Lir, ValType};
        use crate::backend::wasm::runtime_abi::OPS;
        use crate::backend::wasm::select::SelectedFunc;
        use crate::layout::{ExportPlan, Layout};
        use crate::lower::LiftedLambda;
        use crate::ty::{IntTy, Ty};

        let s64 = Ty::Int(IntTy::fixed(true, 64));
        // The heap ops the core imports, in the SORTED order `collect_used_ops` (a BTreeSet) produces:
        // arr-alloc, arr-get, arr-set, box-int, get-int. `call` uses arr-get/get-int; the export body's
        // cell build uses arr-alloc/arr-set/box-int.
        let imports = vec![
            OPS.arr_alloc,
            OPS.arr_get,
            OPS.arr_set,
            OPS.box_int,
            OPS.get_int,
        ];
        // Defined func 0 = the export body `main : () -> own<closure>` — its RESULT is the closure type,
        // whose machine valtype is an i32 cell handle. Build a 1-slot cell holding box-int(0) (the
        // closure's table slot 0, no captures), returning the cell handle. (Mirrors `Core::Closure`
        // selection.) Uses arr-alloc/box-int/arr-set.
        let fn_ty = Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone()));
        let export_body = SelectedFunc {
            params: vec![],
            ret: fn_ty.clone(), // a closure result → an i32 cell handle in the type section
            code: vec![
                Lir::ConstI32(1),
                Lir::CallImport("arr-alloc"),
                Lir::ConstI32(0),
                Lir::ConstI64(0),
                Lir::CallImport("box-int"),
                Lir::CallImport("arr-set"),
            ],
            declared: vec![],
            src_body: None,
            locals: vec![],
            scopes: vec![],
            stmt_lines: vec![],
        };
        // Defined func 1 = the lifted closure `(env: i32, x: i64) -> i64` = x + 1.
        let lifted_body = SelectedFunc {
            params: vec![ValType::I32, ValType::I64],
            ret: s64.clone(),
            code: vec![Lir::LocalGet(1), Lir::ConstI64(1), Lir::I64Add],
            declared: vec![],
            src_body: None,
            locals: vec![],
            scopes: vec![],
            stmt_lines: vec![],
        };
        let funcs = vec![export_body, lifted_body];

        // Layout: one export (def 0), order [0, 1] (export then lifted-as-def), one lifted lambda in the
        // table. The lifted lambda's `body`/`params` here are placeholder (StructId 0 / the arg types) —
        // only its presence in `layout.lifted` matters for the table + `lifted_type_index`.
        let export = ExportPlan {
            name: "main".to_string(),
            def: 0,
            body: crate::ast::StructId(0),
            params: vec![],
            result: Ty::Fn(Box::new(s64.clone()), Box::new(s64.clone())),
        };
        let lifted = LiftedLambda {
            body: crate::ast::StructId(0),
            params: vec![(crate::ast::StructId(0), s64.clone())],
            ret_ty: s64.clone(),
            captures: vec![],
        };
        let import_base = imports.len() as u32 + 2; // k ops + resource-new + resource-rep
        // `order` is just the ONE export def; the lifted closure is appended to `funcs` AFTER the order
        // defs (as the production path does), so its abs index + type index are `import_base +
        // order.len() + slot`. `funcs = [export_body, lifted_body]` matches this layout.
        let layout =
            Layout::with_lifted(vec![export], vec![0], import_base, vec![lifted], vec![true]);

        // The export body is at emission position 0 → absolute core-func index `import_base + 0`.
        let export_abs = import_base;
        // The lifted functype index (env i32, x i64) -> i64 in the core type section.
        let lifted_type_idx = layout.lifted_type_index(0, import_base);

        let core = crate::backend::wasm::serialize::closure_resource_core_module(
            &funcs,
            &imports,
            export_abs,
            &[ValType::I64],
            ValType::I64,
            &[], // nullary export → make() has no params
            lifted_type_idx,
            &layout,
        )
        .expect("closure-resource core serializes");

        // The emitted CORE MODULE must be structurally valid wasm (funcref table + call_indirect + the
        // resource-intrinsic imports all well-formed). This is the compiler-side byte proof; the runnable
        // end-to-end path (below) wires the WHOLE pipeline through the composed runtime.
        let mut validator =
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        validator
            .validate_all(&core)
            .expect("closure-resource core module validates");
    }

    /// C-HOST-1 END-TO-END (the whole COMPILER pipeline): a real `(def (main) (fn (x) (+ x 1)))` program
    /// compiles to a closure-resource component (`emit_closure_resource` → `closure_resource_core_module`
    /// → `assemble_closure_resource`), and the HOST calls it — `make()` → closure handle, `call(handle, 5)`
    /// → 6 — dispatched through the guest's own `call_indirect`, composing the real value-heap runtime (the
    /// closure cell is a heap allocation). The production analog of the C-HOST-1 oracle. `#[ignore]` — needs
    /// `xtask build` to have populated the store with the runtime wasm.
    #[test]
    fn a_compiled_closure_export_is_called_by_the_host() {
        use crate::testkit::parse;
        use wasmtime::component::Val;
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not in the store (run `cargo xtask build`); skipping");
            return;
        };
        // A nullary export returning a closure `(-> Int64 Int64)`. The compiler routes it through the
        // closure-resource escape; the host holds the resource and calls it.
        let src = "(module m (def (main) (fn ((: x Int64)) (+ x 1))) (export main))";
        let program =
            crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        // It must import the value-heap runtime (the closure cell is a heap allocation) and export the
        // closure interface — not a bare function.
        assert!(
            cdz_run::required_runtime(&program)
                .expect("valid")
                .is_some(),
            "a closure-resource export imports the value-heap runtime (the cell is a heap value)"
        );
        let mut rt = super::ComposedRuntime::new(&program, &runtime);
        // make() → closure handle (the export is nullary); call(handle, 5) → 6.
        assert_eq!(
            rt.closure_make_call(&[], &[Val::S64(5)]),
            Val::S64(6),
            "the exported closure (fn (x) (+ x 1)) applied to 5 = 6"
        );
        // A fresh handle called with 41 → 42 (own<t> consumed the first; each call mints a new handle).
        assert_eq!(rt.closure_make_call(&[], &[Val::S64(41)]), Val::S64(42));
    }

    /// C-HOST-2: a PARAMETERIZED export returning a CAPTURING closure. `(def (adder (: k Int64)) (fn (x)
    /// (+ x k)))` crosses as `adder : (s64) -> own<closure>`; the host calls `make(10)` (which runs the
    /// export body, closing over k=10 into the cell), then `call(handle, 5)` → 15. This proves (a) the
    /// closure handle is COMPUTED from the host's input (make forwards the export param), and (b) the
    /// captured environment (k) rides along in the cell and is read back inside `call`'s dispatch.
    #[test]
    fn a_compiled_capturing_closure_export_is_called_by_the_host() {
        use crate::testkit::parse;
        use wasmtime::component::Val;
        let Some(runtime) = super::find_runtime_wasm() else {
            eprintln!("runtime wasm not in the store (run `cargo xtask build`); skipping");
            return;
        };
        let src = "(module m (def (adder (: k Int64)) (fn ((: x Int64)) (+ x k))) (export adder))";
        let program =
            crate::compile::compile_component(&crate::codec::encode(&parse(src))).expect("compile");
        assert!(
            cdz_run::required_runtime(&program).expect("valid").is_some(),
            "a capturing closure export imports the value-heap runtime"
        );
        let mut rt = super::ComposedRuntime::new(&program, &runtime);
        // make(10) → a closure capturing k=10; call(handle, 5) → 5 + 10 = 15.
        assert_eq!(
            rt.closure_make_call(&[Val::S64(10)], &[Val::S64(5)]),
            Val::S64(15),
            "adder(10) then call(5) = 15 — the captured k rides in the cell"
        );
        // A different capture (k=100) + a different call arg (7) → 107 — the handle tracks make's input.
        assert_eq!(
            rt.closure_make_call(&[Val::S64(100)], &[Val::S64(7)]),
            Val::S64(107)
        );
    }
}
