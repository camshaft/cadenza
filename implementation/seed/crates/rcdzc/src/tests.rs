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
/// To keep the record OPAQUE to the fold (a call-returned record now folds through — cycle 16), it comes
/// from an `if` whose branches DIFFER in the `a` field (`n` vs `n+1`): `reduce_to_record_id` does not
/// see through an `if`, so `main` selects one at run time and reads `a` off the heap.
#[test]
fn a_field_of_a_runtime_record_reads_the_heap_at_its_sorted_index() {
    use crate::testkit::parse;
    // The record's `a` field is the parameter; fields written z-before-a so the sorted layout (a=0, z=1)
    // differs from the write order. The `if` has a RUNTIME condition `(< v 100)` (not compile-time
    // constant, so it does not fold to a taken literal branch) and branches that DIFFER in `a` — so the
    // record stays a runtime heap value. For v = 41 the condition is true, `a` = v, and projecting `a`
    // reads slot 0 = the argument.
    let src = "(module m \
                 (def (main (: v Int64)) \
                   (. (if (< v 100) (record (z 9) (a v)) (record (z 9) (a (+ v 1)))) a)) \
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
/// The argument is an `if`-selected record (runtime condition, branches differing in `x`) so it stays
/// OPAQUE to the fold (a plain `(mk v)` would fold through — cycle 16): `get-x` reads `x` off the heap
/// at run time. For v = 41 the condition holds, so `x` = v; composed run: 41.
#[test]
fn a_projected_bare_parameter_is_constrained_at_the_call_site() {
    use crate::testkit::parse;
    let src = "(module m (def (get-x r) (. r x)) (def (mk n) (record (x n) (y 2))) \
                 (def (main (: v Int64)) (get-x (if (< v 100) (mk v) (mk (+ v 1))))) (export main))";
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "if-list-len b={arg}"),
                cdz_run::Outcome::Trap(t) => panic!("if-list-len run trapped: {t}"),
            }
        }
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
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => Some(s),
            cdz_run::Outcome::Trap(t) => panic!("list heap run trapped (miscompile?): {t}"),
        }
    }

    #[test]
    fn a_list_push_extends_and_its_runtime_length_counts() {
        // `List.push(l, x)` appends `x`, building a new persistent list on the `vec-*` heap. The result
        // is NOT a compile-time-visible literal (it is a `Core::ListPush`), so `List.len` of it does NOT
        // fold — it emits the runtime `vec-len`. So `(List.len (List.push (list 1 2 3) 4))` exercises the
        // full runtime path: `vec-empty` + 3 boxed `vec-push` (the literal) + one more `vec-push` (the
        // appended 4) + `vec-len` → 4.
        let Some(out) = run_on_heap(
            "(module m (def (main) ((. List len) ((. List push) (list 1 2 3) 4))) (export main))",
        ) else {
            eprintln!("runtime wasm not found (run `cargo xtask build`); skipping composed run");
            return;
        };
        assert_eq!(out, "4", "push then runtime length");
    }

    #[test]
    fn a_list_concat_joins_and_its_runtime_length_sums() {
        // `List.concat(a, b)` joins two lists into a new persistent one (`vec-concat`, consuming both).
        // `(List.len (List.concat (list 1 2) (list 3 4 5)))` builds both spines then concatenates, so the
        // runtime `vec-len` counts 2 + 3 = 5. Pins the runtime concat + length path.
        let Some(out) = run_on_heap(
            "(module m (def (main) ((. List len) ((. List concat) (list 1 2) (list 3 4 5)))) (export main))",
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
    fn a_boolean_connective_operand_must_be_bool() {
        // A non-Bool operand is a type mismatch (CDZ0203), the same class as a non-Bool `if` condition.
        assert_eq!(
            reject_code("(module m (def (main) (and true 1)) (export main))").as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            reject_code("(module m (def (main) (or 5 false)) (export main))").as_deref(),
            Some("CDZ0203")
        );
        assert_eq!(
            reject_code("(module m (def (main) (not 7)) (export main))").as_deref(),
            Some("CDZ0203")
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "classify {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("guarded runtime match trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_guard_over_a_variant_pattern_binds_the_payload_and_declines_cleanly() {
        // A guard over a VARIANT pattern `(guard (Some x) (> x 0))` — the payload binder `x` must be in
        // SCOPE for the guard cond (resolve Case 6 sees through the `(guard …)` wrapper to `(Some x)`),
        // so the guard is NOT a spurious "unbound name `x`" (CDZ0101, the pre-fix diagnostic). The sum
        // decision tree has no guard support yet, so the arm DECLINES — but CLEANLY, with an honest
        // "guard over a variant pattern is not yet supported" message, NOT the misleading "a sum match
        // pattern head is not a variant constructor" (misreading the `(guard …)` head). A decline is an
        // uncoded reject: `reject_code` returns `None`, and the message names the real gap.
        let src = "(module m \
                     (def (f (: o (Option Int64))) \
                        (match o ((guard (Some x) (> x 0)) x) ((Some y) (- 0 y)) ((None) 0))) \
                     (def (main (: n Int64)) (f (Some n))) (export main))";
        let out = compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "m",
                crate::codec::encode(&parse(src)),
            )],
            &[Target::Wasm],
        );
        let err = out
            .diagnostics
            .iter()
            .find(|d| d.severity == crate::abi::Severity::Error)
            .expect("a guarded variant pattern declines");
        // A DECLINE (uncoded), NOT the CDZ0101 unbound-name the resolve bug produced.
        assert_eq!(err.code, None, "must be a clean decline, not a coded reject: {err:?}");
        assert!(
            err.message.contains("guard over a variant pattern"),
            "the decline must name the real gap (guarded variant match), got: {}",
            err.message
        );
        // CONTROLS unaffected: a plain `(Some x)` destructure compiles, and a scalar guard compiles.
        assert!(
            compile_component(&crate::codec::encode(&parse(
                "(module m (def (f (: o (Option Int64))) (match o ((Some x) x) ((None) 0))) \
                 (def (main (: n Int64)) (f (Some n))) (export main))"
            )))
            .is_ok(),
            "a plain variant destructure must still compile"
        );
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
            let ws = warnings_of(src);
            assert_eq!(
                ws.len(),
                1,
                "expected exactly one dead-trap warning for `{src}`, got {ws:?}"
            );
            assert_eq!(ws[0].code.as_deref(), Some("CDZ0305"), "for `{src}`");
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
}

// ── Stage 1: let + records (compile-time folded) ────────────────────────────────────────────────
//
// Each case mirrors a `05-compound-types.sexp` / `02-binding-and-control.sexp` witness. A record
// whose field is read FOLDS to a scalar (no value heap), so the component runs to that scalar; a
// record used as a runtime value declines (needs the heap, a later stage). Programs are built with
// the test s-expr reader in `testkit`.
mod stage1 {
    use super::{FromVal, run_returns};
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
        assert!(
            err.message.contains("value heap")
                || err.message.contains("renderer")
                || err.message.contains("boundary"),
            "got: {}",
            err.message
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
    fn comparison_of_a_compound_declines() {
        // Structural comparison over the value heap is a later stage — comparing records declines
        // cleanly (the operator's type stays generic; only the fold's coverage is bounded).
        let msg = expect_decline("(= (record (x 1)) (record (x 1)))");
        assert!(
            msg.contains("compound") || msg.contains("value heap") || msg.contains("not yet"),
            "got: {msg}"
        );
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
    // On a constant it FOLDS via `IntValue::wrap_to` to a `ConstInt` already at the target width. It is
    // the honest, principled form of the old `to-byte`: the target width comes from the TYPE (`UInt8`),
    // not a magic op name. The result crosses the boundary as the target's faithful primitive (u8/s8/…).

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
        // `(UInt8.wrap -1)` = 255 — the low 8 bits of -1's two's-complement (all ones). This is exactly
        // the old `(Int.to-byte -1)`, now typed as UInt8 with the width from the type.
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
        // A top-level declaration the compiler does not model — `(effect …)`, `(pragma …)` — makes the
        // whole program DECLINE (decline-don't-miscompile), NOT silently ignore it and run `main` as if
        // it were absent. `(effect E …)` is unmodeled, so the program declines rather than compiling
        // `(def (main) 1)` alone (which would hide the effect's ill-formedness).
        let src = "(do (effect E (op f (-> Int64 Int64))) (def (main) 1) (export main))";
        assert!(
            compile_component(&crate::codec::encode(&parse(src))).is_err(),
            "a program with an unmodeled top-level form must decline"
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "generic pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed generic-sum-match run trapped: {t}"),
            }
        }
    }

    #[test]
    fn a_generic_sum_escapes_to_the_host_at_a_concrete_instantiation() {
        // A GENERIC `Option` built at `Int64` and RETURNED across the host boundary renders `(: (Some 5)
        // (Option Int64))` — the parameterized type surface (§158, the corpus form), driven by the
        // sum's solved `Ty::Sum{args:[Int64]}`. The escape walker's per-variant template reads the
        // concrete payload type (`Int64`) from the instantiation, so the `Some` arm holds a real Int64
        // hole. Composed + run through `cdz-run` (the resource shape, `export: None`).
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
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(
                    s, "(: (Some 5) (Option Int64))",
                    "generic sum escape renders the instantiation"
                )
            }
            cdz_run::Outcome::Trap(t) => panic!("composed generic-escape run trapped: {t}"),
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "prelude pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed prelude-option run trapped: {t}"),
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
    fn a_variant_construction_emits_the_heap_build_ops() {
        // Construction lowers to the right value-heap OPS — `collect_used_ops` reports exactly what
        // `emit` lays down (they must agree, or the import section omits a called op). A single-payload
        // `(Some a)` uses `sum-new` + `box-int` (box the Int64 payload); a nullary `None` uses `sum-new`
        // + `arr-alloc` (the empty-array unit payload). This proves construction reaches the heap builder
        // without needing the sum to escape (next tick) or a composed run (a dead sum folds away — a sum
        // is only observable once it escapes or is matched).
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
        // A nullary variant: `sum-new` + `arr-alloc` (the empty-array unit payload).
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
            ops2.contains("arr-alloc"),
            "None's unit payload is an empty array; got {ops2:?}"
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
        // The R2 sum escape: a single NULLARY export returning a sum crosses as a resource whose
        // `encode()` SWITCHES on `sum-disc` and renders the matching variant's `(: (Variant payload)
        // SumType)` bytes. `(Some 5)` → `(: (Some 5) Option)`; the walker builds the sum on the heap
        // (`sum-new`), reads its disc (0 → the `Some` arm), walks `sum-payload` → `get-int` → 5, and
        // returns the rendered bytes. Composed + run through `cdz-run` (the canonical host).
        use crate::testkit::parse;
        let src = "(module m (type Option (Some Int64) None) \
                     (def (main) (Option.Some 5)) (export main))";
        let bytes =
            compile_component(&crate::codec::encode(&parse(src))).expect("compile a sum escape");
        assert!(
            cdz_run::required_runtime(&bytes).expect("valid").is_some(),
            "a sum escape imports the value-heap runtime"
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
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(s, "(: (Some 5) Option)", "sum escape renders the variant")
            }
            cdz_run::Outcome::Trap(t) => panic!("composed sum-escape run trapped: {t}"),
        }
    }

    #[test]
    fn a_nullary_variant_export_escapes_as_unit_payload() {
        // The nullary arm of the disc-switch: `None` → `(: (None unit) Option)` (the corpus form — a
        // nullary variant carries the unit value). The walker reads disc 1 (the `None` arm), whose
        // template has NO holes (the `unit` payload is static), and returns its bytes.
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
        };
        match cdz_run::run(&bytes, &opts).expect("run") {
            cdz_run::Outcome::Value(s) => {
                assert_eq!(s, "(: (None unit) Option)", "nullary variant escape")
            }
            cdz_run::Outcome::Trap(t) => panic!("composed None-escape run trapped: {t}"),
        }
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "pick {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed sum-match run trapped: {t}"),
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
            };
            match cdz_run::run(&bytes, &opts).expect("run") {
                cdz_run::Outcome::Value(s) => assert_eq!(s, want, "code {arg}"),
                cdz_run::Outcome::Trap(t) => panic!("composed 3-variant run trapped: {t}"),
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
