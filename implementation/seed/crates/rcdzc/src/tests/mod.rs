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

/// Phase 1 of the operator's repo-wide `(.. v)` migration: `Arenas::rest_marker` dual-recognizes the
/// WRAPPED `(.. operand)` rest node ALONGSIDE the flat `Name("..")`+sibling marker, so a wrapped rest
/// pattern binds its rest identically to the flat one. The sexpr reader already reads `(.. v)` as a
/// `..`-headed list, so this is exercisable BEFORE v-syntax's Phase 2 surface flip. Asserts PARITY — the
/// wrapped and flat forms produce IDENTICAL diagnostics (additive: recognition only, no behavior change).
/// If they ever diverge, a rest-marker scan site was missed.
#[test]
fn a_wrapped_dotdot_rest_pattern_binds_identically_to_the_flat_marker() {
    let compile = |src: &str| {
        crate::host::run_with_compiler_stack(|| {
            crate::compile::compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    crate::codec::encode(&crate::testkit::parse(src)),
                )],
                &[crate::backend::Target::Wasm],
            )
        })
    };
    // LIST rest: `(list a (.. rest))` (WRAPPED) vs `(list a .. rest)` (FLAT) — `rest` is bound + measured.
    let flat_list = compile(
        "(module m (def (f (: xs (List Int64))) (match xs ((list a .. rest) (List.len rest)) (_ 0))) (export f))",
    );
    let wrap_list = compile(
        "(module m (def (f (: xs (List Int64))) (match xs ((list a (.. rest)) (List.len rest)) (_ 0))) (export f))",
    );
    assert_eq!(
        wrap_list
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>(),
        flat_list
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>(),
        "wrapped `(list a (.. rest))` must behave identically to flat `(list a .. rest)` (Phase 1 \
         dual-recognize) — a divergence means a rest-marker scan site was missed",
    );
    // MAP rest: `(map (= 1 v) (.. rest))` (WRAPPED) vs flat — same parity invariant.
    let flat_map = compile(
        "(module m (def (f (: mp (Map Int64 Int64))) (match mp ((map (= 1 v) .. rest) v) (_ 0))) (export f))",
    );
    let wrap_map = compile(
        "(module m (def (f (: mp (Map Int64 Int64))) (match mp ((map (= 1 v) (.. rest)) v) (_ 0))) (export f))",
    );
    assert_eq!(
        wrap_map
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>(),
        flat_map
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>(),
        "wrapped `(map … (.. rest))` must behave identically to flat `(map … .. rest)` (Phase 1 \
         dual-recognize)",
    );
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

/// LEVEL-EQUIVALENCE: a program compiled at EVERY `OptLevel` must emit a BYTE-IDENTICAL wasm artifact —
/// the correctness bar of the tiered-optimization framework (`DESIGN-tiered-optimization-levels-rcdzc.md`
/// §The one rule). Today the `PassManager` registers no passes, so every level is trivially identical;
/// this pins the invariant at the unit tier NOW, so a future migration slice that adds a pass at O2/O3
/// which accidentally changes emitted bytes fails HERE. (A later slice adds a corpus-wide `--opt-sweep`
/// gate that asserts the same VALUE at every level on both backends; this is the fast byte-identity
/// companion over a couple of representative programs.)
#[test]
fn every_opt_level_emits_byte_identical_wasm() {
    use crate::backend::Target;
    use crate::compile::{compile, compile_with_opt};
    use crate::opt::OptLevel;
    for prog in [prog_scalar(), prog_if()] {
        let art = |level: OptLevel| {
            crate::host::run_with_compiler_stack(|| {
                let out = compile_with_opt(
                    &[crate::abi::Artifact::new(
                        crate::abi::Artifact::KIND_AST,
                        "main",
                        prog.clone(),
                    )],
                    &[Target::Wasm],
                    level,
                );
                out.artifact(Target::Wasm.artifact_kind())
                    .expect("clean program emits a wasm artifact")
                    .to_vec()
            })
        };
        let baseline = art(OptLevel::O0);
        for level in OptLevel::ALL {
            assert_eq!(
                art(level),
                baseline,
                "opt level {level} changed the emitted wasm — a higher level must be \
                 observably identical, only faster/smaller"
            );
        }
        // `compile` (no level) must equal the default level `O1` — the thin-wrapper contract.
        let default_art = crate::host::run_with_compiler_stack(|| {
            let out = compile(
                &[crate::abi::Artifact::new(
                    crate::abi::Artifact::KIND_AST,
                    "main",
                    prog.clone(),
                )],
                &[Target::Wasm],
            );
            out.artifact(Target::Wasm.artifact_kind())
                .expect("clean program emits a wasm artifact")
                .to_vec()
        });
        assert_eq!(
            default_art,
            art(OptLevel::O1),
            "compile() must equal compile_with_opt(.., OptLevel::default())"
        );
    }
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

/// Value-heap H1b: the hand-emitted import envelope PARSES and TYPE-CHECKS structurally (`wasmparser`)
/// — structural + component-type validity, the semantic floor that catches every
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
    // (wasmparser, not wasmtime — the rcdzc wasmtime dev-dep drop, v-wasmtime-migration; instantiation/run
    // validity lands in the corpus heap-runs that compose the runtime via this import.)
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .expect("import envelope must be a structurally valid component");

    // The emitted bytes carry the exact versioned import name (interface@semver+hash) verbatim — the
    // name the linker matches the composed runtime against. Assert its literal bytes are present.
    let needle = import_name.as_bytes();
    assert!(
        bytes.windows(needle.len()).any(|w| w == needle),
        "versioned import name `{import_name}` not found in the emitted component"
    );
}

/// Does `component_bytes` import the value-heap runtime (`cadenza:runtime/heap…`)? A byte-level replacement
/// for the former `cdz_run::required_runtime(..).is_some()` — the runtime import name appears VERBATIM as a
/// string in the component's import section, so a byte scan is a faithful present/absent check (the H2c
/// "no per-call heap" pins only need present vs absent, never the content-address hash). Keeps these
/// compile-artifact pins off the `cdz-run` dep.
fn imports_value_heap_runtime(component_bytes: &[u8]) -> bool {
    let needle = b"cadenza:runtime/heap";
    component_bytes.windows(needle.len()).any(|w| w == needle)
}

/// Count the core-module instructions in `component_bytes` matching `pred` — an emission-strategy probe
/// (e.g. `i64.mul` count for inline-vs-emit-once, `call_indirect` count for dict erasure). Walks every
/// code-section entry with `wasmparser`; `pred` classifies each operator.
fn count_opcode(component_bytes: &[u8], pred: impl Fn(&wasmparser::Operator) -> bool) -> usize {
    use wasmparser::{Parser, Payload};
    let mut n = 0;
    for payload in Parser::new(0).parse_all(component_bytes) {
        if let Ok(Payload::CodeSectionEntry(body)) = payload {
            let mut ops = body.get_operators_reader().expect("ops");
            while let Ok(op) = ops.read() {
                if pred(&op) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// A KIND_WIT_WORLD declaring an export interface with TWO record-param members `f(m: record{a: s64}) -> s64`
/// and `g(m: record{b: s64}) -> s64` — the multi-export case a real reducer needs (on-message/on-response/
/// on-notification are three members). Each member gets its own wrapper appended to the core module.
fn two_member_record_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let s64 = |b: &mut Builder| {
        let h = b.name("s64");
        b.list(vec![h])
    };
    let member = |b: &mut Builder, member_name: &str, field: &str| {
        let fty = s64(b);
        let fname = b.name(field);
        let f_field = b.list(vec![fname, fty]);
        let rec_head = b.atom_leaf(Leaf::Str("record".into()));
        let rec = b.list(vec![rec_head, f_field]);
        let res_ty = s64(b);
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("m");
        let param_node = b.list(vec![param_h, pn, rec]);
        let result_h = b.name("result");
        let result_node = b.list(vec![result_h, res_ty]);
        let func = b.list(vec![func_h, param_node, result_node]);
        let member_h = b.name("member");
        let mn = b.name(member_name);
        b.list(vec![member_h, mn, func])
    };
    let f = member(&mut b, "f", "a");
    let g = member(&mut b, "g", "b");
    let exp_h = b.name("export");
    let iname = b.name("iface");
    let export = b.list(vec![exp_h, iname, f, g]);
    let world_h = b.name("world");
    let wn = b.name("w");
    let world = b.list(vec![world_h, wn, export]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// GENERIC partial-export decline: the partial-guest decline is driven purely off the world's WIT shape, NOT
/// any particular interface. Here the world is `two_member_record_world_bytes` — an interface `iface` with
/// arbitrarily-named record-param members `f`/`g` (NO reducer/`guest`/on-message knowledge anywhere). A guest
/// that defines only `f` (not `g`) must DECLINE, exactly as a partial reducer guest does: a component must
/// export every member of the interface it exports. This pins the operator's generic-compiler invariant — the
/// decline is not reducer-specific — so a future change that special-cased a particular interface would flip
/// this from decline to (mis-)emit and fail the gate.
#[test]
fn a_partial_guest_of_any_multi_member_interface_declines_cleanly() {
    use crate::testkit::parse;
    // Only `f` — `g` (the world's other member) is unimplemented.
    let src = "(module m \
                 (def (f (: m (Record (a Int64)))) (. m a)) \
                 (export f))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:demo/iface"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                two_member_record_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.has_error(),
        "a partial guest (1 of 2 interface members) must decline, not silently heap-fall"
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "no wasm artifact — no silent heap-handle mis-emit"
    );
    assert!(
        out.diagnostics.iter().any(|d| d
            .message
            .contains("does not fully implement the world's typed export interface")),
        "the generic decline names the missing-export-member cause: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// The FULL 3-member reducer world — `on-message`/`on-response`/`on-notification`, matching
/// `cdz-platform/wit/world.wit` exactly so the emitted component is drivable by v-platform's host. `message`
/// (decl-ordered, nested `sender`), `response` (`contract, token, answer: result<list<u8>, error>` where
/// `error` is the 4-nullary `variant`), `notification` (`contract, payload`); each member returns the shared
/// `step`. Exactly one exported interface with all three members.
fn reducer_full_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let list_u8 = |b: &mut Builder| {
        let u8h = b.name("u8");
        let u8p = b.list(vec![u8h]);
        let lh = b.atom_leaf(Leaf::Str("list".into()));
        b.list(vec![lh, u8p])
    };
    let field = |b: &mut Builder, name: &str, ty| {
        let n = b.name(name);
        b.list(vec![n, ty])
    };
    let record = |b: &mut Builder, fields: Vec<crate::ast::StructId>| {
        let h = b.atom_leaf(Leaf::Str("record".into()));
        let mut v = vec![h];
        v.extend(fields);
        b.list(v)
    };
    let bytes_field = |b: &mut Builder, name: &str| {
        let t = list_u8(b);
        field(b, name, t)
    };
    // The shared `step` record (requests: list<request>, outcome: variant{continue, close(closed)}).
    let step = |b: &mut Builder| {
        let request = {
            let r_contract = bytes_field(b, "contract");
            let r_payload = bytes_field(b, "payload");
            let r_token = bytes_field(b, "token");
            let r_deadline = {
                let u64h = b.name("u64");
                let u64t = b.list(vec![u64h]);
                let oh = b.atom_leaf(Leaf::Str("option".into()));
                let ot = b.list(vec![oh, u64t]);
                field(b, "deadline-nanos", ot)
            };
            record(b, vec![r_contract, r_payload, r_token, r_deadline])
        };
        let requests_list = {
            let lh = b.atom_leaf(Leaf::Str("list".into()));
            b.list(vec![lh, request])
        };
        let closed = {
            let c_schema = bytes_field(b, "schema");
            let c_reason = bytes_field(b, "reason");
            record(b, vec![c_schema, c_reason])
        };
        let outcome = {
            let cont_case = {
                let n = b.name("continue");
                b.list(vec![n])
            };
            let close_case = {
                let n = b.name("close");
                b.list(vec![n, closed])
            };
            let vh = b.atom_leaf(Leaf::Str("variant".into()));
            b.list(vec![vh, cont_case, close_case])
        };
        let s_requests = field(b, "requests", requests_list);
        let s_outcome = field(b, "outcome", outcome);
        record(b, vec![s_requests, s_outcome])
    };
    let mk_member = |b: &mut Builder, mname: &str, pname: &str, param_ty| {
        let step_ty = step(b);
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name(pname);
        let param_node = b.list(vec![param_h, pn, param_ty]);
        let result_h = b.name("result");
        let result_node = b.list(vec![result_h, step_ty]);
        let func = b.list(vec![func_h, param_node, result_node]);
        let member_h = b.name("member");
        let mn = b.name(mname);
        b.list(vec![member_h, mn, func])
    };
    // message{contract, sender{reducer, host}, payload, token} — decl order.
    let message = {
        let sender = {
            let s_reducer = bytes_field(&mut b, "reducer");
            let s_host = bytes_field(&mut b, "host");
            record(&mut b, vec![s_reducer, s_host])
        };
        let m_contract = bytes_field(&mut b, "contract");
        let m_sender = field(&mut b, "sender", sender);
        let m_payload = bytes_field(&mut b, "payload");
        let m_token = bytes_field(&mut b, "token");
        record(&mut b, vec![m_contract, m_sender, m_payload, m_token])
    };
    // response{contract, token, answer: result<list<u8>, error>} — error = 4-nullary variant.
    let response = {
        let r_contract = bytes_field(&mut b, "contract");
        let r_token = bytes_field(&mut b, "token");
        let answer = {
            let ok_ty = list_u8(&mut b);
            let err_ty = {
                let vh = b.atom_leaf(Leaf::Str("variant".into()));
                let mut cases = vec![vh];
                for c in ["timeout", "missing-handler", "schema-violation", "faulted"] {
                    let n = b.name(c);
                    cases.push(b.list(vec![n]));
                }
                b.list(cases)
            };
            let rh = b.atom_leaf(Leaf::Str("result".into()));
            let a_ty = b.list(vec![rh, ok_ty, err_ty]);
            field(&mut b, "answer", a_ty)
        };
        record(&mut b, vec![r_contract, r_token, answer])
    };
    // notification{contract, payload}.
    let notification = {
        let n_contract = bytes_field(&mut b, "contract");
        let n_payload = bytes_field(&mut b, "payload");
        record(&mut b, vec![n_contract, n_payload])
    };
    let on_message = mk_member(&mut b, "on-message", "m", message);
    let on_response = mk_member(&mut b, "on-response", "r", response);
    let on_notification = mk_member(&mut b, "on-notification", "n", notification);
    let exp_h = b.name("export");
    let iname = b.name("iface");
    let export = b.list(vec![exp_h, iname, on_message, on_response, on_notification]);
    let world_h = b.name("world");
    let wn = b.name("w");
    let world = b.list(vec![world_h, wn, export]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// W4c-b-iii DECLINE-DON'T-MISCOMPILE: a PARTIAL guest — defines only `onMessage` but the world's `guest`
/// export interface declares all three members (on-message/on-response/on-notification) — must DECLINE
/// cleanly, not silently fall through to a raw heap-handle export (`on-message: u32 -> u32`) the platform
/// host cannot bind. (v-platform's reducer-echo-step probe hit exactly this silent heap fallback.) A
/// component must export every func of the interface it exports.
#[test]
fn a_partial_guest_missing_world_export_members_declines_cleanly() {
    use crate::testkit::parse;
    // Only `onMessage` — no `onResponse`/`onNotification`. The world (reducer_full_world_bytes) declares all 3.
    let src = "(module m \
                 (type Outcome Continue (Close (Record (schema Bytes) (reason Bytes)))) \
                 (def (onMessage (: m (Record (contract Bytes) \
                                      (sender (Record (reducer Bytes) (host Bytes))) \
                                      (payload Bytes) (token Bytes)))) \
                   (record \
                     (= requests (list)) \
                     (= outcome Outcome.Continue))) \
                 (export onMessage))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:platform/guest"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_full_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.has_error(),
        "a partial guest (1 of 3 world export members) must decline, not silently heap-fall"
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "no wasm artifact — no silent heap-handle mis-emit"
    );
    assert!(
        out.diagnostics.iter().any(|d| d
            .message
            .contains("does not fully implement the world's typed export interface")),
        "the decline names the missing-export-member cause: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// A reducer world importing `cadenza:platform/run`.`run : func(program: list<u8>, contract: list<u8>,
/// input: list<u8>) -> result<list<u8>, variant(timeout, faulted)>` — a host op with a COMPOUND `result<T,E>`
/// RESULT whose err arm is a payload-less variant. Drives the spilled `result` lift + the nominal-export-for-
/// results (the err type is DEFINED + EXPORTED in the instance-type).
fn run_result_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let list_u8 = |b: &mut Builder| {
        let u8h = b.name("u8");
        let u8p = b.list(vec![u8h]);
        let lh = b.atom_leaf(Leaf::Str("list".into()));
        b.list(vec![lh, u8p])
    };
    let field = |b: &mut Builder, name: &str, ty| {
        let n = b.name(name);
        b.list(vec![n, ty])
    };
    let record = |b: &mut Builder, fields: Vec<crate::ast::StructId>| {
        let h = b.atom_leaf(Leaf::Str("record".into()));
        let mut v = vec![h];
        v.extend(fields);
        b.list(v)
    };
    let bytes_field = |b: &mut Builder, name: &str| {
        let t = list_u8(b);
        field(b, name, t)
    };
    let message = {
        let m_contract = bytes_field(&mut b, "contract");
        let m_payload = bytes_field(&mut b, "payload");
        let m_token = bytes_field(&mut b, "token");
        record(&mut b, vec![m_contract, m_payload, m_token])
    };
    let step = {
        let request = {
            let r_contract = bytes_field(&mut b, "contract");
            let r_payload = bytes_field(&mut b, "payload");
            let r_token = bytes_field(&mut b, "token");
            let r_deadline = {
                let u64h = b.name("u64");
                let u64t = b.list(vec![u64h]);
                let oh = b.atom_leaf(Leaf::Str("option".into()));
                let ot = b.list(vec![oh, u64t]);
                field(&mut b, "deadline-nanos", ot)
            };
            record(&mut b, vec![r_contract, r_payload, r_token, r_deadline])
        };
        let requests_list = {
            let lh = b.atom_leaf(Leaf::Str("list".into()));
            b.list(vec![lh, request])
        };
        let outcome = {
            let cont_case = {
                let n = b.name("continue");
                b.list(vec![n])
            };
            let close_case = {
                let cs = bytes_field(&mut b, "schema");
                let cr = bytes_field(&mut b, "reason");
                let rec = record(&mut b, vec![cs, cr]);
                let n = b.name("close");
                b.list(vec![n, rec])
            };
            let vh = b.atom_leaf(Leaf::Str("variant".into()));
            b.list(vec![vh, cont_case, close_case])
        };
        let s_requests = field(&mut b, "requests", requests_list);
        let s_outcome = field(&mut b, "outcome", outcome);
        record(&mut b, vec![s_requests, s_outcome])
    };
    let on_message = {
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("m");
        let param_node = b.list(vec![param_h, pn, message]);
        let result_h = b.name("result");
        let result_node = b.list(vec![result_h, step]);
        let func = b.list(vec![func_h, param_node, result_node]);
        let member_h = b.name("member");
        let mn = b.name("on-message");
        b.list(vec![member_h, mn, func])
    };
    let exp_h = b.name("export");
    let iname = b.name("guest");
    let export = b.list(vec![exp_h, iname, on_message]);
    // import cadenza:platform/run { run: func(program, contract, input: list<u8>) -> result<list<u8>, err> }
    let run_member = {
        let func_h = b.name("func");
        let param = |b: &mut Builder, name: &str| {
            let ph = b.name("param");
            let pn = b.name(name);
            let ty = list_u8(b);
            b.list(vec![ph, pn, ty])
        };
        let p1 = param(&mut b, "program");
        let p2 = param(&mut b, "contract");
        let p3 = param(&mut b, "input");
        let res_ty = {
            let err_variant = {
                let vh = b.atom_leaf(Leaf::Str("variant".into()));
                let c1 = {
                    let n = b.name("timeout");
                    b.list(vec![n])
                };
                let c2 = {
                    let n = b.name("faulted");
                    b.list(vec![n])
                };
                b.list(vec![vh, c1, c2])
            };
            let rh = b.atom_leaf(Leaf::Str("result".into()));
            let ok = list_u8(&mut b);
            b.list(vec![rh, ok, err_variant])
        };
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, res_ty]);
        let func = b.list(vec![func_h, p1, p2, p3, rnode]);
        let member_h = b.name("member");
        let mn = b.name("run");
        b.list(vec![member_h, mn, func])
    };
    let imp_h = b.name("import");
    let run_name = b.name("cadenza:platform/run");
    let run = b.list(vec![imp_h, run_name, run_member]);
    let world_h = b.name("world");
    let wn = b.name("w");
    let world = b.list(vec![world_h, wn, export, run]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// The spilled-RESULT component type follows the WORLD's declared result WIT type, so its err arm emits the
/// host's CONSTRUCTOR (`variant`/`enum`) rather than the guest-`Ty`-derived one — the #3228 variant-vs-enum
/// rule, result-side. `wit_op_result_type` reads `run.run`'s `result<list<u8>, VARIANT error>` off the world;
/// `build_host_result_types` prefers it (over `spilled_result_wit_type`), so the emitted result err arm is a
/// `variant` matching the host (`result<_, enum>` and `result<_, variant>` are distinct component types — a
/// mismatch silently fails to instantiate).
#[test]
fn a_host_op_result_wit_type_follows_the_world_variant() {
    use crate::db::Db;
    use crate::wit_world::WitType;
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (main) 0) (export main))",
    ));
    db.wit_world = Some(run_result_world_bytes()); // run.run : (...) -> result<list<u8>, variant(...)>
    let wt = crate::backend::wasm::host::wit_op_result_type(&mut db, "run", "run")
        .expect("run.run has a declared result WIT type in the world");
    match wt {
        WitType::Result { ok, err } => {
            assert!(
                matches!(ok.as_deref(), Some(WitType::List(_))),
                "the Ok arm is `list<u8>`, got {ok:?}"
            );
            assert!(
                matches!(err.as_deref(), Some(WitType::Variant(_))),
                "the err arm follows the world's `variant` constructor (NOT an enum), got {err:?}"
            );
        }
        other => panic!("run.run's declared result is a WIT `result`, got {other:?}"),
    }
}

/// W4c-b-iii DX: a reducer with an IN-SOURCE `(world … (export <FQ-iface> …))` and NO `--component-name`
/// (NO `KIND_COMPONENT_NAME` artifact) routes to the typed interface-instance emit — `compile` DERIVES the
/// component name from the world's FQ export interface (`cadenza:platform/guest`). Without the derive, the
/// CLI `cdz compile <reducer>.sexp --target wasm` path hits the older "parameterized heap-return export"
/// decline on the `list<u8>`-field record param (v-platform's probe). Proves a self-describing `.cdz`
/// reducer compiles to a component with no redundant flag; the emitted component exports the guest instance.
#[test]
fn an_in_source_world_reducer_derives_its_component_name_and_emits() {
    use crate::testkit::parse;
    // The typed `message`-record param (list<u8> leaves) + `list<u8>` result, world declared IN SOURCE with a
    // FQ export interface — the shape v-nix's `cdz compile --target wasm` pipeline feeds (no artifacts).
    let src = "(module reducer \
                 (world reducer-world \
                   (export cadenza:platform/guest \
                     (member on-message \
                       (func (param msg (\"record\" (contract (\"list\" (u8))) (payload (\"list\" (u8))) (token (\"list\" (u8))))) \
                             (result (\"list\" (u8))))))) \
                 (def (on-message (: msg (Record (contract Bytes) (payload Bytes) (token Bytes)))) \
                   (. msg payload)) \
                 (export on-message))";
    // ONLY the KIND_AST artifact — no component-name, no external world (the world is in-source).
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "an in-source FQ-export-world reducer must emit via the derived component name (no --component-name): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the in-source-world reducer emits a component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes)
        .expect("the in-source-world reducer component validates");
    assert!(
        String::from_utf8_lossy(bytes).contains("cadenza:platform/guest"),
        "the component exports the guest instance under the world's FQ export interface name"
    );
}

/// The KIND_WIT_WORLD reader derives the SAME `TargetWorld` whether the world tree was built bottom-up by
/// the in-crate `Builder` (`option_result_world_bytes`) or top-down by the sexpr PARSER front-end (the `cdz
/// convert` path). Since the ast-consolidation (#5158) `crate::{ast,codec}` ARE the single `cadenza-ast`
/// crate, BOTH paths now go through the ONE Builder+codec, so they encode to IDENTICAL bytes (one source of
/// truth — previously the two SEPARATE builder impls interned in different order and produced different
/// bytes, which this test used to exploit). The reader stays STRUCTURAL regardless: `parse_target_world`/
/// `parse_wit_type` walk by `head_name`/`head_ctor`/indexed children, never by leaf-intern order — so the
/// export member types derive identically (disproving a reported "the reader mis-derives on cdz-convert
/// bytes" hypothesis).
#[test]
fn the_world_reader_is_leaf_intern_order_independent() {
    let builder_bytes = option_result_world_bytes();
    let parser_bytes = crate::codec::encode(&crate::testkit::parse(
        "(world w (export iface (member f \
           (func (param m (\"record\" (x (s64)))) \
                 (result (\"record\" (d (\"option\" (s64)))))))))",
    ));
    // Post ast-consolidation (#5158) both paths share the ONE cadenza-ast Builder+codec → identical bytes
    // (the single-source-of-truth win; the pre-consolidation two-builder byte divergence this once pinned is
    // gone). The reader's order-agnosticism is proven below by both decoding to the same `TargetWorld`.
    assert_eq!(
        builder_bytes, parser_bytes,
        "post-consolidation, the Builder and parser paths share the one cadenza-ast codec → identical bytes"
    );
    let ba = crate::codec::decode(&builder_bytes).expect("builder decodes");
    let pa = crate::codec::decode(&parser_bytes).expect("parser decodes");
    let bw = crate::wit_world::parse_target_world(&ba, ba.root);
    let pw = crate::wit_world::parse_target_world(&pa, pa.root);
    assert_eq!(
        bw, pw,
        "the world reader is structural (leaf-order-independent) — both codecs parse the same TargetWorld"
    );
    // And the export param reads as record{x:s64} from BOTH — not a nested/handle mis-read.
    let want =
        crate::wit_world::WitType::Record(vec![("x".into(), crate::wit_world::WitType::S64)]);
    assert_eq!(
        &bw.as_ref().unwrap().exports[0].members[0].func.params[0].1,
        &want
    );
    assert_eq!(
        &pw.as_ref().unwrap().exports[0].members[0].func.params[0].1,
        &want
    );
}

fn option_result_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let s64 = |b: &mut Builder| {
        let h = b.name("s64");
        b.list(vec![h])
    };
    // param: ("record" (x (s64)))
    let x_ty = s64(&mut b);
    let x_name = b.name("x");
    let x_field = b.list(vec![x_name, x_ty]);
    let prec_head = b.atom_leaf(Leaf::Str("record".into()));
    let prec = b.list(vec![prec_head, x_field]);
    // result: ("record" (d ("option" (s64))))
    let opt_inner = s64(&mut b);
    let opt_head = b.atom_leaf(Leaf::Str("option".into()));
    let opt_ty = b.list(vec![opt_head, opt_inner]);
    let d_name = b.name("d");
    let d_field = b.list(vec![d_name, opt_ty]);
    let rrec_head = b.atom_leaf(Leaf::Str("record".into()));
    let rrec = b.list(vec![rrec_head, d_field]);
    let func_h = b.name("func");
    let param_h = b.name("param");
    let pn = b.name("m");
    let param_node = b.list(vec![param_h, pn, prec]);
    let result_h = b.name("result");
    let result_node = b.list(vec![result_h, rrec]);
    let func = b.list(vec![func_h, param_node, result_node]);
    let member_h = b.name("member");
    let mn = b.name("f");
    let member = b.list(vec![member_h, mn, func]);
    let exp_h = b.name("export");
    let iname = b.name("iface");
    let export = b.list(vec![exp_h, iname, member]);
    let world_h = b.name("world");
    let wn = b.name("w");
    let world = b.list(vec![world_h, wn, export]);
    let a = b.finish(world);
    crate::codec::encode(&a)
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
    // Runtime params, but `t` is only projected → folds; the program touches the value heap not at all.
    let src = "(module m (def (pair-sum (: a Int64) (: b Int64)) \
                 (let ((t (tuple a b))) (+ (. t 0) (. t 1)))) (export pair-sum))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a projection-only tuple must fold, importing no runtime op (no per-call arr-alloc)"
    );
    // Value parity (pair-sum(20,22)=42) migrated to the corpus (run via cdz-run): case "a projection-only
    // runtime tuple folds to the sum of its elements" in spec/semantics/02-binding-and-control.sexp.
}

#[test]
fn a_multiuse_param_bound_to_a_pure_recursive_helper_call_inlines_and_runs() {
    // Guards the `arg_reaches_perform_evalonce` fix (cross-module HM-unify "parameter reference has no
    // local slot" miscompile, v-wasm-opt/v-compiler-ml 2026-08-08). A helper `bc` that uses a param in ≥2
    // sum-match arms is inlined into a caller that binds that param to `resolve(·)` — a PURE but fuel-
    // RECURSIVE resolver. The evaluate-once trigger USED to over-report ANY recursive callee as "may
    // perform" (blanket `is_recursive → true`), let-wrapping the pure arg instead of substituting it; in the
    // real multi-level cross-module inline (unify-int-su → unify-sign-su → bind-or-check-sign over `Sign`)
    // the leftover residual param refs were then pinned as false captures and emitted slot-less. The walk
    // now follows a recursive callee ONCE (visited set) and correctly sees it performs nothing, so the arg
    // is substituted. (The full RED witness is the compiler-ml unify-subst 6th-heavy-@test, gated at
    // cdz-test scale by v-wasm-opt; this pins the fix's core behavior — a pure recursive arg substitutes —
    // as a fast rcdzc-lib compile+run check.)
    let src = "(module m \
        (type S (SVar Int64) (SFix Int64)) \
        (def (go (: acc Int64) (: n Int64)) (if (= n 0) acc (go (+ acc n) (- n 1)))) \
        (def (resolve (: v Int64)) (S.SFix (go 0 v))) \
        (def (bc (: ra S) (: rb S)) \
          (match ra \
            ((S.SVar ia) (match rb ((S.SVar ib) (+ ia ib)) ((S.SFix fb) (+ ia fb)))) \
            ((S.SFix fa) (match rb ((S.SVar ib) (+ fa ib)) ((S.SFix fb) (+ fa fb)))))) \
        (def (mid (: a Int64) (: b Int64)) (bc (resolve a) (resolve b))) \
        (def (main (: k Int64)) (mid k k)) \
        (export main))";
    let _ = compile_component(&crate::codec::encode(&crate::testkit::parse(src))).expect(
        "a multi-use param bound to a pure-recursive-helper call must compile (no slotless param)",
    );
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
    // `.0` of `(if p (tuple a b) (tuple b a))` → `(if p a b)`: p true selects a, p false selects b.
    let src = "(module m (def (pick (: p Bool) (: a Int64) (: b Int64)) \
                 (. (if p (tuple a b) (tuple b a)) 0)) (export pick))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a single projection of an if-of-tuples must fold (no per-call arr-alloc, no runtime import)"
    );
    // Value parity (p true → a, p false → b) migrated to the corpus (run via cdz-run): case "a projection
    // of an if-selected tuple selects the branch's element" in spec/semantics/02-binding-and-control.sexp.
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
    use crate::core::Core;
    use crate::testkit::parse;
    // `.x` of `(if p (record (y b)(x a)) (record (y a)(x b)))` → `(if p a b)`.
    let src = "(module m (def (pick (: p Bool) (: a Int64) (: b Int64)) \
                 (. (if p (record (= y b) (= x a)) (record (= y a) (= x b))) x)) (export pick))";
    // Compiles, and the member read PUSHES INTO the branches: the folded Core is a bare `Core::If` over the
    // selected field values (by NAME — fields written out of order), NOT a `Core::Proj` over an if-of-records
    // (the un-folded form, which would per-call `arr-alloc` both branch records then arr-get one field back).
    // A `Core::If` top means both records folded away — no heap, no runtime import. (wasmtime-free: the fold
    // is inspected on the lowered Core.) Value parity (p true → a, p false → b, by field NAME) is corpus-
    // covered: case "a member read of an if-selected record selects the branch's field by name" in
    // spec/semantics/02-binding-and-control.sexp.
    compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("pick").expect("def pick");
    let body = db.defs[d].body.expect("pick has a body");
    assert!(
        matches!(crate::lower::core_of(&mut db, body), Core::If { .. }),
        "a single member read of an if-of-records must fold to a bare `if` over the field values (records \
         eliminated), not a `Core::Proj` over an if-of-records"
    );
}

/// The MATCH analogue of the tuple/record `*-INTO-IF` folds: a `match` over a SUM built through an `if`
/// pushes the match INTO each branch — `(match (if c (Some x) (None)) ((Some v) v) ((None) 0))` →
/// `(if c (match (Some x) …) (match (None) …))` → `(if c x 0)`. The `if`-selected sum was a THROWAWAY heap
/// value (per-branch `arr-alloc` + payload box + the deconstruct's `arr-get`/unbox, purely to read the
/// payload back), so the un-fused form imports the value-heap runtime; the fused form folds each branch's
/// constant constructor to the arm body directly — no heap, no runtime import. Fires only when BOTH branches
/// build a visible constructor AND both inner matches FULLY fold (else it backs out to the runtime match).
#[test]
fn a_match_over_an_if_selected_sum_pushes_into_the_branches() {
    use crate::testkit::parse;
    // `(match (if (> x 0) (Some x) (None)) ((Some v) v) ((None) 0))` → `(if (> x 0) x 0)`.
    let src = "(module m (type Option (Some Int64) None) \
                 (def (f (: x Int64)) \
                   (match (if (> x 0) (Option.Some x) Option.None) \
                     ((Option.Some v) v) (Option.None 0))) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a match over an if-of-constructors must fold (no throwaway sum build, no runtime import)"
    );
    // Value parity (x>0 → 5, x<=0 → 0) migrated to the corpus (run via cdz-run): case "a match over an
    // if-selected sum folds each branch's constructor to its arm body" in
    // spec/semantics/02-binding-and-control.sexp.
}

/// The CASE-OF-MATCH twin of the above (`fuse_match_into_match`): a `match` over a SUM built through an
/// INNER match pushes the outer match INTO each inner-arm body — `(match (match (> n 0) (true (Some n))
/// (false (None))) ((Some v) v) ((None) 0))` → `(match (> n 0) (true n) (false 0))`. The inner match built
/// a THROWAWAY sum per arm purely to be deconstructed by the outer match; pushing the outer match in folds
/// each inner arm's constant constructor to a body directly — no heap, no runtime import. Fires only on a
/// DIRECTLY-VISIBLE inner match (not a call scrutinee) and only when every pushed-in inner match FULLY folds
/// (else it backs out to the runtime match).
#[test]
fn a_match_over_a_match_selected_sum_pushes_into_the_arms() {
    use crate::testkit::parse;
    let src = "(module m (type Option (Some Int64) None) \
                 (def (f (: n Int64)) \
                   (match (match (> n 0) (true (Option.Some n)) (false Option.None)) \
                     ((Option.Some v) v) (Option.None 0))) (export f))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a match over a match-of-constructors must fold (no throwaway sum build, no runtime import)"
    );
    // Value parity (n>0 → 5, n<=0 → 0) migrated to the corpus (run via cdz-run): case "a match over a
    // match-selected sum folds into the inner arms" in spec/semantics/02-binding-and-control.sexp.
}

/// A NARROW-width element crosses the heap boundary with an explicit slot conversion. The heap stores
/// an integer as one i64 cell, but a narrow width (UInt8) lives in an i32 slot — so a narrow element is
/// extended i32→i64 into the heap and narrowed i64→i32 out of it. To exercise a GENUINE heap round-trip
/// the tuple must be OPAQUE to the fold: a projection-only tuple LITERAL folds away (no heap), a
/// call-returned one now folds through too (the callee β-reduces — cycle 16), and an `if` whose two
/// branches DIFFER now ALSO folds through (the common-constructor hoist pushes the projections into
/// per-element `select`s — no heap). So the tuple comes from a RECURSIVE helper (`mk`): `is_recursive`
/// declines the β-reduction, so `reduce_to_tuple_elems` cannot see through `mk` and `t` stays a runtime
/// handle whose projections are `arr-get`s (mirroring the runtime-record test's opacity trick). `(+ (.
/// t 0) (. t 1))` must build, validate, and compute 100+50 = 150 through the heap.
#[test]
fn a_narrow_runtime_tuple_element_crosses_the_heap_boundary() {
    use crate::testkit::parse;
    // `mk` bottoms out immediately (`n >= 0`) with the tuple, but being recursive it is not inlined, so
    // the tuple stays opaque and `t` is a runtime heap handle.
    let src = "(module m \
                 (def (mk (: a UInt8) (: b UInt8) (: n Int64)) \
                   (if (< n 0) (mk a b (+ n 1)) (tuple a b))) \
                 (def (pair-sum (: a UInt8) (: b UInt8)) \
                   (let ((t (mk a b 0))) (+ (. t 0) (. t 1)))) \
                 (export pair-sum))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        imports_value_heap_runtime(&bytes),
        "an if-selected narrow tuple, projected, must build on the heap (import the runtime)"
    );
    // The RUN — pair-sum(100, 50) reads both narrow UInt8 elements back off the heap tuple and adds them =
    // 150 — is corpus-covered by 05-compound-types "two narrow tuple elements are projected and added"
    // (same (+ (. t 0) (. t 1)) over UInt8 params = 150); this test keeps the builds-on-the-heap pin.
}

/// COMMON-CONSTRUCTOR HOIST: `(if c (Some a) (Some b))` builds the `Some` variant ONCE and pushes the
/// differing payload into a branchless `(if c a b)` (a scalar `select`), instead of DUPLICATING the
/// whole `sum-new` construction in both `if`/`else` arms. Structurally: exactly ONE `sum-new`-style
/// heap-construct call reaches the boundary and a `select` decides the payload (down from a two-arm
/// `if` block each with its own build). The value must still round-trip through the value heap in BOTH
/// branch directions — c=true → the first payload, c=false → the second — proving the single build with
/// a selected payload reproduces the per-arm builds. A `Some Int64` payload is scalar, so this exercises
/// the hoist without hitting the (separate, pre-existing) heap-payload-in-a-sum boundary limitation.
#[test]
fn a_common_constructor_hoists_out_of_both_if_arms_building_once() {
    use crate::testkit::parse;
    // `pick` returns `Some a` or `Some b` by the condition — the `if` the hoist targets. `main` observes
    // the payload via a match, but keeps the sum OPAQUE to the fold with a RECURSIVE helper (`walk`):
    // `is_recursive` declines the compile-time β-reduction, so the match cannot see through `pick` and
    // `main` selects a genuine runtime Option off the heap (as the runtime-record test does — an `if`
    // alone folds through). `walk` bottoms out immediately for `n >= 0`, forwarding to `pick`.
    let src = "(module m \
                 (def (pick (: c Bool) (: a Int64) (: b Int64)) (if c (Some a) (Some b))) \
                 (def (walk (: c Bool) (: a Int64) (: b Int64) (: n Int64)) \
                   (if (< n 0) (walk c a b (+ n 1)) (pick c a b))) \
                 (def (main (: c Bool) (: a Int64) (: b Int64)) \
                   (match (walk c a b 0) ((Some v) v) (None -1))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The hoisted payload becomes a scalar `select` (branchless). Its presence witnesses the hoist fired
    // (the pre-hoist form was a two-arm `if` over duplicated sum builds, with no payload `select`).
    let selects = count_opcode(&bytes, |op| {
        matches!(
            op,
            wasmparser::Operator::Select | wasmparser::Operator::TypedSelect { .. }
        )
    });
    assert!(
        selects >= 1,
        "the common-constructor hoist must lower the differing payload to a branchless select \
         (build-once); found no select"
    );
    // The payload-select VALUE + the untaken-arm guard (a trapping untaken payload does not fire) are
    // covered by the corpus common-constructor-hoist family (spec/semantics/02-binding-and-control.sexp:
    // "a trapping payload in the untaken arm of a same-constructor if does not trap" + "a nested if of a
    // common constructor builds it once and dispatches to each arm"). Only the build-once EMISSION witness
    // (the select count above) stays here — the corpus cannot observe the opcode count.
}

/// The common-OPERATOR hoist (the arith sibling of the constructor hoist): `(if c (+ a 1) (+ b 1))`
/// applies the checked add + its overflow guard ONCE over the SELECTED operand — `(+ (if c a b) 1)` —
/// instead of duplicating the add+guard in both `if`/`else` arms. Structurally, exactly ONE `i64.add`
/// reaches the module (the differing operand becomes a branchless `select`). Behaviorally it must be
/// operand SELECTION, not eager evaluation of both: the trap fires iff the TAKEN arm's add overflows, so
/// (a) values round-trip in both directions, (b) c=true with a=Int64.max TRAPS (the taken `(+ a 1)`
/// overflows), and (c) c=false with the SAME a=Int64.max does NOT trap (the untaken arm's overflow is
/// never evaluated — `(if c a b)` selects b) — a hoist that eagerly added both operands would wrongly
/// trap here.
#[test]
fn a_common_operator_hoists_out_of_both_if_arms_computing_once() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (main (: c Bool) (: a Int64) (: b Int64)) (if c (+ a 1) (+ b 1))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // Exactly one checked add survives (the differing operand is a `select`, not a second add).
    let adds = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::I64Add));
    assert_eq!(
        adds, 1,
        "the common-operator hoist applies the add once over the selected operand, got {adds} adds"
    );
    // Value + trap parity (both directions, and the untaken arm's overflow is NOT evaluated) migrated to
    // the corpus (run via cdz-run): case "a common operator hoisted out of both if arms selects the operand
    // and computes once" in spec/semantics/02-binding-and-control.sexp.
}

/// A CONST bitwise / shift operation FOLDS at compile time — the emitted module carries the constant
/// result and NO runtime bitwise/shift opcode. This pins the FOLD half of the const-bitwise/shift support
/// (#4136): corpus 06-numeric-model carries the value cases (`(& 255 127)` = 127, etc.), but a `run_main`
/// value assertion CANNOT catch a regression that stops folding and emits a runtime `i32.and`/`i64.shl`
/// (the runtime op yields the same value) — so the opcode-count pin is what protects the fold. (The
/// value-only `run_main` tests bitwise_ops_fold/left_shift_.../arithmetic_right_shift_folds that used to
/// sit here were cite-deleted to corpus 06 by corpus-mig-4; this pins the fold they never asserted.)
#[test]
fn const_bitwise_and_shift_fold_emit_no_runtime_op() {
    use crate::testkit::parse;
    let no_op = |body: &str, pred: fn(&wasmparser::Operator) -> bool, opname: &str| {
        let src = format!("(module m (def (main) {body}) (export main))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        let n = count_opcode(&bytes, pred);
        assert_eq!(
            n, 0,
            "`{body}` must CONST-FOLD to its result, emitting no runtime {opname}, got {n}"
        );
    };
    // `&`/`|`/`^` fold — no i32/i64 and/or/xor survives.
    no_op(
        "(& 255 127)",
        |op| {
            matches!(
                op,
                wasmparser::Operator::I32And | wasmparser::Operator::I64And
            )
        },
        "and",
    );
    no_op(
        "(| 12 3)",
        |op| {
            matches!(
                op,
                wasmparser::Operator::I32Or | wasmparser::Operator::I64Or
            )
        },
        "or",
    );
    no_op(
        "(^ 12 10)",
        |op| {
            matches!(
                op,
                wasmparser::Operator::I32Xor | wasmparser::Operator::I64Xor
            )
        },
        "xor",
    );
    // `<<` folds (exact multiply by 2^count) — no shl survives.
    no_op(
        "(<< 1 7)",
        |op| {
            matches!(
                op,
                wasmparser::Operator::I32Shl | wasmparser::Operator::I64Shl
            )
        },
        "shl",
    );
    // `>>` folds (arithmetic, sign-extending) — no shr_s survives.
    no_op(
        "(>> 256 7)",
        |op| {
            matches!(
                op,
                wasmparser::Operator::I32ShrS | wasmparser::Operator::I64ShrS
            )
        },
        "shr_s",
    );
}

/// The common-operator hoist also covers a COMPARISON: `(if c (< a k) (< b k))` → `(< (if c a b) k)` —
/// one compare over the selected operand, not two. A comparison is TOTAL (no trap, no guard), so the
/// hoist is unconditionally value-safe; the win is a single `i64.lt_s` (the differing operand becomes a
/// branchless select) instead of two compares feeding a select. Value parity in every direction: the
/// hoisted compare returns the same boolean the taken arm's compare would.
#[test]
fn a_common_comparison_hoists_out_of_both_if_arms() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (main (: c Bool) (: a Int64) (: b Int64) (: k Int64)) \
                   (if (if c (< a k) (< b k)) 1 0)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // Exactly one signed-less-than survives (the differing operand is a select, not a second compare).
    let lts = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::I64LtS));
    assert_eq!(
        lts, 1,
        "the common-comparison hoist compares once over the selected operand, got {lts} compares"
    );
    // Value parity migrated to the corpus (run via cdz-run): case "a common integer comparison hoisted out
    // of both if arms compares once over the selected operand" in spec/semantics/02-binding-and-control.sexp.
}

/// The common-operator hoist also covers a FLOAT equality (`Core::FloatCompare`): `(if c (= a k) (= b k))`
/// over `Float64` operands → `(= (if c a b) k)` — ONE canonical-byte compare over the selected operand, not
/// two. `FloatCompare` canonicalizes each operand's NaN then compares the bit patterns (`i64.eq` at width
/// 64), and is TOTAL (never traps on its own), so the hoist is value-safe exactly like the integer
/// comparison above. Both arms share the operator and the WIDTH (the `wt == we` guard). Value parity in
/// both directions proves the single canon-and-compare over the selected operand reproduces the per-arm
/// compares. (`k` is a distinct param so the compare stays runtime — a constant fold would erase it.)
#[test]
fn a_common_float_equality_hoists_out_of_both_if_arms() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (main (: c Bool) (: a Float64) (: b Float64) (: k Float64)) \
                   (if (if c (= a k) (= b k)) 1 0)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // Exactly one equality survives (the differing operand is a select, not a second canon-and-compare).
    let eqs = count_opcode(&bytes, |op| matches!(op, wasmparser::Operator::I64Eq));
    assert_eq!(
        eqs, 1,
        "the common-float-equality hoist compares once over the selected operand, got {eqs} compares"
    );
    // Value parity migrated to the corpus (run via cdz-run): case "a common float equality hoisted out of
    // both if arms compares once over the selected operand" in spec/semantics/02-binding-and-control.sexp.
}

/// The common-operator hoist AND the value-numbering CSE also cover a FLOAT ORDERING compare — the same
/// `Core::FloatCompare` node the equality above uses, now carrying an ordering prim (`<`/`>`/`<=`/`>=`,
/// implemented as the raw IEEE machine compare `f64.lt`/… — NaN compares false, no trap). Because the hoist
/// and `core_eq` match `FloatCompare` on its operator GENERICALLY (`op == op && width == width`), the
/// ordering variants are covered for FREE by the equality machinery — this pins that. `(if c (< a k) (< b
/// k))` hoists to `(< (if c a b) k)` — ONE `f64.lt` over the selected operand — and a repeated `(< a b)`
/// CSEs to one. Value parity includes the NaN case: `f64.lt` is TOTAL (NaN → false), and the hoist selects
/// the same operand the taken arm would compare, so the hoisted result is identical for every input,
/// INCLUDING a NaN operand (which the ordering landing 88f15533d fixed as the IEEE partial order).
#[test]
fn a_common_float_ordering_hoists_and_cses_like_equality() {
    use crate::testkit::parse;
    // HOIST: `(if c (< a k) (< b k))` → one `f64.lt` (the differing operand becomes a select).
    let hsrc = "(module m \
                  (def (main (: c Bool) (: a Float64) (: b Float64) (: k Float64)) \
                    (if (if c (< a k) (< b k)) 1 0)) \
                  (export main))";
    let hbytes = compile_component(&crate::codec::encode(&parse(hsrc))).expect("compile");
    let lts = count_opcode(&hbytes, |op| matches!(op, wasmparser::Operator::F64Lt));
    assert_eq!(
        lts, 1,
        "the common-float-ORDERING hoist compares once over the selected operand, got {lts} f64.lt"
    );
    // Value parity (incl. the NaN IEEE partial-order face) migrated to the corpus (run via cdz-run): case
    // "a common float ordering hoist reproduces the IEEE partial order including NaN" in
    // spec/semantics/02-binding-and-control.sexp.
    // CSE: a repeated `(< a b)` computes one `f64.lt` (value-numbering via the `core_eq` FloatCompare arm).
    let csrc = "(module m \
                  (def (main (: a Float64) (: b Float64)) (+ (if (< a b) 10 0) (if (< a b) 20 0))) \
                  (export main))";
    let cbytes = compile_component(&crate::codec::encode(&parse(csrc))).expect("compile");
    let clts = count_opcode(&cbytes, |op| matches!(op, wasmparser::Operator::F64Lt));
    assert_eq!(
        clts, 1,
        "the repeated float-ordering compare is CSE'd to one f64.lt, got {clts}"
    );
    // Value parity migrated to the corpus (run via cdz-run): case "a repeated float comparison is computed
    // once and reused (value-transparent CSE)" in spec/semantics/02-binding-and-control.sexp.
}

/// The common-constructor hoist also fires for a RECORD: `(if c (record (a x) (b 1)) (record (a y) (b
/// 1)))` builds the record ONCE and pushes the DIFFERING field `a` into a branchless `(if c x y)` select
/// while the SHARED field `b` (the constant `1`) is emitted once — instead of duplicating the whole
/// `arr-alloc` + two field stores in each `if`/`else` arm. A record's fields are keyed (a `BTreeMap`), so
/// the hoist requires the SAME KEY SET across the arms (a differing key set is a different record TYPE,
/// which `infer` already rejects). Value parity in BOTH directions proves the single build with a
/// selected field reproduces the per-arm builds: c=true → field `a` = x, c=false → field `a` = y, and
/// the shared field `b` = 1 regardless. The record is kept OPAQUE to the fold via a recursive helper so
/// it is a genuine runtime heap value (a literal / an `if` alone would fold away).
#[test]
fn a_common_constructor_hoist_covers_records_by_shared_key_set() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (mk (: c Bool) (: x Int64) (: y Int64) (: n Int64)) \
                   (if (< n 0) (mk c x y (+ n 1)) \
                     (if c (record (= a x) (= b 1)) (record (= a y) (= b 1))))) \
                 (def (main (: c Bool) (: x Int64) (: y Int64)) \
                   (. (mk c x y 0) a)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The differing field becomes a branchless scalar `select` — the hoist's witness (the pre-hoist form
    // was a two-arm `if` over duplicated record builds, with no field `select`).
    let selects = count_opcode(&bytes, |op| {
        matches!(
            op,
            wasmparser::Operator::Select | wasmparser::Operator::TypedSelect { .. }
        )
    });
    assert!(
        selects >= 1,
        "the record common-constructor hoist must lower the differing field to a branchless select \
         (build-once); found no select"
    );
    // The VALUE + arm/element guard are corpus-covered (02-binding-and-control common-constructor
    // hoist family); only the build-once EMISSION witness (the opcode count above) stays here — the
    // corpus cannot observe an opcode count.
}

/// The common-constructor hoist also fires for a same-length LIST: `(if c (list a 1) (list b 1))` builds
/// the list ONCE (`vec-empty` + per-element `vec-push`) and pushes the DIFFERING element into a
/// branchless `(if c a b)` select, sharing the whole push chain — instead of duplicating it in each
/// `if`/`else` arm. A list's LENGTH is part of its value, so the hoist requires the SAME LENGTH across
/// the arms (two lists of different lengths are distinct values — the hoist declines and keeps the `if`);
/// a list is homogeneous, so a per-element select is well-typed. Value parity in BOTH directions plus
/// the element that DIFFERS (element 0 → a/b) and the SHARED element (element 1 = 1) prove the single
/// build reproduces the per-arm builds. Kept opaque via a recursive helper so it is a genuine runtime
/// heap list.
#[test]
fn a_common_constructor_hoist_covers_same_length_lists() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (mk (: c Bool) (: a Int64) (: b Int64) (: n Int64)) \
                   (if (< n 0) (mk c a b (+ n 1)) \
                     (if c (list a 1) (list b 1)))) \
                 (def (main (: c Bool) (: a Int64) (: b Int64)) \
                   (match (List.at (mk c a b 0) 0) ((Some v) v) (None -1))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The differing element becomes a branchless scalar `select` — the hoist's witness (the pre-hoist
    // form was a two-arm `if` over duplicated list builds, with no element `select`).
    let selects = count_opcode(&bytes, |op| {
        matches!(
            op,
            wasmparser::Operator::Select | wasmparser::Operator::TypedSelect { .. }
        )
    });
    assert!(
        selects >= 1,
        "the list common-constructor hoist must lower the differing element to a branchless select \
         (build-once); found no select"
    );
    // The VALUE + arm/element guard are corpus-covered (02-binding-and-control common-constructor
    // hoist family); only the build-once EMISSION witness (the opcode count above) stays here — the
    // corpus cannot observe an opcode count.
}

/// The Lir-level + `||` twin of the adv-55 short-circuit CSE-frontier pin (`cse_does_not_hoist_a_
/// trapping_subexpr_out_of_a_short_circuit_and` above, which is `&&`-only + runtime-only). `Core::And`
/// is the UNIFIED short-circuit node (`is_and=true` = `&&`, rhs runs iff lhs TRUE; `is_and=false` = `||`,
/// rhs runs iff lhs FALSE), so BOTH connectives shield their rhs and BOTH must keep a repeated trapping
/// rhs OUT of the dominating frontier (else the always-on `select.rs` CSE hoists it before the connective
/// → spurious trap on the short-circuited path). This pins it at the LIR level (no `I64DivS` emitted
/// BEFORE the connective's branch) for `||` — the polarity the `&&` runtime witness does not exercise —
/// so a future `collect_dominating_frontier` change that drops the `Core::And` arm re-regresses THIS test
/// at the emit-shape level, not only on a specific trapping input.
#[test]
fn cse_keeps_a_trapping_rhs_inside_a_short_circuit_or_at_the_lir_level() {
    use crate::backend::wasm::lir::Lir;
    use crate::db::Db;
    let lir = |body: &str| -> Vec<Lir> {
        let ast = crate::testkit::parse(&format!(
            "(module m (def (f (: x Int64)) {body}) (def (main) 0) (export main))"
        ));
        let mut db = Db::load(ast);
        let layout = crate::layout::compute(&mut db).expect("layout");
        let d = db.def_by_name("f").expect("def f");
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
        crate::backend::wasm::select::select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // `(or (= x 0) (< (/ 100 x) 5))` returns a Bool: `||` short-circuits — the rhs `(< (/ 100 x) 5)`
    // (containing the trapping `(/ 100 x)`) runs ONLY when the lhs `(= x 0)` is FALSE (x != 0). The
    // repeated-node CSE could target `(/ 100 x)` only if it were a class member in the dominating
    // frontier; the `Core::And` frontier arm (lhs-only) keeps the shielded rhs OUT, so NO divide is
    // emitted before the connective's short-circuit branch.
    let code = lir("(or (= x 0) (< (/ 100 x) 5))");
    let branch_ix = code
        .iter()
        .position(|i| matches!(i, Lir::If(_)))
        .expect("a short-circuit `||` lowers to an if for its rhs");
    let div_before = code[..branch_ix]
        .iter()
        .filter(|i| matches!(i, Lir::I64DivS))
        .count();
    assert_eq!(
        div_before, 0,
        "no `(/ 100 x)` may be hoisted before the `||` short-circuit branch (would trap at x=0 where \
         the lhs is true and the rhs must never run): {code:?}"
    );
    // RUNTIME trap/value parity for the `||` (x=0 short-circuits on the true lhs → the trapping rhs never
    // runs → true, no trap; x=4 → false; x=50 → true) migrated to the corpus (run via cdz-run): case "a
    // self-shielding or does not hoist its trapping right operand past the short-circuit" in
    // spec/semantics/02-binding-and-control.sexp.
}

/// CSE must not hoist a repeated trapping subexpr out of a conditionally-selected MATCH ARM (the match
/// sibling of the short-circuit-`and`/`or` cases above + the `if`-arm witness). `(match k (0 100) (_ (+
/// (/ 10 d) (/ 10 d))))`: the wildcard arm's repeated `(/ 10 d)` is a CSE class, but a match runs ONLY the
/// selected arm — the frontier walk descends only the SCRUTINEE (`k`), not the arm bodies, so an arm-local
/// candidate is never in the frontier a hoist targets. At `main(0, 0)` the `k=0` arm is taken → the
/// division-arm never runs → result 100, no trap. Pins the `Core::Match` conditional-position frontier arm
/// (analogous to `If`-cond and `And`-lhs): a future frontier change dropping the Match arm would re-regress
/// exactly as the missing And arm did in adv-55 — this witness catches it. The last unwitnessed
/// conditional position (And/Or have the witnesses above; the if-arm has the cont.101 select.rs witness).
///
/// The program is ALL-SCALAR (Int64 params/result, no value-heap), so it imports NO runtime — the test
/// runs STORE-FREE (a compile-artifact check via `imports_value_heap_runtime`, no `find_runtime_wasm` skip) and therefore executes
/// in storeless / clean CI too, where it must actually guard. (A `find_runtime_wasm() else return` skip —
/// copied from the heap-value sibling tests — would silently disable this regression witness.)
#[test]
fn cse_does_not_hoist_a_trapping_subexpr_out_of_a_match_arm() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (main (: k Int64) (: d Int64)) \
                   (match k (0 100) (_ (+ (/ 10 d) (/ 10 d))))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        !imports_value_heap_runtime(&bytes),
        "an all-scalar Int64 match program imports no value-heap runtime — the witness runs store-free"
    );
    // Value parity (k=0 selects the constant arm → 100; the wildcard arm's trapping `(/ 10 0)` must not be
    // hoisted/run) migrated to the corpus (run via cdz-run): case "a repeated trapping divide inside one
    // match arm is not hoisted past the match" in spec/semantics/02-binding-and-control.sexp.
}

/// The common-constructor sink also fires for a MATCH whose every (unguarded) arm builds the same
/// constructor: `(match k (0 (Some 10)) (1 (Some 20)) (_ (Some 30)))` builds `Some` ONCE and sinks the
/// payload into a per-position `match` (a scalar decision tree), instead of DUPLICATING the `sum-new`
/// build once per arm. This is the multi-arm analogue of the `if`-arm hoist (which already collapses a
/// nested `if` of a common constructor); a `match` lowers to a probe chain, not nested `Core::If`, so it
/// needs its own sink. Structurally the payload becomes a branchless `select` (the arm count here folds
/// to one), and the value must round-trip in EVERY arm direction — k=0→10, k=1→20, wildcard→30 — proving
/// the single build with a selected payload reproduces the per-arm builds. Kept opaque via a recursive
/// helper so the sum is a genuine runtime heap value.
#[test]
fn a_common_constructor_sinks_out_of_all_match_arms() {
    use crate::testkit::parse;
    // `mk` bottoms out immediately (`n >= 0`) with the match; being recursive it is not inlined, so the
    // Option stays a runtime heap value observed by `main`'s match.
    let src = "(module m \
                 (def (mk (: n Int64) (: k Int64)) \
                   (if (< n 0) (mk (+ n 1) k) \
                     (match k (0 (Some 10)) (1 (Some 20)) (_ (Some 30))))) \
                 (def (main (: k Int64)) \
                   (match (mk 0 k) ((Some v) v) (None -1))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The sunk payload folds to a branchless scalar `select`/`if` decision tree — a `select` witnesses
    // that the constructor was pulled out (pre-sink there was a per-arm `sum-new` and no payload select).
    let selects = count_opcode(&bytes, |op| {
        matches!(
            op,
            wasmparser::Operator::Select | wasmparser::Operator::TypedSelect { .. }
        )
    });
    assert!(
        selects >= 1,
        "the match common-constructor sink must lower the differing payload to a branchless select \
         (build-once); found no select"
    );
    // The payload VALUE (a match over the scrutinee selects the arm's Some payload) is ordinary match/sum
    // behavior covered by the corpus; only the build-once EMISSION witness (the select count above) stays
    // here — the corpus cannot observe the opcode count.
}

/// The build-once hoists COMPOSE through the differing positions they synthesize: when a hoist pushes a
/// differing field into a fresh `(if c pᵢ qᵢ)` and THAT field is itself a common constructor across the
/// arms, the synthesized `if` is re-run through the hoist (via `synth_if_hoisted`) so the nested
/// constructor is hoisted too. `(if c1 (tuple (Some a) 1) (if c2 (tuple (Some b) 1) (tuple (Some a) 1)))`
/// hoists the outer tuple ONCE, and its differing field-0 — a nested-if of `Some` — hoists to one `Some`:
/// the emitted body has exactly ONE `sum-new` (not three) and ONE `arr-alloc`. Value parity confirms the
/// deep composition preserves the arm dispatch (c1 → a, else-c2 → b, else → a) and its Perceus
/// consumption (only the selected payload materializes).
#[test]
fn the_build_once_hoists_compose_through_synthesized_differing_positions() {
    use crate::testkit::parse;
    // `f` is recursive so it is not inlined — the tuple/Some stay runtime heap values observed via `main`.
    let src = "(module m \
                 (def (f (: c1 Bool) (: c2 Bool) (: a Int64) (: b Int64) (: n Int64)) \
                   (if (< n 0) (f c1 c2 a b (+ n 1)) \
                     (if c1 (tuple (Option.Some a) 1) \
                       (if c2 (tuple (Option.Some b) 1) (tuple (Option.Some a) 1))))) \
                 (def (main (: c1 Bool) (: c2 Bool) (: a Int64) (: b Int64)) \
                   (match (. (f c1 c2 a b 0) 0) \
                     ((Option.Some v) (+ v (. (f c1 c2 a b 0) 1))) \
                     (_ -1))) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // Exactly ONE sum-new (the nested Some hoisted) — a select present witnesses the composed hoist.
    let selects = count_opcode(&bytes, |op| {
        matches!(
            op,
            wasmparser::Operator::Select | wasmparser::Operator::TypedSelect { .. }
        )
    });
    assert!(
        selects >= 1,
        "the composed hoist must lower the deepest differing operand to a branchless select"
    );
    // The VALUE + arm/element guard are corpus-covered (02-binding-and-control common-constructor
    // hoist family); only the build-once EMISSION witness (the opcode count above) stays here — the
    // corpus cannot observe an opcode count.
}

/// A repeated bounds-checked indexed read `(Option.expect (List.at xs i))` is shared by CSE (one
/// `vec-get` — pinned at the Lir level in `select.rs`); this is the RUNTIME companion, proving the
/// SHARED read is refcount-correct on the value heap. `List.at` BORROWS the list, so sharing the read
/// must leave `xs` fully live: the program reads element 2 TWICE (shared) AND `List.len xs` after — if
/// the shared `vec-get` mishandled the borrow (a premature drop of `xs` or a double-consume of the boxed
/// element), the later length read would see a freed/corrupted list. Built at run time via a push loop
/// so it genuinely imports the runtime (a constant list would fold away). xs = [4,3,2,1,0] → element 2
/// is 2; 2 + 2 + len(5) = 9.
#[test]
fn a_cse_shared_indexed_read_is_refcount_correct_and_leaves_the_list_live() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (build (: n Int64) (: acc (List Int64))) \
                   (if (< n 0) acc (build (- n 1) (List.push acc n)))) \
                 (def (f (: xs (List Int64)) (: k Int64)) \
                   (if (< k 0) (f xs (+ k 1)) \
                     (+ (+ (Option.expect (List.at xs 2) \"v\") (Option.expect (List.at xs 2) \"v\")) \
                        (List.len xs)))) \
                 (def (main (: n Int64)) (f (build n (list)) 0)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        imports_value_heap_runtime(&bytes),
        "a runtime list must build on the value heap (import the runtime)"
    );
    // The RUN — the shared element-2 read (2+2) plus a still-live List.len (5) = 9, proving the CSE-shared
    // borrow leaves the list live — is corpus-covered by 05-compound-types "a repeated same-index List.at
    // shared by CSE leaves the list live for a later length read"; this test keeps the builds-on-the-heap pin.
}

/// A repeated keyed lookup `(Option.expect (Map.lookup m k))` is shared by CSE (one `map-lookup` — an
/// O(log n) CHAMP walk — pinned at the Lir level in `select.rs`); this is the RUNTIME companion proving
/// the SHARED lookup is refcount-correct. `Map.lookup` BORROWS the map, so sharing must leave `m` fully
/// live: the program reads key 2 TWICE (shared) AND `Map.len m` after. If the shared lookup mishandled
/// the borrow (a premature drop of `m` or a double-consume of the boxed value), the later size read
/// would see a freed/corrupted map. Built at run time via an insert loop so it imports the runtime.
/// m = {0:0,1:10,2:20,3:30,4:40} → lookup 2 = 20; 20 + 20 + size(5) = 45.
#[test]
fn a_cse_shared_map_lookup_is_refcount_correct_and_leaves_the_map_live() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (build (: n Int64) (: acc (Map Int64 Int64))) \
                   (if (< n 0) acc (build (- n 1) (Map.insert acc n (* n 10))))) \
                 (def (f (: mp (Map Int64 Int64)) (: k Int64)) \
                   (if (< k 0) (f mp (+ k 1)) \
                     (+ (+ (Option.expect (Map.lookup mp 2) \"v\") (Option.expect (Map.lookup mp 2) \"v\")) \
                        (Map.len mp)))) \
                 (def (main (: n Int64)) (f (build n Map.empty) 0)) \
                 (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        imports_value_heap_runtime(&bytes),
        "a runtime map must build on the value heap (import the runtime)"
    );
    // The RUN — the shared key-2 lookup (20+20) plus a still-live Map.len (5) = 45, proving the CSE-shared
    // borrow leaves the map live — is corpus-covered by 05-compound-types "a repeated same-key Map.lookup
    // shared by CSE leaves the map live for a later size read"; this test keeps the builds-on-the-heap pin.
}

#[test]
fn record_with_over_a_runtime_record_materializes_the_operand_once_not_per_preserved_field() {
    use crate::testkit::parse;
    // REGRESSION (reviewer post-merge on 49d6eec14): `runtime_record_fields` builds a synth `(. record
    // field)` projection for EVERY preserved field, all sharing the ONE `record` operand node. The backend
    // has no CSE, so before the fix each `Core::Proj` RE-EMITTED the operand's computation — an ARITY≥3
    // record (≥2 preserved fields), one field updated, re-evaluated the operand once PER preserved field.
    // For a PURE operand that is a perf cliff (value still correct); for an EFFECTFUL operand the effect
    // would fire N times — an observable miscompile. FIX: materialize the operand ONCE into a self-keyed
    // `Core::Let`, every projection reading the shared slot. This gate pins that the operand's DISTINCTIVE
    // computation emits exactly ONCE. Operand = `(mk (+ v 987654321))`, a CALL (opaque → runtime path, not a
    // const record) whose ARGUMENT carries the unmistakable constant `987654321` — so the constant lands at
    // the operand's USE site (re-emitting the operand duplicates the whole call, arg included). `Record.with`
    // updates `b`, leaving TWO preserved fields (`a`, `c`). Before the fix the constant appeared TWICE (once
    // per preserved-field projection); the fix makes it ONE. (Verified: neutralizing the let-bind → 2.)
    let src = "(module m \
        (def (mk (: n Int64)) (if (= n 0) (record (= a 1) (= b 2) (= c 3)) (mk (- n 1)))) \
        (def (upd (: v Int64)) (. (Record.with (mk (+ v 987654321)) #\"b\" 99) a)) \
        (def (main (: v Int64)) (upd v)) \
        (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The operand's distinctive constant must appear EXACTLY ONCE — the operand computation is emitted once.
    let occurrences = count_opcode(
        &bytes,
        |op| matches!(op, wasmparser::Operator::I64Const { value } if *value == 987_654_321),
    );
    assert_eq!(
        occurrences, 1,
        "the runtime operand of `Record.with` must be materialized ONCE, not re-emitted per preserved \
         field (found the operand's distinctive constant {occurrences} times, expected 1)"
    );
    // The run VALUE (a preserved field reads its source through the materialized operand) is corpus-pinned
    // in 05-compound-types.sexp ("Record.with over a runtime-produced record leaves no live heap objects"
    // + the project/without kept-field cases); this test keeps only the corpus-inexpressible emit pin.
}

/// The CDZ0215 bare-field-name reject NAMES THE CORRECT SYMBOL SYNTAX and carries an applyable fix. Two
/// diagnostic-quality points: (1) the message's example must be the QUOTED form `#"z"` — a bare `#z` is NOT
/// symbol syntax (the reader reads it as an identifier and it hits this same reject), so an author copying a
/// `#z` example would loop; (2) when the offending operand is a bare IDENTIFIER (`Record.with r x 5`), the
/// near-certain intent is that name's field label, so a heuristic `x` → `#"x"` Replace fix makes the repair
/// one-shot. Round-trips: applying the fix (`x` → `#"x"`) clears the reject.
#[test]
fn a_bare_record_field_name_names_the_quoted_symbol_syntax_and_carries_a_fix() {
    use crate::testkit::parse;
    let diags = |src: &str| crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    let src = "(module m (def (f (: r (Record (x Int64)))) (Record.with r x 5)) (export f))";
    let ds = diags(src);
    let d = ds
        .iter()
        .find(|d| d.code.as_deref() == Some("CDZ0215"))
        .unwrap_or_else(|| panic!("a bare field name rejects CDZ0215: {ds:?}"));
    // The message names the QUOTED symbol form, NOT a bare `#z` (which would re-trigger the same error).
    assert!(
        d.message.contains("#\"field\"") && d.message.contains("#\"z\""),
        "the message names the quoted `#\"z\"` symbol syntax, not a bare `#z`: {}",
        d.message
    );
    assert!(
        !d.message.contains("`#z`") && !d.message.contains("`#field`"),
        "the misleading bare-`#z` example is gone (it is not valid symbol syntax): {}",
        d.message
    );
    // The fix rewrites the bare identifier to the field-label symbol.
    let fix = d
        .fix
        .as_ref()
        .expect("a bare identifier carries the `#\"x\"` fix");
    assert_eq!(fix.kind, crate::abi::FixKind::Replace);
    assert_eq!(fix.replacement, "#\"x\"");
    // ROUND-TRIP: applying the fix (`x` → `#"x"`) clears the reject — the annotation checks clean.
    let fixed = "(module m (def (f (: r (Record (x Int64)))) (Record.with r #\"x\" 5)) (export f))";
    assert!(
        diags(fixed)
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "applying the `x` → `#\"x\"` fix type-checks clean: {:?}",
        diags(fixed)
    );
}

/// The old 2-operand `Record.with r (z v)` pair form's MIGRATION hint names the QUOTED symbol syntax
/// `r #"z" v` — not a bare `r #z v` (which is not symbol syntax and would fail again). Companion of the
/// CDZ0215 wording fix: every field-label EXAMPLE in the record-op messages is the quoted `#"…"` form, so
/// following any of them compiles. Round-trips: the suggested 3-operand `#"x"` form checks clean.
#[test]
fn the_record_with_migration_hint_names_the_quoted_symbol_syntax() {
    use crate::testkit::parse;
    let diags = |src: &str| crate::diagnostics(&mut crate::db::Db::load(parse(src)));
    // The old pair form `(Record.with r (x 5))` → migration hint.
    let ds =
        diags("(module m (def (f (: r (Record (x Int64)))) (Record.with r (x 5))) (export f))");
    let d = ds
        .iter()
        .find(|d| d.message.contains("now takes three operands"))
        .unwrap_or_else(|| panic!("the pair form gets the migration hint: {ds:?}"));
    assert!(
        d.message.contains("#\"x\"") && !d.message.contains("`r #x v`"),
        "the migration hint names the quoted `#\"x\"` form, not a bare `#x`: {}",
        d.message
    );
    // ROUND-TRIP: the suggested 3-operand `#"x"` form checks clean.
    assert!(
        diags("(module m (def (f (: r (Record (x Int64)))) (Record.with r #\"x\" 5)) (export f))")
            .iter()
            .all(|d| d.severity != crate::abi::Severity::Error),
        "the migrated `r #\"x\" v` form the hint suggests type-checks clean"
    );
}

#[test]
fn record_project_and_without_over_a_runtime_record_build_from_projections_materialize_once() {
    use crate::testkit::parse;
    // FOLLOW-UP to `Record.with` over a runtime record: `Record.project` (keep the named fields) and
    // `Record.without` (drop the named fields) over a genuinely-RUNTIME record (a call result / param /
    // projection — NOT a compile-time-visible record literal) used to DECLINE "a record row operation over
    // a runtime record is not yet built" (they only folded a const `Core::Record`). FIX: build the kept
    // fields from synth `(. record field)` projections (the same `runtime_record_fields` helper `Record.with`
    // uses) and materialize the shared operand ONCE (self-keyed `Core::Let`, the reviewer-49d6eec14
    // materialize-once fix, now shared via `materialize_row_op_operand`). Operand = `(mk (+ v 987654321))`,
    // a call (runtime path) whose arg carries a distinctive constant landing at the use site; the result
    // keeps ≥2 fields, so a re-emit would duplicate the constant. Asserts: COMPILES (was a decline) and the
    // operand's constant emits EXACTLY ONCE. (The run VALUES — project/without over a runtime record read a
    // KEPT field = 1 — are corpus-pinned in 05-compound-types.sexp's runtime-row-op leak cases; this keeps
    // only the corpus-inexpressible materialize-once emit pin.)
    let check = |src: &str, what: &str| {
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .unwrap_or_else(|e| panic!("{what}: runtime row-op must compile, not decline: {e:?}"));
        let n = count_opcode(
            &bytes,
            |op| matches!(op, wasmparser::Operator::I64Const { value } if *value == 987_654_321),
        );
        assert_eq!(
            n, 1,
            "{what}: the runtime operand must be materialized ONCE (found {n}, expected 1)"
        );
    };
    // `project` KEEPS {a, c} (2 fields, so ≥2 projections share the operand).
    check(
        "(module m \
         (def (mk (: n Int64)) (if (= n 0) (record (= a 1) (= b 2) (= c 3)) (mk (- n 1)))) \
         (def (upd (: v Int64)) (. (Record.project (mk (+ v 987654321)) (a c)) a)) \
         (def (main (: v Int64)) (upd v)) \
         (export main))",
        "Record.project over a runtime record",
    );
    // `without` DROPS {b}, keeping {a, c} (2 fields).
    check(
        "(module m \
         (def (mk (: n Int64)) (if (= n 0) (record (= a 1) (= b 2) (= c 3)) (mk (- n 1)))) \
         (def (upd (: v Int64)) (. (Record.without (mk (+ v 987654321)) (b)) a)) \
         (def (main (: v Int64)) (upd v)) \
         (export main))",
        "Record.without over a runtime record",
    );
}

#[test]
fn record_merge_and_pop_over_a_runtime_record_build_from_projections_materialize_once() {
    use crate::testkit::parse;
    // FINAL slice of the record row-op-over-runtime-record family (after with / project / without):
    // `Record.merge` (UNION of two records) and `Record.pop` (split into `(popped-value, rest-record)`)
    // over a genuinely-RUNTIME record used to DECLINE "not yet built" (they folded only const records).
    // FIX: build fields from synth `(. record field)` projections and materialize each runtime operand ONCE
    // — pop via the shared `materialize_row_op_operand` (its result is a TUPLE, whose value + rest both read
    // the one operand); merge via the TWO-operand generalization (a `Core::Let` binding per runtime operand,
    // since either of merge's two operands may be runtime independently). The distinctive per-operand
    // constants (987654321 / 111222333) each land at their operand's use site, so a re-emit would duplicate
    // them; assert each appears EXACTLY ONCE.
    // POP: `(Record.pop (mk (+ v 987654321)) a)` → `(a-value, {b,c})`; read tuple element 0 = popped a = 1.
    // The operand feeds BOTH the popped value's projection AND the rest record's ≥1 projection.
    let pop_src = "(module m \
        (def (mk (: n Int64)) (if (= n 0) (record (= a 1) (= b 2) (= c 3)) (mk (- n 1)))) \
        (def (upd (: v Int64)) (. (Record.pop (mk (+ v 987654321)) a) 0)) \
        (def (main (: v Int64)) (upd v)) \
        (export main))";
    let pop_bytes = compile_component(&crate::codec::encode(&parse(pop_src)))
        .expect("Record.pop over a runtime record must compile, not decline");
    let pop_n = count_opcode(
        &pop_bytes,
        |op| matches!(op, wasmparser::Operator::I64Const { value } if *value == 987_654_321),
    );
    assert_eq!(
        pop_n, 1,
        "Record.pop: runtime operand materialized ONCE (found {pop_n}, expected 1)"
    );
    // MERGE: union of TWO runtime records `(mkA (+ v 987654321))` and `(mkB (+ v 111222333))`; read `c`
    // (from `b`, the second operand) = 3. Each operand carries its own distinctive constant → each once.
    let merge_src = "(module m \
        (def (mkA (: n Int64)) (if (= n 0) (record (= a 1) (= b 2)) (mkA (- n 1)))) \
        (def (mkB (: n Int64)) (if (= n 0) (record (= c 3) (= d 4)) (mkB (- n 1)))) \
        (def (upd (: v Int64)) (. (Record.merge (mkA (+ v 987654321)) (mkB (+ v 111222333))) c)) \
        (def (main (: v Int64)) (upd v)) \
        (export main))";
    let merge_bytes = compile_component(&crate::codec::encode(&parse(merge_src)))
        .expect("Record.merge over runtime records must compile, not decline");
    for k in [987_654_321i64, 111_222_333] {
        let n = count_opcode(
            &merge_bytes,
            |op| matches!(op, wasmparser::Operator::I64Const { value } if *value == k),
        );
        assert_eq!(
            n, 1,
            "Record.merge: operand carrying {k} materialized ONCE (found {n}, expected 1)"
        );
    }
    // The run VALUES (pop's popped `a` = 1, merge's `c` = 3) are corpus-pinned in 05-compound-types.sexp
    // (the runtime-row-op leak cases + `ce03 Record.merge of two RUNTIME-built records`); this keeps only
    // the corpus-inexpressible per-operand materialize-once emit pins above.
}

/// §3c REDUCER-SHAPE de-risk (WHITE-BOX half): a runtime-built record with a STRING field and a BYTES field —
/// the essence of the reducer Event (`{content-type, payload}`) — that escapes must IMPORT the value-heap
/// runtime, i.e. it takes the runtime value-encode WALKER (not a const-fold pre-render). This compile-artifact
/// pin (the import name is not observable behavior, so it is corpus-inexpressible) stays in-crate; the RUN half
/// — that the mixed String+Bytes shape value-encodes to the canonical `(= name value)` wire with the Bytes
/// field as `b".."` — is corpus case 05-compound-types "vse12 a RUNTIME-BUILT record with String + Bytes
/// fields escapes via the runtime value-encode walker (the reducer-Event shape)".
#[test]
fn a_reducer_event_shaped_record_with_bytes_imports_the_value_encode_runtime() {
    use crate::testkit::parse;
    // Fields sorted (BTreeMap): `ct` < `pl`. `ct` = String, `pl` = Bytes. Runtime-built (recursion defeats
    // the constant fold), so it takes the runtime value-encode walker — the reducer apply's result path.
    let src = "(module m (def (f n) (if (= n 0) (record (= ct \"wasm\") (= pl (Bytes.of (list 1 2 3)))) (f (- n 1)))) \
                 (def (main) (f 2)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    assert!(
        imports_value_heap_runtime(&bytes),
        "a runtime record-with-Bytes escape must import the value-heap runtime"
    );
}

/// The PREPARSED TARGET WIT WORLD (KIND_WIT_WORLD bytes) declaring a single export interface `fold` with
/// member `apply : func(input: list<u8>) -> list<u8>` — the world the §3c bytes-provider tests target so the
/// world-driven gate (`world_bytes_crossing_export` → `bridge_decision`) routes `apply` to the bytes emit.
/// Hand-built via the cadenza-ast `Builder` (the same node `world_schema_tree`/`wit_interface`/`wit_func_sig`
/// and v-syntax's inline decl produce; descriptors are `build_type`-form: `("list" (u8))` for `list<u8>`),
/// then `codec::encode`d — exactly the shape `wit_world::parse_target_world` reads. PURE (no `kv` imports):
/// a kv-importing world's reducer would decline until the host-fused slice.
fn reducer_apply_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let h = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![h, u])
    };
    let param_ty = byte_list(&mut b);
    let result_ty = byte_list(&mut b);
    // (member "apply" (func (param "input" <list<u8>>) (result <list<u8>>)))
    let func_h = b.name("func");
    let param_h = b.name("param");
    let pn = b.name("input");
    let param_node = b.list(vec![param_h, pn, param_ty]);
    let result_h = b.name("result");
    let result_node = b.list(vec![result_h, result_ty]);
    let func = b.list(vec![func_h, param_node, result_node]);
    let member_h = b.name("member");
    let mn = b.name("apply");
    let apply = b.list(vec![member_h, mn, func]);
    // (export "fold" <apply>)
    let exp_h = b.name("export");
    let iname = b.name("fold");
    let fold = b.list(vec![exp_h, iname, apply]);
    // (world "reducer" <fold>)
    let world_h = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![world_h, wn, fold]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// A KIND_WIT_WORLD declaring the scope-A kv-genesis world: export interface `fold` member `apply`
/// (list<u8>->list<u8>) AND import interface `cadenza:agent-kernel/kv` (the FQ name v-ah's host binds)
/// member `put` (key,value: list<u8> -> unit). Drives the host-fused (GAP B) emit: the world-driven gate
/// routes `apply` to the bytes emit, and the emit imports the kv host interface at the world's FQ name.
fn reducer_kv_put_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let hh = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![hh, u])
    };
    // export fold { apply: func(input: list<u8>) -> list<u8> }
    let a_in = byte_list(&mut b);
    let a_out = byte_list(&mut b);
    let apply = {
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("input");
        let pnode = b.list(vec![param_h, pn, a_in]);
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, a_out]);
        let func = b.list(vec![func_h, pnode, rnode]);
        let member_h = b.name("member");
        let mn = b.name("apply");
        b.list(vec![member_h, mn, func])
    };
    let exp_h = b.name("export");
    let ename = b.name("fold");
    let fold = b.list(vec![exp_h, ename, apply]);
    // import cadenza:agent-kernel/kv { put: func(key: list<u8>, value: list<u8>) -> unit }
    let k_key = byte_list(&mut b);
    let k_val = byte_list(&mut b);
    let put = {
        let func_h = b.name("func");
        let p1h = b.name("param");
        let p1n = b.name("key");
        let p1 = b.list(vec![p1h, p1n, k_key]);
        let p2h = b.name("param");
        let p2n = b.name("value");
        let p2 = b.list(vec![p2h, p2n, k_val]);
        let result_h = b.name("result");
        let unit_h = b.atom_leaf(Leaf::Str("unit".into()));
        let unit_ty = b.list(vec![unit_h]);
        let rnode = b.list(vec![result_h, unit_ty]);
        let func = b.list(vec![func_h, p1, p2, rnode]);
        let member_h = b.name("member");
        let mn = b.name("put");
        b.list(vec![member_h, mn, func])
    };
    let imp_h = b.name("import");
    let kv_name = b.name("cadenza:agent-kernel/kv");
    let kv = b.list(vec![imp_h, kv_name, put]);
    // world reducer { export fold; import kv }
    let world_h = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![world_h, wn, fold, kv]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// A KIND_WIT_WORLD declaring the kv-GET world (B1b): export interface `fold` member `apply`
/// (list<u8>->list<u8>) AND import interface `cadenza:agent-kernel/kv` member `get`
/// (key: list<u8> -> option<list<u8>>). This is the world v-ah's `reducer_world_artifact` declares via
/// `opt_bytes_desc`; the `option<list<u8>>` result descriptor decodes to `WitType::Option(List(U8))` in
/// `parse_wit_type`. Drives the kv.get emit slice (S0): the guest lifts the host's `option<list<u8>>`
/// result into a value-heap `Option<Bytes>` (Some = a Bytes leaf, None = absent).
fn reducer_kv_get_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let hh = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![hh, u])
    };
    // export fold { apply: func(input: list<u8>) -> list<u8> }
    let a_in = byte_list(&mut b);
    let a_out = byte_list(&mut b);
    let apply = {
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("input");
        let pnode = b.list(vec![param_h, pn, a_in]);
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, a_out]);
        let func = b.list(vec![func_h, pnode, rnode]);
        let member_h = b.name("member");
        let mn = b.name("apply");
        b.list(vec![member_h, mn, func])
    };
    let exp_h = b.name("export");
    let ename = b.name("fold");
    let fold = b.list(vec![exp_h, ename, apply]);
    // import cadenza:agent-kernel/kv { get: func(key: list<u8>) -> option<list<u8>> }
    let g_key = byte_list(&mut b);
    let opt_bytes = {
        let opt_h = b.atom_leaf(Leaf::Str("option".into()));
        let inner = byte_list(&mut b);
        b.list(vec![opt_h, inner])
    };
    let get = {
        let func_h = b.name("func");
        let p1h = b.name("param");
        let p1n = b.name("key");
        let p1 = b.list(vec![p1h, p1n, g_key]);
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, opt_bytes]);
        let func = b.list(vec![func_h, p1, rnode]);
        let member_h = b.name("member");
        let mn = b.name("get");
        b.list(vec![member_h, mn, func])
    };
    let imp_h = b.name("import");
    let kv_name = b.name("cadenza:agent-kernel/kv");
    let kv = b.list(vec![imp_h, kv_name, get]);
    let world_h = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![world_h, wn, fold, kv]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// A KIND_WIT_WORLD declaring a NULLARY, `list<u8>`-returning import: export interface `fold` member
/// `apply` (list<u8>->list<u8>) AND import interface `cadenza:agent-kernel/identity` member `id`
/// (NO params -> list<u8>) — the `identity.id : () -> reducer-id` shape the operator's generic
/// world-import directive names. Exercises the zero-param + `list<u8>`-result branch of `parse_wit_func`
/// / `parse_wit_type` and drives a NULLARY synchronous `Core::HostCall` (v-inference generic-surface B0).
fn reducer_nullary_identity_import_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let hh = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![hh, u])
    };
    // export fold { apply: func(input: list<u8>) -> list<u8> }
    let a_in = byte_list(&mut b);
    let a_out = byte_list(&mut b);
    let apply = {
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("input");
        let pnode = b.list(vec![param_h, pn, a_in]);
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, a_out]);
        let func = b.list(vec![func_h, pnode, rnode]);
        let member_h = b.name("member");
        let mn = b.name("apply");
        b.list(vec![member_h, mn, func])
    };
    let exp_h = b.name("export");
    let ename = b.name("fold");
    let fold = b.list(vec![exp_h, ename, apply]);
    // import cadenza:agent-kernel/identity { id: func() -> list<u8> } — ZERO params.
    let id_out = byte_list(&mut b);
    let id_member = {
        let func_h = b.name("func");
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, id_out]);
        let func = b.list(vec![func_h, rnode]); // no param nodes → nullary
        let member_h = b.name("member");
        let mn = b.name("id");
        b.list(vec![member_h, mn, func])
    };
    let imp_h = b.name("import");
    let id_iface_name = b.name("cadenza:agent-kernel/identity");
    let identity = b.list(vec![imp_h, id_iface_name, id_member]);
    // world reducer { export fold; import identity }
    let world_h = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![world_h, wn, fold, identity]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// B0 (generic world-import surface, v-inference FRONT-END invariant): a NULLARY, `list<u8>`-returning
/// world import (`identity.id : () -> reducer-id`, the shape the operator's no-hardcoded-interfaces
/// directive names) TYPES + LOWERS to a synchronous `Core::HostCall{args:[], result:Bytes}` — driven
/// purely off the declared `world.wit` via `is_world_import_op`, with ZERO per-interface arms. The perform
/// is a zero-arg application of the member projection `((. identity id))` (v-syntax ruling) under
/// `(host (identity) …)`. This pins the front-end half generically for a nullary + `list<u8>` import,
/// DECOUPLED from the backend boundary-emit increment (Bytes/`list<u8>` host results don't cross the
/// component boundary yet — `backend/wasm/mod.rs:182`, v-rust-backend's lane), so it stays green while
/// that increment lands.
#[test]
fn a_nullary_list_u8_world_import_lowers_to_a_synchronous_host_call() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // A reducer performs the nullary import identity.id() under `(host (identity) …)`. is_world_import_op
    // reads the declared world (set manually — Db::load, unlike compile::compile, does not populate it), so
    // the perform is a SYNCHRONOUS host import: it stays a Core::HostCall (NOT reified to a record).
    let src = "(module m \
                 (effect identity (op id (-> Bytes))) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (identity) ((. identity id)))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    db.wit_world = Some(reducer_nullary_identity_import_world_bytes());
    let d = db.def_by_name("apply").expect("def apply");
    let body = db.defs[d].body.expect("apply has a body");
    match core_of(&mut db, body) {
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            assert_eq!(
                &*effect, "identity",
                "the host call carries the import interface short name"
            );
            assert_eq!(
                &*op, "id",
                "the host call carries the import member name verbatim"
            );
            assert!(
                args.is_empty(),
                "a nullary import perform lowers to a HostCall with ZERO args, got {}",
                args.len()
            );
            assert!(
                matches!(result, crate::ty::Ty::Bytes),
                "the nullary import's list<u8> result types as Bytes, got {result:?}"
            );
        }
        other => panic!(
            "a nullary world-import perform must lower to a synchronous Core::HostCall (driven off \
             is_world_import_op, no per-interface arm), got {other:?}"
        ),
    }
}

/// A KIND_WIT_WORLD declaring a MULTI-ARG import member whose SECOND arg is a nested RECORD — the
/// `deliver.deliver-message : (reducer-id, message) -> bool` shape (a privileged event-reducer import,
/// message = `record { contract, payload }`). Exercises the multi-arg + record-param branch a generic
/// world-import perform must lower, beyond the nullary/single-arg cases already pinned.
fn reducer_deliver_record_import_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let hh = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![hh, u])
    };
    // export fold { apply: func(input: list<u8>) -> list<u8> }
    let a_in = byte_list(&mut b);
    let a_out = byte_list(&mut b);
    let apply = {
        let func_h = b.name("func");
        let param_h = b.name("param");
        let pn = b.name("input");
        let pnode = b.list(vec![param_h, pn, a_in]);
        let result_h = b.name("result");
        let rnode = b.list(vec![result_h, a_out]);
        let func = b.list(vec![func_h, pnode, rnode]);
        let member_h = b.name("member");
        let mn = b.name("apply");
        b.list(vec![member_h, mn, func])
    };
    let exp_h = b.name("export");
    let ename = b.name("fold");
    let fold = b.list(vec![exp_h, ename, apply]);
    // import cadenza:agent-kernel/deliver { deliver-message: func(target: list<u8>,
    //   message: record { contract: list<u8>, payload: list<u8> }) -> bool } — a 2-arg member,
    //   the 2nd arg a nested record.
    let msg_record = {
        let rh = b.atom_leaf(Leaf::Str("record".into()));
        let cn = b.name("contract");
        let ct = byte_list(&mut b);
        let cfield = b.list(vec![cn, ct]);
        let pn = b.name("payload");
        let pt = byte_list(&mut b);
        let pfield = b.list(vec![pn, pt]);
        b.list(vec![rh, cfield, pfield])
    };
    let deliver_member = {
        let func_h = b.name("func");
        let p1h = b.name("param");
        let p1n = b.name("target");
        let p1t = byte_list(&mut b);
        let p1 = b.list(vec![p1h, p1n, p1t]);
        let p2h = b.name("param");
        let p2n = b.name("message");
        let p2 = b.list(vec![p2h, p2n, msg_record]);
        let result_h = b.name("result");
        let bool_h = b.name("bool");
        let bool_ty = b.list(vec![bool_h]);
        let rnode = b.list(vec![result_h, bool_ty]);
        let func = b.list(vec![func_h, p1, p2, rnode]);
        let member_h = b.name("member");
        let mn = b.name("deliver-message");
        b.list(vec![member_h, mn, func])
    };
    let imp_h = b.name("import");
    let iface_name = b.name("cadenza:agent-kernel/deliver");
    let deliver = b.list(vec![imp_h, iface_name, deliver_member]);
    let world_h = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![world_h, wn, fold, deliver]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// A MULTI-ARG world-import perform whose second argument is a RECORD (`deliver.deliver-message(target,
/// message)`, the privileged event-reducer shape) lowers to a synchronous `Core::HostCall{args:[_, _],
/// result:Bool}` — the front-end places BOTH operands (a Bytes and a record literal) into the HostCall
/// generically, driven off `is_world_import_op`, ZERO per-interface arms. Pins the multi-arg + compound-arg
/// branch beyond the nullary/single-arg cases, decoupled from the backend boundary-emit increment (v-rb).
#[test]
fn a_multi_arg_world_import_perform_with_a_record_arg_lowers_to_a_synchronous_host_call() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let src = "(module m \
                 (effect deliver \
                   (op deliver-message (-> Bytes (Record (contract Bytes) (payload Bytes)) Bool))) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (deliver) \
                     ((. deliver deliver-message) (. e pl) \
                        (record (= contract (. e pl)) (= payload (. e pl)))))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    db.wit_world = Some(reducer_deliver_record_import_world_bytes());
    let d = db.def_by_name("apply").expect("def apply");
    let body = db.defs[d].body.expect("apply has a body");
    match core_of(&mut db, body) {
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            assert_eq!(
                &*effect, "deliver",
                "the host call carries the import interface short name"
            );
            assert_eq!(
                &*op, "deliver-message",
                "the host call carries the import member name verbatim"
            );
            assert_eq!(
                args.len(),
                2,
                "a 2-arg import perform lowers to a HostCall with BOTH operands, got {}",
                args.len()
            );
            assert!(
                matches!(result, crate::ty::Ty::Bool),
                "the deliver-message import's bool result types as Bool, got {result:?}"
            );
        }
        other => panic!(
            "a multi-arg record-arg world-import perform must lower to a synchronous Core::HostCall \
             (generic, no per-interface arm), got {other:?}"
        ),
    }
}

/// NO-REDECLARE end-to-end: a reducer performs a world import WITHOUT a hand-written `(effect …)` decl — it
/// declares the world IN SOURCE and performs `identity.id` under `(host (identity) …)`. The load-time
/// world-import sidecar (`wit_world::inject_world_import_effects`, run before `scan_top_level`) synthesizes
/// the `(effect identity (op id (-> Bytes)))` decl from the world's import, so `identity` binds (no CDZ0101)
/// and the perform lowers to a synchronous `Core::HostCall{args:[], result:Bytes}` — the tick-6 gap
/// (`unbound name identity`) is closed. This is the drift-free surface: the guest names the world once and
/// A `(do …)`-ROOTED reducer (a bare source file with NO `(module …)` wrapper — the natural authoring form
/// v-syntax's ML renders to) must get its in-source world synthesized too. The sidecar's `top_level_items`
/// previously only recognized a `(module …)` / bare root, so an in-source `(world …)` inside a `(do …)` root
/// was invisible → nothing synthesized → `(host (identity) …)` was CDZ0101 unbound. Now the `(do …)` root's
/// items are scanned, so `identity` binds and the perform lowers to a `Core::HostCall`.
#[test]
fn a_do_rooted_reducer_synthesizes_its_in_source_world_import_effects() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // Bare file: a top-level `(do (world …) (def …) (export …))` — no `(module …)` wrapper.
    let src = "(do \
                 (world reducer-world \
                   (import identity (member id (func (result (\"list\" (u8)))))) \
                   (export guest (member on-message (func (param msg (\"list\" (u8))) (result (\"list\" (u8))))))) \
                 (def (on-message (: msg Bytes)) (host (identity) ((. identity id)))) \
                 (export on-message))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("unbound name `identity`")),
        "a (do)-rooted in-source world must synthesize its identity import (no CDZ0101): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let d = db.def_by_name("on-message").expect("def on-message");
    let body = db.defs[d].body.expect("on-message body");
    match core_of(&mut db, body) {
        Core::HostCall { effect, op, .. } => {
            assert_eq!((&*effect, &*op), ("identity", "id"));
        }
        other => {
            panic!("identity.id in a (do)-rooted reducer must lower to a HostCall, got {other:?}")
        }
    }
}

/// WIT enum self-declaration, sub-slice 2 (resolve/infer half): an IMPOSED in-source world imports an op
/// returning an ANONYMOUS `enum { active, closed }` the guest does NOT mirror. Sub-slice 1
/// (`synthesize_missing_enum_type_decls`) injects the nominal `(type Wit… Active Closed)`; per the operator
/// ruling the guest references the NAMEABLE case constructors (`Active`/`Closed`) — NOT the internal synth
/// type name — to match the imported enum result. This pins that the bare constructors RESOLVE and the match
/// TYPE-CHECKS (the front-end half; the run-level boundary crossing is emit, v-rb's Direction A).
#[test]
fn a_no_mirror_world_import_enum_result_is_matched_via_synthesized_nameable_constructors() {
    use crate::db::Db;
    let src = "(module m \
                 (world reducer (import lifecycle (member status (func (result (\"enum\" active closed)))))) \
                 (def (apply (: e Bytes)) \
                   (host (lifecycle) \
                     (match ((. lifecycle status)) ((Active) 1) ((Closed) 2)))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| {
            let m = &d.message;
            m.contains("unbound")
                || m.contains("Active")
                || m.contains("Closed")
                || m.contains("no field")
                || m.contains("requires a record")
                || m.contains("no machine representation")
        })
        .map(|d| d.message.clone())
        .collect();
    assert!(
        bad.is_empty(),
        "a no-mirror imported enum result matches via the synthesized nameable constructors Active/Closed \
         with no unbound/type error (sub-slice 2 resolve/infer half): {bad:?}"
    );
}

/// performs any import, zero per-interface arms, no mirrored effect decl.
#[test]
fn a_no_redeclare_world_import_perform_resolves_via_the_injected_effect() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // In-source (world) importing identity.id : () -> list<u8>; the WIT type descriptor is the STRING-head
    // canonical form `("list" (u8))` the world reader expects.
    let src = "(module m \
                 (world reducer (import identity (member id (func (result (\"list\" (u8))))))) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (identity) ((. identity id)))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    // The injected effect binds `identity` — no CDZ0101 for the un-declared name.
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("unbound name `identity`")),
        "the injected world-import effect must bind `identity` (no CDZ0101): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let d = db.def_by_name("apply").expect("def apply");
    let body = db.defs[d].body.expect("apply has a body");
    match core_of(&mut db, body) {
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            assert_eq!(&*effect, "identity");
            assert_eq!(&*op, "id");
            assert!(
                args.is_empty(),
                "nullary import → HostCall with zero args, got {}",
                args.len()
            );
            assert!(
                matches!(result, crate::ty::Ty::Bytes),
                "identity.id's list<u8> result types as Bytes, got {result:?}"
            );
        }
        other => panic!(
            "a no-redeclare world-import perform must resolve + lower to a Core::HostCall via the injected \
             effect, got {other:?}"
        ),
    }
}

/// NAMESPACED `Ordering.of` (former top-level `compare`): the three-way comparison is a member on the
/// built-in `Ordering` record (operator directive: prelude records with associated functions, no bare
/// globals) — carried on the `Ordering` `TypeDecl.associated`, the SAME pattern as `Ast.module`.
/// `(. Ordering of) a b : Ordering`, reducing identically to the old `compare` (same `Prim::Compare`).
#[test]
fn ordering_of_resolves_and_types_as_the_three_way_compare() {
    use crate::db::Db;
    let src = "(module m (def (cmp (: a Int64) (: b Int64)) ((. Ordering of) a b)) (export cmp))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("unbound") || d.message.contains("field `of`")),
        "(. Ordering of) must resolve on the built-in Ordering record: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let d = db.def_by_name("cmp").expect("def cmp");
    let body = db.defs[d].body.expect("cmp body");
    let ty = crate::infer::type_of(&mut db, body);
    assert_eq!(
        ty.render_name(&db.name_ctx()),
        "Ordering",
        "Ordering.of a b : Ordering (same reduction as the former `compare`), got {ty:?}"
    );
}

/// NAMESPACED `Ast.print` / `Ast.read` (former top-level `print` / `read`): the compiler-exposed printer and
/// reader are members on the built-in `Ast` record (operator directive: prelude records with associated
/// functions, no bare globals) — carried on the `Ast` `TypeDecl.associated`, the SAME pattern as `Ast.module`.
/// `(. Ast print) v : String` and `(. Ast read) s : Ast`, reducing identically to the old bare names (same
/// `Prim::Print` / `Prim::Read`). Pins the FRONT-END: both resolve + type, no unbound / no-such-field.
#[test]
fn ast_print_and_read_resolve_and_type_as_the_printer_and_reader() {
    use crate::db::Db;
    let src = "(module m (def (pr (: v Ast)) ((. Ast print) v)) (def (rd (: s String)) ((. Ast read) s)) (export pr) (export rd))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags.iter().any(|d| d.message.contains("unbound")
            || d.message.contains("field `print`")
            || d.message.contains("field `read`")),
        "(. Ast print)/(. Ast read) must resolve on the built-in Ast record: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let pr = db.def_by_name("pr").expect("def pr");
    let pr_body = db.defs[pr].body.expect("pr body");
    let pr_ty = crate::infer::type_of(&mut db, pr_body);
    assert_eq!(
        pr_ty.render_name(&db.name_ctx()),
        "String",
        "Ast.print v : String (same reduction as the former `print`), got {pr_ty:?}"
    );
    let rd = db.def_by_name("rd").expect("def rd");
    let rd_body = db.defs[rd].body.expect("rd body");
    let rd_ty = crate::infer::type_of(&mut db, rd_body);
    assert_eq!(
        rd_ty.render_name(&db.name_ctx()),
        "Ast",
        "Ast.read s : Ast (same reduction as the former `read`), got {rd_ty:?}"
    );
}

/// SELF-REFLECTION `Ast.module` (front-end): the reflection member on the built-in `Ast` record resolves via
/// ordinary member access and TYPES as the built-in `Ast` sum — the type-directed, prelude-derived,
/// NAMESPACED replacement for the retired `(. Ast self)` blind syntax-rewrite (operator directive: namespace
/// on the Ast record, no bare global; named `module` for the capture's scope). (The compile-time FILL of the
/// module's source is v-compiler-primitives' lowering slice; this pins the FRONT-END: `(. Ast module)` binds
/// + types, no unbound / no-field.)
#[test]
fn ast_module_resolves_and_types_as_the_builtin_ast() {
    use crate::db::Db;
    let src = "(module m (def (self-ast) (. Ast module)) (export self-ast))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("field `module`") || d.message.contains("unbound")),
        "(. Ast module) must resolve on the built-in Ast record: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    let d = db.def_by_name("self-ast").expect("def self-ast");
    let body = db.defs[d].body.expect("self-ast body");
    let ty = crate::infer::type_of(&mut db, body);
    assert_eq!(
        ty.render_name(&db.name_ctx()),
        "Ast",
        "Ast.module is a bare value of the built-in Ast sum, got {ty:?}"
    );
}

// NOTE: the self-reflection shadow-correctness (a user `(type Ast …)` shadows away `Ast.module`, so it is a
// no-such-field member access → CDZ0201, not the reflection) is a LANGUAGE SEMANTIC and lives in the
// language-neutral corpus (`spec/semantics/08-self-hosting-surface.sexp`, "a user (type Ast …) shadows the
// built-in, so Ast.module is no longer the self-reflection"), NOT a Rust `#[test]` locked to this
// implementation (operator directive: semantics belong in the corpus, other-language compilers are planned).

// NOTE: the `(const <expr>)` FORCE-EVAL block front-end (see-through resolve/type/lower + the arity
// reject) is BEHAVIORAL coverage, so it lives in the language-neutral corpus, NOT a Rust `#[test]`
// (operator directive: compiler/behavioral coverage belongs in the corpus, other-language compilers are
// planned). See `spec/semantics/28-compiler-primitives.sexp`, section "Primitive 2: const execution — the
// `(const <expr>)` force-eval BLOCK (front-end see-through; force semantics pending)": the see-through fold
// value, the see-through inner-annotation width, the application-head see-through, and the arity reject
// (CDZ0201). The force-eval / reject-if-not-const semantics + the const-demand flag are v-compiler-
// primitives' downstream cases in the same file.

/// NO-ANNOTATION EXPORT BOUNDARY (`derive_world_export_param_annotations`): a reducer writes its guest-export
/// entry point with a BARE param — `(def (on-message msg) …)` — and the export-param pre-pass DERIVES `msg`'s
/// type from the matching world guest-export member (`on-message`'s declared `msg` param, a record), injecting
/// the annotation BEFORE resolve. So the body's field access `(. msg payload)` type-checks and the boundary
/// param is NOT the CDZ0201 "parameter type is ambiguous" it would be without a world (verified below). This
/// removes the last boundary annotation a reducer needs; its own nominal sums stay.
#[test]
fn a_bare_export_param_is_typed_from_the_world_guest_export_member() {
    use crate::db::Db;
    // Bare param `msg` (NO annotation); the world's guest export `on-message` declares `msg` as a record
    // `{contract: list<u8>, payload: list<u8>}` (STRING-head WIT descriptors). The body reads `msg.payload`,
    // which only resolves if `msg` is typed as that record.
    let src = "(do \
                 (world reducer-world \
                   (export guest (member on-message \
                     (func (param msg (\"record\" (contract (\"list\" (u8))) (payload (\"list\" (u8))))) \
                           (result (\"list\" (u8))))))) \
                 (def (on-message msg) (. msg payload)) \
                 (export on-message))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "a bare export param must be typed from the world member (no CDZ0201 ambiguity): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // The derived param types as the world's record — `contract` + `payload` fields, both Bytes (list<u8>).
    let d = db.def_by_name("on-message").expect("def on-message");
    let param = db.defs[d].params[0];
    let ty = crate::infer::type_of(&mut db, param);
    match &ty {
        crate::ty::Ty::Record(fields) => {
            let names: Vec<&str> = fields.keys().map(|k| k.name.as_ref()).collect();
            assert!(
                names.contains(&"contract") && names.contains(&"payload"),
                "derived record must carry the world member's fields, got {names:?}"
            );
            assert!(
                fields
                    .iter()
                    .all(|(_, t)| matches!(t, crate::ty::Ty::Bytes)),
                "each list<u8> field types as Bytes, got {ty:?}"
            );
        }
        other => panic!("bare `msg` must derive the world member's record type, got {other:?}"),
    }
}

/// MEANINGFULNESS GUARD for [`a_bare_export_param_is_typed_from_the_world_guest_export_member`]: the SAME bare
/// `(def (on-message msg) (. msg payload))` WITHOUT an in-source `(world …)` has nothing to derive `msg` from,
/// so it IS the CDZ0201 "parameter type is ambiguous" boundary fault. This proves the world-driven derivation
/// (not some unrelated inference) is what types the param in the test above.
#[test]
fn a_bare_export_param_without_a_world_is_still_ambiguous() {
    use crate::db::Db;
    let src = "(do (def (on-message msg) (. msg payload)) (export on-message))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "without a world, a bare export param has no derivation source → CDZ0201: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// A RECORD/TUPLE entry param on the plain scalar-result export declines with a message naming the PARAM,
/// not the result. Before, `export_result_valtype` (shared by result AND param positions) surfaced its
/// RESULT-phrased error ("returning a (Record …) on the multi-export boundary") for a record PARAM — doubly
/// wrong (it is a parameter; there is one export). Pins the truthful param-constraint message. (Admitting
/// fixed-shape scalar record/tuple params — the make-forwarding compound-param rebuild the resource-escape
/// path already does — is a later feature slice; this only corrects the diagnostic.)
#[test]
fn a_record_entry_param_declines_naming_the_param_not_a_bogus_multi_export_return() {
    use crate::testkit::parse;
    let src = "(do (def (main (: r (Record (: x Int64) (: y Int64)))) (+ (. r x) (. r y))) (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "a record entry param declines on this export path"
    );
    let msgs: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(
        !msgs
            .iter()
            .any(|m| m.contains("returning a") || m.contains("multi-export boundary")),
        "must NOT surface the result-phrased multi-export message for a PARAM: {msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("has no scalar boundary representation") && m.contains("parameter")),
        "must name the PARAM constraint truthfully: {msgs:?}"
    );
}

/// A native-rust `Value.encode`/`Value.decode` over a RECURSIVE type (`Ast = … (List (List Ast))`) DECLINES
/// rather than emitting non-terminating rust that HANGS rustc. The recursion runs through a `List` payload,
/// which the enum-sizing recursion check (`enums::variant_is_recursive` / `reaches_decl`) deliberately
/// excludes (a `Vec<Ast>` field is finite-sized), so the value-form WALK's unbounded recursion was NOT
/// guarded — the codec generated rust rustc could not compile in bounded time. Pins the up-front decline
/// (fast: compile-only, no rustc), guarding against a regression back to the compile hang.
#[test]
fn a_recursive_type_value_codec_declines_on_rust_not_hangs() {
    use crate::testkit::parse;
    let src = "(do (def (main (: b Bool)) \
                 (match (: ((. Value decode) ((. Value encode) ((. Ast Bool) b))) (Option Ast)) \
                   ((Some m) 1) ((None u) 0))) \
               (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Rust],
    );
    assert!(
        out.artifact(crate::backend::Target::Rust.artifact_kind())
            .is_none(),
        "a recursive-type value codec must DECLINE on rust (not emit rustc-hanging rust)"
    );
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("recursive-type value codec")),
        "the decline must name the recursive-type-codec constraint: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// MAX-FLAT-PARAMS boundary: an export whose params flatten to MORE than the canonical-ABI limit (16 core
/// values) needs the MEMORY-INDIRECT calling convention (params spilled to a linear-memory area, passed by
/// pointer), which this backend does not yet emit. Emitting the flat form regardless past 16 produced an
/// INVALID component — a SILENT bad artifact (a compile-success-only check reads it as green), the worst
/// failure mode. So >16 flat params must DECLINE, not ship garbage. Pins that 17 scalar params declines
/// (no wasm artifact) while 16 (at the boundary) still compiles. The Rust backend has no flat limit and
/// compiles both, so this is a wasm-emit guard only.
fn build_sum_export(n: usize) -> String {
    // `(do (def (main (: p0 Int64) … (: p{n-1} Int64)) (+ (+ … p0 p1) … p{n-1})) (export main))`
    let params: String = (0..n).map(|i| format!("(: p{i} Int64) ")).collect();
    let mut body = "p0".to_string();
    for i in 1..n {
        body = format!("(+ {body} p{i})");
    }
    format!("(do (def (main {params}) {body}) (export main))")
}

#[test]
fn seventeen_scalar_entry_params_over_max_flat_declines_not_invalid_wasm() {
    use crate::testkit::parse;
    let compile_wasm = |src: &str| {
        crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        )
    };
    // 17 flat params exceed MAX_FLAT_PARAMS (16): the backend must decline (no artifact), NOT emit invalid wasm.
    let over = compile_wasm(&build_sum_export(17));
    assert!(
        over.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "17 scalar entry params (over the max-flat boundary) must DECLINE, not emit an invalid component"
    );
    // 16 params — exactly at the boundary — still compiles to a component.
    let at = compile_wasm(&build_sum_export(16));
    assert!(
        at.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some(),
        "16 scalar entry params (at the max-flat boundary) must still compile"
    );
}

/// An `option<T>` ENTRY param crosses the plain-export boundary as a native component `option<T>` (flattened
/// to `(disc, payload)`), lifted by a guest WRAPPER that branches on the boundary disc and builds the guest
/// `Option` cell via `sum-new` (reusing the closure-arg `SumArgRebuild` via `emit_sum_field`), then drops the
/// built shell after the call (the def borrows it). `ty_natural_wit` declines a `Ty::Sum`, so the WIT type
/// comes from the Db-aware `spilled_result_wit_type`. Pins that the bare path emits a valid component for an
/// Option entry param (the behavioral value/None coverage is corpus `eo1-3`). Off-slice shapes (a non-option
/// sum, a compound result) still decline — no artifact.
#[test]
fn an_option_scalar_entry_param_compiles_to_a_component() {
    use crate::testkit::parse;
    let compile_wasm = |src: &str| {
        crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        )
    };
    // `(match o (Some v -> v) (None -> -1))` — an `Option Int64` param, matched. Compiles to a component.
    let src = "(do (def (main (: o (Option Int64))) \
                 (match o ((Option.Some v) v) ((Option.None) -1))) \
               (export main))";
    let out = compile_wasm(src);
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some(),
        "an option<Int64> entry param must compile to a component (the sum-lift wrapper)"
    );
}

/// A `result<scalar,scalar>` ENTRY param must DECLINE (no artifact), NOT emit an invalid component. The
/// option-shaped sum lift is admitted ONLY for a genuine `option<T>` WIT type; a `Result` resolves (via
/// `spilled_result_wit_type`) to a WIT `variant` whose flattening/disc convention disagrees with the
/// option-shaped `SumArgRebuild` lift — admitting it produced malformed wasm (a silent bad artifact). Until a
/// dedicated Result entry-param sub-slice, a Result param declines honestly. (Companion to the option test;
/// the Option path stays green.)
#[test]
fn a_result_scalar_entry_param_declines_cleanly_not_invalid_wasm() {
    use crate::testkit::parse;
    let src = "(do (def (main (: r (Result Int64 Int64))) \
                 (match r ((Result.Ok v) v) ((Result.Err e) (* e -1)))) \
               (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "a result<scalar,scalar> entry param must DECLINE, not emit an invalid component"
    );
}

/// A PARAMETERIZED Symbol-returning export declines — but with a TRUTHFUL message. `(main (: n Int64)) #"hot"`
/// has a scalar `Int64` param (fine) and a `Symbol` result; the runtime value-encode has no canonical Symbol
/// render (no `Shape::Sym` — a Symbol renders as a String), so a parameterized symbol return (which cannot use
/// the nullary constant-bake path that DOES produce `(Symbol.of …)`) declines. The prior diagnostic falsely
/// blamed the param ("a parameter with no scalar boundary type") though the `Int64` param is a scalar. Pins
/// that the message names the RESULT constraint, not the scalar param (the wrong-diagnostic fix).
#[test]
fn a_parameterized_symbol_return_declines_with_a_truthful_message_not_blaming_the_scalar_param() {
    use crate::testkit::parse;
    let src = "(do (def (main (: n Int64)) #\"hot\") (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "a parameterized Symbol return declines (no canonical runtime value-encode render yet)"
    );
    let msgs: Vec<&str> = out.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(
        !msgs
            .iter()
            .any(|m| m.contains("a parameter with no scalar boundary type")),
        "the decline must NOT falsely blame the scalar Int64 param: {msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.contains("parameterized export cannot return this heap type")),
        "the decline must truthfully name the Symbol RESULT as the constraint: {msgs:?}"
    );
}

/// VARIANT/ENUM in a derived param resolves to the guest's NAMED sum. A world export member's param type
/// carries a WIT `variant`/`enum` (here the `error` inside `on-response`'s `result<list<u8>, variant…>`),
/// which is declared ANONYMOUSLY in the world. A Cadenza sum carries a DECL identity, so the derived boundary
/// type references the guest's mirroring `type Err = | A | B` (matched by case-name set, kebab). Without this,
/// `wit_type_to_type_expr` returns `None` for the variant → the whole record type-expr fails → the param is
/// left bare → CDZ0201 (this is exactly the `on-response` gap in the echo reducer). Asserts no ambiguity.
#[test]
fn a_bare_export_param_with_a_variant_field_resolves_to_the_guest_named_sum() {
    use crate::db::Db;
    // `on-response`'s param carries `answer: result<list<u8>, variant(a, b)>`; the guest declares
    // `type Err = | A | B` mirroring the anonymous world variant. STRING-head compounds, NAME-head prims.
    let src = "(do \
                 (world reducer-world \
                   (export guest (member on-response \
                     (func (param resp (\"record\" \
                                         (contract (\"list\" (u8))) \
                                         (answer (\"result\" (\"list\" (u8)) (\"variant\" (a) (b)))))) \
                           (result (\"list\" (u8))))))) \
                 (type Err A B) \
                 (def (on-response resp) (. resp contract)) \
                 (export on-response))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "a variant field must resolve to the guest named sum (no CDZ0201): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// MEANINGFULNESS GUARD for the above: the SAME world + bare-param def WITHOUT the guest `type Err` decl has
/// no named sum to resolve the anonymous world variant against, so the derived record type-expr fails and the
/// param stays bare → CDZ0201. Proves the guest-named-sum matching (not something else) is what types the
/// variant-carrying param.
#[test]
fn a_variant_param_without_a_matching_guest_sum_stays_ambiguous() {
    use crate::db::Db;
    let src = "(do \
                 (world reducer-world \
                   (export guest (member on-response \
                     (func (param resp (\"record\" \
                                         (contract (\"list\" (u8))) \
                                         (answer (\"result\" (\"list\" (u8)) (\"variant\" (a) (b)))))) \
                           (result (\"list\" (u8))))))) \
                 (def (on-response resp) (. resp contract)) \
                 (export on-response))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "no matching guest sum → the variant-carrying param stays ambiguous (CDZ0201): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// MULTI-FILE guest (linked package, #3158): the guest's shared nominal sum lives in a LIBRARY file and is
/// IMPORTED — the derived export-param type must still resolve the world's anonymous variant to that
/// imported named sum. `lib` exports `type Err` concretely (`(. Err *)`); the entry imports it and its
/// `on-response` param carries `result<list<u8>, variant(a,b)>` with a BARE binder. `guest_sum_names` scans
/// the MERGED (spliced) arena's top-level items, so it finds the library's `(type Err (A)(B))` and derives
/// `(: resp (Record … (Result Bytes Err)))`; because the entry imported `Err`, the injected reference
/// resolves (no `unbound name Err`) and the param is not ambiguous (no CDZ0201). Pins that the export-param
/// derivation composes with cross-file linkage — the shared-types-in-a-library shape the checker guests use.
#[test]
fn a_bare_export_param_resolves_a_variant_via_an_imported_library_sum() {
    use crate::abi::Artifact;
    use crate::backend::Target;
    let lib = crate::codec::encode(&crate::testkit::parse(
        "(do (type Err (A) (B)) (def (dummy) 0) (export (. Err *) dummy))",
    ));
    // Entry: imports `Err`, in-source world declares on-response with a variant-carrying param, BARE binder.
    let app = crate::codec::encode(&crate::testkit::parse(
        "(do (import \"lib\" (Err)) \
           (world reducer-world \
             (export guest (member on-response \
               (func (param resp (\"record\" \
                                   (contract (\"list\" (u8))) \
                                   (answer (\"result\" (\"list\" (u8)) (\"variant\" (a) (b)))))) \
                     (result (\"list\" (u8))))))) \
           (def (on-response resp) (. resp contract)) \
           (export on-response))",
    ));
    let out = crate::compile::compile(
        &[
            Artifact::new(Artifact::KIND_AST, "lib", lib),
            Artifact::new(Artifact::KIND_AST, "app", app),
            cadenza_compile_abi::abi::entry_artifact("app"),
        ],
        &[Target::Wasm],
    );
    let msgs: Vec<&String> = out.diagnostics.iter().map(|d| &d.message).collect();
    assert!(
        !msgs
            .iter()
            .any(|m| m.contains("parameter type is ambiguous")),
        "a cross-file imported sum must let the variant param derive (no CDZ0201): {msgs:?}"
    );
    assert!(
        !msgs.iter().any(|m| m.contains("unbound name `Err`")),
        "the derived reference to the imported `Err` must resolve (no unbound): {msgs:?}"
    );
}

/// EXTERNAL-ARTIFACT world, no-redeclare: a reducer delivered with a `KIND_WIT_WORLD` artifact (NOT an
/// in-source `(world …)`) and NO hand-written `(effect …)` decl must still resolve its world-import perform
/// — `compile` injects the synthesized effects from the artifact BEFORE `Db::load`, so `(host (identity)
/// …)` binds. (The `identity.id` `list<u8>` RESULT still declines at the backend boundary — v-rb's slice —
/// so this asserts the FRONT-END resolution, i.e. NO `unbound name identity`, independent of emit.)
#[test]
fn an_external_artifact_world_import_resolves_without_a_hand_declared_effect() {
    let src = "(module m \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (identity) \
                     (list (record (= op (. e ct)) (= arg ((. identity id))))))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&crate::testkit::parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_nullary_identity_import_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    // The artifact-world effects were injected → `identity` binds (no CDZ0101), even though the guest wrote
    // no `(effect identity …)`. (A backend decline for the list<u8> host result is expected + orthogonal.)
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("unbound name `identity`")),
        "the external-artifact world's `identity` import must resolve via the injected effect (no CDZ0101): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// NO-ANNOTATION EXPORT BOUNDARY, EXTERNAL ARTIFACT world: the real flagship path — identity / provenance /
/// the §4 dispatch reducer target an external `KIND_WIT_WORLD` artifact (NOT an in-source `(world …)`), so the
/// artifact-side derivation must type a bare guest-export param too. A reducer with `(def (on-message msg) (.
/// msg payload))` and NO param annotation, compiled against an artifact world whose guest export `on-message`
/// declares `msg : record { contract: list<u8>, payload: list<u8> }`, must NOT report CDZ0201 "parameter type
/// is ambiguous": `derive_world_export_param_annotations_from_bytes` (run in `compile` before `Db::load`)
/// derives `msg`'s record type from the artifact member, so the body's `msg.payload` resolves. (A backend
/// boundary decline for the record export is orthogonal — the assertion is only that the param is no longer
/// ambiguous.)
#[test]
fn a_bare_export_param_is_typed_from_an_external_artifact_world_member() {
    use crate::ast::{Builder, Leaf};
    // KIND_WIT_WORLD artifact: export guest { on-message: func(msg: record{contract: list<u8>, payload:
    // list<u8>}) -> list<u8> }. STRING-head compound descriptors, NAME-head primitives — the exact shape
    // `parse_target_world` reads (same as an in-source world / v-platform's `world-artifact` output).
    let world_bytes = {
        let mut b = Builder::new();
        let byte_list = |b: &mut Builder| {
            let h = b.atom_leaf(Leaf::Str("list".into()));
            let uh = b.name("u8");
            let u = b.list(vec![uh]);
            b.list(vec![h, u])
        };
        let cfield = {
            let n = b.name("contract");
            let t = byte_list(&mut b);
            b.list(vec![n, t])
        };
        let pfield = {
            let n = b.name("payload");
            let t = byte_list(&mut b);
            b.list(vec![n, t])
        };
        let rec_h = b.atom_leaf(Leaf::Str("record".into()));
        let msg_rec = b.list(vec![rec_h, cfield, pfield]);
        let func = {
            let func_h = b.name("func");
            let param_h = b.name("param");
            let pn = b.name("msg");
            let pnode = b.list(vec![param_h, pn, msg_rec]);
            let result_h = b.name("result");
            let rout = byte_list(&mut b);
            let rnode = b.list(vec![result_h, rout]);
            b.list(vec![func_h, pnode, rnode])
        };
        let member = {
            let member_h = b.name("member");
            let mn = b.name("on-message");
            b.list(vec![member_h, mn, func])
        };
        let guest = {
            let exp_h = b.name("export");
            let en = b.name("guest");
            b.list(vec![exp_h, en, member])
        };
        let world_h = b.name("world");
        let wn = b.name("reducer-world");
        let world = b.list(vec![world_h, wn, guest]);
        let a = b.finish(world);
        crate::codec::encode(&a)
    };
    // Guest: bare param `msg` (NO annotation), body reads `msg.payload` (resolves only if `msg` is typed).
    let src = "(module m (def (on-message msg) (. msg payload)) (export on-message))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&crate::testkit::parse(src)),
            ),
            crate::abi::Artifact::new(crate::link::KIND_WIT_WORLD, "wit-world", world_bytes),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("parameter type is ambiguous")),
        "an external-artifact world's guest-export member must type the bare `msg` param (no CDZ0201): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// A MALFORMED in-source `(world …)` descriptor is REPORTED, not silently dropped. Here the `deliver.send`
/// result is written `("bool")` (a STRING head) — but `bool` is a NAME-head primitive `(bool)`; the wrong
/// head makes `parse_target_world` return `None`, which would silently drop the whole world and leave every
/// import unbound (a misleading `unbound name deliver` cascade with no root cause). `collect_faults` now
/// surfaces the malformed world instead. (A world author hand-writing descriptors WILL hit this.)
#[test]
fn a_malformed_in_source_world_descriptor_is_reported_not_silently_dropped() {
    use crate::db::Db;
    let src = "(module m \
                 (world reducer-world \
                   (import deliver (member send (func (param x (\"list\" (u8))) (result (\"bool\")))))) \
                 (def (apply (: e Bytes)) (host (deliver) ((. deliver send) e))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    assert!(
        diags.iter().any(|d| d
            .message
            .contains("did not parse as a valid world descriptor")),
        "a malformed (world …) descriptor must be reported (not a silent drop + unbound cascade): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// MULTI-INTERFACE no-redeclare: the platform reducer-world imports SEVERAL interfaces (state / blobs /
/// identity / run). A reducer that declares such a world in source and performs imports from MORE THAN ONE
/// of them — with NO hand-written `(effect …)` decls — must have the load-time sidecar synthesize ONE effect
/// per import interface, so every perform resolves + lowers to a synchronous `Core::HostCall`. Pins that
/// `inject_world_import_effects` handles a multi-import world (not just the single-interface case), across
/// mixed shapes: a nullary `list<u8>` result (identity.id) and a single-arg `option<list<u8>>` result
/// (state.get → Option Bytes).
#[test]
fn a_multi_interface_no_redeclare_world_synthesizes_an_effect_per_import() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // In-source world importing TWO interfaces; WIT descriptors are STRING-head canonical forms.
    let src = "(module m \
                 (world reducer \
                   (import identity (member id (func (result (\"list\" (u8)))))) \
                   (import state (member get (func (param key (\"list\" (u8))) \
                     (result (\"option\" (\"list\" (u8)))))))) \
                 (def (fetch-id) (host (identity) ((. identity id)))) \
                 (def (lookup (: k Bytes)) (host (state) ((. state get) k))) \
                 (def (main) 0) \
                 (export main))";
    let mut db = Db::load(crate::testkit::parse(src));
    // Neither un-declared interface name is unbound — both injected effects bound them.
    let diags = crate::compile::diagnostics(&mut db);
    for name in ["identity", "state"] {
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains(&format!("unbound name `{name}`"))),
            "the injected effect must bind `{name}` (no CDZ0101): {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
    // fetch-id → HostCall{identity, id, [], Bytes}.
    let fetch = db.def_by_name("fetch-id").expect("def fetch-id");
    let fetch_body = db.defs[fetch].body.expect("fetch-id body");
    match core_of(&mut db, fetch_body) {
        Core::HostCall {
            effect, op, args, ..
        } => {
            assert_eq!((&*effect, &*op, args.len()), ("identity", "id", 0));
        }
        other => panic!("identity.id must lower to a HostCall, got {other:?}"),
    }
    // lookup → HostCall{state, get, [k], Option Bytes}.
    let lookup = db.def_by_name("lookup").expect("def lookup");
    let lookup_body = db.defs[lookup].body.expect("lookup body");
    match core_of(&mut db, lookup_body) {
        Core::HostCall {
            effect, op, args, ..
        } => {
            assert_eq!((&*effect, &*op, args.len()), ("state", "get", 1));
        }
        other => panic!("state.get must lower to a HostCall, got {other:?}"),
    }
}

/// The world-import sidecar must NOT duplicate a HAND-DECLARED effect. A guest that declares its world in
/// source AND writes its own `(effect identity …)` — the REDECLARE path, or an interface carrying an
/// `enum`/`variant` type it must declare in full — would otherwise get TWO `(effect identity …)` decls (the
/// hand-written one + the synthesized one) → a duplicate-definition CDZ0201 + poisoned resolution. The
/// sidecar skips any interface the guest already declares, so the hand-decl wins and the perform still
/// lowers cleanly to a `Core::HostCall`. (Guards a latent regression in the #3101 pre-pass.)
#[test]
fn the_world_import_sidecar_does_not_duplicate_a_hand_declared_effect() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // The guest hand-declares a RICHER `identity` effect — an extra op `probe` the world does NOT import —
    // and performs it. A synthesized partial `(effect identity (op id …))` (only the imported member) must
    // NOT shadow the hand-decl, else `identity.probe` would go unbound (CDZ0101).
    let src = "(module m \
                 (world reducer (import identity (member id (func (result (\"list\" (u8))))))) \
                 (effect identity (op id (-> Bytes)) (op probe (-> Bytes Unit))) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (identity) (do ((. identity id)) ((. identity probe) (. e pl))))) \
                 (export apply))";
    let mut db = Db::load(crate::testkit::parse(src));
    let diags = crate::compile::diagnostics(&mut db);
    // The hand-declared effect's ops (incl. `probe`, which the world does NOT import) all resolve — the
    // sidecar skipped synthesizing a partial `identity` that would have shadowed the richer hand-decl.
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("unbound name `probe`")
                || (d.message.contains("probe") && d.message.contains("operation"))),
        "the guest's hand-declared `identity.probe` must resolve (sidecar must not shadow it with a \
         partial synthesized effect): {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
    // And the perform still lowers to a HostCall (resolution is clean).
    let d = db.def_by_name("apply").expect("def apply");
    let body = db.defs[d].body.expect("apply body");
    assert!(
        !matches!(core_of(&mut db, body), Core::Poison(_)),
        "the reducer body lowers cleanly (not poisoned) with the hand-declared effect"
    );
}

/// §3c full-A FOLLOW-ON — the top-level `(world …)` DECLARATION compile arm: a target world declared
/// IN SOURCE (the inline-world surface's lowered node, a top-level module item) must drive the SAME
/// bytes-provider emit as the external KIND_WIT_WORLD artifact — the cross-source identity guarantee (a
/// target world means one emit whether it comes from an in-source decl, an external binary-AST artifact,
/// or v-cml's own emit; v-syntax + v-agent-harness rulings 2026-08-12). Before this arm, a top-level
/// `(world …)` form DECLINED with "unbound name `world` at the top level" (the scan recognized only
/// def/type/effect/module/import). Now `world` is recognized-and-excluded (`TOP_LEVEL_FORMS`, the
/// `module-doc` recognize-register-nothing precedent) and `compile` codec-encodes its subtree into
/// `db.wit_world` when no artifact overrides. This gate compiles ONE reducer body two ways — via the
/// artifact and via an in-source decl — and asserts BYTE-IDENTICAL wasm: the emit reads the DECODED
/// `TargetWorld` (`parse_target_world`), so both paths that decode to the same world emit the same
/// component. The compound type ctors use QUOTED-string heads (`("list" (u8))`) — the head-kind-fixed
/// descriptor form `parse_wit_type` reads (a Str head is a compound, a Name head is a primitive).
#[test]
fn an_in_source_world_decl_drives_the_bytes_provider_emit_identically_to_the_artifact() {
    use crate::testkit::parse;
    // The reducer body is IDENTICAL in both compiles; the sole difference is where the target world comes
    // from. A pure fold reading the Event's fields into a one-element effect-list — the first full-A shape.
    let reducer_body = "(def (apply (: e (Record (ct String) (pl Bytes)))) \
                          (list (record (= op (. e ct)) (= arg (. e pl))))) \
                        (export apply)";

    // (A) ARTIFACT PATH (the §3b ingest): the reducer source alone + the world as an external artifact.
    let artifact_src = format!("(module m {reducer_body})");
    let via_artifact = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(&artifact_src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_apply_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );

    // (B) IN-SOURCE PATH: the SAME reducer plus a top-level `(world …)` decl, NO artifact supplied. The
    // world declares the same `fold.apply : (input: list<u8>) -> list<u8>` boundary as the artifact.
    let world_decl = "(world reducer (export fold (member apply \
                       (func (param input (\"list\" (u8))) (result (\"list\" (u8)))))))";
    let inline_src = format!("(module m {world_decl} {reducer_body})");
    let via_inline = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(&inline_src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
        ],
        &[crate::backend::Target::Wasm],
    );

    // The in-source path must NOT decline (the pre-arm behavior was an "unbound name `world`" reject) —
    // the top-level `world` decl is recognized-and-excluded, then read into `db.wit_world`.
    assert!(
        !via_inline.has_error(),
        "an in-source (world …) decl must compile with no unbound-name decline: {:?}",
        via_inline
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let inline_wasm = via_inline
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the in-source-world reducer emits a bytes-roundtrip provider component");
    let artifact_wasm = via_artifact
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the artifact-world reducer emits a bytes-roundtrip provider component");
    // THE cross-source identity: the same reducer + the same world (declared vs supplied) emits the SAME
    // component — the emit is a function of the DECODED world, not of where the world bytes originated.
    assert_eq!(
        inline_wasm, artifact_wasm,
        "a target world drives the SAME emit whether declared in-source or supplied as an artifact"
    );
    // …and the in-source-world component is well-formed (the wasmparser structural guard).
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(inline_wasm)
        .expect("the in-source-world provider component validates");
}

/// DECLINE-DON'T-MISCOMPILE for the in-source world arm: a module with TWO top-level `(world …)`
/// declarations targets no single world, so `compile` DECLINES (naming the count) rather than silently
/// encoding the first and dropping the second — a reducer targets at most one world. Pins the guard in
/// `compile` (`db.top_world_forms().len() > 1`).
#[test]
fn a_module_with_two_top_level_world_decls_declines_no_miscompile() {
    use crate::testkit::parse;
    let two_worlds = "(module m \
                        (world reducer (export fold (member apply \
                          (func (param input (\"list\" (u8))) (result (\"list\" (u8))))))) \
                        (world other (export fold (member apply \
                          (func (param input (\"list\" (u8))) (result (\"list\" (u8))))))) \
                        (def (apply (: e (Record (ct String) (pl Bytes)))) \
                          (list (record (= op (. e ct)) (= arg (. e pl))))) \
                        (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(two_worlds)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
        ],
        &[crate::backend::Target::Wasm],
    );
    // It must DECLINE (an error diagnostic, no wasm artifact) — not silently compile the first world.
    assert!(
        out.has_error(),
        "two top-level (world …) decls must decline, not silently pick the first"
    );
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("at most one world")),
        "the decline names the at-most-one-world rule: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "a declined multi-world module emits no wasm artifact"
    );
}

/// §3c full-A DECLINE-DON'T-MISCOMPILE: a member named as a `list<u8>` bytes boundary whose PARAM is a
/// bare scalar (not a value-form compound the runtime value-decode walker can reconstruct) must DECLINE
/// cleanly — no wasm artifact, a diagnostic naming the reason — rather than mis-emit a value-decode of a
/// shape that has no descriptor. Pins the param-side guard in `emit_bytes_provider_member`
/// (`sum_shape_descriptor` on the param returns `None`), the twin of the result-side guard. The RESULT is
/// a compound (a `List<Record>`), so the emit reaches the param check before declining — this isolates the
/// param descriptor, not the earlier compound-result / exactly-one-param guards.
#[test]
fn a_bytes_member_with_a_scalar_param_declines_cleanly_no_miscompile() {
    use crate::testkit::parse;
    // `apply` takes a scalar Int64 (no value-form shape) and returns a compound effect-list.
    let src = "(module m \
                 (def (apply (: n Int64)) (list (record (= op \"x\") (= arg n)))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_apply_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    // No mis-emit: the bytes path declines rather than shipping a component.
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_none(),
        "a scalar-param bytes member must NOT emit a component (decline-don't-miscompile)"
    );
    // …and the decline names WHY (the param has no value-form shape descriptor).
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("no value-form shape descriptor")),
        "the decline names the missing param shape descriptor: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// §3c full-A BEHAVIORAL (the SUM result shape): a pure reducer whose effect-list result is a
/// `List<Effect>` where `Effect` is a SUM (variant) — the realistic reducer shape, an effect-list of
/// heterogeneous effect requests — emits through `emit_bytes_provider_member` and LOADS on the pinned
/// wasmtime. Distinct from the record-result sibling: the result-side `sum_shape_descriptor` builds the
/// `List(sum)` shape and the value-encode walker renders per-element sum-disc + payload, exercising the
/// sum-render path through the bytes-provider marshal (not just records). Pins that a variant effect-list
/// result crosses the `list<u8>` boundary as a well-formed, loadable component.
#[test]
fn a_reducer_with_a_variant_effect_list_result_emits_a_valid_provider() {
    use crate::testkit::parse;
    // Effect is a sum with a String arm and a Bytes arm; the fold returns a two-element effect-list.
    let src = "(module m \
                 (type Effect (Log String) (Emit Bytes)) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (list (Effect.Log (. e ct)) (Effect.Emit (. e pl)))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_apply_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "a variant-effect-list reducer bytes-roundtrip emit must not decline: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the variant-result reducer emits a bytes-roundtrip provider component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes)
        .expect("the variant-result bytes-roundtrip provider component validates");
    assert!(
        imports_value_heap_runtime(bytes),
        "a variant-effect-list reducer marshals through the value-heap runtime import"
    );
}

/// SCHEMA-HASH phase-1a TYPING CONTRACT (v-inference↔v-rust-backend co-land): a REIFIED async world-effect
/// perform types as the effect-request RECORD `{correlation:Option Bytes, kind:String, payload:Option
/// Bytes}`, so a reducer returning `(list (Model.request e))` type-checks against a `List(Record(...))`
/// effect-list. The discriminator is REIFY-WHEN-`!is_world_import_op` under a host home: the world imports
/// ONLY `kv`, so `kv.*` (a world import) stays SYNCHRONOUS (op-sig result, consumed inline — the host-fused
/// kv reducers), while `Model.request` (NOT a world import, performed under its `(host (Model))` home)
/// reifies to the record. This asserts the reify perform type-checks in effect-list position with NO
/// CDZ0203 (the record unifies with the returned list) and NO CDZ0401 (the host block is its home).
/// Diagnostics-only: the single-`Bytes`-arg case (payload `Option Bytes`); structured/multi-arg rides R2.
#[test]
fn a_reified_async_world_effect_perform_types_as_the_request_record_in_effect_list_position() {
    // The world imports ONLY kv (so Model is NOT a world-import → reifies). `apply` performs Model.request
    // under its (host (Model)) home and returns it in the one-element effect-list; the declared fold result
    // is list<u8> (the bytes boundary), and the internal effect-list element is the reified record.
    use crate::testkit::parse;
    // The WIT-world artifact imports ONLY `cadenza:agent-kernel/kv` — so `Model` is NOT a world import and
    // its perform reifies. `apply` performs `Model.request` under its `(host (Model))` home (which gives it
    // a CDZ0401 home) and returns it in the one-element effect-list. Uses the ARTIFACT world (not an inline
    // `(world …)` decl) so `db.wit_world` carries the real FQ import set `is_world_import_op` reads.
    let src = "(module m \
                 (effect Model (op request (-> Bytes Unit))) \
                 (def (apply (: e Bytes)) (host (Model) (list (Model.request e)))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_kv_put_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0203") && d.code.as_deref() != Some("CDZ0401")),
        "a reified async world-effect perform must type as the request record in effect-list position \
         (no CDZ0203 list-element mismatch, no CDZ0401 no-home): {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// SCHEMA_DESCRIPTOR field presence (v-inference typing half of the phase-3 producer-bake co-land, landed
/// `30af6769b`): a reified world-effect whose op has a buildable schema descriptor (`(op beat (-> Bytes
/// Unit))` — a typed op) reifies to a record that INCLUDES `schema_descriptor: Bytes` (name-sorted after
/// `payload`). `world_effect_request_ty` gains that field iff `lower::effect_has_schema_descriptor` (the
/// SAME predicate the reify emit gates on). This asserts the typed shape MATCHES the 4-field emitted shape
/// via the type system: `apply`'s effect-list is annotated `List(Record{correlation,kind,payload,
/// schema_descriptor})` and must type CLEAN — if the typing produced only the 3-field record, the
/// annotation would mismatch (CDZ0203). Guards the bug case-32 hit (emit 4-field, type 3-field → field
/// dropped → schema_hash None). Target-free single-Bytes-arg (Beat.beat, like the platform case-32 pin).
#[test]
fn a_reified_descriptor_bearing_effect_types_with_the_schema_descriptor_field() {
    use crate::testkit::parse;
    // Beat is NOT a world import (the artifact imports only kv) → Beat.beat reifies. Its op `(-> Bytes
    // Unit)` builds a schema descriptor, so the reified record is 4-field incl. schema_descriptor. The
    // apply effect-list is annotated with exactly that 4-field record — types clean iff the field is typed.
    let src = "(module m \
                 (effect Beat (op beat (-> Bytes Unit))) \
                 (def (apply (: e Bytes)) \
                   (: (host (Beat) (list (Beat.beat e))) \
                      (List (Record (: correlation (Option Bytes)) (: kind String) \
                                    (: payload (Option Bytes)) (: schema_descriptor Bytes))))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:reducer/api"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_kv_put_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.diagnostics
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0203")),
        "a descriptor-bearing effect's reified record must TYPE with the schema_descriptor field (the \
         4-field annotation must match the typed shape — no CDZ0203): {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// R2 (in-fold `Value.encode`/`decode` — operator-ruled binary-AST canonical encoder; the v-inference
/// SURFACE + typing half, co-landed with v-rust-backend's `Core::ValueEncode`/`ValueDecode` prims + emit).
/// `Value.encode : ∀a. a → Bytes` (TOTAL) and `Value.decode : ∀a. Bytes → Option a` (PARTIAL, `a` grounded
/// by the call-site expected type). Asserts: encoding a STRUCTURED record types clean (`a` = the record,
/// result Bytes — the whole point, unblocking Model.request's structured payload), and decoding into an
/// annotated `Option Record` types clean (the annotation grounds `a`). No CDZ0203/CDZ0101.
#[test]
fn value_encode_decode_type_a_structured_record_round_trip_surface() {
    use crate::testkit::parse;
    let src = "(module m \
                 (def (enc (: r (Record (a Int64) (b Int64)))) (Value.encode r)) \
                 (def (dec (: bs Bytes)) (: (Value.decode bs) (Option (Record (a Int64) (b Int64))))) \
                 (export enc))";
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
        out.diagnostics
            .iter()
            .all(|d| d.code.as_deref() != Some("CDZ0203") && d.code.as_deref() != Some("CDZ0101")),
        "Value.encode of a structured record + Value.decode into an annotated Option Record must type \
         clean (encode ∀a.a→Bytes total, decode ∀a.Bytes→Option a grounded by the annotation): {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// R2 DECODE GROUNDING (the v-inference typing fix v-rust-backend's backend surfaced): `Value.decode`'s
/// result `a` is unconstrained by its `Bytes` argument, so the decode APPLICATION node types `(Option ?a)`
/// bottom-up. An enclosing `(: (Value.decode bs) (Option Pt))` annotation must ground `?a := Pt` AT the
/// decode node's own `type_of` — because `lower_value_decode` reads `type_of(decode-node)` to build the
/// shape descriptor, and a free `?a` there declines "target unsolved". The fix climbs from the decode node
/// to the enclosing annotation (mirroring `literal_binop_context_ty`'s deferred-width climb). This asserts
/// the DECODE application node's `type_of` is the GROUND `(Option Pt)`, not `(Option ?)` — the precise thing
/// the lower reads.
#[test]
fn value_decode_grounds_its_target_from_the_enclosing_annotation() {
    use crate::db::Db;
    use crate::testkit::parse;
    let src = "(module m (type Pt (Mk Int64)) \
                 (def (dec (: bs Bytes)) (: (Value.decode bs) (Option Pt))) (export dec))";
    let mut db = Db::load(parse(src));
    let decode_ty = (0..db.ast.structure.len() as u32)
        .map(crate::ast::StructId)
        .find_map(|id| {
            matches!(crate::resolve::resolved_of(&mut db, id), crate::resolved::Resolved::Apply { head, .. }
                if crate::eval::meta_apply_of(&mut db, head) == Some(crate::resolved::Prim::ValueDecode))
                .then(|| crate::infer::type_of(&mut db, id))
        })
        .expect("a Value.decode application node");
    match &decode_ty {
        crate::ty::Ty::Sum { args, .. } => {
            let target = args.first().expect("Option carries its element arg");
            assert!(
                !matches!(target, crate::ty::Ty::Var(_)),
                "Value.decode's target must be grounded from the (Option Pt) annotation, not left a free \
                 Var (lower reads this node's type to build the descriptor): got {}",
                decode_ty.render_name(&db.name_ctx())
            );
        }
        other => panic!("Value.decode node must type as (Option _), got {other:?}"),
    }
}

/// R2 DECODE GROUNDING through a TYPED LET-BINDER (`annotation_context_ty`'s `let_binder_annotation_ty`
/// extension): the idiomatic `(let (((: p (Option Pt)) (Value.decode bs))) …)` names the target type on the
/// BINDER, not by wrapping the decode in a direct `(: … (Option Pt))`. The author annotated `p` either way,
/// so the binder form must ground the decode node's target `Pt` IDENTICALLY to the direct-annotation form —
/// otherwise the exact same annotated decode declines at lower ("no value-form descriptor") purely because
/// the annotation sits on the binder rather than around the application. Asserts the decode application node's
/// `type_of` is the GROUND `(Option Pt)`, not `(Option ?)`. Twin of
/// `value_decode_grounds_its_target_from_the_enclosing_annotation` (the direct-annotation form).
#[test]
fn value_decode_grounds_its_target_from_a_typed_let_binder() {
    use crate::db::Db;
    use crate::testkit::parse;
    let src = "(module m (type Pt (Mk Int64)) \
                 (def (dec (: bs Bytes)) (let (((: p (Option Pt)) (Value.decode bs))) p)) (export dec))";
    let mut db = Db::load(parse(src));
    let decode_ty = (0..db.ast.structure.len() as u32)
        .map(crate::ast::StructId)
        .find_map(|id| {
            matches!(crate::resolve::resolved_of(&mut db, id), crate::resolved::Resolved::Apply { head, .. }
                if crate::eval::meta_apply_of(&mut db, head) == Some(crate::resolved::Prim::ValueDecode))
                .then(|| crate::infer::type_of(&mut db, id))
        })
        .expect("a Value.decode application node");
    match &decode_ty {
        crate::ty::Ty::Sum { args, .. } => {
            let target = args.first().expect("Option carries its element arg");
            assert!(
                !matches!(target, crate::ty::Ty::Var(_)),
                "Value.decode's target must be grounded from the typed let-binder `(: p (Option Pt))`, not \
                 left a free Var — a binder annotation grounds identically to a direct one: got {}",
                decode_ty.render_name(&db.name_ctx())
            );
        }
        other => panic!("Value.decode node must type as (Option _), got {other:?}"),
    }
}

/// R2 DECODE GROUNDING — the NEGATIVE half of the contract (the `annotation_context_ty` fix's comment states
/// it, but nothing pinned it): an UNANNOTATED `Value.decode` has NO enclosing annotation to ground its target
/// `a`, so the decode node's `type_of` stays `(Option ?a)` with `?a` free. Typing does NOT hang or fault — a
/// `Bytes -> (Option ?a)` is well-formed, so `check` passes. The honesty is enforced at LOWER:
/// `lower_value_decode` peels `a`, finds no value-form descriptor for a free var, and DECLINES cleanly ("no
/// binary-AST value-form descriptor") — NOT a panic, NOT invalid wasm, NOT a silently-wrong descriptor. This
/// pins that a decode you can't ground is a clean compile-time decline, so the grounding path can never be
/// "relaxed" into emitting a bogus descriptor for an unsolved target. Complements
/// `value_decode_grounds_its_target_from_the_enclosing_annotation` (the positive half).
#[test]
fn an_unannotated_value_decode_declines_at_lower_not_a_panic_or_invalid_wasm() {
    use crate::testkit::parse;
    // No `(: ... (Option T))` around the decode — nothing grounds `a`, so it stays a free Var.
    let src = "(module m (def (dec (: bs Bytes)) (Value.decode bs)) (export dec))";
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
        out.has_error(),
        "an unannotated Value.decode (ungrounded target) must DECLINE at lower, not compile a bogus \
         descriptor for a free var: {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
    assert!(
        out.diagnostics.iter().any(
            |d| d.message.contains("target type is unsolved") && d.message.contains("annotate")
        ),
        "the decline must be the ACTIONABLE unsolved-target message (a free-var target is undetermined, so \
         the fix is to annotate — NOT the generic 'no value-form descriptor' which misreads it as an \
         unsupported concrete type): {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// R2 DECODE DIAGNOSTIC — a bare `Value.decode` as a MATCH SCRUTINEE whose target is only implied by the arm
/// patterns (`(match (Value.decode bs) ((Some (Pt.Mk r)) …))`) is STILL an ungrounded free-var target: the
/// arm patterns do not thread their type back into the decode node's `type_of` (same root as the unannotated
/// case). It must decline with the ACTIONABLE unsolved-target message (fix: annotate the scrutinee), NOT the
/// generic "no value-form descriptor" that would misread the undetermined target as an unsupported concrete
/// type. Pins the diagnostic split (`has_free_var` in `lower_value_decode`) for the match-scrutinee shape a
/// user is most likely to hit.
#[test]
fn a_bare_value_decode_match_scrutinee_declines_with_the_actionable_unsolved_target_message() {
    use crate::testkit::parse;
    // Scrutinee is a BARE decode; the target Pt is only implied by the `(Some (Pt.Mk r))` arm pattern, which
    // does not ground the decode node — so its target stays a free Var.
    let src = "(module m (type Pt (Mk (Record (: x Int64) (: y Int64)))) \
                 (def (dec (: bs Bytes)) \
                   (match (Value.decode bs) \
                     ((Some p) (match p ((Pt.Mk r) (+ (. r x) (. r y))))) ((None) -1))) \
                 (export dec))";
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
        out.has_error(),
        "a bare Value.decode match scrutinee (target only implied by arm patterns) must DECLINE — the arms \
         do not ground the decode node: {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
    assert!(
        out.diagnostics.iter().any(
            |d| d.message.contains("target type is unsolved") && d.message.contains("annotate")
        ),
        "the decline must be the ACTIONABLE unsolved-target message directing the user to annotate the \
         scrutinee, not the generic no-descriptor message: {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// R2 ENCODE — a scalar-erased newtype over a WIDER-than-i32 scalar now COMPILES via the boxing increment
/// (was a DECLINE; the value-model boxing slice, DESIGN-compiler-primitives-adjacent). `Value.encode` walks
/// an i32 HEAP HANDLE; a scalar-erased newtype `(type W (Mk Int64))` erases to a bare `i64`, NOT a handle.
/// The emit no longer declines it: it BOXES the erased scalar into a leaf handle (`box-int`, with the
/// narrow-int i32→i64 extend for narrower widths) before `value-encode`, whose descriptor is the
/// `Named(W, Int)` frame — so the canonical form is the elided-head `(: 7 W)`. This closes the encode hole
/// PROPERLY (the earlier `valtype_of != I32` guard just declined to avoid the live-origin i64-into-i32-slot
/// invalid-wasm bug; boxing is the real fix). Pins that the scalar-erased single-ctor now emits a VALID
/// component instead of declining. The e2e round-trip (`Value.decode (Value.encode (FireAfter n)) == Some`)
/// is pinned by the `spec/semantics/22` corpus case; here we pin the compile-succeeds-not-declines flip.
/// (A NULLARY-rep newtype `(type Ack (Ack))` — Unit rep — still declines; that box-the-unit-leaf sub-slice
/// is a follow-up.)
#[test]
fn value_encode_on_a_wide_scalar_erased_newtype_boxes_and_compiles() {
    use crate::testkit::parse;
    let src = "(module m (type W (Mk Int64)) (def (main) (Value.encode (W.Mk 7))) (export main))";
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
        "Value.encode of a scalar-erased newtype over Int64 must now COMPILE via the boxing increment \
         (box the erased scalar to a leaf handle before value-encode), not decline: {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
}

/// RULING A (2026-08-14, resolving v-effects' freeze-blocking reify/2b crux): a TARGET-HAVING world-effect
/// — one whose op carries an `@resource` marker (`Emit.send(@resource dest, payload)`) — reifies the dest to
/// its OWN `target: Bytes` wire field (the dest is a runtime value SEC-F1 authorizes against a resource
/// predicate, so it rides the wire, NOT dropped and NOT value-encoded with the payload). The perform must
/// type as the 4-field record `{correlation, kind, payload, target}` (typing gains `target` iff the op has a
/// resource marker) AND emit a valid component. Contrast the target-FREE sibling above (`Model.request`, no
/// `@resource`) which stays 3-field. `Emit` is NOT a world import (world imports only `kv`), so it reifies;
/// `(resource 0)` marks arg0 (dest) as the SEC-F1 resource → routed to `target`, payload = arg1.
#[test]
fn a_target_having_resource_effect_reifies_the_dest_to_a_target_field_and_emits() {
    use crate::testkit::parse;
    // Emit.send(@resource dest, payload): a 2-arg perform whose op declares `(resource 0)` (arg0=dest is the
    // SEC-F1 resource). Both args Bytes. Reifies to {correlation, kind="effect/Emit", payload=Some(arg1),
    // target=arg0}. The world imports ONLY kv, so Emit reifies (is_world_import_op=false).
    let src = "(module m \
                 (effect Emit (op send (-> Bytes Bytes Unit) (resource 0))) \
                 (def (apply (: e (Record (dest Bytes) (pl Bytes)))) \
                   (host (Emit) (list (Emit.send (. e dest) (. e pl))))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_kv_get_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "a target-having @resource effect must reify (dest→target field) + emit (typing 4-field record \
         agrees with reify): {:?}",
        out.diagnostics
            .iter()
            .map(|d| (&d.code, &d.message))
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the target-having reify reducer emits a component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes).expect(
        "the target-having reify reducer's component validates (4-field record value-encodes)",
    );
}

// NOTE: the SYNCHRONOUS-world-import sibling (a `kv.*` perform keeps its op-sig result, NOT the request
// record) is pinned by the existing host-fused kv reducer tests (`a_host_fused_kv_get/put/delete_*`), which
// use the real KIND_WIT_WORLD artifact importing `cadenza:agent-kernel/kv` and match on the op result — so
// a polarity regression (reifying a world-import) breaks them (the 7-test regression that caught the
// original inverted ruling). An inline `(world … (import kv …))` decl is NOT a reliable witness here (its
// import interface does not reach `is_world_import_op` the way the artifact does — a separate inline-world
// import-parse question), so the sync side is covered by the artifact-based kv tests, not a duplicate here.

/// SCHEMA-HASH PHASE-1a REIFY end-to-end (the 3-lane co-gate: v-inference typing + this reify emit +
/// v-effects kernel-parse, all now on trunk): a reducer that PERFORMS a target-free single-Bytes-arg
/// WORLD-effect must reify the perform into its returned effect-list + emit a VALID, LOADABLE component —
/// proving the typing (the perform types as the 3-field record), `reify_effect_to_tuple` (emits it), and
/// value-encode all agree end-to-end. `Ping` is NOT a member of the target world's imports (which is `kv`),
/// so `Ping.ping` is a world-effect (is_world_import_op = false) → the fork REIFIES it (vs a synchronous
/// kv `Core::HostCall`). The reified 3-field record `{correlation, kind, payload}` flows into the returned
/// `list`, which value-encodes to the `list<u8>` fold-boundary result. Contrast the kv reducers above: kv
/// IS an import → its ops stay synchronous, consumed inline. This is the emit-roundtrip half v-inference
/// asked v-rust-backend to lead (they landed + typing-tested the typing half, `9b12c0684`).
#[test]
fn a_reducer_performing_a_world_effect_reifies_it_and_emits_a_valid_component() {
    use crate::testkit::parse;
    // `Ping` is a target-free single-Bytes-arg world-effect (ping: Bytes -> Unit) — NOT a world import, so
    // `Ping.ping e` reifies. Delegated at the entrypoint via `(host (Ping) …)` (its CDZ0401 home). The body
    // returns the perform in a `list` — the effect-list. (kv stays the sole import in the world artifact;
    // Ping's non-membership is what routes it to reify.)
    let src = "(module m \
                 (effect Ping (op ping (-> Bytes Unit))) \
                 (def (apply (: e (Record (ct String) (pl Bytes)))) \
                   (host (Ping) (list (Ping.ping (. e pl))))) \
                 (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_kv_get_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "a reducer performing a world-effect must reify it + emit (typing + reify + value-encode agree): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the reify reducer emits a component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes).expect(
        "the reify reducer's component validates (the reified effect-list value-encodes cleanly)",
    );
}

/// A COMPOSED call over a record-transforming function whose field is a CHECKED-ARITH op compiles to a
/// VALID module and runs to the right value — the regression guard for the record-field slot-collision
/// miscompile (`adv-nested-call-multifield-record-arith-field-invalid-wasm`). `f` takes a ≥2-field
/// record and rebuilds it with field `a` bumped by a checked `+` (which stashes an i64 into a scratch
/// slot behind an overflow guard) and `b` copied. Applied twice — `f(f({a:0,b:5}))` — the inner call's
/// record RESULT feeds the outer call's ARGUMENT, and the `Core::Record` arm used to emit every field at
/// a FIXED `base`, so field `a`'s i64 arith scratch reused a slot the outer record-assembler had already
/// typed i32 → one wasm local declared at two widths → `wasm-tools validate: expected i64, found i32`,
/// the component rejected at load. The fix advances each field's scratch base past the running
/// high-water (the same disjoint-slot discipline `Core::Tuple`/`Core::ListNew` already applied), so
/// sibling/nested fields never alias a slot at two widths. `f(f({a:0,b:5}))` → `{a:2, b:5}`, `.a` = 2.
#[test]
fn a_composed_call_over_a_record_with_a_checked_arith_field_runs() {
    use crate::testkit::parse;
    // The fix must at minimum produce a VALID module — the miscompile was a wasm-validation failure
    // (`expected i64, found i32`), so `compile_component` returning `Ok` + the wasmparser validate below is
    // the primary guard, and `imports_value_heap_runtime` confirms the record-transform reaches the heap.
    // The composed run below then confirms the VALUE when the runtime store is present.
    // Field `a` is seeded from a RUNTIME parameter so the record cannot const-fold away — the general
    // const-evaluator now folds a record threaded through a record-param fn (`(f (f (record …)))` over an
    // all-constant record reduces to the projected scalar, no heap), which is the correct behavior but would
    // sidestep the runtime record-assembler this test guards. The runtime seed keeps the composed build on
    // the heap path where the slot-collision miscompile lived. `f(f({a:seed,b:5}))` → `{a:seed+2, b:5}`;
    // with seed = 0, `.a` = 2.
    let src = "(module m \
                 (def (f (: r (Record (a Int64) (b Int64)))) \
                    (record (= a (+ (. r a) 1)) (= b (. r b)))) \
                 (def (main (: seed Int64)) (. (f (f (record (= a seed) (= b 5)))) a)) (export main))";
    let bytes = compile_component(&crate::codec::encode(&parse(src)))
        .expect("a composed record-transform call must compile");
    assert!(
        imports_value_heap_runtime(&bytes),
        "the nested record-transform builds a runtime record, so it imports the value-heap runtime"
    );
    // The RUN VALUE — f(f({a:seed,b:5})) with seed=0 bumps a twice → .a = 2 — is corpus-covered by
    // 05-compound-types "a composed record-transform call bumps a field, applied twice, and the result field
    // is read"; this test keeps the slot-collision COMPILE-VALIDITY guard (a compile-artifact the corpus
    // cannot assert: the miscompile was a wasm-validation failure, expected i64/found i32).
    // A THREE-field record with TWO checked-arith fields and the arith field read LAST — exercises more
    // than one arith scratch interleaved with the assembler slots. `(* c 3)` twice over c=2 → 18.
    // Field `a` seeded from the runtime parameter (same reason as above) keeps the three-field build on the
    // heap; `.c` is independent of the seed — `(* c 3)` twice over c = 2 → 18 regardless.
    let three = "(module m \
                   (def (f (: r (Record (a Int64) (b Int64) (c Int64)))) \
                      (record (= a (+ (. r a) 1)) (= b (. r b)) (= c (* (. r c) 3)))) \
                   (def (main (: seed Int64)) (. (f (f (record (= a seed) (= b 5) (= c 2)))) c)) (export main))";
    let three_bytes = compile_component(&crate::codec::encode(&parse(three)))
        .expect("a three-field composed record-transform call must compile");
    assert!(
        imports_value_heap_runtime(&three_bytes),
        "the three-field nested record-transform imports the value-heap runtime"
    );
    // The RUN VALUE — c = 2 tripled twice = 18 — is corpus-covered by 05-compound-types "a composed
    // three-field record-transform triples a field twice and reads it last"; this test keeps the
    // slot-collision compile-validity guard (the interleaved-arith-scratch face the corpus cannot assert).
}

/// A COLLECTION construction whose element/key/value siblings interleave a BigInt HEAP HANDLE (i32) with
/// GUARDED Int64 arithmetic (i64) compiles to a VALID module and runs to the right value — the regression
/// guard for the `Set.of`/`Map.insert` slot-collision miscompile
/// (`adv-bigint-arith-elem-then-of-int64-arith-elem-set-map-invalid-wasm-slot-clash`). `(Set.of (list (+
/// (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2))))` builds a set whose FIRST element is a BigInt sum
/// (its `bigint-add` operands stash i32 handles in scratch slots) and whose SECOND element boxes a checked
/// `(+ n 2)` (an i64 overflow-guard temp). The `Core::SetOf`/`MapNew`/`MapInsert`/`SetInsert` arms used to
/// emit every sibling at a FIXED `base`, so the second element's i64 arith temp reused the slot the first
/// element's i32 handle had already typed → one wasm local declared at two widths → `wasm-tools validate:
/// expected i64, found i32`, the component rejected at load (O0 and O2 alike). The fix advances each
/// sibling's scratch base past the running high-water (the disjoint-slot discipline
/// `Core::Tuple`/`Core::Record`/`Core::ListNew` already applied). `{n+1, n+2}` with n=5 is `{6, 7}` — two
/// distinct elements, so `Set.len` = 2; the `Map.insert` twin keys `{6, 7}` → size 2.
#[test]
fn a_bigint_arith_then_of_arith_collection_element_pair_runs() {
    use crate::testkit::parse;
    // A helper: compile the module and assert it is VALID wasm importing the value-heap runtime. The
    // miscompile this guards was a wasm-VALIDATION failure (a boxed-numeric element's i32 handle and a
    // sibling's i64 arith temp reused one collection-assembler slot at two widths, "expected i64/found
    // i32") — a compile-artifact the corpus cannot assert, so `compile_component` returning `Ok` +
    // `imports_value_heap_runtime` IS the regression guard. The RUN VALUE (each is `Set.len`/`Map.len` = 2
    // over the two distinct boxed-numeric elements {n+1, n+2} at n=5 = {6,7}) is corpus territory —
    // collection length + BigInt/Rational value-distinctness (03-equality / 19-sets). `_expect` is retained
    // for documentation at the call sites.
    let check = |src: &str, _expect: &str, what: &str| {
        let bytes = compile_component(&crate::codec::encode(&parse(src)))
            .unwrap_or_else(|e| panic!("{what} must compile: {e:?}"));
        assert!(
            imports_value_heap_runtime(&bytes),
            "{what} builds a runtime collection, so it imports the value-heap runtime"
        );
    };
    // Set: element 1 = BigInt sum (i32 handles), element 2 = BigInt.of(Int64 arith) (i64 guard temp).
    check(
        "(module m (def (main (: n Int64)) \
           (Set.len (Set.of (list (+ (BigInt.of n) (BigInt.of 1)) (BigInt.of (+ n 2)))))) (export main))",
        "2",
        "Set.of {n+1, n+2} at n=5 is {6,7} → len 2",
    );
    // Map twin: nested `Map.insert`s over the same key pair — key 1 a BigInt sum, key 2 a BigInt.of(arith).
    check(
        "(module m (def (main (: n Int64)) \
           (Map.len (Map.insert (Map.insert (Map.empty) (+ (BigInt.of n) (BigInt.of 1)) 1) \
                                (BigInt.of (+ n 2)) 2))) (export main))",
        "2",
        "Map.insert keys {n+1, n+2} at n=5 → size 2",
    );
    // RATIONAL twin — the slot-clash is generic over cdz-num BOXED-NUMERIC elements, not BigInt-specific
    // (the fix advanced the scratch floor for the whole family). Same recipe with Rational: element 1 is a
    // Rational SUM (`(+ (Rational.of n 1) (Rational.of 1 1))` = (n+1)/1 — its `rational-add` operands stash
    // i32 handles in scratch), element 2 is a `Rational.of` over checked Int64 arith (`(Rational.of (+ n 2)
    // 1)` = (n+2)/1 — the `(+ n 2)` carries an i64 overflow-guard temp). At n=5 the values 6/1 and 7/1 are
    // distinct → Set.len 2. Was manually verified at fix time but never PINNED; this guards the Rational
    // box-arg path against a future regression of the disjoint-slot discipline.
    check(
        "(module m (def (main (: n Int64)) \
           (Set.len (Set.of (list (+ (Rational.of n 1) (Rational.of 1 1)) (Rational.of (+ n 2) 1))))) (export main))",
        "2",
        "Set.of {(n+1)/1, (n+2)/1} at n=5 is {6/1,7/1} → len 2",
    );
    // Rational Map twin.
    check(
        "(module m (def (main (: n Int64)) \
           (Map.len (Map.insert (Map.insert (Map.empty) (+ (Rational.of n 1) (Rational.of 1 1)) 1) \
                                (Rational.of (+ n 2) 1) 2))) (export main))",
        "2",
        "Map.insert Rational keys {(n+1)/1, (n+2)/1} at n=5 → size 2",
    );
}

/// `T.wrap(U.wrap x)` where the outer width N ≤ the inner width M — the inner wrap is redundant (the
/// outer keeps only the low N ≤ M bits, unchanged by the inner). The inner Convert is elided; VALUE
/// parity is preserved (the composed wrap gives the same low bits as the single outer wrap).
#[test]
fn a_narrowing_wrap_of_a_wider_wrap_elides_the_inner_wrap() {
    use crate::testkit::parse;
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

/// `T.wrap x` where the OPERAND'S VALUE (not just its type) provably fits the target width — the
/// truncation mask / sign-extend is redundant and elided, even when the operand's TYPE is wider.
/// `UInt8.wrap(& x 255)`: the operand is Int64-typed but its value ∈ [0,255], so the trailing `& 255`
/// is a no-op. Consults the value-range lattice (a mask, a flow-refinement). VALUE parity preserved.
#[test]
fn a_wrap_of_an_in_range_value_elides_the_truncation() {
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::select::select_function;
    use crate::testkit::parse;
    let code_of = |src: &str| -> Vec<Lir> {
        let full = format!(
            "(module m (def (f {}) {}) (def (main) 0) (export main))",
            "(: x Int64)", src
        );
        let mut db = crate::db::Db::load(parse(&full));
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
        select_function(&mut db, body, &ps, &layout)
            .expect("select")
            .code
    };
    // UInt8.wrap of a [0,255] value: the operand's `& 255` is at the i64 source width (`I64And`); the
    // wrap's OWN truncation mask would be an `i32.and` AFTER `i32.wrap_i64` — and it is ELIDED (zero
    // `I32And`). (A bare `UInt8.wrap x` DOES emit that `i32.and`; see `bare` below.)
    let masked = code_of("(: ((. UInt8 wrap) (& x 255)) UInt8)");
    assert!(
        !masked.iter().any(|i| matches!(i, Lir::I32And)),
        "the wrap's redundant i32 truncation mask is elided; got {masked:?}"
    );
    // Int8.wrap of a [0,7] value (fits [-128,127]): the sign-extend pair (shl/shr_s) is elided.
    let signed = code_of("(: ((. Int8 wrap) (& x 7)) Int8)");
    assert!(
        !signed.iter().any(|i| matches!(i, Lir::I32Shl)),
        "a value in [0,7] fits Int8 — no sign-extend reshape; got {signed:?}"
    );
    // SAFETY: a wrap of a NOT-in-range value keeps the truncation — the `i32.and` after `i32.wrap_i64`.
    let bare = code_of("(: ((. UInt8 wrap) x) UInt8)");
    assert!(
        bare.iter().any(|i| matches!(i, Lir::I32And)),
        "a bare narrowing wrap keeps its i32 truncation mask; got {bare:?}"
    );
    // Value parity (the elided-mask/sign-extend wrap computes the same as the full one) migrated to the
    // corpus (run via cdz-run): cases "an unsigned wrap of a value the range lattice proves in-range elides
    // its mask" and "a signed wrap of a value the range lattice proves in-range elides its sign-extend" in
    // spec/semantics/06-numeric-model.sexp.
}

/// A SYNTHESIZED def-form's body reference to its OWN parameter still resolves even after an UNRELATED
/// `extend_scope_skip_*` call grows the scope-skip vector PAST it. This is the monomorphization SCC bug
/// (`type_specialize` builds a spec-copy `(def (spec p…) body)` after load; a peer's later map-match /
/// refutable-literal desugar `resize(structure.len(), None)`s the skip vector): before the fix,
/// `scope_skip_covers` read "id < vector length" as "covered", so the spec body's `p` took the skip
/// FAST-PATH, bottomed at the resize-default `None`, and reported its own parameter UNBOUND (a spurious
/// CDZ0101 at whole-program compile — `apply-def-by-name`'s `defs` in the compiler-ml eval-db repro).
/// The fix distinguishes a GENUINELY-seeded entry from a resize-default: an unseeded synth node falls back
/// to the exhaustive parent walk. Built directly against `Db` (no ML shape reliably reproduces the timing
/// — the copy must exist when a later resize fires; see the eval-db `run-src-typed` repro).
#[test]
fn a_synth_def_bodys_own_param_resolves_after_an_unrelated_scope_skip_resize() {
    use crate::db::Db;
    use crate::resolve::resolved_of;
    use crate::resolved::Resolved;
    // Any loaded program — we synthesize NEW nodes past its load boundary below.
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (main) 0) (export main))",
    ));

    // Synthesize a def-form `(def (spec p) p)` AFTER load — the shape `type_specialize` builds for a
    // monomorphized SCC member. Both `p` occurrences are fresh synth atoms of the same name; the body `p`
    // must resolve to the sig's binder `p`.
    let def_head = db.push_name("def");
    let spec_name = db.push_name("spec#mono0");
    let param_binder = db.push_name("p");
    let sig = db.push_list(vec![spec_name, param_binder]);
    let body_ref = db.push_name("p");
    let _def_form = db.push_list(vec![def_head, sig, body_ref]);

    // Sanity: the body reference resolves to the sig's parameter (the exhaustive walk finds it — the synth
    // node is not yet swept into the skip vector, so `scope_skip_covers` is false and the walk runs).
    assert!(
        matches!(resolved_of(&mut db, body_ref), Resolved::Ref { .. }),
        "the synth def body's own param must resolve to its binder before any resize"
    );

    // Now an UNRELATED synth subtree + a seeding call that RESIZES the skip vector to the full arena length
    // (`resize(structure.len(), None)`), sweeping `def_form`/`body_ref` into the vector with a default-`None`
    // entry. This is exactly what a peer's map-match / refutable-literal desugar does after a spec copy
    // already exists. Forget the memoized resolution so it re-resolves against the (now larger) index.
    let plus = db.push_name("+");
    let q = db.push_name("q");
    let r = db.push_name("r");
    let other = db.push_list(vec![plus, q, r]);
    db.extend_scope_skip_into_subtree(other);
    crate::resolve::forget_subtree(&mut db, body_ref);

    // The body `p` STILL resolves to its binder — it was never seeded, so `scope_skip_covers` is false and
    // the exhaustive walk runs, rather than the poisoned fast-path bottoming at the resize-default `None`.
    assert!(
        matches!(resolved_of(&mut db, body_ref), Resolved::Ref { .. }),
        "an unrelated scope-skip resize must NOT sweep the synth def body's param into a spurious unbound"
    );
}

/// A record binding pattern projects each field by NAME→sorted-slot, not by written order — so it binds
/// correctly when the pattern names fields in a DIFFERENT order than the record value and when it names a
/// SUBSET of the fields (a partial record pattern, more flexible than a tuple's full-arity match). Here the
/// pattern writes `z` before `a` while the value writes `a` before `z`, and both are bound by name: `c` =
/// field `a` = 10, `b` = field `z` = 20 → 100*10 + 20 = 1020. This is the `Member`→`Proj` fold handling the
/// field order for free (`runtime_member_index`'s sorted slot), the record analogue of a tuple's positional
/// `Elem(i)`.
/// A record binding pattern's WELL-FORMEDNESS faults, each with the actionable coded diagnostic (the
/// binding-position twin of the tuple arm's checks): a REFUTABLE literal field value is CDZ0210; a field
/// the value's record type does NOT have is CDZ0203 (naming the missing field); a NON-record bound value
/// is CDZ0203; a NON-LINEAR pattern (two fields binding one name) is CDZ0102; and a NESTED compound field
/// value — irrefutable but not-yet-wired (Increment B binds fields to bare names) — is a clean DECLINE that
/// names the feature, NOT a misleading CDZ0101 "unbound name" on the nested binder (the `let` twin of the
/// match-side Case-6rec cascade suppression).
#[test]
fn a_record_binding_pattern_faults_are_actionable_and_lockstep() {
    use crate::testkit::parse;
    let code_of = |src: &str| {
        compile_component(&crate::codec::encode(&parse(src)))
            .err()
            .and_then(|d| d.code)
    };
    // (a) refutable literal field value → CDZ0210.
    assert_eq!(
        code_of(
            "(module m (def (main) \
               (let (((record (x 0) (y b)) (record (= x 3) (= y 4)))) b)) (export main))"
        )
        .as_deref(),
        Some("CDZ0210")
    );
    // (b) a field the record type does not have → CDZ0203.
    assert_eq!(
        code_of(
            "(module m (def (main) \
               (let (((record (nope a)) (record (= x 3) (= y 4)))) a)) (export main))"
        )
        .as_deref(),
        Some("CDZ0203")
    );
    // (c) non-record bound value → CDZ0203.
    assert_eq!(
        code_of("(module m (def (main) (let (((record (x a)) 5)) a)) (export main))").as_deref(),
        Some("CDZ0203")
    );
    // (d) non-linear (two fields bind `a`) → CDZ0102.
    assert_eq!(
        code_of(
            "(module m (def (main) \
               (let (((record (x a) (y a)) (record (= x 3) (= y 4)))) a)) (export main))"
        )
        .as_deref(),
        Some("CDZ0102")
    );
    // (e) nested compound field value → a CLEAN decline (uncoded), NOT a phantom CDZ0101. The reference to
    // the nested binder `a` must attribute to the unwired feature, so the code is NOT the unbound-name one.
    let nested = compile_component(&crate::codec::encode(&parse(
        "(module m (def (main) \
           (let (((record (p (tuple a b))) (record (= p (tuple 1 2))))) (+ a b))) (export main))",
    )))
    .expect_err("nested compound record field value declines");
    assert_ne!(nested.code.as_deref(), Some("CDZ0101"));
    assert!(
        nested.message.contains("nested compound sub-pattern"),
        "nested-field decline should name the feature, got: {}",
        nested.message
    );
}

/// A record match pattern's faults are actionable and LOCKSTEP with resolve: a field the scrutinee's
/// record type lacks is CDZ0201 (naming the field); a NESTED compound field value — irrefutable but not
/// yet wired (a record match binds fields to bare names) — is a clean DECLINE naming the feature, NOT a
/// misleading CDZ0101 on the nested binder (the match twin of the record-BINDING lockstep).
#[test]
fn a_record_match_pattern_faults_are_actionable_and_lockstep() {
    use crate::testkit::parse;
    // absent field → CDZ0201
    let absent = compile_component(&crate::codec::encode(&parse(
        "(module m (def (main) (match (record (= x 3)) ((record (nope a)) a))) (export main))",
    )))
    .expect_err("absent field rejects");
    assert_eq!(absent.code.as_deref(), Some("CDZ0201"));
    // SINGLE diagnostic — no redundant CDZ0212. The arm BODY's reference to the field binder `a` resolves
    // (Case 6rec) to a Member projection of the scrutinee at `nope`; when `nope` is absent that Member used
    // to ALSO fire a generic "record has no field `nope`" (CDZ0212) at the bare-name body-ref, double-
    // reporting one error. `collect_node`'s `Member` arm now early-returns for a BARE-NAME node (a
    // pattern-binder ref, vs a genuine `(. …)` access) BEFORE calling `no_field_reject`, so the canonical
    // pattern CDZ0201 is the sole primary.
    let diags = crate::host::run_with_compiler_stack(|| {
        crate::diagnostics(&mut crate::db::Db::load(parse(
            "(module m (def (main) (match (record (= x 3)) ((record (nope a)) a))) (export main))",
        )))
    });
    let field_faults: Vec<_> = diags
        .iter()
        .filter(|d| d.message.contains("field `nope`"))
        .collect();
    assert_eq!(
        field_faults.len(),
        1,
        "one primary for an absent record-pattern field (the pattern CDZ0201), no redundant member CDZ0212: {field_faults:?}"
    );
    assert_eq!(field_faults[0].code.as_deref(), Some("CDZ0201"));
    // A nested compound field value — `(record (p (tuple a b)))`, field `p`'s value the further
    // `(tuple a b)` — now BINDS (§235: a record field value binder may be any nested pattern to any depth),
    // via a `Resolved::RecordField` reading field `p` then descending `sub_path` into the tuple (a record
    // field read lowers to `Elem(<sorted-slot>)`, so the descent is positional). Over the CONSTANT scrutinee
    // `(record (= p (tuple 1 2)))` it const-folds: a=1, b=2 → `a + b` compiles clean (formerly the CDZ0900
    // deeper-nesting decline #6838; now wired). A record/variant BELOW the field is still deferred.
    compile_component(&crate::codec::encode(&parse(
        "(module m (def (main) \
           (match (record (= p (tuple 1 2))) ((record (p (tuple a b))) (+ a b)))) (export main))",
    )))
    .expect("nested compound record match field binds via the RecordField sub_path (§235)");
}

// (a_record_match_pattern_typo_field_suggests_the_nearest_field migrated to corpus 05-compound-types:
// "a record match pattern typo field suggests the nearest field with a rename fix" (CDZ0201, near-miss
// suggest + (fix (kind replace)(replacement "helper")(unverified))) + the "a far-miss record match pattern
// field carries no baseless suggestion or fix" twin (CDZ0201 (not "did you mean")(no-fix)). Fully corpus-expressible.)

#[test]
fn a_variant_payload_binder_shadowing_a_param_in_a_called_def_binds_the_payload() {
    // REGRESSION: a VARIANT-PAYLOAD binder that shadows the enclosing def's param, in a def reached via a
    // CALL, was misresolved. `(def (f (: n Int64)) (match (W.V (+ n 1)) ((W.V n) n) ((W.Z) 0)))` binds the
    // payload as `n` in the `(W.V n)` arm, shadowing the param `n`. When `f` is CALLED (`(main) (f k)`),
    // the call path runs `resolve_subtree` over the callee body to pin arguments before β-reduction — and
    // that eager walk resolved the PATTERN occurrence `n` via `resolve_name`, binding it to the PARAM `n`
    // (an in-scope outer binding). That corrupted the pattern head detection → "not a variant constructor"
    // / a spurious CDZ0101. An EXPORTED-directly `f` (no call, no `resolve_subtree`) was unaffected, so the
    // bug only showed for a helper. Fix: a variant-payload binder occurrence that SHADOWS an outer binding
    // resolves INERT (like a list/map pattern binder), so the eager walk can't misbind it; a body reference
    // still resolves to the payload (Case 6, nearest-scope). The arm binder must bind the PAYLOAD, not the
    // param: `f(5)` → `(W.V (5+1))` matched `(W.V n)` → n = 6 (NOT the param 5). The bug was a
    // compile-time REJECT (the pattern head detection broke), so COMPILING is the precise guard here; the
    // RUN (that the binder binds the payload, not the param) is exercised by the corpus case
    // "a variant-payload binder shadowing a param in a called def binds the payload" (which links the
    // value-heap runtime a sum construction needs).
    use crate::testkit::parse;
    let src = "(module m (type W (V Int64) (Z)) \
                 (def (f (: n Int64)) (match (W.V (+ n 1)) ((W.V n) n) ((W.Z) 0))) \
                 (def (main (: k Int64)) (f k)) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_ok(),
        "a variant-payload binder shadowing the def param, in a CALLED def, must COMPILE (was a spurious \
         reject: the eager call-site subtree walk bound the pattern occurrence to the shadowed param)"
    );

    // A MULTI-PAYLOAD variant binder shadowing the param, also called: `(P.Mk a b)` binds `a` to the
    // FIRST payload, shadowing the param `a`.
    let multi = "(module m (type P (Mk Int64 Int64) (Z)) \
                   (def (f (: a Int64)) (match (P.Mk (+ a 1) 5) ((P.Mk a b) (+ a b)) ((P.Z) 0))) \
                   (def (main (: k Int64)) (f k)) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(multi))).is_ok(),
        "a multi-payload arm binder shadowing the def param, in a called def, must COMPILE"
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
    // The H2c property (the point of this test, a compile-artifact only white-box can assert): NO runtime
    // import — the static tuple left no heap trace at all.
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a constant tuple must fold away, importing no runtime op (no per-call arr-alloc)"
    );
    // It runs to the folded scalar (10+20=30); that RUN value is corpus-covered by the constant
    // tuple-projection folds in 05-compound-types (e.g. "constant tuple projections fold at boundary and
    // multi-digit indices" + "a let-bound tuple produced by an if, projected at two indices").
}

/// The routing pin: a projection-only tuple LITERAL folds whether or not its elements are runtime —
/// what a projection reads is the element's own computation, not a heap cell. `(let ((t (tuple 10 a)))
/// (+ (. t 0) (. t 1)))` folds to `(+ 10 a)`: no heap, no runtime import, even though `a` is a runtime
/// param. A tuple reaches the heap only when it is used as a WHOLE value (returned/passed) and so must
/// be built — see `a_narrow_runtime_tuple_element_crosses_the_heap_boundary` (a call-returned tuple)
/// and `a_recursive_runtime_tuple_escapes_to_the_host` (a returned tuple). Shape alone never forces the
/// heap; escaping-as-a-value does.
#[test]
fn a_projection_only_tuple_folds_importing_no_runtime() {
    use crate::testkit::parse;
    let src = "(module m (def (with-param (: a Int64)) \
                 (let ((t (tuple 10 a))) (+ (. t 0) (. t 1)))) (export with-param))";
    let bytes = compile_component(&crate::codec::encode(&parse(src))).expect("compile");
    // The wasmtime-free structural half: a projection-only tuple folds, importing NO value-heap runtime,
    // even with a runtime element. The RUN value (with-param 32 → 42) is pinned by the corpus case
    // "a projection-only tuple over a runtime element folds to the sum of its elements" (05-compound-types).
    assert!(
        !imports_value_heap_runtime(&bytes),
        "a projection-only tuple folds — no heap — even with a runtime element"
    );
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

/// `Set.to-list` of a provably-EMPTY constant set folds to the empty list — no ordering descriptor, no
/// element type needed. An empty `Set.of (list)` LITERAL has an UNDETERMINED element type (no element
/// ever constrained it), so no shape descriptor can be baked; but an empty set enumerates to `[]`
/// regardless of element type, so the compiler folds it straight to `Core::ListNew { elems: [] }`. This
/// is a reject-don't-diverge fix: before the fold the type-checker ACCEPTED the program while the backend
/// declined at emit ("no orderable descriptor") — a check/compile divergence. The FOLD unit needs no
/// runtime; the wasmtime run (`#[ignore]`, needs the store) confirms the empty list has length 0.
/// The first-class native set literal `#set(…)` (operator ruling 2026-08-27: data structures pulled all
/// the way through the compiler, NOT desugared to `Set.of` in the reader) resolves to `Resolved::Set` and
/// lowers to the SAME `Core::SetOf` `Set.of (list …)` produces — so a set VALUE still renders
/// `(Set.of (list …sorted))`, constant elements dedup at build, and `Set.to-list` folds identically.
/// (Nativized from the legacy `("set" …)` string-head surface to the native `#set(…)` ctor-leaf form —
/// the M3 canonical spelling — ahead of the reader-flip that retires string-head recognition.)
#[test]
fn set_literal_ctor_lowers_to_the_same_set_as_set_of() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let fold = |body: &str| -> Core {
        let src = format!("(module m (def (main) {body}) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let d = db.def_by_name("main").expect("def main");
        let m_body = db.defs[d].body.expect("main has a body");
        core_of(&mut db, m_body)
    };
    // `#set(3 1 2)` lowers directly to a `Core::SetOf` of 3 elements.
    match fold("#set(3 1 2)") {
        Core::SetOf { elems, .. } => assert_eq!(
            elems.len(),
            3,
            "#set(3 1 2) lowers to a 3-element Core::SetOf, got {}",
            elems.len()
        ),
        other => panic!("#set(3 1 2) must lower to Core::SetOf, got {other:?}"),
    }
    // Constant elements DEDUP at build: `#set(1 1 2)` is a 2-element set.
    match fold("#set(1 1 2)") {
        Core::SetOf { elems, .. } => assert_eq!(
            elems.len(),
            2,
            "#set(1 1 2) dedups to a 2-element Core::SetOf, got {}",
            elems.len()
        ),
        other => panic!("#set(1 1 2) must lower to Core::SetOf, got {other:?}"),
    }
    // Through `Set.to-list` a `#set(…)` folds to the SAME sorted list as `Set.of (list …)`.
    match (
        fold("(Set.to-list #set(3 1 2))"),
        fold("(Set.to-list (Set.of (list 3 1 2)))"),
    ) {
        (Core::ListNew { elems: a }, Core::ListNew { elems: b }) => assert_eq!(
            a.len(),
            b.len(),
            "#set(…) and Set.of build the same set: {} vs {}",
            a.len(),
            b.len()
        ),
        (x, y) => panic!("both must fold to a sorted ListNew: {x:?} vs {y:?}"),
    }
}

/// The SHADOWABLE `set` NAME alias (operator ruling 2026-08-31: keep a shadowable name constructor for set
/// as the string-form `("set" …)` is dropped at the M3 reader-flip; the set companion of the tuple/record/
/// list/map aliases, which set uniquely lacked). `(set …)` reduces (via the `set-new` intrinsic) to the
/// native `#set(…)`, so it lowers to the SAME `Core::SetOf` — constant-element dedup at build — and it is
/// SHADOWABLE: a local `set` binding suppresses the alias (unlike the unshadowable native `#set(…)` leaf).
#[test]
fn set_name_alias_lowers_like_native_and_is_shadowable() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let fold = |body: &str| -> Core {
        let src = format!("(module m (def (main) {body}) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let d = db.def_by_name("main").expect("def main");
        let mbody = db.defs[d].body.expect("main has a body");
        core_of(&mut db, mbody)
    };
    // `(set 3 1 2)` — the shadowable name alias — lowers to a 3-element `Core::SetOf`, same as `#set(3 1 2)`.
    match fold("(set 3 1 2)") {
        Core::SetOf { elems, .. } => assert_eq!(elems.len(), 3, "(set 3 1 2) → 3-element SetOf"),
        other => panic!("(set 3 1 2) must lower to Core::SetOf, got {other:?}"),
    }
    // Constant elements DEDUP at build (like the native literal): `(set 1 1 2)` is a 2-element set.
    match fold("(set 1 1 2)") {
        Core::SetOf { elems, .. } => {
            assert_eq!(elems.len(), 2, "(set 1 1 2) dedups to 2-element SetOf")
        }
        other => panic!("(set 1 1 2) must lower to Core::SetOf, got {other:?}"),
    }
    // SHADOWABLE: a local `set` binding suppresses the alias — `(let ((set 7)) set)` is the Int 7, NOT a
    // set constructor. (The native `#set(…)` leaf is unshadowable; the NAME alias is the shadowable form the
    // operator ruling endorses.) The whole `main` returns 7, so it lowers to a constant, not a `SetOf`.
    // main returns 7 (a constant), so it lowers to a constant int, NOT a Core::SetOf.
    if let Core::SetOf { .. } = fold("(let ((set 7)) set)") {
        panic!("a local `set` binding must SHADOW the set alias, not build a set");
    }
    // A shadowed `set` used arithmetically still lowers cleanly (the alias does not leak into the local).
    if let Core::SetOf { .. } = fold("(let ((set 7)) (+ set 1))") {
        panic!("a shadowed `set` in `(+ set 1)` must not resurrect the set alias");
    }
}

#[test]
fn set_to_list_of_an_empty_set_folds_to_the_empty_list() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // FOLD unit: `(Set.to-list (Set.of (list)))` — an empty set literal, element type undetermined — folds
    // to the empty `Core::ListNew` (NOT a `SetToList` needing a descriptor, NOT a decline).
    let fold = |body: &str| -> Core {
        let src = format!("(module m (def (main) {body}) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let d = db.def_by_name("main").expect("def main");
        let m_body = db.defs[d].body.expect("main has a body");
        core_of(&mut db, m_body)
    };
    match fold("(Set.to-list (Set.of (list)))") {
        Core::ListNew { elems } => assert!(
            elems.is_empty(),
            "an empty set's to-list must fold to the EMPTY list, got {} elements",
            elems.len()
        ),
        other => panic!(
            "Set.to-list of an empty set must fold to an empty ListNew (no descriptor needed), got {other:?}"
        ),
    }
    // The fold also fires through an inlined nullary call whose body is the empty-set literal — the
    // element type is undetermined at the call site too, so the descriptor-free fold is what saves it.
    // A NON-empty CONSTANT set now FOLDS to a canonically-sorted `Core::ListNew` (the runtime `set-to-list`
    // op sorts by the SAME spec-pinned canonical value order — `const_key_order` == the runtime's
    // `value_cmp_shaped`, confirmed a contract by v-runtime — so materializing here byte-matches the op).
    match fold("(Set.to-list (Set.of (list 3 1 2)))") {
        Core::ListNew { elems } => assert_eq!(
            elems.len(),
            3,
            "a non-empty constant Set.to-list folds to a 3-element sorted ListNew, got {} elems",
            elems.len()
        ),
        other => {
            panic!("a non-empty constant Set.to-list must fold to a sorted ListNew, got {other:?}")
        }
    }
    // A tuple of ORDERABLE scalars now folds (element-wise lexicographic) — a 2-element sorted ListNew.
    match fold("(Set.to-list (Set.of (list (tuple 3 4) (tuple 1 2))))") {
        Core::ListNew { elems } => assert_eq!(
            elems.len(),
            2,
            "a const Set.to-list of orderable tuples folds to a sorted ListNew, got {} elems",
            elems.len()
        ),
        other => panic!("a const Set.to-list of orderable tuples must fold, got {other:?}"),
    }
    // A set whose element the canonical order cannot rank (a tuple carrying a FLOAT) keeps the runtime op.
    match fold("(Set.to-list (Set.of (list (tuple 1.5 2) (tuple 3.5 4))))") {
        Core::SetToList { .. } => {}
        other => {
            panic!(
                "a non-orderable-element (float-in-tuple) Set.to-list must emit the runtime op, got {other:?}"
            )
        }
    }
}

/// WARNING: SOUNDNESS regression (breaker #43, routed 2026-07-29): ordering an ALL-NULLARY sum (every variant
/// payload-free — a user `(type Tri (Lo)(Mid)(Hi))`, `Sign`, `Ordering` itself) mis-lowered on wasm. An
/// all-nullary sum is an ENUM-DISC: a BARE i32 discriminant with NO heap box. But `<`/`>`/`compare` routed
/// it to `Core::ValueCmp` (the runtime `value-cmp` heap walk, since an all-nullary `Ty::Sum` passes
/// `is_orderable_compound`), and `value-cmp`'s `op_sum_disc` expects a BOXED sum node — fed a bare immediate
/// i32 it read discriminant 0 for both operands → `compare` returned Equal for DISTINCT variants and `<`
/// was false both ways, while `=` (correctly enum-disc-routed to `i32.eq`) said false. compare contradicted
/// `=` (§331) and the sum didn't order by discriminant (§Compound Ordering). Fix: `<`/`>`/`compare` on an
/// enum-disc operand route to a scalar `Core::Compare` i32 TAG compare (the sum's runtime value IS its
/// discriminant), the same representation `=` already used. This Core-level witness pins the ROUTE (no
/// runtime needed): an enum-disc `<` lowers to `Core::Compare`, NOT `Core::ValueCmp`; a PAYLOAD-carrying
/// sum's ordering still boxes → `Core::ValueCmp` (the fix's perimeter — must not disturb the boxed walk).
#[test]
fn an_all_nullary_sum_orders_by_i32_discriminant_not_the_value_cmp_heap_walk() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    // Lower the body of `cmp` (a `<` over two all-nullary-sum values built from params) and inspect the Core.
    let lower_body = |src: &str, def: &str| -> Core {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let m_body = db.defs[d].body.expect("body");
        core_of(&mut db, m_body)
    };
    // An ALL-NULLARY sum `<`: `(< x y)` where x,y : Tri (all variants payload-free). Must be a scalar i32
    // tag `Core::Compare`, NOT a `Core::ValueCmp` heap walk (which misreads the bare disc → the #43 bug).
    let all_nullary = "(module m (type Tri (Lo) (Mid) (Hi)) \
        (def (lt (: x Tri) (: y Tri)) (< x y)) \
        (def (main) 0) (export main))";
    match lower_body(all_nullary, "lt") {
        Core::Compare { .. } => {} // ✔ i32 tag compare — orders by discriminant
        Core::ValueCmp { .. } => panic!(
            "an all-nullary sum `<` must lower to a scalar i32 tag Core::Compare, NOT Core::ValueCmp \
             (the heap walk misreads a bare enum-disc → #43 Equal-for-distinct-variants soundness bug)"
        ),
        other => panic!("an all-nullary sum `<` must be a Core::Compare, got {other:?}"),
    }
    // CONTROL — a PAYLOAD-carrying sum's `<` still boxes, so it MUST take the ValueCmp heap walk (the fix's
    // perimeter: only a wholly-nullary sum is a bare disc; a sum with any payload variant is heap-boxed).
    let payload_sum = "(module m (type Mix (P Int64) (N1) (N2)) \
        (def (lt (: x Mix) (: y Mix)) (< x y)) \
        (def (main) 0) (export main))";
    match lower_body(payload_sum, "lt") {
        Core::ValueCmp { .. } => {} // ✔ boxed sum → the descriptor-guided heap walk (unchanged)
        other => panic!(
            "a PAYLOAD-carrying sum `<` must still lower to Core::ValueCmp (boxed heap walk), got {other:?}"
        ),
    }
}

/// SROA escape-gate (operator directive, tick ~353): `sroa_tuple_scrutinee_candidate` must FIRE on a
/// `match` whose scrutinee is a fresh runtime tuple AND every arm is a tuple-destructure (the alloc-eliding
/// candidate v-patterns' dispatch consumes), and DECLINE (fail-closed) whenever the tuple escapes — a
/// whole-value binder arm, a wildcard-root arm, a const-visible tuple (already folded), or a non-tuple
/// scrutinee. Reaches the private helper via `crate::lower::…`; extracts the match's scrutinee/arms from
/// the resolved `main` body.
#[test]
fn sroa_tuple_scrutinee_candidate_fires_only_on_an_escape_free_runtime_tuple_destructure() {
    use crate::db::Db;
    use crate::resolve::resolved_of;
    use crate::resolved::Resolved;
    // Build `main`, resolve its body to the `Resolved::Match`, and run the gate on (scrutinee, arms).
    let gate = |body: &str| -> Option<usize> {
        let src = format!("(module m (def (main (: a Int64) (: b Int64)) {body}) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let d = db.def_by_name("main").expect("def main");
        let m_body = db.defs[d].body.expect("main has a body");
        let (scrutinee, arms) = match resolved_of(&mut db, m_body) {
            Resolved::Match { scrutinee, arms } => (scrutinee, arms),
            other => panic!("expected a Resolved::Match, got {other:?}"),
        };
        crate::lower::sroa_tuple_scrutinee_candidate(&mut db, scrutinee, &arms).map(|e| e.len())
    };
    // FIRES: a fresh runtime tuple `(a, b)` with lit-test + binder tuple-destructure arms — the canonical
    // pair_match shape. Two elements.
    assert_eq!(
        gate("(match (tuple a b) ((tuple 0 y) y) ((tuple x y) (+ x y)))"),
        Some(2),
        "a runtime-tuple scrutinee matched only by tuple-destructure arms is a SROA candidate (2 elems)"
    );
    // FIRES: a guarded tuple-destructure arm peels to its inner tuple pattern (still positional).
    assert_eq!(
        gate("(match (tuple a b) ((guard (tuple x y) (> x 0)) x) ((tuple x y) y))"),
        Some(2),
        "a guarded tuple-destructure arm is still positional — candidate"
    );
    // DECLINES: a WHOLE-VALUE binder arm `((t) …)` binds the tuple itself — it escapes.
    assert_eq!(
        gate("(match (tuple a b) ((tuple 0 y) y) (t 7))"),
        None,
        "a bare-name (whole-value) arm binds the aggregate — the tuple escapes, no SROA"
    );
    // DECLINES: a wildcard-root arm keeps the aggregate.
    assert_eq!(
        gate("(match (tuple a b) ((tuple 0 y) y) (_ 7))"),
        None,
        "a wildcard-root arm keeps the aggregate — no SROA"
    );
    // DECLINES: a compile-time-CONSTANT tuple already folds its Elem reads (never allocates) — not a
    // candidate (nothing to replace).
    assert_eq!(
        gate("(match (tuple 3 5) ((tuple x y) (+ x y)))"),
        None,
        "a const-visible tuple already folds — not a SROA candidate"
    );
    // DECLINES: a tuple with MORE than SROA_MAX_TUPLE_FIELDS (16) elements — the profitability cap
    // (exploding a huge aggregate spends locals + copies for no win). A 17-field runtime tuple (all `a`s so
    // the scrutinee stays a fresh runtime Core::Tuple) with a 17-binder tuple-destructure arm.
    {
        let seventeen = std::iter::repeat_n("a", 17).collect::<Vec<_>>().join(" ");
        let binders = (0..17)
            .map(|i| format!("x{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            gate(&format!(
                "(match (tuple {seventeen}) ((tuple {binders}) 0))"
            )),
            None,
            "a 17-field tuple exceeds the SROA field-count profitability cap (16) — declines"
        );
    }
    // PRE-BIND: on a firing candidate, every returned elem is marked a KEPT binding (so references lower to
    // a `LocalRef`, evaluated once), and `sroa_let_wrap` builds the matching self-keyed `Core::Let` binding
    // them — the eval-once/trap-once guarantee at the bind site (v-patterns' interface).
    {
        use crate::core::Core;
        use crate::db::Db;
        use crate::resolve::resolved_of;
        use crate::resolved::Resolved;
        let src = "(module m (def (main (: a Int64) (: b Int64)) \
                     (match (tuple a b) ((tuple 0 y) y) ((tuple x y) (+ x y)))) (export main))";
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name("main").expect("def main");
        let m_body = db.defs[d].body.expect("main body");
        let (scrutinee, arms) = match resolved_of(&mut db, m_body) {
            Resolved::Match { scrutinee, arms } => (scrutinee, arms),
            other => panic!("expected Resolved::Match, got {other:?}"),
        };
        let elems = crate::lower::sroa_tuple_scrutinee_candidate(&mut db, scrutinee, &arms)
            .expect("the pair_match shape is a candidate");
        assert_eq!(elems.len(), 2, "a 2-tuple has 2 elems");
        for &e in &elems {
            assert!(
                db.kept_bindings.contains(&e),
                "each SROA elem must be marked a KEPT binding (evaluated once, read via LocalRef)"
            );
        }
        // The caller-side Let-wrap binds every elem self-keyed `(e, e)` around the (here dummy) dispatch body.
        match crate::lower::sroa_let_wrap(&elems, m_body) {
            Core::Let { bindings, .. } => {
                assert_eq!(bindings.len(), 2, "Let binds both elems");
                for (b, init) in bindings.iter() {
                    assert_eq!(
                        b, init,
                        "SROA bindings are self-keyed (binder == init == the elem)"
                    );
                    assert!(elems.contains(b), "each binding keys off a returned elem");
                }
            }
            other => panic!("sroa_let_wrap must build a Core::Let, got {other:?}"),
        }
    }
}

/// The MAP twin of the empty-set fold: `Map.to-list` of an empty `Map.empty` (key/value types
/// undetermined) folds to the empty `Core::ListNew` — no ordering descriptor needed. Before this fold the
/// type-checker ACCEPTED `Map.to-list(Map.empty)` while the backend DECLINED at emit ("Map.to-list
/// key/value shape has no orderable descriptor") — a check/compile divergence (the Set-half fix left the
/// Map half open). An empty map enumerates to `[]` regardless of key/value type. The whole expression
/// const-folds to `0`, so no runtime store is needed (a plain compile check, not `#[ignore]`).
#[test]
fn map_to_list_of_an_empty_map_folds_to_the_empty_list() {
    use crate::core::Core;
    use crate::db::Db;
    use crate::lower::core_of;
    let fold = |body: &str| -> Core {
        let src = format!("(module m (def (main) {body}) (export main))");
        let mut db = Db::load(crate::testkit::parse(&src));
        let d = db.def_by_name("main").expect("def main");
        let m_body = db.defs[d].body.expect("main has a body");
        core_of(&mut db, m_body)
    };
    // FOLD unit: `(Map.to-list Map.empty)` — key/value types undetermined — folds to the empty ListNew
    // (NOT a `MapToList` needing a descriptor, NOT a decline).
    match fold("(Map.to-list Map.empty)") {
        Core::ListNew { elems } => assert!(
            elems.is_empty(),
            "an empty map's to-list must fold to the EMPTY list, got {} elements",
            elems.len()
        ),
        other => panic!(
            "Map.to-list of an empty map must fold to an empty ListNew (no descriptor needed), got {other:?}"
        ),
    }
    // A NON-empty CONSTANT map now FOLDS to a key-sorted `Core::ListNew` of `(key value)` tuples (the runtime
    // `map-to-list` op sorts by the SAME spec-pinned canonical value order — `const_key_order` == the runtime's
    // `value_cmp_shaped`, a v-runtime contract — so materializing here byte-matches the op; the Map twin of the
    // Set.to-list fold).
    match fold("(Map.to-list (Map.insert (Map.insert Map.empty 2 20) 1 10))") {
        Core::ListNew { elems } => assert_eq!(
            elems.len(),
            2,
            "a non-empty constant Map.to-list folds to a 2-element sorted ListNew of tuples, got {} elems",
            elems.len()
        ),
        other => {
            panic!("a non-empty constant Map.to-list must fold to a sorted ListNew, got {other:?}")
        }
    }
    // A map keyed on a tuple of ORDERABLE scalars now folds (element-wise lexicographic key order).
    match fold("(Map.to-list (Map.insert (Map.insert Map.empty (tuple 3 4) 20) (tuple 1 2) 10))") {
        Core::ListNew { elems } => assert_eq!(
            elems.len(),
            2,
            "a const Map.to-list keyed on orderable tuples folds to a sorted ListNew, got {} elems",
            elems.len()
        ),
        other => panic!("a const Map.to-list keyed on orderable tuples must fold, got {other:?}"),
    }
    // A map whose KEY the canonical order cannot rank (a tuple carrying a FLOAT) keeps the runtime op.
    match fold("(Map.to-list (Map.insert Map.empty (tuple 1.5 2) 99))") {
        Core::MapToList { .. } => {}
        other => {
            panic!(
                "a non-orderable-key (float-in-tuple) Map.to-list must emit the runtime op, got {other:?}"
            )
        }
    }
    // The fold sees through an inlined nullary `(def (em) Map.empty)` — the exact seed shape; the whole
    // expression const-folds so no runtime is imported. It COMPILES (the fold-through-nullary pin); its RUN
    // value (empty Map.to-list is the empty list, length 0) is corpus case 05-compound-types "Map.to-list
    // of an empty map is the empty list".
    let src = "(module m (def (em) Map.empty) \
               (def (main) (List.len (Map.to-list (em)))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&crate::testkit::parse(src))).is_ok(),
        "the fold sees through an inlined nullary def returning Map.empty (empty Map.to-list compiles)"
    );
}

// A PARTIALLY-APPLIED CONSTRUCTOR held in a GENUINELY-RUNTIME compound element, projected + applied,
// completes via the synthesized runtime eta-closure lift — the WORKING fix for the FACE-B miscompile
// (was invalid wasm: a newtype erased `(T.Mk 10)` to the bare `10`, storing a scalar where a closure
// handle belonged → `func N: expected i32, found i64`). `mk` builds `(tuple (T.Mk 10) n)` behind a
// recursive `if` so the tuple is not fold-visible; `((. p 0) 5)` projects the partial ctor and applies
// the last payload. `lower_sum_new`'s partial-arity path synthesizes `(fn (y) (T.Mk 10 y))` (capturing
// the supplied `10`), lifts it as an ordinary closure, and the application completes `(T.Mk 10 5)` → 15.
// Compiles to a VALID module (the miscompile was an INVALID one) and runs to 15.
// ── value-heap H3a: a compound RETURNED transfers ownership; the HOST-facing return is a RESOURCE ──
//
// A compound leaving a function transfers ownership of its handle to the consumer (the callee does not
// drop it — `binding_escapes`). The HOST-facing return (a compound crossing the component boundary) is
// NOT a raw handle: a single nullary export returning a tuple/record crosses as a component-model
// RESOURCE whose `encode() -> list<u8>` walks the live handle into the canonical binary value form
// (the resource vertical, tests below). A compound on the MULTI-EXPORT boundary has no such shape and
// still DECLINES (reject-don't-miscompile — `export_result_valtype`). The internal ownership-transfer
// behavior is exercised by the H2 round-trips + the balance probe.

// ── exported parameterized functions: runtime operands, end-to-end (compile → run with args) ─────

// (an_escaping_closure_whose_body_performs_still_declines, an_escaping_captured_continuation_is_refused_not_miscompiled,
//  a_partial_application_of_a_performing_closure_under_a_handler_declines_cleanly — the escaping/captured-continuation/
//  partial-application effect safe-rejects — migrated to corpus 14b-effects-and-handlers (error CDZ0401 / (declines) /
//  error CDZ0201), 2026-08-30 delanguaging handoff from v-rcdzc-test-shrink.)

/// APPLY-SITE HOMING (root-caused from v-cad's passed-closure-under-handler codegen issue): a performing
/// lambda passed as a fn-PARAM to a function that APPLIES it UNDER a handler — the `handler runs a passed-in
/// closure` idiom (`with-seed(body) = handle Rand … (body unit)`, `main` calls `(with-seed (fn (u)
/// (Rand.roll)))`) — must COMPILE. The perform is homed at runtime (the lambda is applied inside `with-seed`
/// under the `Rand` handler), but `check_no_home_walk` used to walk the lambda ARGUMENT's body at its
/// DEFINITION site (in `main`, no handler in scope) → false CDZ0401. The fix: `param_apply_extra_handled`
/// computes, per callee param, the effects the callee applies it under (here Rand), and the arg-walk walks
/// a lambda argument under `handled` PLUS those — so the perform is homed. Folds to 5 (the roll arm resumes
/// with the seed 5). SOUNDNESS COMPLEMENT: the same lambda applied by a callee with NO handler (`apply-fn =
/// (body unit)`) adds nothing → the ungranted effect STAYS rejected CDZ0401. This is the exact pair that a
/// blanket lambda-arg skip got wrong (it dropped the soundness reject to an uncoded decline; `gate --check`
/// caught it) — the apply-site-handled-set discipline distinguishes them.
#[test]
fn a_performing_lambda_applied_under_a_handler_via_a_fn_param_is_homed_at_the_apply_site() {
    use crate::testkit::parse;
    // HOMED: with-seed applies its body param under `handle Rand` → the passed lambda's Rand.roll is homed.
    let homed = "(do (effect Rand (op roll (-> Unit Int64))) \
                 (def (with-seed (: body (-> Unit Int64))) \
                   (handle Rand 5 ((roll (u) s (resume s s))) (body unit))) \
                 (def (main) (with-seed (fn (u) (Rand.roll)))) (export main))";
    // The homing COMPILES (does not false-reject CDZ0401); the RUN value (roll resumes seed 5 → 5) is
    // pinned in the corpus by 14-effects "a performing closure passed to a function that applies it UNDER a
    // handler is homed at the apply site" (= 5), so no wasmtime run here.
    compile_component(&crate::codec::encode(&parse(homed))).expect(
        "a performing lambda applied under a handler (via a fn-param) is homed at the apply site, \
         not falsely CDZ0401'd at its definition site",
    );
    // SOUNDNESS: the same lambda applied by a callee with NO handler must STILL be rejected CDZ0401 (the
    // apply-site handled set is empty, so the ungranted effect is not homed). The blanket-skip bug dropped
    // this to an uncoded decline; the apply-site discipline keeps the coded reject.
    let unhomed = "(do (effect Rand (op roll (-> Unit Int64))) \
                   (def (apply-it (: body (-> Unit Int64))) (body unit)) \
                   (def (main) (apply-it (fn (u) (Rand.roll)))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(unhomed))).is_err(),
        "a performing lambda applied with NO enclosing handler must stay rejected (soundness)"
    );
}

/// TRANSITIVE apply-site homing: a performing lambda passed through a PASS-THROUGH function to one that
/// applies it under a handler. `outer(b) = inner(b)`, `inner(b) = handle R … (b unit)`; `main` calls
/// `(outer (fn (u) (R.roll)))`. `outer`'s `b` is applied (via `inner`) under `handle R`, so its `R.roll`
/// is homed at that apply site — must COMPILE. `param_apply_extra_handled` propagates the sub-callee's
/// extra-handled set through a param passed onward: `outer`'s `b` inherits `inner`'s `{R}`. SOUNDNESS: if
/// the pass-through's target applies the param under NO handler (`inner(b) = (b unit)`), nothing propagates
/// → the ungranted effect STAYS CDZ0401. Extends the single-level homing (which fixed v-cad's bug) one call
/// deeper; the sub-callee recursion is depth-bounded and skips a recursive sub-callee.
#[test]
fn a_performing_lambda_homed_transitively_through_a_pass_through_function() {
    use crate::testkit::parse;
    // HOMED through a pass-through: outer → inner → handle R applies b.
    let homed = "(do (effect R (op roll (-> Unit Int64))) \
                 (def (inner (: b (-> Unit Int64))) (handle R 5 ((roll (u) s (resume s s))) (b unit))) \
                 (def (outer (: b (-> Unit Int64))) (inner b)) \
                 (def (main) (outer (fn (u) (R.roll)))) (export main))";
    // The transitive homing COMPILES (does not false-reject); the RUN value (=5) is pinned in the corpus
    // by 14-effects "a performing closure homed TRANSITIVELY through a pass-through function is not falsely
    // rejected" (= 5), so no wasmtime run here.
    compile_component(&crate::codec::encode(&parse(homed)))
        .expect("a performing lambda applied under a handler through a pass-through fn is homed transitively");
    // SOUNDNESS: pass-through whose target has NO handler → stays rejected.
    let unhomed = "(do (effect R (op roll (-> Unit Int64))) \
                   (def (inner (: b (-> Unit Int64))) (b unit)) \
                   (def (outer (: b (-> Unit Int64))) (inner b)) \
                   (def (main) (outer (fn (u) (R.roll)))) (export main))";
    assert!(
        compile_component(&crate::codec::encode(&parse(unhomed))).is_err(),
        "a performing lambda applied through a pass-through with NO handler must stay rejected (soundness)"
    );
}

/// A recursive function whose self-call is HIDDEN inside a nested fold closure — `(count node)` matching
/// `(Ast.List es) → (List.fold es 1 (fn (acc e) (+ acc (count e))))` — must DECLINE, never overflow the
/// compile stack. `param_apply_extra_handled`'s transitive apply-site homing follows a known non-recursive
/// sub-callee by re-entering itself; `is_recursive` cannot see this cycle because `collect_callees` stops at
/// the nested-`fn` boundary, so `count` reads as non-recursive and was chased forever — the inner `walk`'s
/// `depth` was never threaded across the re-entry (it re-seeded at 0 every level), so the `depth < 32`
/// follow-gate never fired and the mutual recursion `walk → param_apply_extra_handled → walk` SIGABRT'd at
/// check (found by v-metaprogramming, sharpened by corpus-bugfix). The fix threads the inter-procedural
/// `depth` through `param_apply_extra_handled` so the gate terminates the chase. A COMPILER MUST NEVER
/// OVERFLOW ITS STACK (`self-hosting-and-bootstrap.md`) — reaching a coded decline at all IS the guard.
/// NOTE: this MUST run under the full DEBUG lib suite (`cargo test -p rcdzc --lib`); `xtask gate` uses a 64M
/// worker stack that MASKS the overflow.
#[test]
fn a_selfcall_hidden_in_a_nested_fold_closure_declines_not_a_stack_overflow() {
    use crate::testkit::parse;
    let src = "(do \
               (def (count node) \
                 (match node \
                   ((Ast.List es) (List.fold es 1 (fn (acc e) (+ acc (count e))))) \
                   (_ 1))) \
               (def (main) (count (quote (f 1)))) (export main))";
    // The point is that compilation TERMINATES (bounded effects walk) rather than SIGABRT'ing — it declines
    // cleanly (here at the `List.fold` member, once the effects pass no longer overflows).
    assert!(
        compile_component(&crate::codec::encode(&parse(src))).is_err(),
        "a recursive self-call hidden in a nested fold closure must decline, not overflow the compile stack"
    );
}

// (a_selfcall_gated_in_an_if_condition_declines_not_hoisted — the 3 nested-conditional self-call safe-rejects
//  (nested-if-in-cond, nested-if-in-match-scrutinee, and-short-circuit) — migrated to corpus 14b-effects-and-handlers
//  as (declines (message "not reducible by the tail-resumptive fold")) cases, 2026-08-30 delanguaging handoff.)

/// The genuine-mismatch GUARD for the handler state-binder-typing fix (the inline-Qty-resume fold, whose
/// value is pinned in the corpus): the fix must NOT weaken the real CDZ0201 next-state/seed check. An `Int64`-seeded
/// handler that resumes with a `String` next-state STILL rejects — the fix only supplies the binder's type;
/// a definite clash between the (now correctly-typed) next-state and the seed still faults.
#[test]
fn an_int_seeded_handler_resuming_with_a_string_next_state_still_rejects_cdz0201() {
    use crate::testkit::parse;
    let src = "(do (effect St (op tick (-> Unit Int64))) \
        (def (main (: a Int64)) (handle St a ((tick (u) s (resume s \"x\"))) (St.tick))) \
        (export main))";
    let err = compile_component(&crate::codec::encode(&parse(src)))
        .expect_err("a String next-state under an Int64 seed must still reject CDZ0201");
    assert_eq!(
        err.code.as_deref(),
        Some("CDZ0201"),
        "a genuine next-state/seed mismatch must still fault CDZ0201, got: {}",
        err.message
    );
}

/// A recursive performer whose recursion RESULT feeds a separate HELPER call now COMPILES (was a
/// "parameter reference has no local slot" decline at emit). `walk` recurses, then a match arm feeds the
/// recursion result AND a fresh `St.get` to a pure helper `combine`. The specializer threads `walk`'s body
/// into `walk#eff`, but threading returns unchanged subtrees AS-IS, so the spec body could SHARE an
/// ORIGINAL-def param occurrence (the `n` reached through the perform-arm-threaded `(St.get)` state
/// `(+ (. t 1) n)`). `core_of` memoizes by node id, so a shared original node carries a
/// `Core::Param{ORIGINAL binder}` with no slot in the specialized function → the decline when `combine`
/// inlines + lowers it. Fixed by deep-fresh-copying the threaded spec body so every node is a fresh id that
/// re-resolves against the spec signature (its own slots). This is the shape EVERY real demand-query
/// compiler takes (a node's children demanded, then combined by a pure helper), so it must fold. Pins that
/// it COMPILES with no slot-less-param leak; the run value (`main(3)` = 10) is verified by the corpus case
/// `a recursive performer whose recursion result feeds a HELPER call folds…` via cdz-run.
#[test]
fn a_recursive_performer_feeding_its_result_to_a_helper_call_folds() {
    use crate::testkit::parse;
    let src = "(do \
        (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit))) \
        (def (walk (: n Int64)) \
          (if (= n 0) (St.get) \
            (let ((lt (walk (- n 1)))) \
              (match (St.put n) (_ (combine lt (St.get))))))) \
        (def (combine (: a Int64) (: b Int64)) (+ a b)) \
        (def (main (: k Int64)) \
          (handle St 0 \
            ((get (u) s (resume s s)) \
             (put (v) s (resume unit (+ s v)))) \
            (walk k))) \
        (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some(),
        "a recursive performer feeding its result to a helper call must compile (no slot-less spec param)"
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("no local slot")),
        "no 'parameter reference has no local slot' decline"
    );
}

/// A capture-once closure driven by a RECURSIVE higher-order fn now COMPILES (was a "parameter reference
/// has no local slot" ICE at emit — chr1). The closure `(fn (x) (* a x))` captures `a = (St.next)`; the
/// capture-once fold discharges the draw and rewrites the capture as a chain of SYNTHESIZED alias bindings
/// (`#a → #seed → n`) that bottoms out at main's enclosing param `n`. Because `n` escapes into the
/// recursive `drive` (which cannot inline the closure), `collect_captures` must box it into the closure
/// env — but it chased the ref alias only ONE level (saw `Ref{#seed}`, not a `Param`), then treated the
/// non-user synthesized `#a`/`#seed` node as a global constant and skipped it, leaving `n` to lower as a
/// slot-less `Core::Param`. Fixed by chasing the synthesized alias chain to its terminal in
/// `collect_captures`: a chain that bottoms out at an enclosing param IS a capture, keyed by the original
/// body occurrence. Pins that it COMPILES with no slot-less-param leak; the run value (`main(5)` = 6375)
/// and the residual 1-object closure leak are checked by the corpus case `chr1 a capture-once closure
/// driven by a RECURSIVE higher-order fn folds…` via cdz-run.
#[test]
fn a_capture_once_closure_through_a_recursive_hof_folds_via_alias_chain_capture() {
    use crate::testkit::parse;
    let src = "(do \
        (effect St (op next (-> Int64))) \
        (def (drive (: f (-> Int64 Int64)) (: k Int64)) \
          (if (= k 0) 0 (+ (f k) (drive f (- k 1))))) \
        (def (main (: n Int64)) \
          (handle St n ((next () s (resume s (+ s 1)))) \
            (let ((g (let ((a (St.next))) (fn ((: x Int64)) (* a x))))) \
              (drive g 50)))) \
        (export main))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some(),
        "chr1 must compile (no slot-less closure capture)"
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message.contains("no local slot")),
        "no 'parameter reference has no local slot' decline"
    );
}

/// A TUPLE-INDEX PROJECTION of a captured tuple in a closure body now COMPILES (was a "parameter
/// reference has no local slot" ICE — hcx1, the effects-free/recursion-free face of the chr1 ICE class).
/// `(fn (q) (+ q (. a 0)))` captures the enclosing `let`-bound tuple `a = (tuple n 7)`. `collect_captures`
/// correctly records the capture of `a`, but the tuple-projection fold (lower.rs) reduced `(. a 0)`
/// THROUGH the captured `let` to its element `n` — an enclosing param NOT itself captured — lowering it
/// to a slot-less `Core::Param` in the lifted closure body. Fixed by suppressing the projection fold when
/// the operand is a captured occurrence (`db.captured_ref`): emit a runtime `Core::Proj` that reads the
/// element from the captured tuple env cell instead of inlining the element. Pins that it COMPILES with no
/// slot-less-param leak; the run value (`f(1)` applied to 5 = 6) is checked by the corpus case `hcx1 …
/// FOLDS …` (21-host-closures) via the host-closure harness. Returning the tuple WHOLE (hcp1) and LIST
/// captures (hcp2/hcp3) are the boundary controls — this fold-suppression only fires for a captured operand.
#[test]
fn a_captured_tuple_projection_in_a_closure_body_folds_via_runtime_proj_of_the_capture() {
    use crate::testkit::parse;
    // Both the FLAT projection `(. a 0)` (hcx1) and the NESTED projection `(. (. a 0) 0)` (hcx2, whose
    // outer fold would otherwise reduce THROUGH the inner projection to the captured tuple's element)
    // must compile — the fold is suppressed at any projection depth whose base is a captured operand.
    for (label, src) in [
        (
            "flat",
            "(do (def (f (: n Int64)) (let ((a (tuple n 7))) (fn ((: q Int64)) (+ q (. a 0))))) (export f))",
        ),
        (
            "nested",
            "(do (def (f (: n Int64)) (let ((a (tuple (tuple n 1) 7))) (fn ((: q Int64)) (+ q (. (. a 0) 0))))) (export f))",
        ),
    ] {
        let out = crate::compile::compile(
            &[crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "f",
                crate::codec::encode(&parse(src)),
            )],
            &[crate::backend::Target::Wasm],
        );
        assert!(
            out.artifact(crate::backend::Target::Wasm.artifact_kind())
                .is_some(),
            "{label} captured-tuple projection must compile (reads the env cell, no slot-less param)"
        );
        assert!(
            !out.diagnostics
                .iter()
                .any(|d| d.message.contains("no local slot")),
            "{label}: no 'parameter reference has no local slot' decline"
        );
    }
}

/// A mutual-group DEMAND-PERFORM-DEMAND arm inside a `let`-wrapped dispatch now FOLDS (was a tail-resumptive
/// decline that blocked compiler-ml's lazy-DB spine). `demand`/`cache`/`compute` are a mutual SCC over a
/// per-node state effect; `compute`'s arm is `(let ((a (demand child))) (match (St.put …) (_ (demand child))))`
/// — a `let` whose INIT is a mutual-partner call, whose BODY is a dispatch whose arm performs then calls a
/// partner AGAIN (demand-perform-demand). The group pre-check `group_multivalue_leaves_threadable` had only
/// `if`/`match`/leaf arms, so this `(let inits dispatch)` fell to the LEAF case → `reentrant_call_under_
/// conditional` saw the partner `demand` under the arm's `match` → declined the whole SCC up front, before
/// `thread_returning_tuple` (which ALREADY descends a let-wrapped dispatch, the #12 arm) could thread it.
/// Fixed by giving the group pre-check the same `(let inits dispatch)` arm its single-performer twin
/// `multivalue_leaves_threadable` carries: inits on the unconditional strict spine + body dispatch leaves
/// group-threadable. Pins that it COMPILES; the run value is checked below (get always misses → force compute;
/// kids returns a smaller id until 0; demand(3) folds down the finite spine to the leaf id 0).
#[test]
fn a_mutual_group_demand_perform_demand_in_a_let_wrapped_dispatch_folds() {
    use crate::testkit::parse;
    let src = "(do \
        (effect St (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit)) \
                   (op kids (-> Int64 (Option Int64)))) \
        (def (demand (: id Int64)) \
          (match (St.get id) \
            (((. Option Some) v) v) \
            (((. Option None) u) (cache id (compute id))))) \
        (def (cache (: id Int64) (: v Int64)) (match (St.put (tuple id v)) (_ v))) \
        (def (compute (: id Int64)) \
          (match (St.kids id) \
            (((. Option Some) childId) \
              (let ((a (demand childId))) \
                (match (St.put (tuple id a)) (_ (demand childId))))) \
            (((. Option None) u) id))) \
        (def (run (: root Int64)) \
          (handle St root \
            ((get (id) s (resume (. Option None) s)) \
             (put (pair) s (match pair ((tuple x y) (resume unit s)))) \
             (kids (id) s (resume (if (<= id 0) (. Option None) (Some (- id 1))) s))) \
            (demand root))) \
        (export run))";
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(src)),
        )],
        &[crate::backend::Target::Wasm],
    );
    // The group fold specializes the SCC (produces a component) and does NOT decline. Before the fix the group
    // pre-check declined the let-wrapped demand-perform-demand arm ("not yet reducible by the tail-resumptive
    // fold"). The RUN value (demand(3) = 0, folding down the runtime-opaque spine) is verified by the corpus
    // case `a mutual-group demand-perform-demand arm in a let-wrapped dispatch folds…` via cdz-run.
    assert!(
        out.artifact(crate::backend::Target::Wasm.artifact_kind())
            .is_some(),
        "the group let-dispatch pre-check arm must let a mutual-group demand-perform-demand SCC compile",
    );
    assert!(
        !out.diagnostics
            .iter()
            .any(|d| d.message == crate::diag::HANDLER_NOT_REDUCIBLE_DECLINE),
        "no tail-resumptive-fold decline on the demand-perform-demand mutual group",
    );
}

// --- HOST-COMPOSITION INVARIANT boundary (§4.4: a reified/duplicated continuation must NOT span a host
// call or an outer handler's effect). These are documented in 14-effects-and-handlers.sexp as a "clean
// decline" but were not pinned by any test — so a future fold change that admitted the multi-shot /
// re-reducing path over such a body could silently DOUBLE the escaping effect (run a host call or an
// outer-handler op twice per resume) with nothing to catch it. The multi-shot fold is repeatedly
// miscompile-prone; these lock the boundary as a clean decline until the E5 frame machinery lands. ---

// (4 multi-shot continuation safe-reject tests — spanning-a-host-call, reaching-an-outer-handler-effect,
//  outer-effect-via-an-if-branch, spanning-a-host-call-in-a-do — migrated to corpus 14b-effects-and-handlers
//  as (declines (message "not reducible by the tail-resumptive fold")) cases, 2026-08-30 delanguaging handoff
//  from v-rcdzc-test-shrink. They now emit CDZ0900; the corpus message-pin preserves the clean-decline guarantee.)

#[test]
fn a_bin_build_operand_referencing_a_do_def_under_a_handle_binds_it_not_unbound() {
    use crate::testkit::parse;
    // F2 (breaker, corpus-bugfix routed 2026-07-28): a `(bin (u8 (UInt8.wrap a)))` BUILD operand that
    // references a handle-body do-local `(def a …)` declined CDZ0101 "unbound a" — while the IDENTICAL program
    // with a tuple/record/list/Set operand bound it fine. ROOT: the handler fold RE-PARENTS the do-def's frame
    // subtree (lifting the leading do-defs under a fresh `let`), but the bin operand's `a` is a LOAD-TIME node
    // that ascends via its precomputed scope-skip pointer whose `from` is the re-parented frame-def; its LIVE
    // `child_ix_of` then reads the NEW parent, so `do_local_binds`'s window `ix-1` excluded the preceding
    // do-def and the reference read unbound. FIX: when `from`'s live child-index does not land on `from` within
    // this `do`'s forms (a re-parent), recover the TRUE window by finding `from`'s position by IDENTITY.
    // `a = Src.next` seeded 10, wrapped into a 1-byte bin, `Bytes.at 0` → 10.
    let repro = "(do \
        (effect Src (op next (-> Unit UInt8))) \
        (def (main (: x UInt8)) \
          (handle Src 10 ((next (u) s (resume s (+ s x)))) \
            (do (def a (Src.next)) (def frame (bin (u8 (UInt8.wrap a)))) \
                (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1))))) \
        (export main))";
    // The FIX is a resolve/scope change: the guard is that the program COMPILES (before the fix it declined
    // CDZ0101 unbound `a` at the bin operand) and emits a VALID module. (A run needs the value-heap runtime
    // this unit-test linker lacks for the bin/Bytes heap ops — the corpus pin gates the runtime value ×3.)
    let bytes = compile_component(&crate::codec::encode(&parse(repro))).expect(
        "a bin build operand referencing a handle-body do-def must bind it (no CDZ0101 unbound)",
    );
    assert!(
        wasmparser::validate(&bytes).is_ok(),
        "the bin-operand-over-a-do-def program must emit a valid module: {:?}",
        wasmparser::validate(&bytes).err()
    );
    // CONTROL — the perform-free twin (breaker's minimized witness): a NON-performing do-def `(def a (+ x 1))`
    // under the same handle, fed to the same bin, must ALSO compile (the trigger is do-def×bin×handle-fold, not
    // performing-ness — the fold runs regardless, and the re-parent staleness is what broke the bin operand).
    let perf_free = "(do \
        (effect Src (op next (-> Unit UInt8))) \
        (def (main (: x UInt8)) \
          (handle Src 10 ((next (u) s (resume s (+ s x)))) \
            (do (def a (+ x 1)) (def frame (bin (u8 (UInt8.wrap a)))) \
                (match (Bytes.at frame 0) ((Some v) v) ((None _u) -1))))) \
        (export main))";
    let pf = compile_component(&crate::codec::encode(&parse(perf_free)))
        .expect("a perform-free do-def fed to a bin operand under a handle must bind too");
    assert!(
        wasmparser::validate(&pf).is_ok(),
        "the perform-free twin must also emit a valid module"
    );
}

// (an_unannotated_exported_parameter_declines migrated to corpus 07-type-system: "an exported
// function with an unannotated parameter is rejected as ambiguous" — CDZ0201 (message "ambiguous"),
// (do (def (id x) x) (export id)). Deleted its /// doc block too to avoid an orphaned-doc clippy red.)

// ── runtime integer ops, width-generic: the full operator set over boundary parameters ──────────
//
// Every op below runs over RUNTIME operands (exported parameters), so it exercises the emitted machine
// instructions + guards, NOT the constant fold (the corpus `(call …)` cases prove the folded-then-run
// agreement; these pin the emitted path directly and, crucially, the TRAP behavior a folded case can't
// reach without a compile-time-provable input). Each helper compiles `(def (f <params>) <body>)` and
// runs `f` under wasmtime with the given args. Widths 8/16/32/64 (the aliased, boundary-representable
// ones) both signednesses; the emitter is width-generic (a value promotes to the smallest slot that
// holds it, computes there, then range-checks back to its N-bit type).
mod runtime_ops;

// ── runtime functions + recursion (ANF step 2 / B1): a recursive call is a real wasm call ─────────
//
// A recursive function cannot β-reduce to a normal form at compile time, so it is emitted as its own
// wasm function and its self-call is a `Core::Call`. These run whole annotated-recursive PROGRAMS under
// wasmtime — the end-to-end proof that reachability emits the callee, the call ABI is right, and the
// recursion terminates. (The corpus's UNANNOTATED recursive functions stay `todo` until the connected
// parameter solve, A2; an annotated signature is determined by absorption — see `infer::def_scheme`.)
mod recursion;

// ── the match engine: scalar scrutinee, literal + wildcard arms (step 3a) ─────────────────────────
//
// A `(match scrutinee (pattern body)…)` over a SCALAR scrutinee — the build-order Stage 4 "const-fold
// matching + scalar runtime first, before the value heap." A constant scrutinee folds to the selected
// arm; a runtime scalar emits a chain of `if` probes. Well-formedness (a type-mismatched pattern, a
// non-exhaustive match) is checked STRUCTURALLY, before the fold. These run whole programs under
// wasmtime + assert the rejections.
mod match_engine;

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
// ── span-free diagnostics: a fault names a user NODE, never a prelude/synthesized id ─────────────
//
// The compiler holds no source positions; a diagnostic carries the `StructId` (as `node`) the fault
// is about, and the front-end (which holds the span table keyed by that identity) maps it to a text
// region. These pin (a) that a fault anchors to a real user node, and (b) the boundary invariant: a
// prelude/synthesized node id NEVER reaches a diagnostic.
mod diagnostics;

// ── Stage 1: let + records (compile-time folded) ────────────────────────────────────────────────
//
// Each case mirrors a `05-compound-types.sexp` / `02-binding-and-control.sexp` witness. A record
// whose field is read FOLDS to a scalar (no value heap), so the component runs to that scalar; a
// record used as a runtime value declines (needs the heap, a later stage). Programs are built with
// the test s-expr reader in `testkit`.
mod stage1;

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
mod r0;

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
mod r1_reference;

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
mod r2_runtime_resource;

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
mod bench;

// ── the sidecar request list: driving a compilation as Emit + Query requests ─────────────────────
//
// The sidecar generalizes `compile`'s `targets` into a request list crossing as one more kinded INPUT
// artifact. These drive the WHOLE `compile()` path (not the unit codec tests in `sidecar.rs`): a
// request list in, artifacts + diagnostics out, selected by kind. They pin the load-bearing claims of
// `DESIGN-sidecar-api.md`: a Query is a total, pure fact read (answers even when emit fails, never
// denies a component), an Emit request is today's `Target` reached through the list, and the no-sidecar
// path is unchanged.
mod sidecar_driven;

/// Debug information (D0 of `DESIGN-debug-info-rcdzc.md`) — the wasm `name` custom section, enabled by
/// a Mode-E `EMIT_WASM_DEBUG` sidecar request (`Target::WasmDebug`). Two load-bearing properties: the
/// `name` section actually carries the source function names (readable frames), and it is INERT +
/// STRIPPABLE — the debug component stripped of custom sections is byte-identical to the plain one (the
/// §5 reproducibility anchor). The strip is done with `wasm-tools` (the same section-remover the spec
/// mandates); the test skips gracefully if `wasm-tools` is not on PATH.
#[cfg(test)]
mod debug_info;

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
mod closure_host_resource;

/// U13 — RECLAMATION across the cross-component boundary: a peer-returned COMPOUND that the consumer
/// projects a scalar field out of must be `drop`'d (rc--, cascade-reclaim) after the borrowing read, or it
/// LEAKS until run-end (a leak for a long-lived host). The `Core::Proj` emit reclaims the aggregate when it
/// is a fresh OWNED temporary (`heap_operand_ownership` == Owned — a peer/host call result, a constructor)
/// and the element is scalar; a projection off a BORROWED binding (a param the owner reclaims) drops nothing.
#[test]
fn u13_a_scalar_projection_off_an_owned_temporary_reclaims_it() {
    use crate::backend::wasm::select::collect_used_ops;
    use crate::db::Db;
    use crate::testkit::parse;
    // OWNED temporary: a peer-bound effect `P.pair x` returns a fresh `(Tuple Int64 Int64)` the consumer
    // owns; `(. (P.pair x) 0)` reads element 0 (scalar) and must reclaim the tuple → `drop` is emitted.
    let src = "(do \
        (effect P (op pair (-> Int64 (Tuple Int64 Int64)))) \
        (bind P \"cadenza:pairs/api\") \
        (def (main (: x Int64)) (host (P) (. (P.pair x) 0))) \
        (export main))";
    let mut db = Db::load(parse(src));
    let main = db
        .defs
        .iter()
        .find(|d| d.name == "main")
        .and_then(|d| d.body)
        .expect("main");
    let mut ops = std::collections::BTreeSet::new();
    collect_used_ops(&mut db, main, &mut ops);
    assert!(
        ops.contains("drop"),
        "a scalar projection off an owned peer-returned tuple reclaims it (drop); got {ops:?}"
    );
    assert!(
        ops.contains("arr-get"),
        "the projection still reads the element via arr-get; got {ops:?}"
    );

    // BORROWED binding: projecting element 0 out of a PARAMETER tuple `t` reclaims NOTHING — `t`'s owner
    // (the caller) reclaims it, so dropping here would be a double-free. No `drop` from the projection.
    let src2 = "(do (def (fst (: t (Tuple Int64 Int64))) (. t 0)) (export fst))";
    let mut db2 = Db::load(parse(src2));
    let fst = db2
        .defs
        .iter()
        .find(|d| d.name == "fst")
        .and_then(|d| d.body)
        .expect("fst");
    let mut ops2 = std::collections::BTreeSet::new();
    collect_used_ops(&mut db2, fst, &mut ops2);
    assert!(
        !ops2.contains("drop"),
        "a projection off a BORROWED param must NOT drop it (the owner reclaims it); got {ops2:?}"
    );

    // NESTED-COMPOUND element off an OWNED temporary (U14): projecting the inner tuple out of a peer-
    // returned nested tuple `dup`s the returned child (rc++) THEN drops the parent — so both `dup` and
    // `drop` are imported (the child survives the parent's reclamation; no use-after-free).
    let src3 = "(do \
        (effect P (op nest (-> Int64 (Tuple (Tuple Int64 Int64) Int64)))) \
        (bind P \"cadenza:nest/api\") \
        (def (main (: x Int64)) (host (P) (. (. (P.nest x) 0) 1))) \
        (export main))";
    let mut db3 = Db::load(parse(src3));
    let main3 = db3
        .defs
        .iter()
        .find(|d| d.name == "main")
        .and_then(|d| d.body)
        .expect("main3");
    let mut ops3 = std::collections::BTreeSet::new();
    collect_used_ops(&mut db3, main3, &mut ops3);
    assert!(
        ops3.contains("dup") && ops3.contains("drop"),
        "a nested-compound projection dup's the child then drops the parent; got {ops3:?}"
    );
}

/// X1 — the cross-component composition ORACLE (`DESIGN-cross-component-interop-rcdzc.md`).
///
/// De-risks the shared-runtime cross-component transport BEFORE any compiler change, the oracle-first
/// move the resource/closure verticals used at every seam. Two SEPARATELY-built Cadenza-shaped
/// components — a provider A that exports `f`, and a consumer B that IMPORTS A's interface and calls
/// it — are composed by a `ComponentBuilder` into one component whose sole export is B's `main`. The
/// point being proved: (X1a) a component can bind a PEER component's export and call across the live
/// boundary; (X1b) two program cores can share ONE value-heap runtime instance, so a handle one
/// produces is meaningful to the other (the load-bearing rule in component-abi.md §A Cross-Component
/// Handle Is Meaningful Only In The Shared Runtime Instance).
mod cross_component_oracle;

/// A variant whose PAYLOAD TYPE is written with the LOWERCASE compound-type alias `(tuple …)` is read as a
/// real single-payload variant, not mistakenly as NULLARY — the v-patterns-routed type-decl-reading bug.
/// `(type P (Mk (tuple Int64 Int64)))` then `(P.Mk (tuple 4 5))` must construct and match. The reader's
/// implicit-type-parameter collector (`collect_type_params`) descended a `(Head arg…)` payload uniformly
/// and, seeing the LOWERCASE alias head `tuple`, harvested it as a spurious type PARAMETER (the free-
/// lowercase-name rule that legitimately picks up `a` in `(Some a)`). That made `P` bogus-generic over
/// `tuple`, so the `Mk` constructor's scheme mis-reduced and read as ARITY 0 → CDZ0201 "a nullary variant
/// takes the unit value, but a payload of type (Tuple Int64 Int64) was applied" on the construction. The
/// fix excludes the lowercase compound-type aliases (`tuple`/`list`/`map`, plus `record` with its label-
/// skipping) from param collection — the lowercase twin of the capital-`Record`/`Qty` head exclusions
/// already present. (An ML-surface corpus pin is deferred: the lowercase-alias payload renders as the
/// comma tuple `(Int64, Int64)` on the ML surface and does not yet round-trip back to the alias spelling —
/// a separate v-syntax concern — so this is pinned as a Rust end-to-end test instead.)
#[test]
fn a_variant_with_a_lowercase_tuple_alias_payload_reads_as_a_payload_variant_not_nullary() {
    use crate::testkit::parse;
    let validate = |bytes: &[u8]| {
        let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
        v.validate_all(bytes)
            .expect("the payload-variant component validates");
    };
    // Lowercase `(tuple …)` payload — the bug's exact shape: construction + match must COMPILE to a valid
    // component (the bug REJECTED them CDZ0201, reading `Mk` as nullary). The regression is the compile; the
    // `(match (P.Mk (tuple 4 5)) ((P.Mk t) 9))` run to 9 is incidental (a constant fold). This stays a
    // compile+validate pin, not a corpus case: the lowercase-alias payload does not round-trip through the ML
    // surface (it renders as the comma tuple `(Int64, Int64)`), so the corpus ML round-trip cannot carry it.
    let low = "(module m (type P (Mk (tuple Int64 Int64))) (def (main) (match (P.Mk (tuple 4 5)) ((P.Mk t) 9))) (export main))";
    validate(
        &compile_component(&crate::codec::encode(&parse(low)))
            .expect("lowercase-alias payload compiles"),
    );
    // The capital `(Tuple …)` spelling always worked — its head is Capitalized, so the lowercase-name param
    // filter never harvested it. Pin it alongside so the two spellings are proven equivalent.
    let cap = "(module m (type P (Mk (Tuple Int64 Int64))) (def (main) (match (P.Mk (tuple 4 5)) ((P.Mk t) 9))) (export main))";
    validate(
        &compile_component(&crate::codec::encode(&parse(cap)))
            .expect("capital-Tuple payload compiles"),
    );
}

// (a_nullary_variant_applied_to_a_payload_names_the_variant_and_the_fix migrated to corpus
// 05-compound-types: "a nullary variant applied to a payload names the variant and that it cannot be
// applied" + the qualified-`Option.None` twin — CDZ0201 (message "`None` is nullary")(cannot be applied)
// (construct it as `None`). No white-box residue; fix facet source-proven.)

/// `eval::fold_ctor_match` folds a `(match (Ctor payload) ((Ctor v) body))` over a VISIBLE constructor
/// scrutinee to the arm body with the payload binder substituted — the resolve-time case-of-known-ctor
/// fold v-effects' DES-inc-4 classifier calls (a stored resume-thunk in a compound, match-extracted before
/// apply). SLICE 1: single-payload single-binder. Verified by folding `(match (Box.Box 41) ((Box.Box t)
/// (+ t 1)))` and checking the folded body reduces to the constant `42` — i.e. the `SumPayload{scrutinee}`
/// binder `t` correctly projected the payload `41`. Also asserts the guards: a match over a NON-ctor
/// scrutinee (a bare param) and a match whose arm has no bare-name binder both return `None` (left runtime).
#[test]
fn fold_ctor_match_folds_a_visible_ctor_match_to_the_substituted_arm_body() {
    use crate::testkit::parse;
    // The def body IS the match over a visible ctor `(Box.Box 41)`.
    let src = "(module m (type Box (Box Int64)) \
        (def (probe) (match (Box.Box 41) ((Box.Box t) (+ t 1)))) (def (main) 0) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("probe").expect("probe def");
    let body = db.defs[d].body.expect("probe body");
    // fold_ctor_match returns the arm body with `t` projected to the payload `41`.
    let folded = crate::eval::fold_ctor_match(&mut db, body)
        .expect("a match over a visible ctor with a single-binder arm must fold");
    // The folded `(+ t 1)` with `t := 41` lowers to the constant `Core::ConstInt(42)` — i.e. the
    // `SumPayload{scrutinee}` binder `t` correctly projected the payload `41`, then constant arithmetic
    // folded `41 + 1`.
    let v = match crate::lower::core_of(&mut db, folded) {
        crate::core::Core::ConstInt(iv) => iv.to_i64(),
        _ => None,
    }
    .expect("the folded body must lower to a constant int");
    assert_eq!(
        v, 42,
        "fold should project the payload 41 into `(+ t 1)` → 42"
    );

    // GUARD: a match over a NON-ctor scrutinee (a bare param) does NOT fold — stays a runtime match.
    let param_src = "(module m (type Box (Box Int64)) \
        (def (probe (: b Box)) (match b ((Box.Box t) (+ t 1)))) (def (main) 0) (export main))";
    let mut db2 = crate::db::Db::load(parse(param_src));
    let d2 = db2.def_by_name("probe").expect("probe2");
    let body2 = db2.defs[d2].body.expect("probe2 body");
    assert!(
        crate::eval::fold_ctor_match(&mut db2, body2).is_none(),
        "a match over a runtime param scrutinee must NOT fold (no visible ctor)"
    );
}

/// `eval::fold_ctor_match` SLICE 2: a variant whose single payload is a TUPLE type, destructured by a tuple
/// sub-pattern `(Ctor (tuple v0 v1 …))` — each binder `vi` resolves to `SumPayload{scrutinee, [Payload,
/// Elem(i)]}` (a payload-read into tuple element `i`), which slice 1 declined. Verified by folding `(match
/// (P.Mk (tuple 40 2)) ((P.Mk (tuple a b)) (+ a b)))` over a visible ctor whose SINGLE payload is the tuple,
/// and checking the folded body lowers to the constant `42` — i.e. `a` projected element 0 (=40) and `b`
/// element 1 (=2). This is v-DES's real pqueue-entry shape (`(PQCons (tuple wake kb rest))`, whose declared
/// payload is `(Tuple …)`); the caller's loop folds any inner single-payload match afterward via slice 1.
/// Guards: a NON-visible (runtime-param) scrutinee still declines.
#[test]
fn fold_ctor_match_slice2_folds_a_tuple_destructured_multi_payload_ctor_match() {
    use crate::testkit::parse;
    // A variant whose SINGLE payload is a `(Tuple Int64 Int64)`, so the ctor arg is one explicit tuple,
    // destructured by the arm's `(tuple a b)` sub-pattern (v-DES's `(PQCons (Tuple …))` shape). The def body
    // IS the match over the visible ctor `(P.Mk (tuple 40 2))`.
    let src = "(module m (type P (Mk (Tuple Int64 Int64))) \
        (def (probe) (match (P.Mk (tuple 40 2)) ((P.Mk (tuple a b)) (+ a b)))) (def (main) 0) (export main))";
    let mut db = crate::db::Db::load(parse(src));
    let d = db.def_by_name("probe").expect("probe def");
    let body = db.defs[d].body.expect("probe body");
    let folded = crate::eval::fold_ctor_match(&mut db, body)
        .expect("a match over a visible tuple-payload ctor with a tuple sub-pattern must fold");
    // The folded `(+ a b)` with `a := 40`, `b := 2` lowers to the constant `42` — i.e. each tuple binder's
    // `SumPayload{scrutinee, [Payload, Elem(i)]}` correctly projected element `i` of the payload tuple.
    let v = match crate::lower::core_of(&mut db, folded) {
        crate::core::Core::ConstInt(iv) => iv.to_i64(),
        _ => None,
    }
    .expect("the folded body must lower to a constant int");
    assert_eq!(
        v, 42,
        "slice 2 should project payload tuple elements 40, 2 into `(+ a b)` → 42"
    );

    // GUARD: a runtime-param scrutinee does NOT fold (no visible ctor) — stays a runtime tuple-match.
    let param_src = "(module m (type P (Mk (Tuple Int64 Int64))) \
        (def (probe (: p P)) (match p ((P.Mk (tuple a b)) (+ a b)))) (def (main) 0) (export main))";
    let mut db2 = crate::db::Db::load(parse(param_src));
    let d2 = db2.def_by_name("probe").expect("probe2");
    let body2 = db2.defs[d2].body.expect("probe2 body");
    assert!(
        crate::eval::fold_ctor_match(&mut db2, body2).is_none(),
        "a multi-payload match over a runtime param scrutinee must NOT fold (no visible ctor)"
    );
}

/// `eval::fold_ctor_match` folds a match over a NULLARY visible ctor to the selected arm's body, for BOTH
/// spellings of the nullary constructor:
/// - APPLIED `(PQNil ())` resolves to `Apply{head, args:[unit]}` (`args.len() == 1`) → slice-1 path, the
///   unit payload substituted into the arm's `_` wildcard (binds nothing), body intact.
/// - BARE `PQNil` (no explicit `()`) resolves to `Resolved::Member` carrying the disc, with NO payload
///   (`args.is_empty()`) → slice-0 path. `reduce_to_visible_ctor`'s `Member` arm recognizes it; without
///   that arm the bare form declined (it is not an `Apply`), silently opacifying the base-arm fold.
///
/// Both fold `(match <PQNil> ((PQNil _) 7) ((PQCons (tuple a r)) a))` to the constant `7` — the base arm,
/// NOT `PQCons`. This pins the invariant v-effects' recursive-callee-base-arm unfold relies on
/// (`des-inc4-recursive-insert`: `pins(PQNil, …)` selects its non-recursive PQNil base arm — and the
/// substituted scrutinee is the BARE `PQNil` Member form, exactly the slice-0 shape). A future change that
/// stops folding either spelling re-opacifies that reach.
#[test]
fn fold_ctor_match_folds_a_nullary_ctor_scrutinee_to_the_selected_base_arm() {
    use crate::testkit::parse;
    for (label, scrut) in [("applied", "(PQ.PQNil ())"), ("bare", "PQ.PQNil")] {
        let src = format!(
            "(module m (type PQ PQNil (PQCons (Tuple Int64 PQ))) \
            (def (probe) (match {scrut} ((PQ.PQNil _) 7) ((PQ.PQCons (tuple a r)) a))) \
            (def (main) 0) (export main))"
        );
        let mut db = crate::db::Db::load(parse(&src));
        let d = db.def_by_name("probe").expect("probe def");
        let body = db.defs[d].body.expect("probe body");
        let folded = crate::eval::fold_ctor_match(&mut db, body)
            .unwrap_or_else(|| panic!("[{label}] a match over a visible nullary ctor must fold"));
        // The folded body is the base arm `7` — the nullary ctor selected the `PQNil` arm, not `PQCons`.
        let v = match crate::lower::core_of(&mut db, folded) {
            crate::core::Core::ConstInt(iv) => iv.to_i64(),
            _ => None,
        }
        .unwrap_or_else(|| panic!("[{label}] the folded body must lower to a constant int"));
        assert_eq!(
            v, 7,
            "[{label}] a nullary-ctor scrutinee folds to its matching base arm (7), not PQCons"
        );
    }
}

/// The IR types (`Resolved`/`Ty`/`Core`) carry their variable-length payloads behind `Rc<[…]>`, NOT
/// `Arc<[…]>` — a deliberate PERFORMANCE choice, not an oversight. The entire compile runs on a SINGLE
/// scoped worker thread (`host::run_with_compiler_stack`, whose `F: Send`/`T: Send` bounds constrain only
/// the closure and its `CompileOutput` result — a `Vec<Artifact>` + `Vec<Diagnostic>`, holding no IR), so
/// a `Db` and every `Resolved`/`Ty`/`Core` value lives and dies on that one thread. The memoized queries
/// (`resolved_of`/`type_of`/`core_of`) return BY VALUE, cloning the payload's smart pointer on every
/// memo-hit; with `Arc` that clone is an ATOMIC inc + drop, and a `perf` profile of a wide program
/// (thousands of defs) measured those atomics (`__aarch64_ldadd8_rel`/`_relax`) at ~14% of the whole
/// compile. `Rc` makes the refcount a plain non-atomic add — sound because nothing shares an IR value
/// across threads — which erased that 14% (and cut `Resolved::clone` self-time roughly in half),
/// ~1.09× faster end-to-end on the 3200-def shape.
///
/// This is a COMPILE-TIME lock-in: an enum with any `Rc<[T]>` field is `!Send`, so a `<T as
/// AmbiguousIfSend<_>>::MARKER` path resolves unambiguously. A WHOLESALE revert to `Arc<[…]>` — the
/// realistic regression vector, a blanket `Rc`→`Arc` swap to make the compiler multi-threaded, which
/// silently reintroduces the atomic traffic (a `StructId`/`Ty` payload is `Send`, so an `Arc` of it is
/// `Send`) — makes the enum `Send`, makes `MARKER` AMBIGUOUS (both trait impls apply, E0283), and fails
/// the crate to compile. This is the noise-free regression signal a wall-clock test could never give for
/// a clone-cost change diluted across the rest of `check`. (Precise scope: a type is `Send` only when
/// EVERY field is `Send`, so this catches a full revert of all payloads on a given enum, not a single
/// field flipped back to `Arc` while its siblings stay `Rc` — a partial flip reintroduces only that one
/// variant's atomics and is not a realistic accidental change.)
#[cfg(test)]
mod ir_payloads_stay_rc_backed_not_arc;

/// The `is_list_byte_pairs` detector recognizes the kv `prefix-scan` result shape `list<tuple<bytes,bytes>>`
/// (a list of key-value byte pairs) — the SECOND compound host result the bytes-provider path will lift
/// (after `Option<Bytes>`). Foundation for the prefix-scan host-result lift; pins the shape match + rejects
/// near-misses so a future change can't quietly widen/narrow what the lift fires on.
#[test]
fn is_list_byte_pairs_detects_the_prefix_scan_result_shape() {
    use crate::backend::wasm::host::is_list_byte_pairs;
    use crate::ty::Ty;
    let pair = |elems: Vec<Ty>| Ty::List(Box::new(Ty::Tuple(std::rc::Rc::from(elems.as_slice()))));
    // POSITIVE: list<tuple<bytes,bytes>>.
    assert!(
        is_list_byte_pairs(&pair(vec![Ty::Bytes, Ty::Bytes])),
        "list of (bytes,bytes) tuples IS the prefix-scan shape"
    );
    // NEGATIVES:
    assert!(
        !is_list_byte_pairs(&Ty::List(Box::new(Ty::Bytes))),
        "list<bytes> (no tuple) is not byte-pairs"
    );
    assert!(
        !is_list_byte_pairs(&pair(vec![Ty::Bytes, Ty::Bytes, Ty::Bytes])),
        "a 3-tuple is not a pair"
    );
    assert!(
        !is_list_byte_pairs(&pair(vec![Ty::Bytes, Ty::int64()])),
        "tuple<bytes,int> is not byte-PAIRS"
    );
    assert!(!is_list_byte_pairs(&Ty::Bytes), "bare bytes is not a list");
    assert!(
        !is_list_byte_pairs(&Ty::Tuple(std::rc::Rc::from([Ty::Bytes, Ty::Bytes]))),
        "a bare tuple (not wrapped in a list) is not the shape"
    );
}

/// The KIND_WIT_WORLD for a kv PREFIX-SCAN reducer: export `fold.apply` (list<u8>->list<u8>) + import
/// `cadenza:agent-kernel/kv` member `prefix-scan` (key: list<u8> -> list<tuple<list<u8>,list<u8>>>, a list
/// of key-value byte pairs). Descriptors: `("list" ("tuple" ("list" (u8)) ("list" (u8))))` for the result.
fn reducer_kv_prefix_scan_world_bytes() -> Vec<u8> {
    use crate::ast::{Builder, Leaf};
    let mut b = Builder::new();
    let byte_list = |b: &mut Builder| {
        let h = b.atom_leaf(Leaf::Str("list".into()));
        let uh = b.name("u8");
        let u = b.list(vec![uh]);
        b.list(vec![h, u])
    };
    let a_in = byte_list(&mut b);
    let a_out = byte_list(&mut b);
    let apply = {
        let fh = b.name("func");
        let ph = b.name("param");
        let pn = b.name("input");
        let pnode = b.list(vec![ph, pn, a_in]);
        let rh = b.name("result");
        let rn = b.list(vec![rh, a_out]);
        let func = b.list(vec![fh, pnode, rn]);
        let mh = b.name("member");
        let mn = b.name("apply");
        b.list(vec![mh, mn, func])
    };
    let eh = b.name("export");
    let en = b.name("fold");
    let fold = b.list(vec![eh, en, apply]);
    // prefix-scan result: ("list" ("tuple" ("list" (u8)) ("list" (u8))))
    let pk = byte_list(&mut b);
    let result_ty = {
        let listh = b.atom_leaf(Leaf::Str("list".into()));
        let tuph = b.atom_leaf(Leaf::Str("tuple".into()));
        let t0 = byte_list(&mut b);
        let t1 = byte_list(&mut b);
        let tup = b.list(vec![tuph, t0, t1]);
        b.list(vec![listh, tup])
    };
    let ps = {
        let fh = b.name("func");
        let ph = b.name("param");
        let pn = b.name("key");
        let pnode = b.list(vec![ph, pn, pk]);
        let rh = b.name("result");
        let rn = b.list(vec![rh, result_ty]);
        let func = b.list(vec![fh, pnode, rn]);
        let mh = b.name("member");
        let mn = b.name("prefix-scan");
        b.list(vec![mh, mn, func])
    };
    let ih = b.name("import");
    let kn = b.name("cadenza:agent-kernel/kv");
    let kv = b.list(vec![ih, kn, ps]);
    let wh = b.name("world");
    let wn = b.name("reducer");
    let world = b.list(vec![wh, wn, fold, kv]);
    let a = b.finish(world);
    crate::codec::encode(&a)
}

/// §3c kv PREFIX-SCAN emit: a host-fused reducer calling `kv.prefix-scan` (result
/// `list<tuple<list<u8>,list<u8>>>`) EMITS through the bytes-provider path — the guest lifts the spilled
/// list-of-byte-pairs into a value-heap `List<Tuple<Bytes,Bytes>>` — and the assembled component VALIDATES
/// and LOADS on the pinned wasmtime (importing kv + runtime). The list-of-tuple host-result lift + its
/// nested `list<tuple<list<u8>,list<u8>>>` component type.
#[test]
fn a_host_fused_kv_prefix_scan_reducer_emits_and_loads() {
    use crate::testkit::parse;
    let src = "(module m \
      (effect kv (op prefix-scan (-> Bytes (List (Tuple Bytes Bytes))))) \
      (def (apply (: e (Record (ct String) (pl Bytes)))) \
        (host (kv) (do (kv.prefix-scan (. e pl)) (list (record (= op (. e ct)) (= arg (. e pl))))))) \
      (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            crate::abi::Artifact::new(
                crate::link::KIND_WIT_WORLD,
                "wit-world",
                reducer_kv_prefix_scan_world_bytes(),
            ),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "the host-fused prefix-scan reducer must emit (the list-of-byte-pairs lift): {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the prefix-scan reducer emits a component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes)
        .expect("the prefix-scan provider component validates");
    // A prefix-scan reducer marshals compound kv results through the value-heap runtime AND imports the
    // kv host interface — both witnessed by the import names in the emitted component bytes (the same
    // dep-free byte-scan the kv-import assertion below uses), no in-crate instantiation needed.
    let text = String::from_utf8_lossy(bytes);
    assert!(
        text.contains("cadenza:runtime/heap"),
        "a prefix-scan reducer marshals through the value-heap runtime"
    );
    assert!(
        text.contains("cadenza:agent-kernel/kv"),
        "the prefix-scan reducer imports the kv host interface"
    );
}

/// §3c kv DELETE is FREE: a host-fused reducer calling `kv.delete` (result BOOL — present-or-removed)
/// EMITS with no host-result lift, because a `bool` is a FLAT SCALAR at the host boundary
/// (`abi_val_type(Ty::Bool)=Some`), unlike the spilled `option<list<u8>>`/`list<tuple>` compounds. Pins
/// that delete needs zero emit work (only the world must declare it) — the co-land guarantee to v-ah.
#[test]
fn a_host_fused_kv_delete_bool_reducer_emits_and_loads() {
    use crate::ast::{Builder, Leaf};
    use crate::testkit::parse;
    let world_bytes = {
        let mut b = Builder::new();
        let byte_list = |b: &mut Builder| {
            let h = b.atom_leaf(Leaf::Str("list".into()));
            let uh = b.name("u8");
            let u = b.list(vec![uh]);
            b.list(vec![h, u])
        };
        let a_in = byte_list(&mut b);
        let a_out = byte_list(&mut b);
        let apply = {
            let fh = b.name("func");
            let ph = b.name("param");
            let pn = b.name("input");
            let pnode = b.list(vec![ph, pn, a_in]);
            let rh = b.name("result");
            let rn = b.list(vec![rh, a_out]);
            let func = b.list(vec![fh, pnode, rn]);
            let mh = b.name("member");
            let mn = b.name("apply");
            b.list(vec![mh, mn, func])
        };
        let eh = b.name("export");
        let en = b.name("fold");
        let fold = b.list(vec![eh, en, apply]);
        let dk = byte_list(&mut b);
        let boolty = {
            let bh = b.name("bool");
            b.list(vec![bh])
        };
        let del = {
            let fh = b.name("func");
            let ph = b.name("param");
            let pn = b.name("key");
            let pnode = b.list(vec![ph, pn, dk]);
            let rh = b.name("result");
            let rn = b.list(vec![rh, boolty]);
            let func = b.list(vec![fh, pnode, rn]);
            let mh = b.name("member");
            let mn = b.name("delete");
            b.list(vec![mh, mn, func])
        };
        let ih = b.name("import");
        let kn = b.name("cadenza:agent-kernel/kv");
        let kv = b.list(vec![ih, kn, del]);
        let wh = b.name("world");
        let wn = b.name("reducer");
        let world = b.list(vec![wh, wn, fold, kv]);
        let a = b.finish(world);
        crate::codec::encode(&a)
    };
    let src = "(module m \
      (effect kv (op delete (-> Bytes Bool))) \
      (def (apply (: e (Record (ct String) (pl Bytes)))) \
        (host (kv) (if (kv.delete (. e pl)) (list (record (= op (. e ct)) (= arg (. e pl)))) (list)))) \
      (export apply))";
    let out = crate::compile::compile(
        &[
            crate::abi::Artifact::new(
                crate::abi::Artifact::KIND_AST,
                "main",
                crate::codec::encode(&parse(src)),
            ),
            crate::cli::component_name_artifact("cadenza:agent-kernel/fold"),
            crate::abi::Artifact::new(crate::link::KIND_WIT_WORLD, "wit-world", world_bytes),
        ],
        &[crate::backend::Target::Wasm],
    );
    assert!(
        !out.has_error(),
        "kv.delete (bool result) emits with no lift — bool is a flat scalar: {:?}",
        out.diagnostics
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    let bytes = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("the delete reducer emits a component");
    let mut v = wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all());
    v.validate_all(bytes)
        .expect("the delete provider component validates");
}

// ── COPY-DON'T-DEPEND byte-identity bridge for the WIT schema-descriptor builders ────────────────
//
// The schema-hash-only effect identity requires that rcdzc's VERBATIM-COPIED WIT builders
// (`ast::Builder::{effect_schema_tree,wit_func_sig,wit_type_*}`) encode BYTE-IDENTICALLY to the
// originals in `cadenza-ast` (the builder the kernel builds its built-in descriptors with) — a
// userspace effect rcdzc reifies must hash to the same value the kernel computes for the same shape.
// This is the descriptor-builder analogue of the existing `codec.rs` byte-identity discipline: build
// the SAME schema tree with rcdzc's copy AND with cadenza-ast's original (reachable via the
// `cadenza-syntax` dev-dep, which re-exports `cadenza_ast::{ast, codec}`), encode each with its own
// crate's codec, and assert the bytes are equal. Any drift in a copied builder (a flipped head-kind,
// a reordered child, a dropped wrapper) reds here instead of silently splitting the effect identity.
//
// Coverage: the builders reachable from cadenza-ast (prim/list/option/unit/tuple + effect_schema_tree
// + wit_func_sig). `wit_type_record`/`wit_type_variant` are copied from the kernel (their originals are
// private to `cdz-kernel::ast_marshal`, not reachable here) — those are cross-checked by the
// vs-built-in schema-hash gate once the kernel field-order fix (v-effects MR 0cbebf470) lands on origin.
//
// Two assertions, because raw `codec::encode` bytes correctly DIFFER between the crates: cadenza-ast's
// `encode` runs a `canon::canonicalize` pre-order-renumber pass before serializing (byte identity is a
// property of that normal form, not the codec); rcdzc's copied `encode` has NO canon pass (rcdzc's
// s-expr reader always builds pre-order, so it never needed one — and a hand-built bottom-up descriptor
// is NOT in pre-order), and rcdzc emits its descriptor RAW. So raw==raw would (correctly) fail.
//
//  (1) LOGICAL TREE: same builder calls → structurally identical tree + identical head kinds (Name vs
//      Str is load-bearing for the prim-vs-compound distinction). Pins that the copy is faithful.
//  (2) ROUND-TRIP-THROUGH-CANON: rcdzc's raw descriptor bytes, DECODED then RE-ENCODED through
//      cadenza-ast's codec (i.e. canonicalized), == cadenza-ast's canonical bytes for the same tree.
//      This is the exact property the effect-identity design relies on (v-effects ruling A, 2026-08-13):
//      rcdzc emits RAW; the kernel re-canonicalizes on ingest (it codec::decodes the descriptor subtree
//      then hashes it through `effect_schema_hash_from_nodes`, which rebuilds + re-encodes via
//      cadenza-ast's canonicalizing codec), so the producer's build order is ERASED and both producers
//      hash the same canonical bytes. No `canon.rs` port in rcdzc — the kernel is the one hasher.
#[test]
fn copied_wit_builders_match_cadenza_ast_by_logical_tree_and_through_canon() {
    // Canonical render of an rcdzc arena subtree: `(head child…)`, each atom tagged by head kind
    // (`N:` a Name, `S:` a Str) so a flipped head-kind — which changes the hash — shows as a diff.
    fn render_ours(a: &crate::ast::Arenas, id: crate::ast::StructId) -> String {
        match a.get(id) {
            crate::ast::Struct::Atom(_) => {
                if let Some(n) = a.as_name(id) {
                    format!("N:{n}")
                } else if let Some(s) = a.as_str(id) {
                    format!("S:{s}")
                } else {
                    panic!("WIT descriptor atom is neither Name nor Str")
                }
            }
            crate::ast::Struct::List(children) => {
                let parts: Vec<String> = children.iter().map(|&c| render_ours(a, c)).collect();
                format!("({})", parts.join(" "))
            }
        }
    }
    fn render_theirs(a: &cadenza_syntax::ast::Arenas, id: cadenza_syntax::ast::StructId) -> String {
        match a.get(id) {
            cadenza_syntax::ast::Struct::Atom(_) => {
                if let Some(n) = a.as_name(id) {
                    format!("N:{n}")
                } else if let Some(s) = a.as_str(id) {
                    format!("S:{s}")
                } else {
                    panic!("WIT descriptor atom is neither Name nor Str")
                }
            }
            cadenza_syntax::ast::Struct::List(children) => {
                let parts: Vec<String> = children.iter().map(|&c| render_theirs(a, c)).collect();
                format!("({})", parts.join(" "))
            }
        }
    }
    // rcdzc's copied builders → an effect schema tree exercising every copied form. Keep the arena so we
    // can BOTH render its logical tree (1) AND encode its RAW bytes for the round-trip check (2).
    //
    // CRITICAL — a FRESH node per occurrence, never a reused StructId. `string` appears twice (param `k`
    // and inside `option`), so we call `wit_type_prim("string")` TWICE. Ruling A has the kernel DECODE
    // rcdzc's descriptor, and `codec::decode` enforces TREE-NESS (a shared subtree — the same StructId
    // reachable from two parents — is rejected as `NotATree`, a decode-bomb guard). The real emitter
    // (`ty_to_wit_desc`, recursive) builds fresh nodes per call for free; it MUST NOT memoize/share type
    // nodes. This test mirrors that: reusing a binding here would (correctly) fail the round-trip decode.
    let ours_arenas = {
        let mut b = crate::ast::Builder::new();
        let s64 = b.wit_type_prim("s64");
        let string_k = b.wit_type_prim("string");
        let u8 = b.wit_type_prim("u8");
        let bytes = b.wit_type_list(u8); // list<u8> — the "bytes-is-list<u8>" spelling
        let string_inner = b.wit_type_prim("string");
        let opt_string = b.wit_type_option(string_inner);
        let unit = b.wit_type_unit();
        let pair = b.wit_type_tuple(&[s64, bytes]);
        // op `run`: (param k string) (param v tuple<s64, list<u8>>) (param meta option<string>) -> unit
        let sig = b.wit_func_sig(&[("k", string_k), ("v", pair), ("meta", opt_string)], unit);
        let root = b.effect_schema_tree("demo", &[("run", sig)]);
        b.finish(root)
    };
    // cadenza-ast's ORIGINAL builders (via the cadenza-syntax re-export) → the same tree (also fresh-per-occurrence).
    let theirs_arenas = {
        use cadenza_syntax::ast::Builder as AstBuilder;
        let mut b = AstBuilder::default();
        let s64 = b.wit_type_prim("s64");
        let string_k = b.wit_type_prim("string");
        let u8 = b.wit_type_prim("u8");
        let bytes = b.wit_type_list(u8);
        let string_inner = b.wit_type_prim("string");
        let opt_string = b.wit_type_option(string_inner);
        let unit = b.wit_type_unit();
        let pair = b.wit_type_tuple(&[s64, bytes]);
        let sig = b.wit_func_sig(&[("k", string_k), ("v", pair), ("meta", opt_string)], unit);
        let root = b.effect_schema_tree("demo", &[("run", sig)]);
        b.finish(root)
    };

    // (1) LOGICAL-TREE identity — the copy is faithful (shape + head kinds).
    let ours_tree = render_ours(&ours_arenas, ours_arenas.root);
    let theirs_tree = render_theirs(&theirs_arenas, theirs_arenas.root);
    assert_eq!(
        ours_tree, theirs_tree,
        "rcdzc's verbatim-copied WIT schema-descriptor builders drifted from cadenza-ast's originals — \
         the effect schema tree differs in shape or a head kind (Name vs Str), which would split the \
         effect schema-hash. Re-sync the copy in ast.rs with cadenza-ast/src/ast.rs."
    );

    // (2) ROUND-TRIP-THROUGH-CANON identity — the property ruling A relies on. rcdzc emits RAW bytes
    // (its codec does not canonicalize); the kernel re-canonicalizes on ingest. Model that here: decode
    // rcdzc's raw bytes into a cadenza-ast arena, then re-encode through cadenza-ast's CANONICALIZING
    // codec — the result must equal cadenza-ast's own canonical bytes for the same tree. (Raw==raw would
    // correctly differ — rcdzc's build order is non-canonical — so we assert the canonicalized form, not
    // the raw form.) Both codecs share the `cdzast\x00\x01` header, so the cross-crate decode is valid.
    let ours_raw = crate::codec::encode(&ours_arenas);
    let ours_canon = {
        let decoded = cadenza_syntax::codec::decode(&ours_raw).expect(
            "rcdzc's raw descriptor bytes decode under cadenza-ast's codec (shared wire format)",
        );
        cadenza_syntax::codec::encode(&decoded)
    };
    let theirs_canon = cadenza_syntax::codec::encode(&theirs_arenas);
    assert_eq!(
        ours_canon, theirs_canon,
        "rcdzc's descriptor, once canonicalized through cadenza-ast's codec, must equal cadenza-ast's \
         canonical bytes for the same tree — this is the invariant ruling A relies on (kernel \
         re-canonicalizes rcdzc's raw descriptor on ingest, so both producers hash the same bytes). \
         A mismatch here means the effect schema-hash would split even after canonicalization."
    );
}

/// §2d STATIC bytes (`DESIGN-static-data.md` increment 1 — constancy detection, byte-neutral).
/// Pins [`crate::lower::is_constant_bytes`]: a `Core::BytesOf` whose every element folds to a
/// constant byte is a static-once candidate (true); a single RUNTIME element (a `UInt8` built from a
/// param) disqualifies the whole literal (false); the empty bytes literal is vacuously constant
/// (true); and a non-`BytesOf` node is never a constant-bytes candidate (false). This is the
/// classification the build-once GLOBAL/START emit (increment 2) keys on; a regression here would
/// either route a runtime-varying literal to a wrongly-shared global (a miscompile) or fail to hoist
/// a genuinely-constant one.
#[test]
fn is_constant_bytes_flags_all_constant_byte_literals_only() {
    use crate::db::Db;
    use crate::lower::is_constant_bytes;
    // Lower a named def's body and ask whether it is a constant-bytes static-once candidate.
    let is_const_bytes = |src: &str, def: &str| -> bool {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let body = db.defs[d].body.expect("body");
        is_constant_bytes(&mut db, body)
    };
    // A fully-constant bytes literal → a static-once candidate.
    assert!(
        is_const_bytes(
            "(module m (def (main) (Bytes.of (list 1 2 3))) (export main))",
            "main"
        ),
        "an all-constant `(Bytes.of (list 1 2 3))` must be a constant-bytes static-once candidate"
    );
    // The EMPTY bytes literal is vacuously constant (no runtime element).
    assert!(
        is_const_bytes(
            "(module m (def (main) (Bytes.of (list))) (export main))",
            "main"
        ),
        "the empty bytes literal `(Bytes.of (list))` must be a constant-bytes candidate (vacuous)"
    );
    // A single RUNTIME element (a `UInt8` built from a param) disqualifies the whole literal.
    assert!(
        !is_const_bytes(
            "(module m (def (f (: n Int64)) (Bytes.of (list (UInt8.wrap n)))) \
             (def (main) 0) (export main))",
            "f"
        ),
        "a `(Bytes.of (list (UInt8.wrap n)))` with a runtime element must NOT be a constant candidate"
    );
    // A MIXED literal (constant bytes plus one runtime element) is also not constant — one runtime
    // element disqualifies the whole value.
    assert!(
        !is_const_bytes(
            "(module m (def (f (: n Int64)) (Bytes.of (list 1 (UInt8.wrap n) 3))) \
             (def (main) 0) (export main))",
            "f"
        ),
        "a mixed constant/runtime `Bytes.of` must NOT be a constant-bytes candidate (one runtime elem)"
    );
    // A non-`BytesOf` node (a bare scalar) is never a constant-bytes candidate — the predicate is
    // narrow to the `Core::BytesOf` shape, not "any constant".
    assert!(
        !is_const_bytes("(module m (def (main) 42) (export main))", "main"),
        "a scalar constant `42` is not a `Core::BytesOf`, so not a constant-bytes candidate"
    );
}

/// §2d STATIC bytes (`DESIGN-static-data.md` increment 2 support — the value extractor, byte-neutral).
/// Pins [`crate::lower::constant_bytes_value`]: it returns the exact byte content of an all-constant
/// `Bytes` literal (the interning key + `(data …)` payload the build-once global materializes from),
/// and `None` for any node that is not an all-constant `BytesOf`. Two literals with identical content
/// must extract equal byte vectors (so interning collapses them to one global); a runtime/mixed literal
/// or a non-bytes node yields `None` (it must build per call, never share a static global).
#[test]
fn constant_bytes_value_extracts_the_byte_content_of_constant_literals_only() {
    use crate::db::Db;
    use crate::lower::constant_bytes_value;
    let extract = |src: &str, def: &str| -> Option<Vec<u8>> {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let body = db.defs[d].body.expect("body");
        constant_bytes_value(&mut db, body)
    };
    // A fully-constant literal → its exact bytes, in order.
    assert_eq!(
        extract(
            "(module m (def (main) (Bytes.of (list 0 1 255))) (export main))",
            "main"
        ),
        Some(vec![0u8, 1, 255]),
        "an all-constant `(Bytes.of (list 0 1 255))` must extract to exactly those bytes"
    );
    // The empty bytes literal → an empty vector (a distinct, internable value).
    assert_eq!(
        extract(
            "(module m (def (main) (Bytes.of (list))) (export main))",
            "main"
        ),
        Some(vec![]),
        "the empty bytes literal must extract to the empty byte vector"
    );
    // Two literals with IDENTICAL content extract equal vectors — the interning key that collapses
    // them to one build-once global.
    assert_eq!(
        extract(
            "(module m (def (a) (Bytes.of (list 7 8 9))) (def (b) (Bytes.of (list 7 8 9))) \
             (def (main) 0) (export main))",
            "a"
        ),
        extract(
            "(module m (def (a) (Bytes.of (list 7 8 9))) (def (b) (Bytes.of (list 7 8 9))) \
             (def (main) 0) (export main))",
            "b"
        ),
        "identical constant bytes literals must extract to equal vectors (the interning key)"
    );
    // A runtime element disqualifies extraction — such a literal must build per call, never share a
    // static global.
    assert_eq!(
        extract(
            "(module m (def (f (: n Int64)) (Bytes.of (list (UInt8.wrap n)))) \
             (def (main) 0) (export main))",
            "f"
        ),
        None,
        "a `Bytes.of` with a runtime element must not extract (no shared static global)"
    );
    // A non-`BytesOf` node (a bare scalar) never extracts.
    assert_eq!(
        extract("(module m (def (main) 42) (export main))", "main"),
        None,
        "a scalar constant is not a `Core::BytesOf`, so it never extracts to bytes"
    );
}

/// §2d STATIC bytes (`DESIGN-static-data.md` increment 2 — the whole-program collection pass).
/// Pins [`crate::backend::wasm::collect_static_bytes`]: it walks the reachable program and returns the
/// DISTINCT constant `Bytes` payloads interned by content in first-seen order — the build-once static-bytes
/// table (index = the module global slot each will occupy). Identical literals across defs collapse to one
/// entry (constant-CSE); distinct literals each get an entry in first-seen order; a runtime `Bytes.of`
/// contributes nothing; and a constant bytes literal NESTED inside a larger value is still found (the walk
/// descends every child via `core_child_ids`). A regression here would either merge distinct literals into
/// one shared global (a miscompile once emit consumes the table) or miss a hoistable literal.
#[test]
fn collect_static_bytes_interns_distinct_constant_bytes_literals() {
    use crate::backend::wasm::collect_static_bytes;
    use crate::db::Db;
    let collect = |src: &str, defs: &[&str]| -> Vec<Vec<u8>> {
        let mut db = Db::load(crate::testkit::parse(src));
        let order: Vec<usize> = defs
            .iter()
            .map(|n| db.def_by_name(n).unwrap_or_else(|| panic!("def {n}")))
            .collect();
        collect_static_bytes(&mut db, &order)
    };
    // Two defs with IDENTICAL constant bytes → interned to ONE entry (constant-CSE across the module).
    assert_eq!(
        collect(
            "(module m (def (a) (Bytes.of (list 7 8 9))) (def (b) (Bytes.of (list 7 8 9))) \
             (def (main) 0) (export main))",
            &["a", "b"]
        ),
        vec![vec![7u8, 8, 9]],
        "identical constant bytes across defs must intern to one static-bytes entry"
    );
    // Distinct literals → distinct entries, in first-seen order (stable global-slot assignment).
    assert_eq!(
        collect(
            "(module m (def (a) (Bytes.of (list 1))) (def (b) (Bytes.of (list 2))) \
             (def (main) 0) (export main))",
            &["a", "b"]
        ),
        vec![vec![1u8], vec![2u8]],
        "distinct constant bytes must each get an entry, in first-seen order"
    );
    // A runtime `Bytes.of` contributes no static-bytes entry (it is not constant — builds per call).
    assert_eq!(
        collect(
            "(module m (def (f (: n Int64)) (Bytes.of (list (UInt8.wrap n)))) \
             (def (main) 0) (export main))",
            &["f"]
        ),
        Vec::<Vec<u8>>::new(),
        "a runtime Bytes.of must contribute no static-bytes entry"
    );
    // A constant bytes NESTED inside a larger value (a record field) is still found — the walk descends
    // every child, so a literal is hoistable wherever it appears, not only as a whole function body.
    assert_eq!(
        collect(
            "(module m (def (g) (record (= pl (Bytes.of (list 5 6))))) (def (main) 0) (export main))",
            &["g"]
        ),
        vec![vec![5u8, 6]],
        "a constant bytes literal nested in a record field must be collected by the descent"
    );
}

/// §2d STATIC bytes (`DESIGN-static-data.md` — the `Core::ConstBytes` representation).
/// `constant_bytes_value` / `is_constant_bytes` must cover BOTH constant-`Bytes` representations, exactly
/// as the backend emit does: a `Core::BytesOf` of constants AND a baked `Core::ConstBytes` leaf (produced
/// by the `Ast.encode` / `Blake3.of` folds). The `ConstBytes` arm re-emits the same per-eval
/// `bytes-alloc`+`bytes-set`, so it is an equally valid build-once hoist target — a regression that missed
/// it would leave those literals re-allocating. Delegating to `const_byte_slice` (the canonical dual-repr
/// reader) is what gives this coverage; this pins it.
#[test]
fn constant_bytes_value_covers_the_baked_const_bytes_leaf_too() {
    use crate::db::Db;
    use crate::lower::{constant_bytes_value, is_constant_bytes};
    // `(Ast.encode (Ast.Bool true))` folds to a `Core::ConstBytes` leaf (a compile-time-known byte
    // sequence baked as one leaf), NOT a `Core::BytesOf` of per-byte `ConstInt`s.
    let src = "(module m (def (main) (Ast.encode (Ast.Bool true))) (export main))";
    let mut db = Db::load(crate::testkit::parse(src));
    let d = db.def_by_name("main").expect("def main");
    let body = db.defs[d].body.expect("body");
    assert!(
        is_constant_bytes(&mut db, body),
        "a baked `Core::ConstBytes` (the Ast.encode fold) must be recognized as constant Bytes"
    );
    let bytes = constant_bytes_value(&mut db, body)
        .expect("a baked Core::ConstBytes must extract its raw payload");
    assert!(
        !bytes.is_empty(),
        "the encoding of `(Ast.Bool true)` is a non-empty byte sequence, got {} bytes",
        bytes.len()
    );
}

/// §2d STATIC strings (`DESIGN-static-data.md` increment 5 groundwork). Pins
/// [`crate::lower::constant_string_value`]: a `Core::ConstStr` yields its exact UTF-8 bytes (the payload
/// the build-once-global mechanism hoists — a constant String is a flat UTF-8 byte-leaf built by the same
/// `bytes-alloc`+`bytes-set` as a `Bytes`), and a non-string node yields `None`. A constant String and a
/// constant `Bytes` with the same bytes extract to EQUAL vectors (they share the same runtime leaf rep),
/// so they can intern to one global at the string increment.
#[test]
fn constant_string_value_extracts_utf8_bytes_of_a_string_literal() {
    use crate::db::Db;
    use crate::lower::{constant_bytes_value, constant_string_value};
    let str_bytes = |src: &str, def: &str| -> Option<Vec<u8>> {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let body = db.defs[d].body.expect("body");
        constant_string_value(&mut db, body)
    };
    // A string literal → its exact UTF-8 bytes (incl. a multibyte char).
    assert_eq!(
        str_bytes("(module m (def (main) \"AB\") (export main))", "main"),
        Some(vec![b'A', b'B']),
        "a string literal must extract to its UTF-8 bytes"
    );
    assert_eq!(
        str_bytes("(module m (def (main) \"é\") (export main))", "main"),
        Some("é".as_bytes().to_vec()),
        "a multibyte string literal must extract to its full UTF-8 encoding"
    );
    // A non-string node (a scalar) → None.
    assert_eq!(
        str_bytes("(module m (def (main) 42) (export main))", "main"),
        None,
        "a scalar is not a Core::ConstStr → no string bytes"
    );
    // A constant String and a constant Bytes with the same content extract to EQUAL vectors (same flat
    // UTF-8 leaf rep → internable to one global).
    let mut db = Db::load(crate::testkit::parse(
        "(module m (def (s) \"AB\") (def (b) (Bytes.of (list 65 66))) (def (main) 0) (export main))",
    ));
    let sd = db.def_by_name("s").expect("def s");
    let s_body = db.defs[sd].body.expect("s body");
    let bd = db.def_by_name("b").expect("def b");
    let b_body = db.defs[bd].body.expect("b body");
    assert_eq!(
        constant_string_value(&mut db, s_body),
        constant_bytes_value(&mut db, b_body),
        "a constant String \"AB\" and constant Bytes 65,66 must extract to equal byte vectors"
    );
}

/// §2d STATIC compounds (`DESIGN-static-data.md` increment 6 — tuple/record markability detection).
/// Pins [`crate::lower::is_markable_constant_compound`]: a constant Tuple/Record whose every element is a
/// per-node-markable constant (scalar Int/Bool/Unit, constant Bytes/String, or nested markable T/R) is a
/// hoist candidate; a runtime element, a `ConstFloat`/`ConstChar`, a nested list, or a bare scalar root are
/// NOT (they build inline). This is the classification the build-once immortal emit (next slice) keys on.
#[test]
fn is_markable_constant_compound_flags_scalar_bytes_and_nested_only() {
    use crate::db::Db;
    use crate::lower::is_markable_constant_compound;
    let markable = |src: &str, def: &str| -> bool {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let body = db.defs[d].body.expect("body");
        is_markable_constant_compound(&mut db, body)
    };
    let m = "(module m ";
    // A constant scalar tuple → markable.
    assert!(
        markable(
            &format!("{m}(def (main) (tuple 1 2 3)) (export main))"),
            "main"
        ),
        "a constant scalar tuple must be markable"
    );
    // A constant record (incl. a String field) → markable.
    assert!(
        markable(
            &format!("{m}(def (main) (record (= x 1) (= y \"hi\"))) (export main))"),
            "main"
        ),
        "a constant record with scalar + string fields must be markable"
    );
    // A NESTED markable tuple → markable (recurse).
    assert!(
        markable(
            &format!("{m}(def (main) (tuple (tuple 1 2) 3)) (export main))"),
            "main"
        ),
        "a tuple of a markable nested tuple must be markable"
    );
    // A tuple with a RUNTIME element → NOT markable.
    assert!(
        !markable(
            &format!("{m}(def (f (: n Int64)) (tuple n 2)) (def (main) 0) (export main))"),
            "f"
        ),
        "a tuple with a runtime element must NOT be markable"
    );
    // A tuple containing a NESTED constant LIST → markable (PR #4989: the root `mark-immortal-DEEP`
    // reaches the nested list's arr/trie + element boxes transitively; a non-bool, non-empty nested list
    // never orphans, so it is an admissible compound element).
    assert!(
        markable(
            &format!("{m}(def (main) (tuple (list 1 2) 3)) (export main))"),
            "main"
        ),
        "a tuple containing a non-bool constant list must be markable (deep-mark reaches the nested list)"
    );
    // A bare scalar root → NOT a compound.
    assert!(
        !markable(&format!("{m}(def (main) 42) (export main))"), "main"),
        "a bare scalar is not a compound"
    );
}

/// Pins [`crate::lower::is_markable_constant_list`]: a fully-constant `(list …)` of markable elements is a
/// build-once hoist candidate at ANY size (the emit deep-marks the root, reaching `> 32` trie internals) —
/// INCLUDING a nested constant list element (PR #4989, recursed via `is_markable_constant_elem`); a list
/// with a runtime element, an ALL-`Bool` list (packed leaf orphans the marked boxes), an EMPTY list
/// (shared singleton), or a non-list root are NOT.
#[test]
fn is_markable_constant_list_flags_const_non_bool_lists_only() {
    use crate::db::Db;
    use crate::lower::is_markable_constant_list;
    let markable = |src: &str, def: &str| -> bool {
        let mut db = Db::load(crate::testkit::parse(src));
        let d = db.def_by_name(def).unwrap_or_else(|| panic!("def {def}"));
        let body = db.defs[d].body.expect("body");
        is_markable_constant_list(&mut db, body)
    };
    let m = "(module m ";
    // A small constant scalar list → markable.
    assert!(
        markable(
            &format!("{m}(def (main) (list 1 2 3)) (export main))"),
            "main"
        ),
        "a small constant scalar list must be markable"
    );
    // A constant list of Strings → markable (String leaf).
    assert!(
        markable(
            &format!("{m}(def (main) (list \"a\" \"b\")) (export main))"),
            "main"
        ),
        "a constant list of strings must be markable"
    );
    // A list of markable Tuples → markable (element recurses through the compound predicate).
    assert!(
        markable(
            &format!("{m}(def (main) (list (tuple 1 2) (tuple 3 4))) (export main))"),
            "main"
        ),
        "a constant list of markable tuples must be markable"
    );
    // 33 elements (a 2-level RRB trie) → markable now (the root deep-mark reaches the trie internals).
    let l33 = (1..=33)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        markable(
            &format!("{m}(def (main) (list {l33})) (export main))"),
            "main"
        ),
        "a 33-element list is markable — deep-mark reaches the trie internals"
    );
    // An ALL-`Bool` list → NOT markable (vec-of-arr packs it into a bit-leaf, orphaning the marked boxes).
    assert!(
        !markable(
            &format!("{m}(def (main) (list true false true)) (export main))"),
            "main"
        ),
        "an all-bool list must NOT be markable (packed bit-leaf orphans the element boxes)"
    );
    // An EMPTY list → NOT markable (shared vec-empty singleton).
    assert!(
        !markable(&format!("{m}(def (main) (list)) (export main))"), "main"),
        "an empty list must NOT be markable (shared vec-empty singleton)"
    );
    // A list with a RUNTIME element → NOT markable.
    assert!(
        !markable(
            &format!("{m}(def (f (: n Int64)) (list n 2)) (def (main) 0) (export main))"),
            "f"
        ),
        "a list with a runtime element must NOT be markable"
    );
    // A list whose element is itself a non-bool LIST → markable (PR #4989: `is_markable_constant_elem`
    // recurses `ListNew`; the root deep-mark reaches every nested list's structure — no orphan).
    assert!(
        markable(
            &format!("{m}(def (main) (list (list 1 2) (list 3 4))) (export main))"),
            "main"
        ),
        "a list of non-bool lists must be markable (nested list recursed, deep-marked)"
    );
    // A bare scalar root → NOT a list.
    assert!(
        !markable(&format!("{m}(def (main) 42) (export main))"), "main"),
        "a bare scalar is not a list"
    );
}

/// §2d STATIC compounds (increment 6 — the collection pass). Pins
/// [`crate::backend::wasm::collect_static_compounds`]: it gathers markable constant Tuple/Record ROOTS,
/// does NOT descend into a collected root (its subtree is built inline), and skips runtime/non-markable
/// compounds. A nested markable tuple contributes only its OUTER root (one global for the whole tree).
#[test]
fn collect_static_compounds_gathers_markable_roots_only() {
    use crate::backend::wasm::collect_static_compounds;
    use crate::db::Db;
    let count = |src: &str, defs: &[&str]| -> usize {
        let mut db = Db::load(crate::testkit::parse(src));
        let order: Vec<usize> = defs
            .iter()
            .map(|n| db.def_by_name(n).unwrap_or_else(|| panic!("def {n}")))
            .collect();
        collect_static_compounds(&mut db, &order).len()
    };
    // A returned constant tuple → 1 root.
    assert_eq!(
        count(
            "(module m (def (main) (tuple 1 2 3)) (export main))",
            &["main"]
        ),
        1,
        "a markable constant tuple must be collected as one root"
    );
    // A NESTED markable tuple → still ONE root (the outer; subtree built inline, not descended).
    assert_eq!(
        count(
            "(module m (def (main) (tuple (tuple 1 2) 3)) (export main))",
            &["main"]
        ),
        1,
        "a nested markable tuple contributes only its outer root (one global for the tree)"
    );
    // A runtime tuple → 0 (not markable).
    assert_eq!(
        count(
            "(module m (def (f (: n Int64)) (tuple n 2)) (def (main) 0) (export main))",
            &["f"]
        ),
        0,
        "a runtime tuple must not be collected"
    );
}

/// bytes-second run-wiring (compiler side): `compile` surfaces the GUEST export result-type map as a
/// `KIND_RESULT_TYPES` artifact — the STRUCTURED binary-AST codec (seq-284/307 B: each export's full `Ty`
/// via `encode_ty_payload`, NOT a `<name>\t<render_name>` string) — AND embeds it as a `cdz-result-type`
/// component custom section (the piped-run path byte-scans it for `render_val_typed`'s leaf disambiguation).
#[test]
fn compile_emits_the_result_type_map_artifact_and_component_custom_section() {
    use crate::testkit::parse;
    let out = crate::compile::compile(
        &[crate::abi::Artifact::new(
            crate::abi::Artifact::KIND_AST,
            "main",
            crate::codec::encode(&parse(
                "(module m (def (g) (Bytes.of (list 65 66))) (export g))",
            )),
        )],
        &[crate::backend::Target::Wasm],
    );
    // The KIND_RESULT_TYPES artifact carries the export→Ty map (g → Bytes), decoded via the shared codec
    // (the producer builds a standalone `encode_ty_payload` arena per export + frames via
    // `encode_result_types`; the round-trip through `decode_result_types` pins the producer shape).
    let map = out
        .artifact(crate::abi::Artifact::KIND_RESULT_TYPES)
        .expect("a result-types artifact");
    let decoded = cadenza_compile_abi::decode_result_types(map);
    assert_eq!(decoded.len(), 1, "one export in the map: {decoded:?}");
    assert_eq!(decoded[0].0, "g", "the export name");
    // Post-#6107 (DecodedTy dropped as the wrong abstraction): `decode_result_types` returns the FULL Ty
    // payload as an `Arenas` per export, and the consumer walks it. `encode_ty_payload` encodes `Ty::Bytes`
    // as the bare name leaf `Bytes` (eval.rs), so the payload arena's root is an atom `Bytes` — assert over
    // the arena, which pins the same producer shape the old `DecodedTy::Bytes` did.
    let ty = &decoded[0].1;
    assert_eq!(
        ty.as_name(ty.root),
        Some("Bytes"),
        "g's result-type payload arena is the bare `Bytes` leaf: {ty:?}"
    );
    // The component bytes embed the `cdz-result-type` custom section (the piped-run byte-scan target).
    let comp = out
        .artifact(crate::backend::Target::Wasm.artifact_kind())
        .expect("a component artifact");
    let needle = b"cdz-result-type";
    assert!(
        comp.windows(needle.len()).any(|w| w == needle),
        "the component embeds the cdz-result-type custom section"
    );
}
