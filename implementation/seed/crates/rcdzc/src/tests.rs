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
    use crate::backend::wasm::envelope::{BoundaryExport, assemble};
    for names in [&["run"][..], &["run", "double"][..]] {
        let core = oracle_core(names);
        let exports: Vec<BoundaryExport> = names
            .iter()
            .map(|n| BoundaryExport {
                name: n.to_string(),
                params: Vec::new(),
                result: Some(0x78),
            })
            .collect();
        let ours = assemble(&core, &exports);
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
    use crate::backend::wasm::envelope::{BoundaryExport, assemble};
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
            result: Some(0x78),
        }],
    );
    assert_eq!(ours, oracle, "parameterized envelope mismatch");
}

/// The NARROW component primitive valtype bytes (`u8`/`s8`/`u16`/`s16`) our `comp_valtype_of` emits are
/// byte-identical to the `wasm-encoder` oracle — the byte-identity discipline for the R2 faithful
/// boundary mapping (a wrong byte would misreport a value's type at the component edge). One export
/// `(p0: u8, p1: s16) -> s8` over a `(i32, i32) -> i32` core covers three of the four narrow primitives.
#[test]
fn narrow_primitive_envelope_matches_wasm_encoder_oracle() {
    use crate::backend::wasm::envelope::{BoundaryExport, assemble};
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
            params: vec![0x7D, 0x7C], // u8, s16
            result: Some(0x7E),       // s8
        }],
    );
    assert_eq!(ours, oracle, "narrow-primitive envelope mismatch");
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
}

// ── runtime functions + recursion (ANF step 2 / B1): a recursive call is a real wasm call ─────────
//
// A recursive function cannot β-reduce to a normal form at compile time, so it is emitted as its own
// wasm function and its self-call is a `Core::Call`. These run whole annotated-recursive PROGRAMS under
// wasmtime — the end-to-end proof that reachability emits the callee, the call ABI is right, and the
// recursion terminates. (The corpus's UNANNOTATED recursive functions stay `todo` until the connected
// parameter solve, A2; an annotated signature is determined by absorption — see `infer::def_scheme`.)
mod recursion {
    use super::run_returns;
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
    fn returning_a_record_declines_pending_the_value_heap() {
        // A record used as a runtime VALUE (returned, not projected) needs the value heap — declines.
        assert!(expect_decline("(record (x 1))").contains("value heap"));
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
    fn a_multi_param_function() {
        // `((fn (a b) (+ a b)) 3 4)` = 7 — both params substituted.
        assert_eq!(run_main("((fn (a b) (+ a b)) 3 4)"), 7);
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
        let a = reduce_ctor(&mut db, p, &[w64]).expect("build");
        let b = reduce_ctor(&mut db, p, &[w64]).expect("build");
        assert_eq!(a, b, "the same width must reduce to the same module node");
        // A different width is a different module.
        let w8 = db.push_atom(crate::ast::Leaf::Int {
            value: crate::ast::IntValue::from_i64(8),
            radix: crate::ast::Radix::Dec,
        });
        let c = reduce_ctor(&mut db, p, &[w8]).expect("build");
        assert_ne!(a, c, "different widths are different modules");
    }
}
