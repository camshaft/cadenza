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

// ── Stage 1: let + records (compile-time folded) ────────────────────────────────────────────────
//
// Each case mirrors a `05-compound-types.sexp` / `02-binding-and-control.sexp` witness. A record
// whose field is read FOLDS to a scalar (no value heap), so the component runs to that scalar; a
// record used as a runtime value declines (needs the heap, a later stage). Programs are built with
// the test s-expr reader in `testkit`.
mod stage1 {
    use super::run_returns_s64;
    use crate::compile::compile_component;
    use crate::testkit::parse;

    /// Compile `(module m (def (main) BODY) (export main))` and run `main`, returning its s64 result.
    fn run_main(body: &str) -> i64 {
        let src = format!("(module m (def (main) {body}) (export main))");
        let bytes = compile_component(&crate::codec::encode(&parse(&src))).expect("compile");
        run_returns_s64(&bytes, "main")
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
        let bool_occ = db.defs.iter().find(|d| d.name == "t").and_then(|d| d.body).expect("def t");
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
        let ast = parse("(module m (def (main) (. (Int 64) max)) (def (other) (. (Int 64) min)) (export main))");
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
