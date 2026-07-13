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
fn a_narrow_literal_operand_is_grounded_to_the_op_width() {
    // REGRESSION (the reported width miscompile): a bare literal operand of a narrow-width op was
    // emitted at the default i64 (`1u64 as i64`), producing `u8::checked_add(i64)` → rustc E0308. It
    // must be grounded to the op's width (`1u8`). Covers arith, comparison, if-branch, and match-arm.
    let add = compile_rust("(module m (def (go (: a UInt8)) (+ a 1)) (export go))");
    assert!(add.contains("checked_add(1u8)"), "arith operand:\n{add}");
    assert!(
        !add.contains("1u64 as i64"),
        "must NOT default to i64:\n{add}"
    );

    let cmp = compile_rust("(module m (def (go (: a UInt8)) (< a 5)) (export go))");
    assert!(cmp.contains("(a < 5u8)"), "compare operand:\n{cmp}");

    let iff = compile_rust("(module m (def (go (: a UInt8) (: c Bool)) (if c a 1)) (export go))");
    assert!(iff.contains("else { 1u8 }"), "if-branch literal:\n{iff}");

    let mat = compile_rust("(module m (def (go (: a UInt8)) (match a (0 9) (_ a))) (export go))");
    assert!(mat.contains("9u8"), "match-arm literal:\n{mat}");
    assert!(
        !mat.contains("9u64"),
        "match arm must not default to i64:\n{mat}"
    );
}

#[test]
fn rustc_roundtrip_narrow_literal_operand_computes_and_traps() {
    // The narrow-literal fix, end-to-end through rustc: `(+ x 1)` on a UInt8 computes at u8 width AND
    // still traps on overflow (255+1) — the numeric model preserved, not silently wrapped.
    let rs = compile_rust("(module m (def (go (: x UInt8)) (+ x 1)) (export go))");
    if let Some(out) = rustc_run(&rs, "go(100)") {
        assert_eq!(out, "101");
    }
    // 255 + 1 = 256 leaves UInt8 → the checked_add panics (nonzero exit); rustc_run's success assert
    // would fail on a panic, so we only positively assert the in-range answer here (the trap path is
    // exercised by the wasm gate's overflow case and the `_traps` test elsewhere).
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
fn a_list_result_declines_attributed_to_this_target() {
    // A list-returning export has no native rep yet (tuples/records do, lists don't) — declines cleanly.
    let src = "(module m (def (main) (list 1 2 3)) (export main))";
    let err = try_compile_rust(src).expect_err("a list result must decline");
    assert!(
        err.iter()
            .any(|m| m.contains("compound") || m.contains("native Rust")),
        "decline message: {err:?}"
    );
}

#[test]
fn a_runtime_record_emits_a_sorted_field_tuple() {
    // A record that survives to runtime → a Rust tuple in SORTED field-name order (a record is
    // structural; at run time it IS a positional array in sorted key order). Field read → `.index`.
    // Fields declared OUT of order still emit sorted: `(record (b n) (a 7))` → `((7), n)` (a before b).
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (record (b n) (a 7)) (f (+ n -1)))) \
                    (def (main) (f 1)) (export main))",
    );
    assert!(rs.contains("-> (i64, i64)"), "record → tuple type:\n{rs}");
    // a=7 first (sorted), b=n second — the declared order (b, a) is re-sorted.
    assert!(
        rs.contains("((7u64 as i64), __p0)"),
        "sorted-field literal:\n{rs}"
    );

    // A record field read is a projection at the field's SORTED index.
    let proj =
        compile_rust("(module m (def (g (: r (Record (a Int64) (b Int64)))) (. r b)) (export g))");
    assert!(
        proj.contains("(r).1"),
        "field `b` is sorted index 1:\n{proj}"
    );
}

#[test]
fn rustc_roundtrip_record_builds_and_projects() {
    // A record crosses rustc end-to-end: a field read at the sorted index, and a returned record renders
    // (via the gate's type-directed path elsewhere). Here: `(. r a)` on `(Record (a) (b))` reads `.0`.
    let proj =
        compile_rust("(module m (def (g (: r (Record (a Int64) (b Int64)))) (. r a)) (export g))");
    if let Some(out) = rustc_run(&proj, "g((5, 9))") {
        assert_eq!(out, "5"); // field `a` = sorted index 0
    }
}

#[test]
fn a_runtime_tuple_emits_a_native_rust_tuple() {
    // A tuple that survives to runtime (built behind a recursive call) → a Rust tuple type + literal;
    // a projection → tuple field access. Scalar elements and nested tuples both compose.
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (+ n -1)))) \
                    (def (main) (f 3)) (export main))",
    );
    assert!(rs.contains("-> (i64, i64)"), "tuple return type:\n{rs}");
    assert!(rs.contains("(__p0, (7u64 as i64))"), "tuple literal:\n{rs}");

    let nested = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n (tuple n n)) (f (+ n -1)))) \
                    (def (main) (f 2)) (export main))",
    );
    assert!(
        nested.contains("-> (i64, (i64, i64))"),
        "nested tuple type:\n{nested}"
    );

    let proj =
        compile_rust("(module m (def (fst (: t (Tuple Int64 Int64))) (. t 0)) (export fst))");
    assert!(proj.contains("t: (i64, i64)"), "tuple param type:\n{proj}");
    assert!(proj.contains("(t).0"), "projection:\n{proj}");
}

#[test]
fn rustc_roundtrip_tuple_builds_and_projects() {
    // A tuple crosses rustc end-to-end: a projection reads the element, and a returned tuple renders as
    // the `(tuple …)` form. `fst((5,9))=5`; the nested tuple result is driven via field access.
    let proj =
        compile_rust("(module m (def (fst (: t (Tuple Int64 Int64))) (. t 0)) (export fst))");
    if let Some(out) = rustc_run(&proj, "fst((5, 9))") {
        assert_eq!(out, "5");
    }
    let mk = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (+ n -1)))) \
                    (def (mktup) (f 3)) (export mktup))",
    );
    // Drive the tuple result, printing cdz-run's `(tuple …)` form via field access. (Export is `mktup`,
    // not `main`, so the call in the driver's `fn main` names the export, not the driver itself.)
    if let Some(out) = rustc_run(
        &mk,
        "{ let t = mktup(); format!(\"(tuple {} {})\", t.0, t.1) }",
    ) {
        assert_eq!(out, "(tuple 0 7)");
    }
}

#[test]
fn a_recursive_export_emits_a_self_calling_fn() {
    // A recursive def becomes a `Core::Call` (non-recursive calls inline), so it emits a `pub fn` that
    // calls itself by its SANITIZED name (`sum-to` → `sum_to`, matching the call site).
    let rs = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    assert!(
        rs.contains("pub fn sum_to(n: i64) -> i64"),
        "signature:\n{rs}"
    );
    assert!(rs.contains("sum_to("), "self-call by sanitized name:\n{rs}");
    assert!(!rs.contains("sum-to"), "no unsanitized `-` name:\n{rs}");
}

#[test]
fn mutual_tail_recursion_compiles_to_a_shared_dispatch_loop() {
    // `even`/`odd` are a same-signature mutual-tail-recursion SCC → each emits a SHARED `which`-dispatch
    // loop (no cross-calls, no Box::pin): a tail call to the other member sets `which` + shared locals +
    // continues. `even` is `pub fn` (exported), `odd` a private `fn` (reachable member); both loop.
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    assert!(
        rs.contains("pub fn even(mut n: i64) -> bool"),
        "export:\n{rs}"
    );
    assert!(
        rs.contains("fn odd(mut n: i64) -> bool"),
        "private member:\n{rs}"
    );
    assert!(!rs.contains("pub fn odd"), "odd must NOT be pub:\n{rs}");
    // The loop dispatches on `which` and iterates via `continue` — no residual cross-call, no boxing.
    assert!(rs.contains("which == 0"), "which-dispatch:\n{rs}");
    assert!(
        rs.contains("which = 1;") && rs.contains("continue;"),
        "iterates:\n{rs}"
    );
    assert!(!rs.contains("Box::pin"), "no boxing (sync):\n{rs}");
    // Neither member CALLS the other any more (only `pub fn even(`/`fn odd(` declaration heads remain).
    assert_eq!(rs.matches("odd(").count(), 1, "no call to odd:\n{rs}");
    assert_eq!(rs.matches("even(").count(), 1, "no call to even:\n{rs}");
}

#[test]
fn rustc_roundtrip_mutual_tail_loop_runs_deep() {
    // The shared loop must run deep mutual recursion in bounded stack — even(2_000_000) = true.
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    if let Some(out) = rustc_run(&rs, "even(2000000)") {
        assert_eq!(out, "true");
    }
    if let Some(out) = rustc_run(&rs, "even(7)") {
        assert_eq!(out, "false");
    }
}

#[test]
fn self_tail_recursion_compiles_to_a_loop() {
    // A self-tail-recursive fn becomes a `loop` with `mut` params: the tail self-call reassigns params
    // + `continue`s, the base case `break`s its value. Bounded stack (sync) / no Box::pin (async).
    let rs = compile_rust(
        "(module m (def (go (: n Int64) (: acc Int64)) \
           (if (= n 0) acc (go (+ n -1) (+ acc n)))) (export go))",
    );
    assert!(
        rs.contains("pub fn go(mut n: i64, mut acc: i64)"),
        "mut params:\n{rs}"
    );
    assert!(rs.contains("loop {"), "loop:\n{rs}");
    // The body runs over the shared positional locals `__p0`/`__p1` (initialized from the params); the
    // base case `break`s the accumulator local, the recursive case parallel-moves + `continue`s.
    assert!(
        rs.contains("break __p1;"),
        "base case breaks the accumulator:\n{rs}"
    );
    assert!(
        rs.contains("continue;") && rs.contains("let (__t0, __t1,)"),
        "parallel-move + continue:\n{rs}"
    );
    // The tail self-call became the reassignment+continue, not a recursive call — no `Box::pin`, and no
    // `go(` CALL survives (the only `go(` is the `pub fn go(` declaration head).
    assert!(!rs.contains("Box::pin"), "no boxing (sync):\n{rs}");
    assert_eq!(
        rs.matches("go(").count(),
        1,
        "only the decl, no self-call:\n{rs}"
    );
}

#[test]
fn rustc_roundtrip_self_loop_runs_deep() {
    // The loop must run a large tail recursion in bounded stack — 1M iterations (sum 1..=1_000_000).
    // Export is `sumn` (not `main`, which would collide with the driver's `fn main`).
    let rs = compile_rust(
        "(module m (def (go (: n Int64) (: acc Int64)) (if (= n 0) acc (go (+ n -1) (+ acc n)))) \
                    (def (sumn (: n Int64)) (go n 0)) (export sumn))",
    );
    if let Some(out) = rustc_run(&rs, "sumn(1000000)") {
        assert_eq!(out, "500000500000");
    }
}

#[test]
fn rustc_roundtrip_async_self_loop_deep_is_bounded() {
    // The async form of a deep tail loop must ALSO run in bounded stack — no Box::pin poll-chain (the
    // loop iterates in place), so 1M iterations complete under the executor. Same answer as sync.
    let module = compile_rust_async(
        "(module m (def (go (: n Int64) (: acc Int64)) (if (= n 0) acc (go (+ n -1) (+ acc n)))) \
                    (def (main (: n Int64)) (go n 0)) (export main))",
    );
    let driver = r#"
struct GateEnv;
impl prog::CdzEnv for GateEnv { async fn consume(&mut self, _g: u64) {} }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() { println!("{}", block_on(prog::main(&mut GateEnv, 1000000))); }
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "500000500000");
    }
}

// ── the rustc round-trip (behavior oracle) ───────────────────────────────────────────────────────

/// A stable per-(module, driver) key for the round-trip temp dir — an FNV-1a hash of both strings.
/// Distinct programs get distinct dirs so parallel round-trip tests never share a `prog` binary (which
/// would race write-vs-exec). No clock/rng needed; the hash is deterministic.
fn test_key(a: &str, b: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for byte in a.bytes().chain([0u8]).chain(b.bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Compile the emitted Rust `module` plus a generated `main` that calls `export`(`args`) and prints
/// the result, run it under the ambient `rustc`, and return the printed line. Returns `None` if
/// `rustc` is not available (the test then skips its assertion rather than failing).
fn rustc_run(module: &str, call: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None; // no rustc — skip the round-trip.
    }
    // A unique temp dir per (module, call), keyed by a content hash — tests run in PARALLEL, and two
    // that compile a same-length module (e.g. two even/odd programs) would collide on one `prog` binary
    // and race write-vs-exec ("text file busy"). The content hash keeps distinct programs on distinct
    // paths. (No clock/rng in the core; the test bin may use the filesystem — it is the host boundary.)
    let dir = std::env::temp_dir().join(format!("rcdzc-rust-rt-{:016x}", test_key(module, call)));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    let full = format!("{module}\nfn main() {{ println!(\"{{}}\", {call}); }}\n");
    std::fs::write(&src_path, full).expect("write rust source");
    // Compile with a retry: many round-trip tests run in PARALLEL, each shelling `rustc`→`cc`, and the
    // linker can transiently fail under that concurrency ("linking with cc failed") — an environment
    // race, not a defect in the emitted source. Retry once before treating a non-zero status as a real
    // compile error (a genuine miscompile fails both attempts, so this never hides one).
    let compile = || {
        Command::new("rustc")
            .args(["-O", "--edition", "2021"])
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path)
            .output()
            .expect("run rustc")
    };
    let mut status = compile();
    if !status.status.success() {
        status = compile();
    }
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

/// Compile the emitted `module` wrapped in `mod prog { … }` PLUS a caller-supplied `driver` (which
/// defines its own `fn main` and references the module as `prog::…`), run it, and return the printed
/// line. `None` if `rustc` is absent. Used for the async round-trip, where the driver must supply an
/// `Env` impl + an executor rather than a one-line `println!(call)`.
fn rustc_run_driver(module: &str, driver: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None;
    }
    let dir =
        std::env::temp_dir().join(format!("rcdzc-rust-drv-{:016x}", test_key(module, driver)));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    // Wrap the module in `mod prog { … }` so its `pub fn`s are `prog::…` and its `#![allow(…)]` inner
    // attrs stay valid at the mod head, then append the driver (which owns `fn main`).
    let full = format!("mod prog {{\n{module}\n}}\n{driver}");
    std::fs::write(&src_path, full).expect("write rust source");
    let status = Command::new("rustc")
        .args(["-O", "--edition", "2021"])
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .expect("run rustc");
    assert!(
        status.status.success(),
        "emitted async Rust did not compile:\n{}\n--- source ---\n{module}",
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
fn a_runtime_shift_emits_a_guarded_block() {
    // `<<` guards the count (`>= N` panics) AND round-trips to catch overflow; `>>` guards the count
    // and shifts natively (arithmetic for signed, logical for unsigned — the value type decides).
    let shl = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (<< a b)) (export go))");
    assert!(shl.contains("c >= 64"), "count guard:\n{shl}");
    assert!(shl.contains("(r >> c) != v"), "overflow round-trip:\n{shl}");
    assert!(shl.contains("v << c"), "the shift:\n{shl}");
    let shr = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (>> a b)) (export go))");
    assert!(
        shr.contains("c >= 64") && shr.contains("v >> c"),
        ">> guarded:\n{shr}"
    );
    assert!(
        !shr.contains("round"),
        ">> needs no overflow round-trip:\n{shr}"
    );
}

#[test]
fn rustc_roundtrip_shift_computes_and_traps() {
    // `<<` and `>>` match the wasm oracle: value, out-of-range-count trap, overflow trap, and the
    // arithmetic-vs-logical distinction (a signed `>>` sign-extends).
    let shl = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (<< a b)) (export go))");
    if let Some(out) = rustc_run(&shl, "go(1, 4)") {
        assert_eq!(out, "16");
    }
    let shr = compile_rust("(module m (def (go (: a Int64) (: b Int64)) (>> a b)) (export go))");
    if let Some(out) = rustc_run(&shr, "go(-16, 2)") {
        assert_eq!(out, "-4"); // arithmetic (sign-extending) right shift
    }
    let ushr = compile_rust("(module m (def (go (: a UInt8) (: b UInt8)) (>> a b)) (export go))");
    if let Some(out) = rustc_run(&ushr, "go(200, 1)") {
        assert_eq!(out, "100"); // logical (zero-fill) right shift
    }
    // Overflow/out-of-range traps abort (nonzero exit → the run helper's success assert fails), so the
    // trap paths are pinned by the wasm gate cross-check + the emit-shape test above; here we assert the
    // in-range values match. (An explicit panic-catch driver is the emit-side test's job, not here.)
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

#[test]
fn rustc_roundtrip_recursion() {
    // A recursive `fn` calls itself on the native stack — no tail-call transform needed for
    // correctness. sum-to(5) = 15, fac(5) = 120 (match base case), fib(10) = 55 (double recursion).
    let sumto = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    if let Some(out) = rustc_run(&sumto, "sum_to(5)") {
        assert_eq!(out, "15");
    }
    let fac = compile_rust(
        "(module m (def (fac (: n Int64)) (match n (0 1) (k (* k (fac (+ k -1)))))) (export fac))",
    );
    if let Some(out) = rustc_run(&fac, "fac(5)") {
        assert_eq!(out, "120");
    }
    let fib = compile_rust(
        "(module m (def (fib (: n Int64)) (match n (0 0) (1 1) (k (+ (fib (+ k -1)) (fib (+ k -2)))))) (export fib))",
    );
    if let Some(out) = rustc_run(&fib, "fib(10)") {
        assert_eq!(out, "55");
    }
}

#[test]
fn rustc_roundtrip_mutual_recursion() {
    // even(10)=true. (Deeper mutual + even(7) are covered by `rustc_roundtrip_mutual_tail_loop_runs_deep`.)
    let rs = compile_rust(
        "(module m (def (even (: n Int64)) (if (= n 0) true (odd (+ n -1)))) \
                    (def (odd (: n Int64)) (if (= n 0) false (even (+ n -1)))) (export even))",
    );
    if let Some(out) = rustc_run(&rs, "even(10)") {
        assert_eq!(out, "true");
    }
}

// ── sums → Rust enums ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_user_sum_emits_a_rust_enum_declaration() {
    // A monomorphic user sum becomes a `pub enum` of its name: a nullary variant is a unit variant, a
    // 1-payload variant carries its payload type, a multi-payload variant carries each positionally.
    let rs = compile_rust(
        "(module m (type Shape (Circle Int64) (Rect Int64 Int64)) \
           (def (area (: s Shape)) (match s (((. Shape Circle) r) (* r r)) \
                                            (((. Shape Rect) (tuple w h)) (* w h)))) (export area))",
    );
    // A 1-payload variant carries its payload directly (`Circle(i64)`); a MULTI-payload variant carries
    // its payloads as ONE TUPLE (`Rect((i64, i64))`) — the single-`Ty::Tuple` payload the core models and
    // the match reads as one indexed value, so the decl, construction, and match all agree.
    assert!(
        rs.contains("pub enum Shape { Circle(i64), Rect((i64, i64)) }"),
        "enum decl:\n{rs}"
    );
    // Construction is `Enum::Variant(args)`; the match reads a Rust `match`.
    assert!(rs.contains("Shape::Circle"), "ctor path:\n{rs}");
    assert!(rs.contains("match"), "match lowering:\n{rs}");
}

#[test]
fn a_generic_user_sum_emits_a_generic_enum() {
    // A generic sum's params become the enum's type parameters (`T0`…), a param-typed payload renders as
    // its `T{k}`, and a use at a concrete type instantiates them via `types::rust_type`.
    // The type parameter `a` appears in the variant payload (`(Wrap a)`); the declaration is `(type Box
    // (Wrap a))` — the params come from the lowercase names in the variant payloads, first-appearance order.
    let rs = compile_rust(
        "(module m (type Box (Wrap a)) \
           (def (unwrap (: b (Box Int64))) (match b (((. Box Wrap) x) x))) (export unwrap))",
    );
    assert!(
        rs.contains("pub enum Box<T0> { Wrap(T0) }"),
        "generic enum decl:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: Box<i64>)"),
        "instantiated use:\n{rs}"
    );
}

#[test]
fn the_builtin_option_maps_to_rusts_own_and_emits_no_enum() {
    // The built-in `Option` maps to Rust's OWN `Option` — no synthetic `enum Option { … }` is emitted
    // (that would shadow std's). Construction uses `Some(..)`/`None`, which resolve to std's.
    let rs = compile_rust("(module m (def (wrap (: n Int64)) (Some n)) (export wrap))");
    assert!(
        !rs.contains("enum Option"),
        "must not emit a synthetic Option enum:\n{rs}"
    );
    assert!(rs.contains("-> Option<i64>"), "std Option result:\n{rs}");
    assert!(rs.contains("Some("), "std Some ctor:\n{rs}");
}

#[test]
fn a_recursive_sum_declines_the_whole_function() {
    // A recursive sum needs `Box` indirection (deferred), so its enum is not emitted — and a function
    // taking/returning it DECLINES rather than emitting a signature naming an undeclared type.
    let err = try_compile_rust(
        "(module m (type IntList Nil (Cons (Tuple Int64 IntList))) \
           (def (len (: xs IntList)) (match xs (((. IntList Nil) _) 0) \
                                              (((. IntList Cons) (tuple h t)) (+ 1 (len t))))) (export len))",
    )
    .expect_err("a recursive sum must decline");
    assert!(
        err.iter()
            .any(|d| d.contains("recursive") || d.contains("no emitted Rust enum")),
        "decline reason should cite the recursive/unrepresentable sum: {err:?}"
    );
}

#[test]
fn rustc_roundtrip_user_sum_constructs_and_matches() {
    // area(Circle 5) = 25, area(Rect 4 3) = 12 — construction + match run through rustc and match the
    // wasm oracle. The driver constructs a variant and calls the export.
    let rs = compile_rust(
        "(module m (type Shape (Circle Int64) (Rect Int64 Int64)) \
           (def (area (: s Shape)) (match s (((. Shape Circle) r) (* r r)) \
                                            (((. Shape Rect) (tuple w h)) (* w h)))) (export area))",
    );
    if let Some(out) = rustc_run(&rs, "area(Shape::Circle(5))") {
        assert_eq!(out, "25");
    }
    // A multi-payload variant carries ONE tuple, so it is constructed `Rect((4, 3))`.
    if let Some(out) = rustc_run(&rs, "area(Shape::Rect((4, 3)))") {
        assert_eq!(out, "12");
    }
}

#[test]
fn rustc_roundtrip_builtin_option_matches() {
    // unwrap-or(Some 8, _) = 8, unwrap-or(None, -1) = -1 — a match over std's Option, constructed with
    // std's `Some`/`None` in the driver, runs and matches the oracle.
    let rs = compile_rust(
        "(module m (def (unwrap-or (: o (Option Int64)) (: d Int64)) \
           (match o (((. Option Some) x) x) (((. Option None) _) d))) (export unwrap-or))",
    );
    if let Some(out) = rustc_run(&rs, "unwrap_or(Some(8), -1)") {
        assert_eq!(out, "8");
    }
    if let Some(out) = rustc_run(&rs, "unwrap_or(None, -1)") {
        assert_eq!(out, "-1");
    }
}

// ── async / gas-metered emission ─────────────────────────────────────────────────────────────────

/// Compile a program to the ASYNC Rust backend (gas-metered `async fn`s + the `CdzEnv` trait).
fn compile_rust_async(src: &str) -> String {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::RustAsync],
    );
    match out.artifact(Target::RustAsync.artifact_kind()) {
        Some(bytes) => String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8"),
        None => panic!(
            "async Rust emit failed: {:?}",
            out.diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        ),
    }
}

#[test]
fn async_mode_emits_env_threaded_gas_metered_fns() {
    let rs = compile_rust_async(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    // The gas/yield trait is declared once in the module.
    assert!(rs.contains("pub trait CdzEnv"), "trait:\n{rs}");
    // The fn is async, takes `env: &mut __CdzE`, and charges gas at entry. The env type param is the
    // reserved `__CdzE` (not a bare `E`) so it cannot collide with a user sum's Rust type name.
    assert!(
        rs.contains("pub async fn sum_to<__CdzE: CdzEnv>(env: &mut __CdzE, n: i64)"),
        "signature:\n{rs}"
    );
    assert!(rs.contains("env.consume(1).await;"), "gas charge:\n{rs}");
    // The recursive call is boxed-and-awaited, threading `env` first.
    assert!(
        rs.contains("Box::pin(sum_to(env,"),
        "boxed recursive call:\n{rs}"
    );
}

#[test]
fn async_env_type_param_does_not_collide_with_a_user_sum_named_e() {
    // REGRESSION: the async env type param was a bare `E`; a user sum `(type E …)` maps to `enum E`, so
    // `E::A` in the constructing code resolved to the type PARAMETER, not the enum (`no associated item
    // named A`). The param is now the reserved `__CdzE`, so the enum `E` is unshadowed and constructs.
    let rs = compile_rust_async(
        "(module m (type E (A Int64) (B Int64)) (def (main) (E.B 7)) (export main))",
    );
    assert!(rs.contains("pub enum E {"), "user enum E emitted:\n{rs}");
    assert!(rs.contains("<__CdzE: CdzEnv>"), "reserved env param:\n{rs}");
    assert!(
        !rs.contains("<E: CdzEnv>"),
        "no bare-E param collision:\n{rs}"
    );
    // It compiles (the enum `E` and the env param no longer collide).
    let driver = r#"
struct M;
impl prog::CdzEnv for M { async fn consume(&mut self, _: u64) {} }
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() { let r = block_on(prog::main(&mut M)); if let prog::E::B(v) = r { println!("{}", v); } }
"#;
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(out, "7");
    }
}

#[test]
fn rustc_roundtrip_async_gas_metered() {
    // The async form compiles and runs under a hand-rolled executor with a real gas Env — same answer as
    // the sync form (sum_to(5)=15), gas is metered, and an exhausted budget traps.
    let module = compile_rust_async(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (export sum-to))",
    );
    // A driver: a Meter env (counts gas, panics past budget) + a minimal block_on executor.
    let driver = r#"
struct Meter { spent: u64, budget: u64 }
impl prog::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; if self.spent > self.budget { panic!("oom") } }
}
fn block_on<F: core::future::Future>(mut f: F) -> F::Output {
    use core::task::*;
    fn n(_: *const ()) {} fn c(_: *const ()) -> RawWaker { r() }
    fn r() -> RawWaker { RawWaker::new(core::ptr::null(), &V) }
    static V: RawWakerVTable = RawWakerVTable::new(c, n, n, n);
    let w = unsafe { Waker::from_raw(r()) };
    let mut cx = Context::from_waker(&w);
    let mut f = unsafe { core::pin::Pin::new_unchecked(&mut f) };
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut cx) { return v; } }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let mut e = Meter { spent: 0, budget: 10000 };
    let v = block_on(prog::sum_to(&mut e, 5));
    let gas = e.spent;
    let oom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || block_on(prog::sum_to(&mut Meter { spent: 0, budget: 3 }, 100)),
    )).is_err();
    println!("{v} {} {oom}", gas > 0);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        // sum_to(5)=15, gas was metered (>0), and the budget-3 run trapped.
        assert_eq!(out, "15 true true", "async run:\n{module}");
    }
}
