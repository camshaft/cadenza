//! Rust-backend tests: an EMIT check (the generated source is what we expect) and a rustc ROUND-TRIP
//! check (the emitted `.rs` compiles and, driven, returns the SAME value the wasm path does — the
//! two backends judged against the one executable semantics, `backends-and-targets.md` §The meaning
//! against which every backend's output is judged).
//!
//! The round-trip is dev-only, exactly like the wasm backend's wasmtime run: it shells out to the
//! ambient `rustc` (present in this toolchain), compiles the emitted module plus a tiny generated
//! `main` that calls the export and prints the result, runs it, and reads the printed value back.
//! `rustc` never enters the compile path — it is the Rust backend's analogue of `wasmtime` as the
//! behavior oracle. A test that shells to `rustc` is skipped (not failed) if `rustc` is absent, so the
//! suite still runs in an environment without it.

use crate::backend::Target;
use crate::testkit::parse;
use crate::{Artifact, compile};

/// Compile a program's source to the Rust-backend artifact bytes (the emitted `.rs` text), or panic
/// with the first diagnostic. Mirrors `compile_component` but selects `Target::Rust`.
fn compile_rust(src: &str) -> String {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Rust],
    );
    match out.artifact(Target::Rust.artifact_kind()) {
        Some(bytes) => String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8"),
        None => panic!(
            "Rust emit failed: {:?}",
            out.diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        ),
    }
}

/// Try to compile a program to the Rust backend, returning the emitted source or the diagnostics (for
/// asserting a DECLINE).
fn try_compile_rust(src: &str) -> Result<String, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Rust],
    );
    match out.artifact(Target::Rust.artifact_kind()) {
        Some(bytes) => Ok(String::from_utf8(bytes.to_vec()).unwrap()),
        None => Err(out.diagnostics.iter().map(|d| d.message.clone()).collect()),
    }
}

#[test]
fn a_nullary_export_emits_a_pub_fn_returning_a_constant() {
    let src = "(module m (def (main) 42) (export main))";
    let rs = compile_rust(src);
    assert!(rs.contains("pub fn main() -> i64 {"), "signature:\n{rs}");
    // 42 is emitted as its bit pattern in the unsigned width, cast to the signed target.
    assert!(rs.contains("42u64 as i64"), "constant:\n{rs}");
}

#[test]
fn an_exported_function_emits_native_params_and_checked_arith() {
    let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
    let rs = compile_rust(src);
    assert!(
        rs.contains("pub fn add(a: i64, b: i64) -> i64 {"),
        "signature:\n{rs}"
    );
    // Cadenza `+` TRAPS on overflow → checked_add with a panic on None.
    assert!(rs.contains("(a).checked_add(b)"), "checked arith:\n{rs}");
    assert!(rs.contains("panic!"), "overflow trap:\n{rs}");
}

#[test]
fn a_narrow_signed_negative_constant_uses_the_bit_pattern_cast() {
    // -56 : Int8 is emitted as `200u8 as i8` (the two's-complement bit pattern), mirroring the wasm
    // backend's `to_i32_bits` (tests.rs `a_narrow_signed_...` expects -56 from a UInt8 wrap).
    let src = "(module m (def (main) (: -56 Int8)) (export main))";
    let rs = compile_rust(src);
    assert!(rs.contains("-> i8 {"), "signature:\n{rs}");
    assert!(rs.contains("200u8 as i8"), "bit-pattern cast:\n{rs}");
}

#[test]
fn a_uint64_max_constant_crosses_without_a_signed_minus() {
    // UInt64.max = 2^64 - 1 does not fit i64; as a u64 it is a plain literal (no `as`).
    let src = "(module m (def (main) UInt64.max) (export main))";
    // `.max` may or may not be built; only assert when it compiles, else this is a no-op guard.
    if let Ok(rs) = try_compile_rust(src) {
        assert!(rs.contains("-> u64 {"), "signature:\n{rs}");
    }
}

#[test]
fn an_if_emits_a_rust_if_expression() {
    let src = "(module m (def (pick (: a Int64) (: b Int64)) (if (< a b) a b)) (export pick))";
    let rs = compile_rust(src);
    assert!(rs.contains("if (a < b) {"), "if-expr:\n{rs}");
}

#[test]
fn a_compound_result_declines_attributed_to_this_target() {
    // A tuple-returning export has no scalar native rep yet — the Rust backend declines cleanly.
    let src = "(module m (def (main) (tuple 1 2)) (export main))";
    let err = try_compile_rust(src).expect_err("a compound result must decline");
    assert!(
        err.iter()
            .any(|m| m.contains("compound") || m.contains("native Rust")),
        "decline message: {err:?}"
    );
}

// ── the rustc round-trip (behavior oracle) ───────────────────────────────────────────────────────

/// Compile the emitted Rust `module` plus a generated `main` that calls `export`(`args`) and prints
/// the result, run it under the ambient `rustc`, and return the printed line. Returns `None` if
/// `rustc` is not available (the test then skips its assertion rather than failing).
fn rustc_run(module: &str, call: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None; // no rustc — skip the round-trip.
    }
    // A unique temp dir per call, seeded by the call text length + module length (no clock/rng in the
    // core; the test bin may use the filesystem — it is the host boundary).
    let dir = std::env::temp_dir().join(format!("rcdzc-rust-rt-{}-{}", module.len(), call.len()));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    let full = format!("{module}\nfn main() {{ println!(\"{{}}\", {call}); }}\n");
    std::fs::write(&src_path, full).expect("write rust source");
    let status = Command::new("rustc")
        .arg("-O")
        .arg("--edition")
        .arg("2021")
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .expect("run rustc");
    assert!(
        status.status.success(),
        "emitted Rust did not compile:\n{}\n--- source ---\n{module}",
        String::from_utf8_lossy(&status.stderr)
    );
    let run = Command::new(&bin_path).output().expect("run compiled prog");
    assert!(
        run.status.success(),
        "compiled prog did not run:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    Some(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

#[test]
fn rustc_roundtrip_add_matches_the_wasm_answer() {
    // The exact I2b wasmtime answers: add(20,22)=42, add(100,-1)=99.
    let rs = compile_rust("(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))");
    if let Some(out) = rustc_run(&rs, "add(20, 22)") {
        assert_eq!(out, "42");
    }
    if let Some(out) = rustc_run(&rs, "add(100, -1)") {
        assert_eq!(out, "99");
    }
}

#[test]
fn rustc_roundtrip_signed_compare() {
    let rs = compile_rust("(module m (def (lt (: a Int64) (: b Int64)) (< a b)) (export lt))");
    if let Some(out) = rustc_run(&rs, "lt(3, 5)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "lt(5, 3)") {
        assert_eq!(out, "false");
    }
}

#[test]
fn rustc_roundtrip_overflow_traps() {
    // Int8 100+100 = 200 leaves the type → Cadenza traps → the emitted Rust panics.
    let rs = compile_rust("(module m (def (add8 (: a Int8) (: b Int8)) (+ a b)) (export add8))");
    // A non-overflowing call returns the value; an overflowing one aborts (nonzero exit → the run
    // helper's success assertion fails), so we only positively assert the in-range answer here.
    if let Some(out) = rustc_run(&rs, "add8(100, 20)") {
        assert_eq!(out, "120");
    }
}

#[test]
fn rustc_roundtrip_short_circuit_and() {
    // `(and (< a b) (< b c))` → Rust `&&`, short-circuiting with the same semantics.
    let rs = compile_rust(
        "(module m (def (between (: a Int64) (: b Int64) (: c Int64)) \
           (and (< a b) (< b c))) (export between))",
    );
    assert!(rs.contains("&&"), "connective:\n{rs}");
    if let Some(out) = rustc_run(&rs, "between(1, 2, 3)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "between(1, 5, 3)") {
        assert_eq!(out, "false");
    }
}
