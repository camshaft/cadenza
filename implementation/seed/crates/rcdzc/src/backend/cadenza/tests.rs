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

#[test]
fn a_non_constant_body_declines_cleanly() {
    // `(+ 1 2)` const-folds to `3`, so it does NOT exercise the decline — use a body the optimizer
    // cannot fold to a constant: a call to a parameterized helper of a runtime-shaped value. The
    // simplest non-foldable body that reaches a non-constant Core node is an addition of two DISTINCT
    // exported nullary results is still foldable; instead use a definition WITH A PARAMETER, whose body
    // references the parameter — that reaches `Core::Param`/`LocalRef`, which B0 declines.
    let err = compile_cadenza("(module m (def (inc (: x Int64)) (+ x 1)) (export inc))")
        .expect_err("a parameterized definition must decline in B0");
    assert!(
        err.iter().any(|m| m.contains("does not yet")),
        "expected a Cadenza-backend decline, got: {err:?}"
    );
}
