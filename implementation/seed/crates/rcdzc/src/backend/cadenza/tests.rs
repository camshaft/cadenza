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
fn a_non_constant_body_declines_cleanly() {
    // A body the optimizer cannot fold and that the backend does not YET lower: `(+ x 1)` over a
    // parameter is a runtime `Core::Arith` (arithmetic emit is B1b). B1a now ACCEPTS the parameter
    // signature `(: x Int64)`, so the decline here is attributed to the Arith body, not the param.
    let err = compile_cadenza("(module m (def (inc (: x Int64)) (+ x 1)) (export inc))")
        .expect_err("an arithmetic body must decline until B1b");
    assert!(
        err.iter().any(|m| m.contains("does not yet")),
        "expected a Cadenza-backend decline, got: {err:?}"
    );
}
// (A function-typed-parameter decline path exists in `emit_def` — `type_ast` returns `None` — but a
// higher-order program that reaches it needs closures/calls (B3), so it is witnessed there, not here:
// an exported fn-param def is rejected at the boundary before the backend anyway.)
