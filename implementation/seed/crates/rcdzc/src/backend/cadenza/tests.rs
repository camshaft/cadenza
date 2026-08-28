//! Cadenza-backend tests: the backend re-emits the optimized program as binary AST, and — the load-
//! bearing property — feeding that binary AST BACK through the compiler re-emits BYTE-IDENTICAL output
//! (round-trip idempotence). B0 covers constant-bodied nullary definitions; a non-constant body or a
//! parameterized definition DECLINES cleanly (no artifact), the frontier later increments push back.

use crate::backend::Target;
use crate::testkit::parse;
use crate::{Artifact, compile};

/// Compile a program's source to the Cadenza-backend artifact bytes (the emitted binary AST), or
/// `Err(diagnostics)` when the backend DECLINES (emits no artifact). Mirrors `compile_rust_result`.
fn compile_cadenza(src: &str) -> Result<Vec<u8>, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    compile_cadenza_bytes(ast_bytes)
}

/// Like [`compile_cadenza`] but takes already-encoded binary-AST bytes as the input artifact — the
/// round-trip step feeds the backend's OWN output straight back in.
fn compile_cadenza_bytes(ast_bytes: Vec<u8>) -> Result<Vec<u8>, Vec<String>> {
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Cadenza],
    );
    match out.artifact(Target::Cadenza.artifact_kind()) {
        Some(bytes) => Ok(bytes.to_vec()),
        None => Err(out.diagnostics.iter().map(|d| d.message.clone()).collect()),
    }
}

/// Assert the backend emits a NON-EMPTY artifact for `src` AND that re-compiling that artifact re-emits
/// BYTE-IDENTICAL output (the fixpoint / round-trip idempotence property). Returns the emitted bytes.
fn assert_roundtrips(src: &str) -> Vec<u8> {
    let emitted =
        compile_cadenza(src).unwrap_or_else(|d| panic!("Cadenza emit declined for `{src}`: {d:?}"));
    assert!(
        !emitted.is_empty(),
        "Cadenza emit produced empty bytes for `{src}`"
    );
    // The emitted binary AST must itself decode (it is a well-formed program).
    assert!(
        crate::codec::decode(&emitted).is_some(),
        "emitted binary AST failed to decode for `{src}`"
    );
    let reemitted = compile_cadenza_bytes(emitted.clone()).unwrap_or_else(|d| {
        panic!("re-compiling the emitted binary AST declined for `{src}`: {d:?}")
    });
    assert_eq!(
        emitted, reemitted,
        "Cadenza backend is not idempotent for `{src}`: emit ≠ re-emit"
    );
    emitted
}

#[test]
fn a_constant_int_definition_round_trips() {
    assert_roundtrips("(module m (def (answer) 42) (export answer))");
}

#[test]
fn constant_bool_string_char_float_definitions_round_trip() {
    // One export each so every def stays reachable (`layout.order` is the reachable set closed over
    // exports); a constant leaf of every B0-covered kind.
    assert_roundtrips(
        "(module m \
           (def (yes) true) \
           (def (greeting) \"hi\") \
           (def (letter) #\\a) \
           (def (half) 0.5) \
           (export yes) (export greeting) (export letter) (export half))",
    );
}

#[test]
fn several_constant_definitions_round_trip_together() {
    assert_roundtrips(
        "(module m \
           (def (a) 1) (def (b) 2) (def (c) 3) \
           (export a) (export b) (export c))",
    );
}

// ── B1a: parameters + a parameter reference ──────────────────────────────────────────────────────

#[test]
fn an_identity_function_over_a_scalar_param_round_trips() {
    // The minimal parameterized def: `(def (id (: x Int64)) x)`. Exercises the param signature
    // `(: x Int64)` (type via lower's `type_ast`) AND a `Core::Param` body reference (the bare name).
    // `x` is a runtime value, so the body does NOT fold — it stays a `Core::Param`, reaching the backend.
    assert_roundtrips("(module m (def (id (: x Int64)) x) (export id))");
}

#[test]
fn identity_functions_over_scalar_param_types_round_trip() {
    // A param of each B1a-representable scalar type — each an identity returning the param, so the body
    // is a bare `Core::Param` and the signature carries the type ascription rendered by `type_ast`.
    assert_roundtrips(
        "(module m \
           (def (i (: x Int64)) x) \
           (def (u (: x UInt8)) x) \
           (def (b (: x Bool)) x) \
           (def (f (: x Float64)) x) \
           (def (s (: x String)) x) \
           (export i) (export u) (export b) (export f) (export s))",
    );
}

#[test]
fn a_multi_parameter_identity_projection_round_trips() {
    // Two params; the body returns the FIRST — exercises multiple `(: p Ty)` slots in one signature and
    // a `Core::Param` selecting one of them by name (the un-returned param is still in the signature).
    assert_roundtrips("(module m (def (fst (: a Int64) (: b Bool)) a) (export fst))");
}

#[test]
fn a_not_yet_lowered_body_declines_cleanly() {
    // A body the optimizer cannot fold and that the backend does not YET lower: `(tuple x x)` over a
    // parameter is a runtime `Core::Tuple` (compound emit is B4). Confirms the decline path still fires
    // for an un-covered node now that params + operators + control are handled.
    let err = compile_cadenza("(module m (def (pair (: x Int64)) (tuple x x)) (export pair))")
        .expect_err("a compound body must decline until B4");
    assert!(
        err.iter().any(|m| m.contains("does not yet")),
        "expected a Cadenza-backend decline, got: {err:?}"
    );
}
// (A function-typed-parameter decline path exists in `emit_def` — `type_ast` returns `None` — but a
// higher-order program that reaches it needs closures/calls (B3), so it is witnessed there, not here:
// an exported fn-param def is rejected at the boundary before the backend anyway.)

// ── B1b: operators (arithmetic / comparison / string-order / float) + control (if / and / or / not) ──

#[test]
fn integer_arithmetic_and_comparison_round_trip() {
    // Each body is a runtime `Core::Arith`/`Core::Compare` over a parameter (non-foldable), re-emitted
    // `(<op> l r)`. Covers a spread of arithmetic + comparison operators in one program.
    assert_roundtrips(
        "(module m \
           (def (add1 (: x Int64)) (+ x 1)) \
           (def (sub1 (: x Int64)) (- x 1)) \
           (def (dbl  (: x Int64)) (* x 2)) \
           (def (lt10 (: x Int64)) (< x 10)) \
           (def (eq0  (: x Int64)) (= x 0)) \
           (def (ge5  (: x Int64)) (>= x 5)) \
           (export add1) (export sub1) (export dbl) (export lt10) (export eq0) (export ge5))",
    );
}

#[test]
fn control_flow_if_and_or_not_round_trip() {
    assert_roundtrips(
        "(module m \
           (def (clamp0 (: x Int64)) (if (< x 0) 0 x)) \
           (def (both (: a Bool) (: b Bool)) (and a b)) \
           (def (either (: a Bool) (: b Bool)) (or a b)) \
           (def (neg (: b Bool)) (not b)) \
           (export clamp0) (export both) (export either) (export neg))",
    );
}

#[test]
fn float_operators_round_trip() {
    // Float arithmetic + equality: the operands solve to `Float`, so `lower` selects the INTERNAL float
    // prims (`FMul`/`FAdd`/`FEq`), which re-emit as the SAME `*`/`+`/`=` surface operators — and re-solve
    // to the same prims on recompile (the shared-operator round-trip property).
    assert_roundtrips(
        "(module m \
           (def (scale (: x Float64)) (* x 2.0)) \
           (def (bump  (: x Float64)) (+ x 1.0)) \
           (def (isone (: x Float64)) (= x 1.0)) \
           (export scale) (export bump) (export isone))",
    );
}

#[test]
fn string_ordering_round_trips() {
    // `<` on `String` operands is a `Core::StrCmp` (content-lexicographic), distinct from integer
    // `Compare`; it re-emits as the same `<` surface operator.
    assert_roundtrips("(module m (def (early (: s String)) (< s \"m\")) (export early))");
}

// ── B2: binding (a kept multi-use `let`) ─────────────────────────────────────────────────────────

#[test]
fn a_multi_use_let_binding_round_trips() {
    // `y` binds a RUNTIME computation `(+ x 1)` used TWICE, so lowering KEEPS it as a `Core::Let`
    // (a single-use or constant binding would be copy-propagated away, leaving no `let`). The backend
    // re-emits it with a synthesized binding name + `LocalRef`s to it; the synthesis is deterministic,
    // so emit==re-emit byte-identically.
    assert_roundtrips("(module m (def (f (: x Int64)) (let ((y (+ x 1))) (+ y y))) (export f))");
}

#[test]
fn sequential_let_bindings_round_trip() {
    // `let*` sequencing: the second binding's value references the first, and both are multi-use, so
    // both are kept. Exercises the environment carrying an earlier binding's name into a later value.
    assert_roundtrips(
        "(module m (def (g (: x Int64)) \
           (let ((a (* x 2)) (b (+ a a))) (+ b b))) \
         (export g))",
    );
}
