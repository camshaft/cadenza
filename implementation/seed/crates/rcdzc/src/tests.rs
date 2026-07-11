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
        // different width yields that width's bounds. Proves the module is parameterized over the width.
        assert_eq!(run_main("(. (Int 8) min)"), -128);
    }

    #[test]
    fn int_ctor_max_of_a_smaller_width() {
        // `(. (Int 8) max)` = 127.
        assert_eq!(run_main("(. (Int 8) max)"), 127);
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
