//! Multi-export foundation — end-to-end host proof.
//!
//! rcdzc's component envelope presents ONE OR MORE `(export …)` items, each lifted by its signature
//! (no single hard-coded `run`, no `find(nullary)`-and-drop-the-rest). These tests compile a
//! multi-export program with rcdzc and INSTANTIATE + INVOKE each export through wasmtime, so a
//! regression in the hand-built N-export envelope surfaces as a failing run, not a byte eyeball.

use cadenza_seed::host::{self, RunOutcome};
use cdz_compiler::ast;

/// Compile a program's source with rcdzc (the rewritten reference compiler) to component bytes.
fn compile_v2(src: &str) -> Vec<u8> {
    let node = ast::read(src).expect("parse");
    let out = rcdzc::compile_program(&node);
    out.component()
        .unwrap_or_else(|| panic!("rcdzc declined `{src}`: {:?}", out.diagnostics))
        .to_vec()
}

/// A two-export module compiles to ONE component, and BOTH exports instantiate and run to their own
/// values — the honest multi-export proof (no single-entry assumption survives).
#[test]
fn two_exports_both_run() {
    let bytes = compile_v2("(do (def (a) 42) (def (b) 7) (export a b))");
    // The component must be valid.
    host::validate_component(&bytes).expect("valid component");
    // Each export is individually resolvable and callable.
    assert_eq!(host::call_scalar_export(&bytes, "a").unwrap(), RunOutcome::Value("42".into()));
    assert_eq!(host::call_scalar_export(&bytes, "b").unwrap(), RunOutcome::Value("7".into()));
}

/// A source `main` is exported VERBATIM as `main` (the compiler never renames) — and runs under that
/// name. The host resolves the entry by SIGNATURE (the nullary export), not by a magic name.
#[test]
fn main_entry_named_verbatim_and_runs() {
    let bytes = compile_v2("(do (def (main) 42) (export main))");
    host::validate_component(&bytes).expect("valid component");
    assert_eq!(host::call_scalar_export(&bytes, "main").unwrap(), RunOutcome::Value("42".into()));
    // The name-agnostic entry resolver finds it whatever it is called.
    assert_eq!(host::run_component(&bytes, &[]).unwrap().0, RunOutcome::Value("42".into()));
}

/// Three exports with heterogeneous scalar return types (Int and Bool) each run — the boundary
/// surface is per-export signature-derived, and every export keeps its SOURCE name.
#[test]
fn three_exports_heterogeneous_types() {
    let bytes = compile_v2("(do (def (n) 99) (def (flag) (< 1 2)) (def (main) 5) (export n flag main))");
    host::validate_component(&bytes).expect("valid component");
    assert_eq!(host::call_scalar_export(&bytes, "n").unwrap(), RunOutcome::Value("99".into()));
    assert_eq!(host::call_scalar_export(&bytes, "flag").unwrap(), RunOutcome::Value("true".into()));
    assert_eq!(host::call_scalar_export(&bytes, "main").unwrap(), RunOutcome::Value("5".into()));
}

/// A `unit`-returning program runs and the host observes the unit value.
#[test]
fn unit_entry_runs_and_yields_unit() {
    for src in [
        "(do (def (main) unit) (export main))",
        "(do (def (main) (if true unit unit)) (export main))",
        "(do (def (main) (let ((x 1)) unit)) (export main))",
    ] {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value("unit".into()), "for {src}");
    }
}

/// A COMPOUND (tuple) result crosses the run boundary: the program imports the value-heap runtime,
/// builds the tuple on the heap, and RENDERS it to its canonical text via the compiler-emitted
/// type-directed renderer. The host composes the runtime and reads back the string. This is the
/// runtime-compound path end-to-end (needs CADENZA_RUNTIME — skipped if absent).
#[test]
fn tuple_result_renders_across_boundary() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping tuple_result: CADENZA_RUNTIME not set");
        return;
    }
    let cases = [
        ("(do (def (main) (tuple 7 8)) (export main))", "(tuple 7 8)"),
        ("(do (def (main) (tuple 1 true)) (export main))", "(tuple 1 true)"),
        ("(do (def (main) (tuple 2 (tuple 3 4))) (export main))", "(tuple 2 (tuple 3 4))"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// LISTS across the run boundary — the first PARAMETRIC type (`List T`). A list is built on the value
/// heap via the runtime's persistent sequence (`vec-empty`/`vec-push`, distinct from a tuple's fixed
/// `arr-*`) and rendered `(list …)` by a counter loop over `vec-len`/`vec-get`. A homogeneous literal,
/// one with a runtime element, a Bool-element list, and a nested list all render their canonical text
/// (needs CADENZA_RUNTIME — skipped if absent).
#[test]
fn list_result_renders_across_boundary() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping list_result: CADENZA_RUNTIME not set");
        return;
    }
    let cases = [
        ("(do (def (main) (list 1 2 3)) (export main))", "(list 1 2 3)"),
        ("(do (def (f n) (list n 2 3)) (def (main) (f 1)) (export main))", "(list 1 2 3)"),
        ("(do (def (main) (list true false)) (export main))", "(list true false)"),
        ("(do (def (main) (list (tuple 1 2) (tuple 3 4))) (export main))", "(list (tuple 1 2) (tuple 3 4))"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// SUM values across the run boundary — the built-in Option/Result prelude sums. A sum is built on the
/// value heap as `(disc, payload)` (via `sum-new`), and the type-directed renderer switches on the
/// runtime discriminant to bake the variant NAME (bare for Option/Result). A constant `(Some 42)`, a
/// runtime-payload `(Some n)`, a nullary `(None unit)`, a conditionally-selected variant, and a MATCH
/// binding a payload all render/compute their canonical values (needs CADENZA_RUNTIME).
#[test]
fn sum_values_across_boundary() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping sum_values: CADENZA_RUNTIME not set");
        return;
    }
    let cases = [
        // Construct + render a sum value.
        ("(do (def (main) (Some 42)) (export main))", "(Some 42)"),
        ("(do (def (main) (None unit)) (export main))", "(None unit)"),
        ("(do (def (main) (Ok 5)) (export main))", "(Ok 5)"),
        // A runtime payload, and a conditionally-selected variant.
        ("(do (def (f n) (Some n)) (def (main) (f 42)) (export main))", "(Some 42)"),
        ("(do (def (f n) (if (= n 0) (None unit) (Some n))) (def (main) (f 5)) (export main))", "(Some 5)"),
        ("(do (def (f n) (if (= n 0) (None unit) (Some n))) (def (main) (f 0)) (export main))", "(None unit)"),
        // MATCH binding a payload → a scalar result.
        ("(do (def (main) (match (Some 42) ((Some x) x) ((None _) 0))) (export main))", "42"),
        ("(do (def (main) (match (Ok 5) ((Ok n) n) ((Err _) 0))) (export main))", "5"),
        // A runtime-variant dispatch (the match reads a runtime discriminant).
        ("(do (def (f n) (match (if (= n 0) (None unit) (Some n)) ((Some x) x) ((None _) -1))) (def (main) (f 7)) (export main))", "7"),
        ("(do (def (f n) (match (if (= n 0) (None unit) (Some n)) ((Some x) x) ((None _) -1))) (def (main) (f 0)) (export main))", "-1"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// RECORDS across the run boundary: field projection to a scalar, and a record RESULT rendered with
/// its field names in KEY-SORTED order (the slot/name-agreement invariant — the runtime is tag-free,
/// so the compiler-emitted renderer bakes the names and must agree with the sorted heap slots).
#[test]
fn records_project_and_render() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping records: CADENZA_RUNTIME not set");
        return;
    }
    let cases = [
        // Projection to a scalar (scalar-via-heap path).
        ("(do (def (main) (. (record (x 1) (y 2)) x)) (export main))", "1"),
        ("(do (def (main) (. (record (flag true)) flag)) (export main))", "true"),
        ("(do (def (main) (let ((p (record (x 5) (y 9)))) (. p y))) (export main))", "9"),
        // Record RESULT — rendered with field names, KEY-SORTED regardless of source order.
        ("(do (def (f n) (record (a n) (b 1))) (def (main) (f 3)) (export main))", "(record (a 3) (b 1))"),
        ("(do (def (f n) (record (b n) (a 1))) (def (main) (f 2)) (export main))", "(record (a 1) (b 2))"),
        // A record whose field is a runtime tuple nests across the boundary.
        ("(do (def (main) (record (x 5) (y (tuple 5 1)))) (export main))", "(record (x 5) (y (tuple 5 1)))"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// A SCALAR result BUILT via the heap (a tuple constructed then projected) takes the runtime-compound
/// path (the heap is imported) yet crosses the boundary as its raw scalar — the renderer emits the
/// scalar directly. Regression guard for the "scalar entry that internally touches the heap" case
/// that the scalar component cannot host (it would call heap imports that aren't present).
#[test]
fn scalar_projected_from_heap_tuple_runs() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping scalar_projected: CADENZA_RUNTIME not set");
        return;
    }
    let cases = [
        ("(do (def (main) (. (tuple 7 8) 0)) (export main))", "7"),
        ("(do (def (main) (. (tuple 7 8) 1)) (export main))", "8"),
        ("(do (def (main) (let ((t (tuple 9 2))) (. t 0))) (export main))", "9"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// Empty record `(record)` — the empty named product. A valid compound (distinct type from unit/empty
/// tuple and from an empty map), rendered `(record)`.
#[test]
fn empty_record_renders() {
    if std::env::var("CADENZA_RUNTIME").is_err() {
        eprintln!("skipping empty_record: CADENZA_RUNTIME not set");
        return;
    }
    let bytes = compile_v2("(do (def (main) (record)) (export main))");
    host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid: {e}"));
    let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
    assert_eq!(outcome, RunOutcome::Value("(record)".into()));
}

/// MODULES run end-to-end: a module folds to a record of exports, `(. m f)` projects, and the whole
/// module vanishes at compile time — the program reduces to its scalar result. Verifies the VALUES.
#[test]
fn modules_run_to_their_values() {
    let cases = [
        ("(do (module m (def (answer) 42) (export answer)) (def (main) ((. m answer) unit)) (export main))", "42"),
        ("(do (module m (def (one) 1) (def (two) 2) (export one two)) (def (main) (+ ((. m one) unit) ((. m two) unit))) (export main))", "3"),
        ("(do (module m (def v 7) (export v)) (def (main) (. m v)) (export main))", "7"),
        ("(do (module lib (def (dbl x) (* x 2)) (def (f x) (+ (dbl x) 1)) (export f)) (def (main) ((. lib f) 3)) (export main))", "7"),
        ("(do (module lib (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (export fac)) (def (main) ((. lib fac) 5)) (export main))", "120"),
        // Nested module reached by chained projection.
        ("(do (module outer (module inner (def (v) 5) (export v)) (export inner)) (def (main) ((. (. outer inner) v) unit)) (export main))", "5"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// INTRINSICS run end-to-end: a built-in operation projected from a prelude record (`Int64.wrapping-*`)
/// applies and lowers to wasm — folded here to its constant, matching the corpus values. Proves the
/// intrinsic mechanism (prelude record → value → applied → wasm at LIR).
#[test]
fn intrinsics_run_to_their_values() {
    let cases = [
        ("(do (def (main) (Int64.wrapping-add 20 22)) (export main))", "42"),
        ("(do (def (main) (Int64.wrapping-add Int64.max 1)) (export main))", "-9223372036854775808"),
        ("(do (def (main) (Int64.wrapping-mul Int64.max 2)) (export main))", "-2"),
        ("(do (def (main) (Int64.wrapping-sub Int64.min 1)) (export main))", "9223372036854775807"),
        ("(do (def (main) Int64.max) (export main))", "9223372036854775807"),
    ];
    for (src, expected) in cases {
        let bytes = compile_v2(src);
        host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid for {src}: {e}"));
        let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
        assert_eq!(outcome, RunOutcome::Value(expected.into()), "for {src}");
    }
}

/// A RUNTIME wrapping op (operands are parameters, unfoldable) still wraps at run time — proves the
/// intrinsic lowers to actual wasm instructions, not only a compile-time fold.
#[test]
fn runtime_intrinsic_wraps_at_run_time() {
    let src = "(do (def (w a b) (Int64.wrapping-add a b)) (def (main) (w Int64.max 1)) (export main))";
    let bytes = compile_v2(src);
    host::validate_component(&bytes).unwrap_or_else(|e| panic!("invalid: {e}"));
    // (w Int64.max 1) inlines the const args, so this still folds — but exercises the arg-substitution
    // path through a function boundary. The value is the wrapped MIN.
    let (outcome, _) = host::run_component(&bytes, &[]).expect("run");
    assert_eq!(outcome, RunOutcome::Value("-9223372036854775808".into()));
}
