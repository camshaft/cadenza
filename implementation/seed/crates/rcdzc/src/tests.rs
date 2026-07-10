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
    let body = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(42), radix: Radix::Dec });
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
    let then_ = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(1), radix: Radix::Dec });
    let else_ = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(2), radix: Radix::Dec });
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
    let one = b.atom_leaf(Leaf::Int { value: IntValue::from_i64(1), radix: Radix::Dec });
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
    c.section(&RawSection { id: ComponentSectionId::CoreModule as u8, data: core });
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
        al.alias(Alias::CoreInstanceExport { instance: 0, kind: ExportKind::Func, name: nm });
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
    use crate::backend::wasm::envelope::{assemble, BoundaryExport};
    for names in [&["run"][..], &["run", "double"][..]] {
        let core = oracle_core(names);
        let exports: Vec<BoundaryExport> =
            names.iter().map(|n| BoundaryExport { name: n.to_string(), result: Some(0x78) }).collect();
        let ours = assemble(&core, &exports);
        assert_eq!(ours, oracle_component(&core, names), "envelope mismatch for {names:?}");
    }
}

// ── the behavior run (wasmtime) ────────────────────────────────────────────────────────────────

/// Instantiate `component_bytes` under wasmtime, call its nullary export `name`, and return the
/// single s64 result. The "run the artifact" behavior check.
fn run_returns_s64(component_bytes: &[u8], name: &str) -> i64 {
    use wasmtime::component::{Component, Linker, Val};
    use wasmtime::{Engine, Store};

    let engine = Engine::default();
    let component = Component::from_binary(&engine, component_bytes).expect("valid component");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &component).expect("instantiate");
    let func = instance.get_func(&mut store, name).expect("export present");
    let mut results = [Val::S64(0)];
    func.call(&mut store, &[], &mut results).expect("call");
    func.post_return(&mut store).expect("post_return");
    match results[0] {
        Val::S64(n) => n,
        ref other => panic!("expected S64 result, got {other:?}"),
    }
}

/// `(def (main) 42)` compiles to a component that runs to 42.
#[test]
fn scalar_runs_to_42() {
    let bytes = compile_component(&prog_scalar()).expect("compile");
    assert_eq!(run_returns_s64(&bytes, "main"), 42);
}

/// `(def (main) (if false 1 2))` compiles to a component that runs to 2 (the else branch).
#[test]
fn if_runs_to_2() {
    let bytes = compile_component(&prog_if()).expect("compile");
    assert_eq!(run_returns_s64(&bytes, "main"), 2);
}

// ── decline-don't-miscompile ───────────────────────────────────────────────────────────────────

/// An unsupported construct reached unconditionally yields NO component and an error diagnostic —
/// never a plausible wrong artifact.
#[test]
fn unsupported_construct_declines() {
    let err = compile_component(&prog_unsupported()).expect_err("must decline");
    assert_eq!(err.severity, crate::abi::Severity::Error);
    assert!(err.message.contains("frobnicate"), "diagnostic should name the construct: {err:?}");
}

/// The `main` export crosses the boundary VERBATIM — the component actually exports `main` (no
/// rename), so a query by that name resolves.
#[test]
fn export_name_is_verbatim() {
    let bytes = compile_component(&prog_scalar()).expect("compile");
    // If the name were renamed, `run_returns_s64(.., "main")` would fail to find the export.
    assert_eq!(run_returns_s64(&bytes, "main"), 42);
}
