use super::*;
use cdz_compiler::ast;

/// Parse a program's source text to its canonical binary AST bytes (the `ast` artifact form) —
/// the same path the CLI takes: read s-expr → `Node` → `ast::encode`.
fn ast_bytes(src: &str) -> Vec<u8> {
    let node = ast::read(src).expect("parse");
    ast::encode(&node)
}

/// Build the NEW canonical program shape for rcdzc from a body expression: `(do (def (main) E)
/// (export main))`. rcdzc requires an explicit `(export …)` (visibility is explicit; no `main`
/// magic), so a test's body is wrapped this way. Returns the decoded `Node`.
fn program_v2(body_src: &str) -> cdz_compiler::ast::Node {
    let src = format!("(do (def (main) {body_src}) (export main))");
    ast::read(&src).expect("parse v2 program")
}

/// Phase 0's core obligation: rcdzc's component bytes are BYTE-IDENTICAL to the old compiler
/// (the oracle) for a scalar-integer entry, across single- and multi-byte LEB values. rcdzc names
/// exports VERBATIM (no `main`→`run` rename), and the old compiler renames its `main` entry to the
/// external name `run`; so the like-for-like comparison uses a source entry already named `run`
/// (`(def (run) …) (export run)`), and both emit the identical `run` export.
#[test]
fn scalar_entry_byte_identical_to_oracle() {
    for body in ["42", "7", "0", "300" /* multi-byte LEB */] {
        let src = format!("(do (def (run) {body}) (export run))");
        let ours = compile_program(&ast::read(&src).expect("parse"))
            .component()
            .expect("rcdzc produced a component")
            .to_vec();
        let oracle_node = ast::read(&format!("(module m (def (main) {body}))")).expect("parse");
        let oracle = cdz_compiler::codegen::compile_program(&oracle_node).expect("oracle compiled");
        assert_eq!(ours, oracle, "byte mismatch for body {body:?}");
    }
}

/// The scalar component is the expected 89 bytes for a `run`-named `42` entry (a fixed anchor, so
/// a frame-segment regression is caught even if the oracle drifts). Uses the entry name `run` so
/// the export-name bytes match the old compiler's anchor (rcdzc names verbatim — a `main` entry
/// would be 91 bytes, the extra byte per `main`/`run` length in the two export records).
#[test]
fn scalar_run_42_is_89_bytes() {
    let out = compile_program(&ast::read("(do (def (run) 42) (export run))").unwrap());
    assert_eq!(out.component().unwrap().len(), 89);
}

/// Phase 1: arithmetic, comparison, `if`, and `let` all produce a VALID component (structural —
/// value-correctness is checked by the corpus behavior gate under `CADENZA_COMPILER=v2`). Each
/// exercises a distinct rung path: checked arith scratch locals, an `if` block, a `let` local.
#[test]
fn phase1_forms_compile() {
    for body in [
        "(+ 2 3)",
        "(- 10 3)",
        "(* 6 7)",
        "(- (+ 1 2) 1)", // nested arith (distinct scratch)
        "(let ((x 10)) x)",
        "(let ((x 5)) (+ x 1))",
        "(let ((a 1)) (let ((b 2)) (+ a b)))", // let* chain
        "(if (< 1 2) 10 20)",
        "(& 255 129)", // bitwise
        "(| 128 1)",
        "(^ 5 3)",
        "(/ 17 5)", // div (traps on /0 natively)
        "(% 17 5)",
        "(>> 300 2)", // shift (count-guarded)
        "(<< 3 4)",
        "(| (& 300 127) 128)",   // the uleb byte idiom
        "(and (< 1 2) (> 3 2))", // connectives desugar to if
        "(or (> 1 2) (< 1 2))",
        "(not (< 1 2))",
        "(do 1 2 (+ 3 4))",       // do yields last
        "(do (def x 5) (+ x 1))", // do-scoped value-def
        "(do (def a 1) (def b 2) (+ a b))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}, got diagnostics {:?}",
            out.diagnostics
        );
    }
}

/// A comparison `main` returns a bool — the run export's type is `bool`/`i32`, not `s64`. Pins
/// that the return type is a read-off of the solved `Ty` (the component functype byte differs).
#[test]
fn comparison_main_is_bool_typed() {
    let out = compile_program(&program_v2("(< 1 2)"));
    let bytes = out.component().expect("component");
    // The component functype section encodes the result valtype; a bool run export ends its
    // type section in 0x7F (bool), an s64 in 0x78. Assert the bool valtype byte is present.
    assert!(
        bytes.windows(3).any(|w| w == [0, 0, 0x7F]),
        "expected a bool-typed run export"
    );
}

/// A type error is a coded rejection (CDZ0201), not a component. `(+ 1 true)` mixes Int and Bool.
#[test]
fn type_error_is_coded_rejection() {
    let out = compile_program(&program_v2("(+ 1 true)"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// Bool ordering compiles (`false < true`, via unsigned i32 comparison) — a valid, bool-typed
/// component, not the old invalid `i64.lt_s` on i32 operands. Each ordering operator + equality.
#[test]
fn bool_ordering_compiles() {
    for body in [
        "(< false true)",
        "(> true false)",
        "(<= false false)",
        "(>= true true)",
        "(= true true)",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// `unit` and the empty tuple `()` are the unit value; a unit result, a unit `if`, and a unit
/// `let` body all compile to a valid (no-result) component.
#[test]
fn unit_forms_compile() {
    for body in [
        "unit",
        "()",
        "(= unit ())",
        "(if true unit unit)",
        "(let ((x 1)) unit)",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// A record literal compiles, and a field is projected by name via `(. r f)` (a scalar-via-heap
/// result). Structural (value-correctness is the corpus/host tests).
#[test]
fn record_forms_compile() {
    for body in [
        "(record (x 1) (y 2))",       // a record result (renders via runtime-compound)
        "(. (record (x 1) (y 2)) x)", // projection to a scalar
        "(. (record (flag true)) flag)", // a bool field
        "(let ((p (record (x 1) (y 2)))) (. p x))", // let-bound then projected
        "(record (b 2) (a 1))",       // source order differs from sorted order
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// Bytes intrinsics compile (the value-heap `bytes-*` path): construct from a byte list, measure,
/// concatenate, and `Int64.to-byte`. Structural — value/render correctness is the host/corpus test.
#[test]
fn bytes_forms_compile() {
    for body in [
        "(Bytes.of (list 1 2 3))",                          // construct + render b"…"
        "(Bytes.of (list 65 66 67))",                        // printable render
        "(Bytes.len (Bytes.of (list 0 255 128)))",           // measure → scalar
        "(Bytes.len (Bytes.concat (Bytes.of (list 1 2)) (Bytes.of (list 3 4))))", // concat
        "(Int64.to-byte 300)",                               // low-8-bits (folds → 44)
        "(Bytes.of (list (Int64.to-byte (| (& 300 127) 128))))", // the LEB128 non-final byte compose
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// List ops compile — `List.len`/`List.push`/`List.concat`, the first PARAMETRIC intrinsics (`List a →
/// Int` etc., element type instantiated fresh per use like a `Ctor`). `List.push` lowers to
/// `Mir::ListPush` carrying the element's SOLVED type, so `select` boxes a scalar element correctly
/// (a literal AND a runtime-scalar element); a compound element is already a handle.
#[test]
fn list_ops_compile() {
    for body in [
        "(List.len (list 1 2 3))",                              // len → scalar
        "(List.len (List.concat (list 1 2) (list 3 4 5)))",      // concat then measure
        "(List.len (List.push (list 1 2) 3))",                   // push a literal Int element
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
    // Pushing a RUNTIME-scalar element (a bound Local, not a literal) also compiles — the solved element
    // type is threaded to select, so the shape-guess that would have mis-boxed a runtime Int is gone.
    let src = "(do (def (g x) (List.len (List.push (list 10 20) x))) (def (main) (g 99)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_some(), "runtime-scalar push: {:?}", out.diagnostics);
}

/// Map/Set compile — the `(map …)`/`(set …)` literals (built via `map-insert`/`set-insert` from empty),
/// `Map.empty`/`Set.empty` (the empty-literal alias), `Map.lookup`(→Option V)/`size`, `Set.contains`(→
/// Bool)/`of`/`size`/`union`. A `Heap` op boxes a scalar key/value/element by its solved type. A map's
/// key SET is runtime data, not part of its type. Value/render correctness is the host/corpus test.
#[test]
fn map_set_forms_compile() {
    for body in [
        "(Map.size (map (1 10) (2 20)))",                                   // map literal + size
        "(Map.size (Map.insert Map.empty 5 42))",                           // Map.empty alias + insert
        "(match (Map.lookup (map (5 42)) 5) ((Some v) v) ((None _) -1))",   // lookup → Option (hit)
        "(match (Map.lookup (map (5 42)) 9) ((Some v) v) ((None _) -1))",   // lookup → Option (miss)
        "(Set.size (set 1 2 1 3))",                                         // set literal (dedup) + size
        "(Set.size (Set.insert Set.empty 7))",                              // Set.empty alias + insert
        "(Set.contains (Set.of (list 1 2 3)) 2)",                           // Set.of + contains → Bool
        "(Set.size (Set.union (set 1 2) (set 2 3)))",                        // set algebra
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
    // A map with values of two different types is CDZ0201 (values are of ONE type).
    let out = compile_program(&program_v2("(map (1 1) (2 true))"));
    assert!(out.component().is_none(), "heterogeneous map values must reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// A match on a TUPLE scrutinee is single-arm destructuring (`(match t ((tuple a b) …))`) — bind the
/// elements against the scrutinee handle (the tuple `arr`) and emit the body, no discriminant. The shape
/// a self-hosted decoder uses to unpack a `(tuple <value> <cursor>)`.
#[test]
fn tuple_scrutinee_match_compiles() {
    for body in [
        "(match (tuple 3 4) ((tuple a b) (+ a b)))",         // destructure + use both
        "(match (tuple 9 (tuple 1 2)) ((tuple a b) a))",     // a nested-tuple element bound as a handle
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// Fallible reads compile — `Bytes.at`/`List.at` → `Option elem`, `Bytes.slice` → `Option Bytes` —
/// consumed by a `match` on the SHARED prelude `Option` (so the result unifies with the `(Some x)`/
/// `(None _)` pattern by `Arc::ptr_eq`). Bounds/negative → the `None` arm (value correctness is the
/// host/corpus test); this is the structural proof the fallible-sum path threads end to end.
#[test]
fn fallible_reads_compile() {
    for body in [
        "(match (Bytes.at (Bytes.of (list 10 20 30)) 1) ((Some x) x) ((None _) -1))",
        "(match (List.at (list 5 6 7) 2) ((Some x) x) ((None _) -1))",
        "(match (Bytes.slice (Bytes.of (list 1 2 3 4)) 1 2) ((Some s) (Bytes.len s)) ((None _) -1))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// String literals compile (a Bytes-backed UTF-8 heap leaf, rendered `"…"`). Multibyte + named escapes
/// + empty all build; value/render correctness is the host/corpus test.
#[test]
fn string_literals_compile() {
    for body in ["\"hello\"", "\"café\"", "\"a\\nb\"", "\"\""] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
    // `String.from-bytes` now compiles: `Bytes → Option String` via the fixed `utf8-valid` helper.
    // Both a well-formed and an ill-formed literal input build a component (the value/`None`
    // correctness is the host/corpus test).
    for body in [
        "(String.from-bytes (Bytes.of (list 104 105)))",            // "hi" — well-formed
        "(String.from-bytes (Bytes.of (list 255)))",                // 0xFF — ill-formed
        "(match (String.from-bytes (Bytes.of (list 104 105))) ((Some s) s) ((None _) \"\"))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// A record with a duplicate field name (adjacent or not) is CDZ0201 — the field names are a set.
#[test]
fn duplicate_record_field_is_cdz0201() {
    for body in ["(record (a 1) (a 2))", "(record (a 1) (b 2) (a 3))"] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should not compile");
        assert_eq!(
            out.diagnostics[0].code.as_deref(),
            Some("CDZ0201"),
            "for {body:?}"
        );
    }
}

/// Member access on a non-record, and of a field the record does not carry, are both CDZ0201
/// (compile-time — the field set is part of the record's type).
#[test]
fn record_projection_type_errors_are_cdz0201() {
    for body in ["(. 5 x)", "(. true x)", "(. (record (x 1)) z)"] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should not compile");
        assert_eq!(
            out.diagnostics[0].code.as_deref(),
            Some("CDZ0201"),
            "for {body:?}"
        );
    }
}

/// List construction compiles to a component (the runtime-compound path via `vec-*`). A list is the
/// first PARAMETRIC type — `List T` — so a homogeneous literal, one with a runtime element, and the
/// empty list (element type left a var but grounded by a sibling) all build. Value-correctness is the
/// host/corpus test; this is the structural proof the ladder threads `Ty::List` end to end.
#[test]
fn list_forms_compile() {
    for src in [
        "(do (def (main) (list 1 2 3)) (export main))", // homogeneous int list
        "(do (def (f n) (list n 2 3)) (def (main) (f 1)) (export main))", // a runtime element
        "(do (def (main) (list true false)) (export main))", // a bool list (element type Bool)
    ] {
        let out = compile_program(&ast::read(src).expect("parse"));
        assert!(
            out.component().is_some(),
            "expected a component for {src:?}: {:?}",
            out.diagnostics
        );
    }
}

/// A list is HOMOGENEOUS: every element unifies to the single element type, so a list mixing an Int
/// and a Bool is a type error (CDZ0201) — the generic-unification payoff (a mixed list clashes at the
/// element var). This is `(list 1 true)`, distinct from a well-typed `(list 1 2)`.
#[test]
fn mixed_element_list_is_cdz0201() {
    let out = compile_program(&program_v2("(list 1 true)"));
    assert!(out.component().is_none(), "a mixed-element list should not compile");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// The list's element type is genuinely inferred: `(list n 2 3)` over an Int param `n` is a
/// `List Int64` (n unifies to the element type Int, from the literal siblings), and the whole program
/// is well-typed. A projection-free structural check that the parametric element type solves.
#[test]
fn list_element_type_is_inferred() {
    // A helper takes the list and the entry passes a runtime element — if the element var did not
    // solve to Int (from `2`/`3`), `ground` would decline "type could not be determined".
    let src = "(do (def (mk n) (list n 2 3)) (def (main) (mk 1)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_some(), "diagnostics {:?}", out.diagnostics);
}

/// SUM construction + match compile (the runtime-compound path via `sum-new`/`sum-disc`). Built-in
/// Option/Result are regular prelude sums; a constructor is a single-arity function value. Structural
/// (value-correctness is the host/corpus test).
#[test]
fn sum_forms_compile() {
    for body in [
        "(Some 42)",                                  // a unary constructor applied
        "(None unit)",                                // a nullary constructor applied to unit
        "(Ok 5)",
        "(match (Some 42) ((Some x) x) ((None _) 0))", // match binding a payload
        "(match (Ok 5) ((Ok n) n) ((Err _) 0))",
        "(let ((c None)) (c unit))",                  // a bare constructor used as a first-class value
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "expected a component for {body:?}: {:?}",
            out.diagnostics
        );
    }
}

/// Constructor arity/payload errors are CDZ0201: a nullary variant applied to a non-unit payload
/// (`(None 5)`), under-application (`(Some)` — must NOT fabricate a unit payload), over-application
/// (`(Some 1 2)`). These fall out of typing the constructor as a single-arity `Fn`.
#[test]
fn constructor_arity_errors_are_cdz0201() {
    for body in ["(None 5)", "(Some)", "(Some 1 2)"] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should not compile");
        assert_eq!(
            out.diagnostics[0].code.as_deref(),
            Some("CDZ0201"),
            "for {body:?}"
        );
    }
}

/// User `(type …)` sum declarations (Increment B): a program declares its own sums (monomorphic +
/// RECURSIVE), constructs and matches their values (bare + qualified, nullary + unary). Structural
/// (value-correctness is the host/corpus test). Each program carries its own `(export main)`.
#[test]
fn user_types_compile() {
    for src in [
        // Nullary-only, qualified value.
        "(do (type Color (Red | Green | Blue)) (def (main) Color.Red) (export main))",
        // Mixed nullary/unary, bare-nullary construct + match (the flipped gate FAIL).
        "(do (type Node (NLit Int64 | NNil)) (def (c n) (if (= n 0) NNil (Node.NLit n))) \
             (def (v x) (match x ((Node.NLit k) k) ((Node.NNil _) 1))) \
             (def (main) (+ (v (c 0)) (v (c 7)))) (export main))",
        // Recursive sum, recursive consumer over the runtime spine.
        "(do (type Expr (Lit Int64 | Neg Expr)) \
             (def (depth e) (match e ((Expr.Lit n) 0) ((Expr.Neg x) (+ 1 (depth x))))) \
             (def (main) (depth (Expr.Neg (Expr.Lit 5)))) (export main))",
        // A payload naming a Tuple + a recursive sum (a linked-list shape).
        "(do (type IntList (Nil | Cons (Tuple Int64 IntList))) \
             (def (main) (match (Cons (tuple 1 Nil)) ((Nil _) 0) ((Cons _) 1))) (export main))",
    ] {
        let out = compile_program(&ast::read(src).expect("parse"));
        assert!(
            out.component().is_some(),
            "expected a component for {src:?}: {:?}",
            out.diagnostics
        );
    }
    // A sum declaring a variant name twice is CDZ0201 (a variant set is a closed name-set) — the
    // flipped gate FAIL.
    let dup = "(do (type T (A Int64 | A Bool)) (def (main) 1) (export main))";
    let out = compile_program(&ast::read(dup).expect("parse"));
    assert!(out.component().is_none(), "duplicate variant must reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// A malformed numeric literal (out of Int64 range, or a bad digit-separator/float shape) that the
/// reader hands through as a name is CDZ0201 (malformed literal), NOT CDZ0101 (unbound name).
#[test]
fn malformed_numeric_literal_is_cdz0201() {
    for body in ["9223372036854775808", "0xFFFFFFFFFFFFFFFF", "1_", "1._5"] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should not compile");
        assert_eq!(
            out.diagnostics[0].code.as_deref(),
            Some("CDZ0201"),
            "{body:?} should be a malformed-literal reject, not unbound-name"
        );
    }
}

/// An unbound name is CDZ0101.
#[test]
fn unbound_name_is_cdz0101() {
    let out = compile_program(&program_v2("x"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0101"));
}

/// An unbound name in a DISCARDED `do`-prefix is still scope-checked (a discarded form must still
/// resolve — 02-binding-and-control): `(do nope 1)` rejects CDZ0101, not silently yields 1.
#[test]
fn unbound_in_discarded_do_prefix_is_rejected() {
    let out = compile_program(&program_v2("(do nope 1)"));
    assert!(
        out.component().is_none(),
        "discarded prefix must still scope-check"
    );
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0101"));
}

/// Multiple defs + explicit `(export …)`: a call to a helper, entry chosen by its (nullary)
/// signature, no `main`-name magic. Compiles to a component.
#[test]
fn functions_and_export_compile() {
    let src = "(do (def (add a b) (+ a b)) (def (main) (add 2 3)) (export main))";
    let node = ast::read(src).expect("parse");
    let out = compile_program(&node);
    assert!(
        out.component().is_some(),
        "diagnostics {:?}",
        out.diagnostics
    );
}

/// TWO exports from one module compile to ONE component presenting BOTH — the multi-export
/// foundation, no single-entry assumption. The component's core module must export both funcs and
/// the envelope must lift both. Structural proof (the host-run proof lives in cadenza-seed).
#[test]
fn two_exports_compile_to_one_component() {
    let src = "(do (def (a) 42) (def (b) 7) (export a b))";
    let node = ast::read(src).expect("parse");
    let out = compile_program(&node);
    let bytes = out.component().expect("component");
    // Both boundary names appear in the bytes (core export + component export).
    assert!(
        bytes.windows(1).filter(|w| w == b"a").count() >= 2,
        "export `a` should appear (core + component export)"
    );
    assert!(
        bytes.windows(1).filter(|w| w == b"b").count() >= 2,
        "export `b` should appear (core + component export)"
    );
}

/// Exports are named VERBATIM — the compiler NEVER renames. A source `main` is exported as
/// `main` (not silently rewritten to `run`); a source `run` is exported as `run`. Which export is
/// the entry, and any conventional name, is the consumer's concern.
#[test]
fn exports_are_named_verbatim() {
    let main_bytes = compile_program(&program_v2("42"))
        .component()
        .unwrap()
        .to_vec();
    assert!(
        main_bytes.windows(4).any(|w| w == b"main"),
        "a source `main` is exported as `main`"
    );
    assert!(
        !main_bytes.windows(3).any(|w| w == b"run"),
        "the compiler must NOT invent a `run` name"
    );

    let run_bytes = compile_program(&ast::read("(do (def (run) 42) (export run))").unwrap())
        .component()
        .unwrap()
        .to_vec();
    assert!(
        run_bytes.windows(3).any(|w| w == b"run"),
        "a source `run` is exported as `run`"
    );
}

/// A compile-only module — an export whose function is NON-nullary — no longer FAILS at LAYOUT
/// (the old `find(nullary)` assumption is gone). With a concretely-typed parameter the pipeline now
/// reaches the component-surface stage and declines there (parameterized exports not yet
/// oracle-verified) — a clean decline, NOT the old "no nullary entry" crash. This pins exactly
/// where the `compile` ABI work resumes. (A fully-polymorphic `(compile b) b` declines even earlier
/// at inference on the unsolved parameter type; here we give `b` a use that fixes it to Int.)
#[test]
fn compile_only_module_no_longer_requires_a_nullary_entry() {
    let node = ast::read("(do (def (compile b) (+ b 1)) (export compile))").expect("parse");
    let out = compile_program(&node);
    // No component yet (parameterized surface unrealized) — but a clean diagnostic, and crucially
    // NOT the old nullary-entry failure.
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 1);
    let msg = &out.diagnostics[0].message;
    assert!(
        !msg.contains("nullary"),
        "layout must not reject for lack of a nullary entry: {msg}"
    );
    assert!(
        msg.contains("parameterized"),
        "should decline on the parameterized surface: {msg}"
    );
}

/// A program with no `(export …)` is not a program (nothing is public) — a clean decline.
#[test]
fn no_export_declines() {
    let node = ast::read("(do (def (main) 42))").expect("parse");
    let out = compile_program(&node);
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 1);
}

/// The general artifact ABI: an `ast` artifact in → a `component` artifact out, no error
/// diagnostics.
#[test]
fn artifact_abi_roundtrip() {
    // A `run`-named entry so the byte count matches the 89-byte anchor (verbatim naming — a
    // `main` entry would be 91 bytes, the extra byte per `main`/`run` length in the export records).
    let bytes = ast::encode(&ast::read("(do (def (run) 42) (export run))").unwrap());
    let out = compile(&[Artifact::new(Artifact::KIND_AST, bytes)]);
    assert!(out.diagnostics.is_empty(), "no diagnostics on success");
    assert_eq!(out.artifacts.len(), 1);
    assert_eq!(out.artifacts[0].kind, Artifact::KIND_COMPONENT);
    assert_eq!(out.artifacts[0].bytes.len(), 89);
}

/// An unsupported form (a float literal — a later phase) declines cleanly: no component
/// artifact, one error diagnostic (decline-don't-miscompile — never an opaque empty byte string).
#[test]
fn unsupported_program_declines_with_diagnostic() {
    let bytes = ast::encode(&program_v2("1.5"));
    let out = compile(&[Artifact::new(Artifact::KIND_AST, bytes)]);
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 1);
    assert_eq!(out.diagnostics[0].severity, Severity::Error);
}

/// A missing `ast` input artifact is an error diagnostic, not a panic.
#[test]
fn missing_ast_artifact_is_a_diagnostic() {
    let out = compile(&[]);
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 1);
}

// ─── compile-time folding (the one evaluator) ─────────────────────────────────────────────

/// A constant arithmetic expression FOLDS to a single `i64.const` — no runtime add/overflow guard.
/// The core `run` body for `(+ 2 3)` is exactly `i64.const 5` then `end` (0x42 0x05 0x0b) — proof the
/// evaluator collapsed the operation, not that it merely compiled.
#[test]
fn constant_arith_folds_to_a_single_const() {
    let bytes = compile_program(&program_v2("(+ 2 3)")).component().unwrap().to_vec();
    // The code body carries `i64.const 5` = 0x42 0x05; a runtime add would carry local.get/i64.add
    // and the overflow-guard `unreachable` (0x00). Assert the const is present and no add (0x7c) /
    // unreachable (0x00) guard bytes are in the code region.
    assert!(bytes.windows(2).any(|w| w == [0x42, 0x05]), "expected a folded i64.const 5");
    // `(+ 2 3)` folded leaves the function body tiny; a runtime checked-add is far larger. Pin the
    // whole component stays small (the 89-byte scalar anchor + a one-byte const payload).
    assert!(bytes.len() <= 92, "folded constant should be a minimal body, got {} bytes", bytes.len());
}

/// A constant operation whose defined outcome is a trap is a COMPILE-TIME diagnostic (CDZ0304), not a
/// shipped runtime trap: overflow, divide-by-zero, `Int64.min / -1`, and an out-of-range shift.
#[test]
fn constant_trap_is_cdz0304() {
    for body in [
        "(+ Int64.max 1)",   // overflow
        "(- Int64.min 1)",   // overflow
        "(* Int64.max 2)",   // overflow
        "(/ 5 0)",           // divide by zero
        "(% 5 0)",           // modulo by zero
        "(/ Int64.min -1)",  // MIN / -1 overflow
        "(<< 1 64)",         // shift count out of range
        "(<< 1 -1)",         // negative shift count
        "(>> 256 64)",       // right-shift count out of range
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should be rejected, not compiled");
        assert_eq!(
            out.diagnostics[0].code.as_deref(),
            Some("CDZ0304"),
            "{body:?} should be a constant-trap rejection"
        );
    }
}

/// A constant trap in a branch the fold proves UNREACHED is DROPPED (dead-code elimination), so the
/// program compiles — reachability falls out of the fold, no separate analysis. `(if false <trap> 42)`
/// compiles to 42; `(if true 42 <trap>)` likewise drops the else.
#[test]
fn unreached_constant_trap_is_dropped() {
    for body in [
        "(if false (+ Int64.max 1) 42)",
        "(if true 42 (/ 5 0))",
        "(if false (<< 1 64) 7)",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(
            out.component().is_some(),
            "{body:?} should compile (the trapping branch is unreachable): {:?}",
            out.diagnostics
        );
    }
}

/// `% -1` is 0 for every dividend, even at `Int64.min` — the fold must NOT manufacture the `/ -1`
/// overflow trap for modulo (wasm `rem_s` defines it as 0). `(% Int64.min -1)` folds to 0, compiles.
#[test]
fn modulo_by_minus_one_folds_to_zero_not_a_trap() {
    let out = compile_program(&program_v2("(% Int64.min -1)"));
    assert!(out.component().is_some(), "`(% Int64.min -1)` must fold to 0, not trap: {:?}", out.diagnostics);
}

/// A constant trap the compiler PROVES by inlining a constant-argument call fails the build (the
/// laundered-constant case). `(def (div a b) (/ a b)) (div 5 0)` → CDZ0304, not a shipped runtime trap.
#[test]
fn laundered_constant_trap_through_a_call_is_cdz0304() {
    let src = "(do (def (div a b) (/ a b)) (def (main) (div 5 0)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_none(), "an inlined constant divide-by-zero should be rejected");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0304"));
}

/// A call with a constant argument β-reduces: `(def (double n) (* n 2)) (double 21)` folds to 42.
#[test]
fn constant_call_beta_reduces() {
    let src = "(do (def (double n) (* n 2)) (def (main) (double 21)) (export main))";
    let bytes = compile_program(&ast::read(src).expect("parse")).component().unwrap().to_vec();
    assert!(bytes.windows(2).any(|w| w == [0x42, 42]), "expected the call to fold to i64.const 42");
}

/// The multi-diagnostic ABI: TWO reached constant traps in one module report BOTH, not just the first
/// (build-tool-interface.md, Amendment 0.8.0). Two exported nullary entries, each a constant trap.
#[test]
fn multiple_constant_traps_report_all() {
    let src = "(do (def (a) (/ 5 0)) (def (b) (+ Int64.max 1)) (export a b))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics.len(), 2, "both constant traps should be reported");
    assert!(out.diagnostics.iter().all(|d| d.code.as_deref() == Some("CDZ0304")));
}

// ─── modules (a compile-time record of exports; folds away) ───────────────────────────────

/// A module compiles: its name binds to a record of its exports, `(. m f)` projects an export, and
/// the whole module folds away — the program reduces to a scalar component. Structural (values are
/// checked by the host tests).
#[test]
fn module_forms_compile() {
    for src in [
        // A nullary function export, applied via `((. m f) unit)`.
        "(do (module m (def (answer) 42) (export answer)) (def (main) ((. m answer) unit)) (export main))",
        // A value-def export, projected directly.
        "(do (module m (def v 7) (export v)) (def (main) (. m v)) (export main))",
        // A sibling call inside the module.
        "(do (module lib (def (dbl x) (* x 2)) (def (f x) (+ (dbl x) 1)) (export f)) (def (main) ((. lib f) 3)) (export main))",
        // Two exports.
        "(do (module m (def (one) 1) (def (two) 2) (export one two)) (def (main) (+ ((. m one) unit) ((. m two) unit))) (export main))",
    ] {
        let out = compile_program(&ast::read(src).expect("parse"));
        assert!(out.component().is_some(), "expected a component for {src:?}: {:?}", out.diagnostics);
    }
}

/// A module with two definitions of the same name is CDZ0201 (a record's field set is fixed — the
/// same ill-formedness a duplicate record field is).
#[test]
fn duplicate_module_definition_is_cdz0201() {
    let src = "(module m (def (f) 1) (def (f) 2) (def (main) (f)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// A module ENCAPSULATES its non-exported internals: the parent cannot reach a name the module does
/// not export. `(. m secret)` where `secret` is not exported is a type error (the record lacks it).
#[test]
fn module_hides_non_exported_definitions() {
    let src = "(do (module m (def (secret) 1) (def (pub) 2) (export pub)) (def (main) ((. m secret) unit)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_none(), "a non-exported member must not be reachable");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"));
}

/// A nested module: a module inside a module, reached by chained projection `(. (. outer inner) f)`.
#[test]
fn nested_module_compiles() {
    let src = "(do (module outer (module inner (def (v) 5) (export v)) (export inner)) (def (main) ((. (. outer inner) v) unit)) (export main))";
    let out = compile_program(&ast::read(src).expect("parse"));
    assert!(out.component().is_some(), "nested module should compile: {:?}", out.diagnostics);
}

/// Module encapsulation is TRANSITIVE: a grandparent reaches a nested submodule ONLY when every
/// intermediate module re-exports the path. An unexported submodule/member is CDZ0201 (the record
/// lacks the field) — there is no ambient reach-through into a nested scope.
#[test]
fn module_encapsulation_is_transitive() {
    // Full re-export chain: reachable via chained projection.
    let ok = "(do (module outer (module mid (def (v) 5) (export v)) (export mid)) (def (main) ((. (. outer mid) v) unit)) (export main))";
    assert!(compile_program(&ast::read(ok).unwrap()).component().is_some());

    // `outer` does not export `mid` → the grandparent cannot project it.
    let no_mid = "(do (module outer (module mid (def (v) 5) (export v)) (export)) (def (main) ((. (. outer mid) v) unit)) (export main))";
    let out = compile_program(&ast::read(no_mid).unwrap());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"), "unexported submodule is unreachable");

    // `mid` does not export `deep` → the grandchild is unreachable even though `outer` exports `mid`.
    let no_deep = "(do (module outer (module mid (module deep (def (v) 5) (export v)) (export)) (export mid)) (def (main) ((. (. (. outer mid) deep) v) unit)) (export main))";
    let out = compile_program(&ast::read(no_deep).unwrap());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0201"), "unexported grandchild is unreachable");
}

/// A module may export MORE than the consumer uses — an unused export must not block compilation.
/// (Regression: a nullary export was once unit-wrapped with a fake `_:Unit` param whose type an
/// unused export left unsolved; the nullary-as-value convention — `((. m f) unit)` drops the unit —
/// removed the fake param.)
#[test]
fn module_with_unused_export_compiles() {
    let src = "(do (module m (def (one) 1) (def (two) 2) (export one two)) (def (main) ((. m one) unit)) (export main))";
    let out = compile_program(&ast::read(src).unwrap());
    assert!(out.component().is_some(), "an unused export must not block compilation: {:?}", out.diagnostics);
}

// ─── intrinsics (prelude-record built-in operations, lowered to wasm at select) ───────────

/// A wrapping Int64 intrinsic projected from the `Int64` prelude record applies and compiles; a
/// constant application folds to the wrapped value (never a CDZ0304 — wrapping does not trap).
#[test]
fn wrapping_intrinsics_compile_and_fold() {
    // Folds to a single const (structural: a component is produced).
    for body in [
        "(Int64.wrapping-add 20 22)",
        "(Int64.wrapping-add Int64.max 1)", // wraps to MIN — NO trap
        "(Int64.wrapping-sub Int64.min 1)",
        "(Int64.wrapping-mul Int64.max 2)",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_some(), "expected a component for {body:?}: {:?}", out.diagnostics);
    }
    // `(Int64.wrapping-add 20 22)` folds to exactly `i64.const 42` (0x42 0x2a) — no runtime add.
    let bytes = compile_program(&program_v2("(Int64.wrapping-add 20 22)")).component().unwrap().to_vec();
    assert!(bytes.windows(2).any(|w| w == [0x42, 0x2a]), "wrapping-add should fold to i64.const 42");
}

/// `Int64.max`/`Int64.min` still resolve to their constants (now prelude-record fields, not a special
/// case); a bare (unapplied) intrinsic value declines (no first-class runtime intrinsic values yet).
#[test]
fn int64_constants_and_bare_intrinsic() {
    assert!(compile_program(&program_v2("Int64.max")).component().is_some());
    assert!(compile_program(&program_v2("Int64.min")).component().is_some());
    // A bare projected intrinsic value is not runtime-emittable → declines (uncoded, a later phase).
    let out = compile_program(&program_v2("(. Int64 wrapping-add)"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code, None, "a bare intrinsic value declines, not a coded reject");
}

/// A user binding named `Int64` SHADOWS the prelude module (built-in modules are ordinary records
/// under ordinary scope — no privileged name recognition).
#[test]
fn user_binding_shadows_prelude_module() {
    let out = compile_program(&ast::read("(do (def (Int64) 5) (def (main) (Int64)) (export main))").unwrap());
    assert!(out.component().is_some(), "a user `Int64` def must shadow the prelude: {:?}", out.diagnostics);
}

/// Layer 1 first-class types: `(: e T)` annotations where `T` is a scalar type name compile and
/// check correctly. `(: 42 Int64)` typechecks; `(: 42 Bool)` is CDZ0203 (annotation mismatch).
#[test]
fn layer1_scalar_type_annotations() {
    // Valid annotations — expression type matches the annotation.
    assert!(compile_program(&program_v2("(: 42 Int64)")).component().is_some());
    let out = compile_program(&program_v2("(: true Bool)"));
    assert!(out.component().is_some(), "expected (: true Bool) to compile: {:?}", out.diagnostics);
    assert!(compile_program(&program_v2("(: (+ 1 2) Int64)")).component().is_some());

    // Invalid annotations — expression type contradicts the annotation (CDZ0203).
    let out = compile_program(&program_v2("(: 42 Bool)"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"));

    let out = compile_program(&program_v2("(: true Int64)"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"));

    let out = compile_program(&program_v2("(: (tuple 1 2) Int64)"));
    assert!(out.component().is_none());
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"));
}

/// Layer 1 first-class types: a bare scalar type name (`Int64`, `Bool`, etc.) as a value types as
/// `Ty::Type` and should compile (though it can't cross to runtime — the fence will catch that).
/// `(let ((t Int64)) t)` binds `t` to the type-value `Int64`.
#[test]
fn layer1_type_values() {
    // A type name bound to a local types correctly. (It can't be returned from main yet — that
    // would trigger the erasure fence since main's result would need to cross to runtime.)
    let out = compile_program(&program_v2("(let ((t Int64)) 42)"));
    assert!(out.component().is_some(), "a type-value local should bind: {:?}", out.diagnostics);
}

/// Layer 2 first-class parametric types: `(: e (List Int64))` and friends type-check via the
/// type-builder Intrinsics (List/Map/Set/Tuple/Option/Result bound as prelude Intrinsic singletons,
/// their `fold_const` building the compound Ty). A matching annotation compiles; a mismatch is CDZ0203.
#[test]
fn layer2_parametric_type_annotations() {
    // Matching compound annotations compile. (A bare Set/Map cannot render at the run boundary yet —
    // an unrelated later phase — so those two are wrapped in a `size` op, exactly as the existing
    // `map_set_forms_compile` test does; the annotation machinery is still fully exercised.)
    for body in [
        "(: (list 1 2 3) (List Int64))",
        "(Set.size (: (set 1 2) (Set Int64)))",
        "(Map.size (: (map (1 2)) (Map Int64 Int64)))",
        "(: (tuple 1 true) (Tuple Int64 Bool))",
        "(: (Some 42) (Option Int64))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_some(),
            "expected {body:?} to compile: {:?}", out.diagnostics);
    }

    // Element-type mismatch inside a compound annotation is CDZ0203.
    for body in [
        "(: (list 1 2) (List Bool))",
        "(: (tuple 1 2) (Tuple Int64 Bool))",
        "(: (Some 42) (Option Bool))",
    ] {
        let out = compile_program(&program_v2(body));
        assert!(out.component().is_none(), "{body:?} should be rejected");
        assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"),
            "{body:?} should be CDZ0203");
    }
}

/// A bare, unapplied parametric type constructor cannot cross to runtime — it declines (UNCODED),
/// like any bare intrinsic; and a type-value it builds leaking to runtime is the erasure fence
/// (CDZ0305). Neither is a coded reject on a *valid* program — this is decline/fence discipline.
#[test]
fn layer2_type_ctor_as_value_declines() {
    // A type-value returned from main must hit the erasure fence (CDZ0305), not emit.
    let out = compile_program(&program_v2("(List Int64)"));
    assert!(out.component().is_none(), "a bare type-value cannot cross to runtime");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0305"));

    // A type-builder bound to a local (never crossing to runtime) is fine — it inlines (is_transient).
    let out = compile_program(&program_v2("(let ((l List)) 42)"));
    assert!(out.component().is_some(),
        "a type-builder bound but not run should compile: {:?}", out.diagnostics);
}

/// Layer 1: `(const e)` asserts `e` fully compile-time-reduces. A constant integer compiles; a
/// runtime expression should eventually be rejected by the fence (though for Layer 1, the fence
/// logic isn't fully implemented yet — we're just ensuring the form parses and types).
#[test]
fn layer1_const_form() {
    // A constant value in a `const` compiles.
    assert!(compile_program(&program_v2("(const 42)")).component().is_some());
    assert!(compile_program(&program_v2("(const (+ 20 22))")).component().is_some());

    // A const form wrapping a non-const expr will eventually be rejected by the fence, but for now
    // it at least types correctly (the fence implementation is the next step).
}

/// Compile-time closures (Increment A): lambdas are transient compile-time values the eval fold
/// β-reduces. An immediately-applied lambda, a let-bound lambda, a lambda passed to a named HOF,
/// a curried application completed to full arity, and a closure factory (capturing a value) all
/// reduce at compile time and emit correctly. A lambda stored in a tuple element and projected
/// also reduces. These cases witness that Increment A unblocks the core `09-functions.sexp` cases.
#[test]
fn compile_time_closures() {
    // Immediately-applied lambda: `((fn (x) (+ x 1)) 5)` → 6 (β-reduces to `(+ 5 1)` → 6).
    let out = compile_program(&program_v2("((fn (x) (+ x 1)) 5)"));
    assert!(out.component().is_some(), "immediate lambda application should β-reduce: {:?}", out.diagnostics);

    // Let-bound lambda: `(let ((inc (fn (x) (+ x 1)))) (inc 10))` → 11 (let inlines the transient).
    let out = compile_program(&program_v2("(let ((inc (fn (x) (+ x 1)))) (inc 10))"));
    assert!(out.component().is_some(), "let-bound lambda should β-reduce on application: {:?}", out.diagnostics);

    // Named HOF with lambda: `(def (ap g v) (g v))` `(ap (fn (x) (* x 2)) 7)` → 14 (force-inline).
    let prog = "(do (def (ap g v) (g v)) (def (main) (ap (fn (x) (* x 2)) 7)) (export main))";
    let out = compile_program(&ast::read(prog).unwrap());
    assert!(out.component().is_some(), "named HOF with lambda arg should β-reduce: {:?}", out.diagnostics);

    // Curried application: `((add 3) 4)` where `add` is 2-arity → spine collapses to `Call{add,[3,4]}` → 7.
    let prog = "(do (def (add x y) (+ x y)) (def (main) ((add 3) 4)) (export main))";
    let out = compile_program(&ast::read(prog).unwrap());
    assert!(out.component().is_some(), "curried application completed at compile time should reduce: {:?}", out.diagnostics);

    // Closure factory: `(adder 10)` returns a lambda capturing `n=10`; `((adder 10) 5)` → 15.
    let prog = "(do (def (adder n) (fn (x) (+ x n))) (def (main) ((adder 10) 5)) (export main))";
    let out = compile_program(&ast::read(prog).unwrap());
    assert!(out.component().is_some(), "closure factory should capture and β-reduce in place: {:?}", out.diagnostics);

    // Lambda stored in tuple: `((tuple.0 (tuple (fn (x) (+ x 1)) 9)) 5)` → 6 (projection + β-reduce).
    let out = compile_program(&program_v2("((. (tuple (fn (x) (+ x 1)) 9) 0) 5)"));
    assert!(out.component().is_some(), "lambda in tuple element should project and β-reduce: {:?}", out.diagnostics);
}

/// Irrefutable binding patterns (Increment A) — `let`, `def` param, `fn` param accept tuple patterns.
/// A destructuring binder desugars to a single-arm match; the tuple path is already green (tests.rs:248).
#[test]
fn binding_patterns_compile() {
    // LET with tuple pattern — destructure + sum.
    let out = compile_program(&program_v2("(let (((tuple a b) (tuple 3 4))) (+ a b))"));
    assert!(out.component().is_some(), "let tuple pattern: {:?}", out.diagnostics);

    // NESTED tuple pattern.
    let out = compile_program(&program_v2("(let (((tuple a (tuple b c)) (tuple 1 (tuple 2 3)))) (+ a (+ b c)))"));
    assert!(out.component().is_some(), "nested tuple pattern: {:?}", out.diagnostics);

    // DEF param with tuple pattern — `fst` extracts first element (used via call).
    let prog = "(do (def (fst p) (match p ((tuple a b) a))) (def (main) (fst (tuple 7 8))) (export main))";
    let out = compile_program(&ast::read(prog).unwrap());
    assert!(out.component().is_some(), "def param (via match baseline): {:?}", out.diagnostics);

    // Actually test pattern param - define inline and apply.
    let out = compile_program(&program_v2("((fn ((tuple a b)) (+ a b)) (tuple 10 20))"));
    assert!(out.component().is_some(), "fn param pattern applied: {:?}", out.diagnostics);

    // FN param with tuple pattern.
    let out = compile_program(&program_v2("((fn ((tuple a b)) (+ a b)) (tuple 3 4))"));
    assert!(out.component().is_some(), "fn param tuple pattern: {:?}", out.diagnostics);

    // WILDCARD discard.
    let out = compile_program(&program_v2("(let ((_ (tuple 1 2))) 42)"));
    assert!(out.component().is_some(), "wildcard discard: {:?}", out.diagnostics);

    // ANNOTATED binder — accept (fn param).
    let out = compile_program(&program_v2("((fn ((: x Int64)) x) 5)"));
    assert!(out.component().is_some(), "annotated param: {:?}", out.diagnostics);

    // Annotated tuple pattern (let).
    let out = compile_program(&program_v2("(let (((: (tuple a b) (Tuple Int64 Int64)) (tuple 1 2))) (+ a b))"));
    assert!(out.component().is_some(), "annotated tuple pattern: {:?}", out.diagnostics);
}

/// Binding patterns must be irrefutable — refutable patterns (ctors, literals) are rejected with CDZ0210.
#[test]
fn refutable_binding_pattern_rejects() {
    // A multi-variant ctor in binding position → CDZ0210 (non-exhaustive).
    let out = compile_program(&program_v2("(let (((Some x) (Some 5))) x)"));
    assert!(out.component().is_none(), "Some binding should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0210"), "refutable ctor → CDZ0210");

    // A literal in binding position → CDZ0210 (fn param with literal).
    let out = compile_program(&program_v2("((fn (0) 42) 0)"));
    assert!(out.component().is_none(), "literal param should reject: {:?}", out.diagnostics);
    if out.diagnostics.is_empty() || out.diagnostics[0].code.is_none() {
        panic!("Expected CDZ0210, got: {:?}", out.diagnostics);
    }
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0210"), "literal → CDZ0210");

    // Boolean literal.
    let out = compile_program(&program_v2("(let ((true v)) 1)"));
    assert!(out.component().is_none(), "bool literal should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0210"));
}

/// Non-linear patterns (a name repeated) are rejected with CDZ0102.
#[test]
fn nonlinear_pattern_rejects() {
    // Flat non-linear — same name twice in one tuple.
    let out = compile_program(&program_v2("(let (((tuple x x) (tuple 1 2))) x)"));
    assert!(out.component().is_none(), "flat non-linear should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0102"), "flat non-linear → CDZ0102");

    // NESTED non-linear — name repeated across sub-patterns (the anticipatory-pinned case).
    let out = compile_program(&program_v2("(let (((tuple x (tuple x y)) (tuple 1 (tuple 2 3)))) x)"));
    assert!(out.component().is_none(), "nested non-linear should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0102"), "nested non-linear → CDZ0102");

    // Non-linear in MATCH arm (the linearity check also fires for match).
    let out = compile_program(&program_v2("(match (tuple 1 2) ((tuple x x) x))"));
    assert!(out.component().is_none(), "match arm non-linear should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0102"));
}

/// Annotated binder contradiction — annotation type does not unify with value → CDZ0203.
#[test]
fn annotated_binding_contradiction_rejects() {
    let out = compile_program(&program_v2("(let (((: x Bool) 42)) x)"));
    assert!(out.component().is_none(), "annotation contradiction should reject");
    assert_eq!(out.diagnostics[0].code.as_deref(), Some("CDZ0203"), "annotation mismatch → CDZ0203");
}

/// Record binding patterns and single-variant-sum patterns are Increment B — decline, not reject.
#[test]
fn increment_b_patterns_decline() {
    // Record pattern → decline (not a reject).
    let out = compile_program(&program_v2("(let (((record (a x) (b y)) (record (a 1) (b 2)))) x)"));
    assert!(out.component().is_none(), "record pattern should decline");
    // Decline means no CDZ code, just an internal decline message.
    assert!(out.diagnostics[0].code.is_none(), "record pattern declines (no code)");
}
