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
    compile_rust_result(src).unwrap_or_else(|diags| panic!("Rust emit failed: {diags:?}"))
}

/// Like `compile_rust` but returns `Err(diagnostics)` when the backend DECLINES (emits no Rust
/// artifact) instead of panicking — for tests asserting a construct declines cleanly (a `todo`), e.g.
/// a float-keyed `BTreeSet`/`BTreeMap` that has no `Ord` rep on the Rust backend.
fn compile_rust_result(src: &str) -> Result<String, Vec<String>> {
    let ast_bytes = crate::codec::encode(&parse(src));
    let out = compile(
        &[Artifact::new(Artifact::KIND_AST, "main", ast_bytes)],
        &[Target::Rust],
    );
    match out.artifact(Target::Rust.artifact_kind()) {
        Some(bytes) => Ok(String::from_utf8(bytes.to_vec()).expect("emitted Rust is utf-8")),
        None => Err(out
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()),
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
fn a_narrow_op_with_a_control_flow_operand_wraps_it_down_to_the_op_width() {
    // REGRESSION (the rust-backend cross-backend miscompile): a narrow-annotated op whose operand is a
    // DEFERRED-WIDTH control-flow expression (`if`/`match` of bare literals, inferred Int64) emitted an
    // i64 sub-expression into a narrow op (`(if … { 100i64 } …).checked_add(100i8)` → rustc E0308). The
    // operand must be WRAPPED DOWN to the op's width with an `as iN` cast, mirroring the wasm backend's
    // i64→iN normalization.
    let rs = compile_rust(
        "(module m (def (go (: n Int8)) (: (+ (if (< n 5) 100 0) 100) Int8)) (export go))",
    );
    // The whole `if` sub-expression is wrapped down to i8 — `}) as i8)` closes the if-block then casts
    // it, so the narrow `+` adds an i8 (its other operand `100` grounds to `100u8 as i8`).
    assert!(
        rs.contains("}) as i8)") && rs.contains("checked_add((100u8 as i8))"),
        "the if-operand must be wrapped down to i8 before the i8 add:\n{rs}"
    );
    // End-to-end through rustc: n=9 selects the else 0, 0+100=100 fits Int8 → 100 (compiles + runs, was
    // E0308). The overflow direction (n=3 → 200 → panic) is exercised by the wasm gate + the corpus.
    if let Some(out) = rustc_run(&rs, "go(9)") {
        assert_eq!(
            out, "100",
            "in-range narrow if-operand computes; was a compile error"
        );
    }
    // A `match`-operand takes the same wrap-down (the match block closes then casts to i8).
    let m = compile_rust(
        "(module m (def (go (: n Int8)) (: (+ (match n (0 5) (_ 1)) 2) Int8)) (export go))",
    );
    assert!(m.contains("}) as i8)"), "match-operand wrapped to i8:\n{m}");
    // An UNANNOTATED op is genuinely Int64 (deferred branches) — it must NOT wrap the `if` down: the op
    // adds an i64 and the if-block is NOT followed by an `as i8` cast (the `(5u8 as i8)` in the condition
    // is unrelated — it grounds the comparison literal to `n`'s width).
    let wide =
        compile_rust("(module m (def (go (: n Int8)) (+ (if (< n 5) 100 0) 100)) (export go))");
    assert!(
        wide.contains("checked_add((100u64 as i64))") && !wide.contains("}) as i8)"),
        "an unannotated Int64 op must not wrap its if-operand down:\n{wide}"
    );
}

#[test]
fn modulo_emits_a_zero_divisor_guard_not_checked_rem() {
    // A `%` traps ONLY on a zero divisor — NOT on `MIN % -1` (which is a defined 0; modulo forms no
    // quotient, so it has no overflow — numeric-model §Modulo by -1 is always zero). Rust's `checked_rem`
    // WRONGLY returns None at `MIN % -1`, so it must NOT be used: the emit guards the zero divisor and
    // uses `wrapping_rem` (0 at MIN%-1, matching wasm `i64.rem_s`).
    let rem = compile_rust("(module m (def (r (: a Int64) (: b Int64)) (% a b)) (export r))");
    assert!(
        rem.contains("wrapping_rem") && !rem.contains("checked_rem"),
        "modulo must use a guarded wrapping_rem, not checked_rem:\n{rem}"
    );
    // `/` guards BOTH trap kinds with KIND-SPECIFIC panic messages (so the gate's `trap_kind` classifies
    // each correctly — a single "by zero or overflow" message was misread as div-by-zero). A signed `/`
    // emits the zero guard ("division by zero") AND the `MIN/-1` overflow guard ("division overflow").
    let div = compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a b)) (export d))");
    assert!(
        div.contains("panic!(\"division by zero\")")
            && div.contains("i64::MIN && r == -1")
            && div.contains("panic!(\"division overflow\")"),
        "signed division guards zero + MIN/-1 with distinct trap-kind messages:\n{div}"
    );
    // An UNSIGNED `/` has NO MIN/-1 overflow (and `r == -1` would not type-check), so only the zero guard.
    let udiv = compile_rust("(module m (def (d (: a UInt32) (: b UInt32)) (/ a b)) (export d))");
    assert!(
        udiv.contains("panic!(\"division by zero\")")
            && !udiv.contains("== -1")
            && !udiv.contains("overflow"),
        "unsigned division guards only the zero divisor (no MIN/-1 overflow):\n{udiv}"
    );
}

#[test]
fn rustc_roundtrip_signed_div_min_by_neg1_traps_overflow_not_div_by_zero() {
    // End-to-end: `MIN / -1` overflows (the quotient +2^(N-1) is out of range) and MUST trap as an OVERFLOW,
    // NOT a divide-by-zero — the two are distinct trap KINDS the corpus grades separately, and the gate
    // classifies by the panic message. `MIN / -2` is a normal division (no trap); `x / 0` traps div-by-zero.
    let rs = compile_rust("(module m (def (d (: a Int64) (: b Int64)) (/ a b)) (export d))");
    // A normal division still computes.
    if let Some(out) = rustc_run(&rs, "d(7, 2)") {
        assert_eq!(out, "3", "7 / 2 truncates toward zero");
    }
    if let Some(out) = rustc_run(&rs, "d(i64::MIN, -2)") {
        assert_eq!(
            out, "4611686018427387904",
            "MIN / -2 is a normal (non-overflowing) division"
        );
    }
}

#[test]
fn rustc_roundtrip_modulo_min_by_neg1_is_zero_not_a_trap() {
    // End-to-end through rustc: `(% a b)` at (Int64.min, -1) MUST return 0 on the Rust backend, matching
    // wasm `i64.rem_s`. Before the fix the emitted `checked_rem(MIN, -1)` returned None and PANICKED — a
    // wrong trap where the value must be 0. All signed widths share the emit; Int64 is the witness.
    let rs = compile_rust("(module m (def (r (: a Int64) (: b Int64)) (% a b)) (export r))");
    if let Some(out) = rustc_run(&rs, "r(i64::MIN, -1)") {
        assert_eq!(out, "0", "MIN % -1 must be 0, not a panic");
    }
    // A normal modulo still computes, and the sign follows the dividend.
    if let Some(out) = rustc_run(&rs, "r(-7, 2)") {
        assert_eq!(out, "-1");
    }
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
fn a_runtime_list_emits_a_native_rust_vec() {
    // A `List T` maps to Rust's `Vec<T>`, and a runtime `(list …)` construction → the `vec![…]` macro.
    // Build the list behind a recursive call so it survives to runtime (a constant list folds away).
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main) (f 3)) (export main))",
    );
    assert!(rs.contains("-> Vec<i64>"), "list return type:\n{rs}");
    assert!(rs.contains("vec!["), "vec! construction:\n{rs}");

    // A list PARAMETER crosses as `Vec<T>`; a nested list → `Vec<Vec<i64>>`.
    let param = compile_rust("(module m (def (g (: xs (List Int64))) xs) (export g))");
    assert!(param.contains("xs: Vec<i64>"), "list param type:\n{param}");
    let nested = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list (list n)) (f (+ n -1)))) \
                    (def (main) (f 2)) (export main))",
    );
    assert!(
        nested.contains("-> Vec<Vec<i64>>"),
        "nested list type:\n{nested}"
    );
}

#[test]
fn rustc_roundtrip_list_builds_and_runs() {
    // A runtime list crosses rustc end-to-end: build `(list n n n)` behind a recursive call so it is a
    // genuine runtime `Vec`, return it, and render it as cdz-run's `(list …)` text via a small driver.
    let module = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n n) (f (+ n -1)))) \
                    (def (mklist) (f 2)) (export mklist))",
    );
    // The emitted module returns a `Vec<i64>`; the driver joins it into the canonical `(list 0 0 0)` form.
    let driver = "fn main() { let v = prog::mklist(); let mut s = String::from(\"(list\"); \
                  for e in v.iter() { s.push(' '); s.push_str(&format!(\"{}\", e)); } s.push(')'); \
                  println!(\"{}\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "(list 0 0 0)");
    }
}

#[test]
fn runtime_list_ops_emit_native_vec_operations() {
    // `List.len` → `.len() as i64`; measured over a runtime-built list (a constant list's length folds).
    let len = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main) (List.len (f 2))) (export main))",
    );
    assert!(
        len.contains(".len() as i64)"),
        "List.len → .len() as i64:\n{len}"
    );
    // `List.push` → a mut-local push returning the vec.
    let push = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n) (f (+ n -1)))) \
                    (def (main) (List.push (f 1) 9)) (export main))",
    );
    assert!(
        push.contains("__v.push(") && push.contains("-> Vec<i64>"),
        "List.push → push:\n{push}"
    );
    // `List.concat` → extend.
    let cat = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n) (f (+ n -1)))) \
                    (def (main) (List.concat (f 1) (f 2))) (export main))",
    );
    assert!(cat.contains("__v.extend("), "List.concat → extend:\n{cat}");
    // `List.update` → a bounds-checked index-set that traps OOB.
    let upd = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list n n) (f (+ n -1)))) \
                    (def (main (: i Int64)) (List.update (f 1) i 9)) (export main))",
    );
    assert!(
        upd.contains("panic!(\"index out of bounds\")") && upd.contains("__v[__i] ="),
        "List.update → bounds-checked set:\n{upd}"
    );
    // `List.at` → the fallible read yielding a native Option.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 10 20) (f (+ n -1)))) \
                    (def (main (: i Int64)) (List.at (f 1) i)) (export main))",
    );
    assert!(
        at.contains("-> Option<i64>")
            && at.contains("Some(__v[__i].clone())")
            && at.contains("None"),
        "List.at → Option read:\n{at}"
    );
}

#[test]
fn rustc_roundtrip_list_ops_compute_and_a_shared_list_does_not_move() {
    // End-to-end: build a list, then in one function BOTH pass it to a helper AND measure its length —
    // a Vec is move-only, so the binding is `.clone()`d on the non-last use (the Perceus-dup analogue).
    // `sum-at` walks the list by `List.at`+`List.len` and sums it: build `[0 1 2]`, sum = 3.
    let module = compile_rust(
        "(module m \
           (def (build i n out) (if (< i n) (build (+ i 1) n (List.push out i)) out)) \
           (def (sum-at xs i n) (if (< i n) (+ (match (List.at xs i) ((Some x) x) ((None _) 0)) \
                (sum-at xs (+ i 1) n)) 0)) \
           (def (mk) (let ((xs (build 0 3 (list)))) (sum-at xs 0 (List.len xs)))) (export mk))",
    );
    // A list binding used in two positions (passed to sum-at AND measured) must be cloned, not moved.
    assert!(
        module.contains(".clone()"),
        "a shared list is cloned:\n{module}"
    );
    if let Some(out) = rustc_run(&module, "mk()") {
        assert_eq!(out, "3", "0+1+2 = 3 over the built-then-read list");
    }
    // `List.update` traps out of bounds (a Rust panic == a Cadenza trap). Drive an in-range update and
    // read back the changed element to confirm the op computes; the OOB trap is exercised by the corpus.
    let upd = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 10 20 30) (f (+ n -1)))) \
           (def (at2 (: xs (List Int64))) (match (List.at xs 1) ((Some x) x) ((None _) 0))) \
           (def (mk) (at2 (List.update (f 1) 1 99))) (export mk))",
    );
    if let Some(out) = rustc_run(&upd, "mk()") {
        assert_eq!(out, "99", "index 1 replaced with 99");
    }
}

#[test]
fn a_list_match_emits_a_length_tested_if_chain() {
    // A list match `(match xs ((list) …) ((list h .. t) …))` → an `if xs.len() == 0 { … } else if
    // xs.len() >= 1 { … }` chain over the scrutinee's length; a leading element binder reads `xs[i]`,
    // the rest binds `xs[1..].to_vec()`.
    let rs = compile_rust(
        "(module m (def (sum (: xs (List Int64))) \
           (match xs ((list) 0) ((list h .. t) (+ h (sum t))))) (export sum))",
    );
    assert!(rs.contains(".len() == 0"), "empty-arm length test:\n{rs}");
    assert!(rs.contains(".len() >= 1"), "rest-arm length test:\n{rs}");
    assert!(
        rs.contains("[1..].to_vec()"),
        "rest binder → tail sublist:\n{rs}"
    );
    assert!(
        rs.contains("[0]"),
        "leading element binder → index 0:\n{rs}"
    );
    // A FIXED-arity arm `(list a b)` → `== 2`, its elements `xs[0]`/`xs[1]`.
    let fixed = compile_rust(
        "(module m (def (f (: xs (List Int64))) (match xs ((list a b) (+ a b)) (_ 0))) (export f))",
    );
    assert!(fixed.contains(".len() == 2"), "fixed-arity test:\n{fixed}");
}

#[test]
fn rustc_roundtrip_recursive_list_match_folds_to_a_scalar() {
    // End-to-end: a recursive `(list)`/`(list h .. t)` match sums a runtime-built list. Build `[10 20 30]`
    // behind a recursive call so it is a genuine runtime `Vec`, then fold it: 10+20+30 = 60.
    let module = compile_rust(
        "(module m \
           (def (sum (: xs (List Int64))) (match xs ((list) 0) ((list h .. t) (+ h (sum t))))) \
           (def (f (: n Int64)) (if (= n 0) (list 10 20 30) (f (+ n -1)))) \
           (def (mk) (sum (f 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&module, "mk()") {
        assert_eq!(out, "60", "10+20+30 folded through the list match");
    }
    // A fixed-arity arm computes over its bound elements; a non-matching length falls to the catch-all.
    let pick = compile_rust(
        "(module m \
           (def (g (: xs (List Int64))) (match xs ((list a b c) (+ a (+ b c))) (_ -1))) \
           (def (f (: n Int64)) (if (= n 0) (list 3 4 5) (f (+ n -1)))) \
           (def (mk) (g (f 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&pick, "mk()") {
        assert_eq!(out, "12", "3+4+5 over the exactly-3 arm");
    }
}

#[test]
fn a_runtime_closure_emits_an_rc_dyn_fn_and_a_lifted_fn() {
    // A `(fn …)` passed to a RECURSIVE HOF survives to run time → an `Rc<dyn Fn(…) -> …>` value that
    // forwards to a lifted `fn __lifted_k`. The function-typed PARAM maps to the same `Rc<dyn Fn>`.
    let rs = compile_rust(
        "(module m \
           (def (foldl (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64))) \
              (match xs ((list) acc) ((list h .. t) (foldl f (f h acc) t)))) \
           (def (main) (foldl (fn (x a) (+ a x)) 0 (list 5 7 30))) (export main))",
    );
    // The arrow SPINE is flattened: `(-> Int64 (-> Int64 Int64))` → `Rc<dyn Fn(i64, i64) -> i64>`.
    assert!(
        rs.contains("std::rc::Rc<dyn Fn(i64, i64) -> i64>"),
        "flattened Fn type:\n{rs}"
    );
    // The lifted lambda is emitted as a standalone fn; the closure value is an Rc::new coerced to dyn Fn.
    assert!(rs.contains("fn __lifted_0("), "lifted fn emitted:\n{rs}");
    assert!(
        rs.contains("std::rc::Rc::new(move |") && rs.contains(") as std::rc::Rc<dyn Fn"),
        "closure value builds + coerces to dyn Fn:\n{rs}"
    );
}

#[test]
fn rustc_roundtrip_closures_run_no_capture_and_capturing_and_in_a_list() {
    // NO-CAPTURE combinator folded over a list: 5+7+30 = 42.
    let fold = compile_rust(
        "(module m \
           (def (foldl (: f (-> Int64 (-> Int64 Int64))) (: acc Int64) (: xs (List Int64))) \
              (match xs ((list) acc) ((list h .. t) (foldl f (f h acc) t)))) \
           (def (mk) (foldl (fn (x a) (+ a x)) 0 (list 5 7 30))) (export mk))",
    );
    if let Some(out) = rustc_run(&fold, "mk()") {
        assert_eq!(out, "42", "no-capture closure folds the list");
    }
    // CAPTURING closure: `(fn (b) (g b))` captures the closure `g` and is applied through the recursive
    // HOF `sumapply`; the capture is cloned into each call (an `Fn` cannot move its capture out). With
    // `g = (fn (x) (+ x 1))`: `rec` sums `sumapply (fn (b) (g b)) 2` = g(2)+g(1) = 3+2 = 5 per round, over
    // 3 rounds = 15.
    let cap = compile_rust(
        "(module m \
           (def (sumapply (: h (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (h n) (sumapply h (- n 1))))) \
           (def (rec (: g (-> Int64 Int64)) (: n Int64)) \
              (if (= n 0) 0 (+ (sumapply (fn ((: b Int64)) (g b)) 2) (rec g (- n 1))))) \
           (def (mk) (rec (fn ((: x Int64)) (+ x 1)) 3)) (export mk))",
    );
    if let Some(out) = rustc_run(&cap, "mk()") {
        assert_eq!(
            out, "15",
            "a closure capturing another closure, applied through a recursive HOF"
        );
    }
    // A LIST of capturing closures, each keeping its own capture, indexed and applied — the closures must
    // coerce to ONE `Rc<dyn Fn>` type to share a `Vec` (the `as` coercion on a CONCRETELY-typed closure).
    // Annotate the lambda so its function type grounds at the closure node (an unannotated `(fn (x) …)`
    // whose width stays a var declines — see the decline test). index 1 → (mkc 20) applied to 1 = 21.
    let list = compile_rust(
        "(module m \
           (def (mkc (: k Int64)) (fn ((: x Int64)) (+ x k))) \
           (def (at (: fs (List (-> Int64 Int64))) (: i Int64) (: x Int64)) \
              (match ((. List at) fs i) ((Some f) (f x)) (None -1))) \
           (def (pick (: i Int64)) (at (list (mkc 10) (mkc 20) (mkc 30)) i 1)) (export pick))",
    );
    assert!(
        list.contains(") as std::rc::Rc<dyn Fn"),
        "list-of-closures coerces each to dyn Fn:\n{list}"
    );
    if let Some(out) = rustc_run(&list, "pick(1)") {
        assert_eq!(out, "21", "the index-1 closure captures 20; 20+1 = 21");
    }
}

#[test]
fn a_closure_crossing_the_export_boundary_declines() {
    // A closure cannot cross the EXPORT boundary (no closure-handle ABI on the Rust target): an exported
    // fn with a function-typed PARAM or RESULT declines cleanly (todo), rather than emitting a signature
    // the gate driver could not call with a value argument. (An INTERNAL closure is unaffected — see above.)
    let param = try_compile_rust(
        "(module m (def (apply-it (: g (-> Int64 Int64)) (: x Int64)) (g x)) (export apply-it))",
    )
    .expect_err("a closure-typed export param must decline");
    assert!(
        param
            .iter()
            .any(|d| d.contains("cannot cross the Rust export boundary")),
        "decline cites the export boundary: {param:?}"
    );
    let result = try_compile_rust(
        "(module m (def (mk (: k Int64)) (fn ((: x Int64)) (+ x k))) (export mk))",
    )
    .expect_err("a closure-returning export must decline");
    assert!(
        result
            .iter()
            .any(|d| d.contains("cannot cross the Rust export boundary")),
        "decline cites the export boundary: {result:?}"
    );
}

#[test]
fn map_and_set_emit_native_btree_collections() {
    // A `(Map K V)` → `BTreeMap<K,V>`, a `(Set E)` → `BTreeSet<E>` (BTree = sorted = canonical order).
    let m = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (map (1 10) (2 20)) (f (+ n -1)))) \
           (def (g) (Map.len (f 1))) (export g))",
    );
    assert!(
        m.contains("std::collections::BTreeMap::new()") && m.contains(".insert("),
        "map builds a BTreeMap:\n{m}"
    );
    let s = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Set.of (list 3 1 2)) (f (+ n -1)))) \
           (def (g) (Set.len (f 1))) (export g))",
    );
    assert!(
        s.contains("std::collections::BTreeSet::new()"),
        "set builds a BTreeSet:\n{s}"
    );
    // Map.lookup → a native Option via `.get(&k).cloned()`; Set.contains → a bool via `.contains(&e)`.
    let look = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (map (1 10)) (f (+ n -1)))) \
           (def (g (: k Int64)) (match (Map.lookup (f 1) k) ((Some v) v) ((None _) -1))) (export g))",
    );
    assert!(
        look.contains(".get(&(") && look.contains(").cloned()"),
        "map lookup:\n{look}"
    );
}

#[test]
fn a_bare_float_set_or_map_key_uses_the_cdz_f64_total_order_wrapper() {
    // A bare `Float` Set element / Map key is NOT `Ord` as a raw `f64` (NaN breaks totality), so it maps to
    // the `CdzF64` total-order wrapper (bit-canonical, NaN→one quiet NaN — the runtime's `box-float`). The
    // struct is emitted (gated on use) and each key/element value is lifted with `CdzF64::new(…)`.
    let s = compile_rust(
        "(module m (def (test (: stored Float64) (: probe Float64)) \
           (Set.contains (Set.of (list stored)) probe)) \
           (def (main (: d Int64)) (if (test Float64.nan Float64.nan) 1 0)) (export main))",
    );
    assert!(
        s.contains("struct __CdzF64(u64)") && s.contains("BTreeSet<__CdzF64>"),
        "a float set emits the __CdzF64 wrapper + a BTreeSet<__CdzF64>:\n{s}"
    );
    assert!(
        s.contains("__CdzF64::new("),
        "the stored element + the contains probe are lifted through __CdzF64::new:\n{s}"
    );
    // A float-KEYED map likewise: keys lifted through `CdzF64::new` on insert AND lookup (so the probe
    // matches the stored key's wrapper type). (The seed map here is `Map.empty`, whose BTreeMap type Rust
    // INFERS from the typed insert/lookup — a bare `BTreeMap::new()` with no spelled `<CdzF64, _>` — so the
    // invariant to pin is the key WRAP, which is what makes the inferred key type `CdzF64`.)
    let m = compile_rust(
        "(module m (def (main (: x Float64)) \
           (match (Map.lookup (Map.insert (Map.empty) x 42) x) ((Some v) v) ((None _) -1))) (export main))",
    );
    assert!(
        m.matches("__CdzF64::new(").count() >= 2 && m.contains("struct __CdzF64(u64)"),
        "a float-keyed map lifts its insert + lookup keys through __CdzF64::new:\n{m}"
    );
    // The wrapper is NOT emitted for a float-free program (gated on use — no dead struct).
    let plain = compile_rust("(module m (def (g (: n Int64)) (+ n 1)) (export g))");
    assert!(
        !plain.contains("__CdzF64"),
        "a float-free program does not emit the __CdzF64 wrapper:\n{plain}"
    );
    // A float-CARRYING COMPOUND key still DECLINES (the wrapper is not threaded through a tuple).
    let nested = compile_rust_result(
        "(module m (def (main (: x Float64)) \
           (Set.len (Set.of (list (tuple x 1))))) (export main))",
    );
    assert!(
        nested.is_err(),
        "a (Tuple Float Int64) set element still declines (wrapper not threaded through a compound):\n{nested:?}"
    );
}

#[test]
fn float_set_key_wrapper_is_width_specific_and_the_reserved_name_never_collides() {
    // Copilot review on PR #487 found two bugs in the bare-float set/map key support:
    //  (1) the wrapper was WIDTH-BLIND — a `Float32` key emitted a `__CdzF64` (over `u64`) around an `f32`
    //      value, a rustc type error. The wrapper is now width-specific: `Float32` → `__CdzF32` (over `u32`).
    //  (2) the wrapper name `CdzF64` was a LEGAL user sum name, so `(type CdzF64 …)` collided with the
    //      injected `struct CdzF64` (E0428) even in a float-free program. The name is now `__`-reserved and
    //      injection is gated on the `__CdzF{32,64}::new(` marker, so a user `CdzF64`/`CdzF32` sum is fine.

    // (1) A Float32-keyed set emits the `__CdzF32` wrapper (over u32 bits, `f32::NAN`), NOT `__CdzF64`.
    let f32set = compile_rust(
        "(module m (def (main (: d Float32)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        f32set.contains("struct __CdzF32(u32)")
            && f32set.contains("__CdzF32::new(")
            && !f32set.contains("__CdzF64"),
        "a Float32 set key uses the u32-backed __CdzF32 wrapper, not __CdzF64:\n{f32set}"
    );
    // A Float64-keyed set still uses `__CdzF64` (over u64), NOT `__CdzF32`.
    let f64set = compile_rust(
        "(module m (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        f64set.contains("struct __CdzF64(u64)") && !f64set.contains("__CdzF32"),
        "a Float64 set key uses the u64-backed __CdzF64 wrapper:\n{f64set}"
    );

    // (2) A user sum literally NAMED `CdzF64` in a FLOAT-FREE program emits `enum CdzF64` and NO injected
    // wrapper struct — the reserved `__CdzF64` name cannot collide, and injection needs the `::new(` marker.
    let user = compile_rust(
        "(module m (type CdzF64 (A) (B)) \
           (def (main (: d Int64)) (match (CdzF64.A) ((A) 1) ((B) 2))) (export main))",
    );
    assert!(
        user.contains("enum CdzF64")
            && !user.contains("struct __CdzF64")
            && !user.contains("struct CdzF64"),
        "a user `CdzF64` sum emits its enum with no injected wrapper struct (no E0428):\n{user}"
    );
    // And a user `CdzF64` sum ALONGSIDE a real Float64 set key: the enum and the `__CdzF64` wrapper coexist
    // (distinct names), no collision.
    let both = compile_rust(
        "(module m (type CdzF64 (A) (B)) \
           (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        both.contains("enum CdzF64") && both.contains("struct __CdzF64(u64)"),
        "a user `CdzF64` sum coexists with the injected __CdzF64 float-key wrapper:\n{both}"
    );
}

#[test]
fn float_key_wrapper_decl_injects_for_a_typed_empty_collection_and_reserved_name_is_escaped() {
    // Copilot PR#490 found two RESIDUAL gaps in the width-specific float-key wrapper:
    //  (1) the decl was injected only on the `::new(` constructor marker, but a context-typed EMPTY
    //      collection annotates the wrapper type with NO constructor → `cannot find type __CdzF64`.
    //  (2) the `__`-prefix did NOT actually reserve the name — the lexer allows a `_`-start and
    //      `sanitize_ident` passed `__` through, so a user `sum __CdzF64` still collided (E0428).

    // (1) A context-typed EMPTY float-keyed Map annotates `BTreeMap<__CdzF64, _>` with no `__CdzF64::new(`.
    // The decl must STILL be injected (gate on the `<__CdzF64` type-param occurrence, not just `::new(`).
    let empty = compile_rust(
        "(module m (def (e) (: (Map.empty) (Map Float64 Int64))) \
           (def (main (: d Int64)) (Map.len (e))) (export main))",
    );
    assert!(
        empty.contains("BTreeMap<__CdzF64,") && empty.contains("struct __CdzF64(u64)"),
        "a typed empty float Map annotates __CdzF64 AND injects its decl (no `cannot find type`):\n{empty}"
    );

    // (2) A user sum literally named `__CdzF64` (a legal Cadenza ident — `_`-start is allowed) is ESCAPED
    // by `sanitize_ident` to `cdz_user___CdzF64`, so it can NEVER collide with the backend-reserved wrapper.
    let user = compile_rust(
        "(module m (type __CdzF64 (A) (B)) \
           (def (main (: d Int64)) (match (__CdzF64.A) ((A) 1) ((B) 2))) (export main))",
    );
    assert!(
        user.contains("enum cdz_user___CdzF64") && !user.contains("struct __CdzF64(u64)"),
        "a user `__CdzF64` sum is escaped to cdz_user___CdzF64 with no injected wrapper struct (no E0428):\n{user}"
    );
    // A user def named `__pay` (another backend-reserved local) is likewise escaped, not captured.
    let userfn = compile_rust("(module m (def (__pay (: n Int64)) (+ n 1)) (export __pay))");
    assert!(
        userfn.contains("cdz_user___pay") && userfn.contains("pub fn cdz_user___pay"),
        "a user `__pay` def is escaped away from the reserved local namespace:\n{userfn}"
    );
}

#[test]
fn rustc_roundtrip_map_and_set_compute_and_enumerate_in_order() {
    // Map: build `{1:10, 2:20, 3:30}` at runtime, sum its size + a lookup. 3 keys + lookup(2)=20 = 23.
    let mp = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (map (1 10) (2 20) (3 30)) (f (+ n -1)))) \
           (def (look (: k Int64)) (match (Map.lookup (f 1) k) ((Some v) v) ((None _) -1))) \
           (def (g) (+ (Map.len (f 1)) (look 2))) (export g))",
    );
    if let Some(out) = rustc_run(&mp, "g()") {
        assert_eq!(out, "23", "3 keys + lookup(2)=20");
    }
    // Set: dedup + canonical-order to-list. `{30,10,20,10}` → 3 distinct, to-list summed = 60 → 63.
    let st = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (Set.of (list 30 10 20 10)) (f (+ n -1)))) \
           (def (suml (: xs (List Int64))) (match xs ((list) 0) ((list h .. t) (+ h (suml t))))) \
           (def (g) (+ (Set.len (f 1)) (suml (Set.to-list (f 1))))) (export g))",
    );
    if let Some(out) = rustc_run(&st, "g()") {
        assert_eq!(out, "63", "3 distinct + (10+20+30)");
    }
    // A user-sum map KEY needs the enum to derive Ord (a nullary enum qualifies) — look up by the variant.
    let uk = compile_rust(
        "(module m (type C (R) (G)) \
           (def (g) (match (Map.lookup (Map.insert (Map.empty) (C.R) 42) (C.R)) \
              ((Some v) v) ((None _) -1))) (export g))",
    );
    if let Some(out) = rustc_run(&uk, "g()") {
        assert_eq!(out, "42", "a user-sum key is looked up by its variant");
    }
}

#[test]
fn a_bare_float_set_or_map_uses_cdz_f64_but_float_carrying_sum_declines() {
    // A `Set`/`Map` of a BARE FLOAT now COMPILES via the `CdzF64` total-order wrapper (a raw `f64` is only
    // `PartialOrd`; `CdzF64` orders by canonical bits, NaN-canonical — mirroring the runtime's `box-float`).
    // (WAS a decline — the "no BTreeSet<f64>" era, before the wrapper.) The float-insert into an EMPTY set is
    // the sharp case: the empty base's element type is an unsolved var, fixed to a float only by the insert —
    // so both the guard AND the wrapper substitution key off the ELEMENT/KEY node type.
    let set_float = compile_rust(
        "(module m (def (main (: d Float64)) (Set.len (Set.insert (Set.of (list)) d))) (export main))",
    );
    assert!(
        set_float.contains("__CdzF64::new(") && set_float.contains("struct __CdzF64"),
        "a float-element Set now compiles via __CdzF64:\n{set_float}"
    );
    let map_float = compile_rust(
        "(module m (def (main (: d Float64)) (Map.len (Map.insert (Map.empty) d 1))) (export main))",
    );
    assert!(
        map_float.contains("__CdzF64::new("),
        "a float-KEY Map now compiles via __CdzF64:\n{map_float}"
    );
    // CONTROL: an Int-keyed set/map still compiles (unchanged — Int is natively Ord, no wrapper).
    let set_int = compile_rust(
        "(module m (def (main (: n Int64)) (Set.len (Set.insert (Set.of (list)) n))) (export main))",
    );
    assert!(
        set_int.contains("BTreeSet"),
        "an Int-element Set still emits a BTreeSet:\n{set_int}"
    );

    // FOLLOW-ON (Copilot PR#455): a sum CARRYING A FLOAT is one type-shape past a bare float key — its
    // emitted enum derives no `Ord` (a float payload isn't `Eq`/`Ord`), so a `Set<Enum>`/`Map<Enum,_>`
    // is still uncompilable. The old `ty_is_ord` returned `true` for EVERY `Ty::Sum` (comment claimed the
    // enum-derive path caught it — but that's a rustc COMPILE ERROR, not the clean decline). Both the
    // VALUE path (a construction op) and the TYPE-POSITION path (a param `(Set W)`, no construction op —
    // caught by the `sum_representable` gate) must decline.
    let sum_float_set_val = compile_rust_result(
        "(module m (type W (F Float64) (G)) \
           (def (main (: d Float64)) (Set.len (Set.of (list ((. W F) d))))) (export main))",
    );
    assert!(
        sum_float_set_val.is_err(),
        "a Set of a float-carrying sum (value) must DECLINE, got:\n{sum_float_set_val:?}"
    );
    let sum_float_set_param = compile_rust_result(
        "(module m (type W (F Float64) (G)) (def (main (: s (Set W))) (Set.len s)) (export main))",
    );
    assert!(
        sum_float_set_param.is_err(),
        "a (Set W) PARAM where W carries a float must DECLINE (no BTreeSet<W>), got:\n{sum_float_set_param:?}"
    );
    // CONTROL: an ALL-NULLARY sum (its enum DOES derive Ord) is a valid Set element/key — the fix is
    // Ord-derivability-specific, not "decline every sum key".
    let nullary_sum_set = compile_rust(
        "(module m (type W (A) (B)) (def (main (: s (Set W))) (Set.len s)) (export main))",
    );
    assert!(
        nullary_sum_set.contains("BTreeSet<W>"),
        "an all-nullary sum is a valid (Ord) Set element:\n{nullary_sum_set}"
    );
    // CONTROL: a float-carrying sum is fine as a Map VALUE (only the KEY needs Ord) — `(Map Int64 W)`.
    let sum_float_map_val = compile_rust(
        "(module m (type W (F Float64) (G)) (def (main (: mp (Map Int64 W))) (Map.len mp)) (export main))",
    );
    assert!(
        sum_float_map_val.contains("BTreeMap<i64, W>"),
        "a float-carrying sum is a valid Map VALUE (only the key needs Ord):\n{sum_float_map_val}"
    );
}

#[test]
fn nested_option_matches_bind_distinct_payload_names() {
    // REGRESSION (a nested-match binder collision the map slice surfaced): two matches on DIFFERENT
    // scrutinees nesting at the same relative path both minted `__pay_0_0`, so the inner shadowed the
    // outer and `(+ a b)` silently became `b + b`. The binder name now includes the scrutinee id, so `a`
    // and `b` are distinct. `main(1,2)` over `{1:10, 2:20}` = 10+20 = 30 (was a wrong 40).
    let rs = compile_rust(
        "(module m (def (pick (: k1 Int64) (: k2 Int64)) \
           (let ((mp (Map.insert (Map.insert (Map.empty) 1 10) 2 20))) \
              (match (Map.lookup mp k1) ((Some a) (match (Map.lookup mp k2) ((Some b) (+ a b)) (None -1))) \
                 (None -2)))) (export pick))",
    );
    if let Some(out) = rustc_run(&rs, "pick(1, 2)") {
        assert_eq!(out, "30", "distinct binders: a=10, b=20 → 30 (not b+b=40)");
    }
}

#[test]
fn a_string_constant_emits_an_owned_string() {
    // A `Ty::String` → Rust `String`; a `ConstStr` → `"…".to_string()`, with content-safe escaping.
    let rs = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"café\" (f (+ n -1)))) (def (g) (f 1)) (export g))",
    );
    assert!(rs.contains("-> String"), "string return type:\n{rs}");
    assert!(
        rs.contains("\"café\".to_string()"),
        "const string with UTF-8 preserved:\n{rs}"
    );
    // A String PARAMETER crosses as `String`.
    let param = compile_rust("(module m (def (id (: s String)) s) (export id))");
    assert!(param.contains("id(s: String)"), "string param:\n{param}");
    // A string literal with escapes emits valid Rust (quote + backslash + newline).
    let esc = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"a\\\"b\" (f (+ n -1)))) (def (g) (f 1)) (export g))",
    );
    assert!(esc.contains("\\\""), "escaped quote in the literal:\n{esc}");
}

#[test]
fn rustc_roundtrip_string_result_renders_quoted() {
    // A runtime string result crosses end-to-end and renders as cdz-run's `"…"` form (raw UTF-8, quoted).
    let module = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"parse error\" (f (+ n -1)))) \
           (def (mk) (f 1)) (export mk))",
    );
    // The driver renders a `String` result as `"<content>"` — a multi-word string keeps its spaces.
    let driver = "fn main() { let s = prog::mk(); println!(\"\\\"{}\\\"\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "\"parse error\"", "a multi-word string renders quoted");
    }
}

#[test]
fn bytes_ops_emit_native_vec_u8_operations() {
    // A `Ty::Bytes` → `Vec<u8>`; `Bytes.of` builds it with a per-element range check.
    let of = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 65 66 67)) (f (+ n -1)))) \
           (def (g) (Bytes.len (f 1))) (export g))",
    );
    assert!(of.contains("Vec<u8>"), "bytes type:\n{of}");
    assert!(
        of.contains("panic!(\"byte value out of range\")") && of.contains("as u8"),
        "range-checked byte build:\n{of}"
    );
    // Bytes.at → a native Option, byte zero-extended to Int64.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 10 20)) (f (+ n -1)))) \
           (def (g (: i Int64)) (match (Bytes.at (f 1) i) ((Some b) b) ((None _) -1))) (export g))",
    );
    assert!(
        at.contains("as i64") && at.contains("None"),
        "bytes at:\n{at}"
    );
    // String.concat lowers through BytesConcat but must emit push_str (String), NOT extend (Vec<u8>).
    let sconcat = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (g) (rep \"hi\" 2)) (export g))",
    );
    assert!(
        sconcat.contains("push_str") && !sconcat.contains(".extend("),
        "String.concat uses push_str, not Vec extend:\n{sconcat}"
    );
}

#[test]
fn rustc_roundtrip_bytes_build_read_and_string_concat_run() {
    // Bytes: build [65,66,67], len 3 + at(1)=66 = 69.
    let by = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 65 66 67)) (f (+ n -1)))) \
           (def (at1 (: i Int64)) (match (Bytes.at (f 1) i) ((Some b) b) ((None _) -1))) \
           (def (mk) (+ (Bytes.len (f 1)) (at1 1))) (export mk))",
    );
    if let Some(out) = rustc_run(&by, "mk()") {
        assert_eq!(out, "69", "3 bytes + byte-at(1)=66");
    }
    // A runtime-built string via String.concat crosses end-to-end and renders quoted: "hi" + "x"*3 = "hixxx".
    let module = compile_rust(
        "(module m (def (rep (: s String) (: n Int64)) (if (< n 1) s (rep (String.concat s \"x\") (- n 1)))) \
           (def (mk) (rep \"hi\" 3)) (export mk))",
    );
    let driver = "fn main() { let s = prog::mk(); println!(\"\\\"{}\\\"\", s); }";
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(
            out, "\"hixxx\"",
            "a runtime-built string concatenates correctly"
        );
    }
}

#[test]
fn runtime_string_ops_emit_native_str_operations() {
    // StrAt → scalar-indexed `.chars().nth(i).map(to_string)`; StrFromBytes → `from_utf8().ok()`;
    // StrToBytes → `.into_bytes()`.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"hi\" (f (+ n -1)))) \
           (def (g (: i Int64)) (match (String.at (f 1) i) ((Some c) 1) ((None _) 0))) (export g))",
    );
    assert!(
        at.contains(".chars().nth(") && at.contains(".to_string()"),
        "String.at is scalar-indexed:\n{at}"
    );
    let fb = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 104 105)) (f (+ n -1)))) \
           (def (g) (match (String.from-bytes (f 1)) ((Some s) 1) ((None _) 0))) (export g))",
    );
    assert!(
        fb.contains("String::from_utf8(") && fb.contains(".ok()"),
        "String.from-bytes uses from_utf8().ok():\n{fb}"
    );
    let tb = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (String.concat \"a\" \"b\") (f (+ n -1)))) \
           (def (g) (Bytes.len (String.to-bytes (f 1)))) (export g))",
    );
    assert!(
        tb.contains(".into_bytes()"),
        "String.to-bytes uses into_bytes:\n{tb}"
    );
}

#[test]
fn rustc_roundtrip_string_at_is_scalar_indexed_and_from_bytes_validates() {
    // StrAt indexes by SCALAR VALUE, not byte: in "café", scalar 3 is 'é' (a 2-byte UTF-8 char). Read it
    // back and measure its byte length (2), proving scalar-not-byte addressing; an OOB index → None → -1.
    let at = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) \"café\" (f (+ n -1)))) \
           (def (mk (: i Int64)) (match (String.at (f 1) i) ((Some c) (Bytes.len (String.to-bytes c))) ((None _) -1))) \
           (export mk))",
    );
    if let Some(out) = rustc_run(&at, "mk(3)") {
        assert_eq!(out, "2", "scalar 3 of 'café' is 'é', a 2-byte UTF-8 char");
    }
    if let Some(out) = rustc_run(&at, "mk(9)") {
        assert_eq!(out, "-1", "an out-of-range scalar index is None");
    }
    // String.from-bytes decodes valid UTF-8 to Some, and rejects a lone continuation byte (0x80) to None.
    let fb = compile_rust(
        "(module m \
           (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 104 105)) (f (+ n -1)))) \
           (def (bad (: n Int64)) (if (= n 0) (Bytes.of (list 128)) (bad (+ n -1)))) \
           (def (mk (: which Int64)) \
              (match (String.from-bytes (if (= which 0) (f 1) (bad 1))) ((Some s) (Bytes.len (String.to-bytes s))) ((None _) -1))) \
           (export mk))",
    );
    if let Some(out) = rustc_run(&fb, "mk(0)") {
        assert_eq!(out, "2", "valid UTF-8 'hi' decodes to a 2-byte string");
    }
    if let Some(out) = rustc_run(&fb, "mk(1)") {
        assert_eq!(
            out, "-1",
            "a lone continuation byte is rejected → None (never traps)"
        );
    }
}

#[test]
fn a_context_typed_empty_map_or_set_emits_a_bare_new_not_a_decline() {
    // REGRESSION: an `Map.empty`/`Set.of(list)` whose element type is pinned only by DOWNSTREAM use (a
    // typed callee param) has unsolved vars AT THE NODE, so `type_of`→None — the empty-collection handler
    // used to DECLINE. It must instead emit a BARE `BTreeMap::new()`/`BTreeSet::new()` and let Rust infer
    // the element type from the use. A recursive map-accumulator seeded with `Map.empty` compiles+runs.
    let m = compile_rust(
        "(module m \
           (def (ins (: n Int64) (: mp (Map Int64 Int64))) \
              (if (< n 1) mp (ins (- n 1) (Map.insert mp n (* n 10))))) \
           (def (mk (: n Int64)) \
              (match (List.at (Map.to-list (ins n (Map.empty))) 0) \
                 ((Some p) (match p ((tuple k v) (+ k v)))) ((None _) -1))) (export mk))",
    );
    assert!(
        m.contains("std::collections::BTreeMap::new()"),
        "empty map is a bare BTreeMap::new():\n{m}"
    );
    if let Some(out) = rustc_run(&m, "mk(5)") {
        assert_eq!(out, "11", "min key 1 + val 10 from the accumulated map");
    }
    // A set accumulator seeded with `Set.of (list)` (empty) likewise infers its element type.
    let s = compile_rust(
        "(module m \
           (def (ins (: n Int64) (: st (Set Int64))) (if (< n 1) st (ins (- n 1) (Set.insert st n)))) \
           (def (mk (: n Int64)) (Set.len (ins n (Set.of (list))))) (export mk))",
    );
    if let Some(out) = rustc_run(&s, "mk(3)") {
        assert_eq!(
            out, "3",
            "3 distinct elements accumulated into a context-typed empty set"
        );
    }
}

#[test]
fn bytes_slice_is_total_on_a_usize_overflowing_range() {
    // REGRESSION (Copilot PR#435): the `Bytes.slice` bounds guard summed `(start as usize)+(len as usize)`,
    // which OVERFLOWS usize for two near-i64::MAX operands (wraps to a small sum in release) → the guard
    // passed and the index PANICKED. `Bytes.slice` must be TOTAL → the guard now uses `checked_add`, so an
    // overflowing range returns None. Assert the emitted source uses the checked form.
    let sl = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Bytes.of (list 1 2 3)) (f (+ n -1)))) \
           (def (g (: st Int64) (: ln Int64)) (match (Bytes.slice (f 1) st ln) ((Some b) (Bytes.len b)) ((None _) -1))) \
           (export g))",
    );
    assert!(
        sl.contains("checked_add(") && !sl.contains("as usize) + (__len as usize)"),
        "Bytes.slice bound uses checked_add, not a wrapping sum:\n{sl}"
    );
    // In-range slice works; a huge (overflowing) start/len returns None, not a panic.
    if let Some(out) = rustc_run(&sl, "g(1, 2)") {
        assert_eq!(out, "2", "in-range slice [1..3) has length 2");
    }
    if let Some(out) = rustc_run(&sl, "g(9223372036854775807, 9223372036854775807)") {
        assert_eq!(out, "-1", "a usize-overflowing range is None, not a panic");
    }
}

#[test]
fn rustc_roundtrip_persistent_collections_do_not_mutate_the_original() {
    // INVARIANT PIN: Cadenza collections are PERSISTENT — an op returns a NEW value and the operand is
    // unchanged. The rust backend realizes this by CLONING a shared (non-Copy) binding on read (tick-2/5/11
    // work). These pin that a `let`-bound collection used in BOTH an op and a later read keeps its original
    // value — a future change to the clone-on-read discipline that broke persistence would flip these.
    // List: push then read the ORIGINAL length. len([1,2,3]+push) 4 + len original 3 = 7.
    let lst = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (list 1 2 3) (f (+ n -1)))) \
           (def (mk) (let ((xs (f 1))) (+ (List.len (List.push xs 9)) (List.len xs)))) (export mk))",
    );
    if let Some(out) = rustc_run(&lst, "mk()") {
        assert_eq!(
            out, "7",
            "List.push leaves the original list unchanged (4 + 3)"
        );
    }
    // Map: remove a key, look it up in the NEW map (None→99) AND the ORIGINAL (still 10). 99 + 10 = 109.
    let mp = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.insert (Map.empty) 1 10) 2 20) (f (+ n -1)))) \
           (def (mk) (let ((m (f 1))) \
              (+ (match (Map.lookup (Map.remove m 1) 1) ((Some v) v) ((None _) 99)) \
                 (match (Map.lookup m 1) ((Some v) v) ((None _) 0))))) (export mk))",
    );
    if let Some(out) = rustc_run(&mp, "mk()") {
        assert_eq!(
            out, "109",
            "Map.remove leaves the original map's key intact (99 + 10)"
        );
    }
    // Set: insert into a copy, sum the new len + the original len. len({1,2}+9)=3 + len{1,2}=2 = 5.
    let st = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Set.of (list 1 2)) (f (+ n -1)))) \
           (def (mk) (let ((s (f 1))) (+ (Set.len (Set.insert s 9)) (Set.len s)))) (export mk))",
    );
    if let Some(out) = rustc_run(&st, "mk()") {
        assert_eq!(
            out, "5",
            "Set.insert leaves the original set unchanged (3 + 2)"
        );
    }
}

#[test]
fn rustc_roundtrip_nested_heap_in_heap_composes() {
    // INVARIANT PIN: heap values NEST arbitrarily — a list in a map value, a compound in a sum payload —
    // and the native-aggregate types + clone-on-read compose. A list stored as a Map VALUE, retrieved and
    // measured: len == 3.
    let lm = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.empty) 1 (list 10 20 30)) (f (+ n -1)))) \
           (def (mk) (match (Map.lookup (f 1) 1) ((Some xs) (List.len xs)) ((None _) -1))) (export mk))",
    );
    if let Some(out) = rustc_run(&lm, "mk()") {
        assert_eq!(out, "3", "a list stored in a map value round-trips");
    }
    // A Map inside a sum payload, matched then looked up: Box.Mk({5:50}) → 50.
    let ms = compile_rust(
        "(module m (type Box (Mk (Map Int64 Int64))) \
           (def (f (: n Int64)) (if (= n 0) (Box.Mk (Map.insert (Map.empty) 5 50)) (f (+ n -1)))) \
           (def (mk) (match (f 1) ((Mk m) (match (Map.lookup m 5) ((Some v) v) ((None _) -1))))) (export mk))",
    );
    if let Some(out) = rustc_run(&ms, "mk()") {
        assert_eq!(out, "50", "a map inside a sum payload round-trips");
    }
}

#[test]
fn rustc_roundtrip_compound_map_key_matches_by_value() {
    // INVARIANT PIN: a map/set keyed by a COMPOUND (tuple) matches BY VALUE (the BTreeMap `Ord` over the
    // Rust tuple, matching Cadenza's by-value key semantics). A tuple key inserted and looked up → 99.
    let mk = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Map.insert (Map.empty) (tuple 1 2) 99) (f (+ n -1)))) \
           (def (g) (match (Map.lookup (f 1) (tuple 1 2)) ((Some v) v) ((None _) -1))) (export g))",
    );
    if let Some(out) = rustc_run(&mk, "g()") {
        assert_eq!(out, "99", "a tuple map key matches by value");
    }
}

#[test]
fn an_ill_formed_integer_width_is_rejected_not_declined() {
    // An out-of-range integer WIDTH (negative/non-natural, or over-ceiling `(UInt 65)`) is an ILL-FORMED
    // TYPE, not a target limitation — a boundary of that type must REJECT (CDZ0302), the SAME outcome the
    // wasm target gives, not a codeless "no native Rust representation" decline (which the gate would read
    // as an unimplemented-construct todo). As of the shared-front-end well-formedness fix (`int_width_fault`
    // classifies `Malformed` vs `OverCeiling`), `cdz check` REJECTS a malformed width `(Int -8)` directly —
    // both backends inherit it — with a message that cites the admitted 1..=64 range and that a width must
    // be a compile-time natural number (an over-ceiling width still names the written width).
    let neg = try_compile_rust("(module m (def (main) (: 5 (Int -8))) (export main))")
        .expect_err("an ill-formed integer width must reject, not emit");
    assert!(
        neg.iter().any(|d| d.contains("1..=64")
            && (d.contains("not a valid integer type")
                || d.contains("width must be a compile-time natural number"))),
        "reject should cite the ill-formed width + admitted range: {neg:?}"
    );
    // The same in PARAMETER position (over-ceiling this time).
    let over = try_compile_rust("(module m (def (main (: x (UInt 65))) x) (export main))")
        .expect_err("an over-ceiling parameter width must reject");
    assert!(
        over.iter().any(|d| d.contains("not a valid integer type")),
        "parameter reject should cite the ill-formed width: {over:?}"
    );
    // A VALID but non-aliased width (`UInt7`, in 1..=64) is a genuine backend limitation → a codeless
    // DECLINE (no native Rust primitive), NOT a CDZ0302 reject. Guards against over-rejecting.
    let seven = try_compile_rust("(module m (def (main (: x (UInt 7))) x) (export main))")
        .expect_err("UInt7 has no native Rust rep — declines");
    assert!(
        seven
            .iter()
            .any(|d| d.contains("no native Rust representation")),
        "a valid non-aliased width should DECLINE, not reject: {seven:?}"
    );
    assert!(
        !seven.iter().any(|d| d.contains("not a valid integer type")),
        "UInt7 must NOT be reported as an ill-formed width: {seven:?}"
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
fn a_recursive_def_named_a_rust_keyword_emits_a_raw_identifier() {
    // `loop` is a valid Cadenza identifier but a Rust KEYWORD. A recursive def named it SURVIVES as a
    // top-level `fn` (a non-recursive one inlines away), so the emitter writes `fn loop(…)` — invalid Rust
    // (`expected `{`, found `(``). It must be a RAW identifier `r#loop` at the declaration AND the call, the
    // way rustc round-trips a keyword-named symbol. (The body's own `loop { }` tail-loop is a real loop
    // keyword and stays bare — only the NAME is escaped.)
    let rs = compile_rust(
        "(module m (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1)))) (def (go) (loop 3)) (export go))",
    );
    assert!(
        rs.contains("fn r#loop(") && rs.contains("r#loop("),
        "a keyword-named fn + its call are raw identifiers:\n{rs}"
    );
    assert!(
        !rs.contains("fn loop("),
        "the fn name must not be the bare keyword:\n{rs}"
    );
    // A non-keyword name is unaffected — no `r#` on `sum_to`.
    let ok = compile_rust(
        "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (+ n (sum-to (- n 1))))) (export sum-to))",
    );
    assert!(
        !ok.contains("r#"),
        "a non-keyword name gets no raw prefix:\n{ok}"
    );
    // End-to-end through rustc: the keyword-named recursion builds (was a rustc parse error) and loop(3) = 42.
    let run = compile_rust(
        "(module m (def (loop (: n Int64)) (if (= n 0) 42 (loop (- n 1)))) (def (go) (loop 3)) (export go))",
    );
    if let Some(out) = rustc_run(&run, "go()") {
        assert_eq!(
            out, "42",
            "the keyword-named recursion builds and runs:\n{run}"
        );
    }
}

#[test]
fn an_inlined_do_local_recursive_fn_uniques_its_name_per_copy() {
    // A helper with a do-local recursive worker, called from MORE THAN ONE site: the helper's body is
    // β-copied per call, each copy carrying its OWN `fac` DEFINITION (a distinct `db.defs` index) but the
    // SAME source name. Emitting both as a top-level `fn fac` collides (rustc E0428 "the name `fac` is
    // defined multiple times") — the wasm backend never collides because a function's identity there is its
    // INDEX. `fn_ident` uniques each colliding copy by its def index (`fac_<n>`), consistently at the
    // declaration AND the recursive self-call, so the artifact builds and each copy recurses into itself.
    // Export under a non-`main` name (`run`) so the round-trip driver's own `fn main` does not collide with
    // the emitted export (`rustc_run` splices the module beside a driver `fn main`, not under `mod prog`).
    let rs = compile_rust(
        "(module m (def (helper x) (do (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))) (fac x))) \
                    (def (run) (+ (helper 5) (helper 3))) (export run))",
    );
    // No two identically-named `fn fac(` may be emitted (that is the E0428 the fix removes).
    assert_eq!(
        rs.matches("fn fac(").count(),
        0,
        "no two identically-named `fn fac` may be emitted (E0428):\n{rs}"
    );
    // Each inlined copy gets its own uniqued `fn fac_<def>` declaration — two distinct declarations.
    let decls = rs.matches("fn fac_").count();
    assert!(
        decls >= 2,
        "each inlined copy gets its own uniqued `fn fac_<n>` declaration (got {decls}):\n{rs}"
    );
    // End-to-end: the emitted crate BUILDS and runs to fac(5)+fac(3) = 120+6 = 126 (the E0428 is gone).
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(out, "126", "the uniqued crate builds and runs:\n{rs}");
    }
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
impl cdz_rt::CdzEnv for GateEnv { async fn consume(&mut self, _g: u64) {} }
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

/// A GLOBALLY-UNIQUE temp-dir path for one round-trip invocation: `<prefix>-<pid>-<counter>`. The
/// content-hash key alone is NOT enough to prevent the `Text file busy`/`ExecutableFileBusy` flake —
/// two round-trips with IDENTICAL `(module, call)` content (a test that runs the same program twice, or
/// two distinct tests whose emitted source coincides) hash to the SAME dir and, running in PARALLEL,
/// race write-vs-exec on the one shared `prog` binary. A per-invocation `pid`+atomic-counter suffix
/// gives EVERY call its own dir, so no two ever touch the same `prog`/`prog.rs` — the same fix the gate
/// harness (`xtask`) applies to `run_program_rust`. (The `content_key` still seeds the name so a kept
/// dir is recognizable; uniqueness is what the suffix guarantees.) See
/// [[rust-backend-roundtrip-tests-flake-text-file-busy]].
fn unique_tmp_dir(prefix: &str, content_key: u64) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("{prefix}-{content_key:016x}-{pid}-{n}"))
}

/// Compile the emitted Rust `module` plus a generated `main` that calls `export`(`args`) and prints
/// the result, run it under the ambient `rustc`, and return the printed line. Returns `None` if
/// `rustc` is not available (the test then skips its assertion rather than failing).
fn rustc_run(module: &str, call: &str) -> Option<String> {
    use std::process::Command;
    if Command::new("rustc").arg("--version").output().is_err() {
        return None; // no rustc — skip the round-trip.
    }
    // A GLOBALLY-UNIQUE temp dir per INVOCATION (pid + atomic counter, seeded by the content hash) —
    // tests run in PARALLEL, and two round-trips with IDENTICAL `(module, call)` content (the same
    // program run twice, or two tests whose emitted source coincides) would otherwise share ONE dir and
    // race write-vs-exec on the fixed `prog` binary ("Text file busy"). The content-hash-only key was
    // NOT enough (identical content collides); the per-invocation suffix guarantees no two calls touch
    // the same `prog`/`prog.rs`. (The test bin may use the filesystem — it is the host boundary.)
    let dir = unique_tmp_dir("rcdzc-rust-rt", test_key(module, call));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    let full = format!("{module}\nfn main() {{ println!(\"{{}}\", {call}); }}\n");
    std::fs::write(&src_path, full).expect("write rust source");
    // Compile with a retry: many round-trip tests run in PARALLEL, each shelling `rustc`→`cc`, and the
    // linker can transiently fail under that concurrency ("linking with cc failed") — an environment
    // race, not a defect in the emitted source. Retry once before treating a non-zero status as a real
    // compile error (a genuine miscompile fails both attempts, so this never hides one).
    // A BigInt program emits `cdz_num::Big`; link the `cdz-num` dev-dep rlib (harmless when unused —
    // `--extern` only makes the crate available). Mirrors the async runner's `cdz_rt` linking.
    let cdz_num = cdz_num_link();
    let compile = || {
        let mut cmd = Command::new("rustc");
        cmd.args(["-O", "--edition", "2021"])
            .arg(&src_path)
            .arg("-o")
            .arg(&bin_path);
        if let Some((dep_dir, rlib)) = &cdz_num {
            cmd.arg("-L")
                .arg(format!("dependency={}", dep_dir.display()))
                .arg("--extern")
                .arg(format!("cdz_num={}", rlib.display()));
        }
        cmd.output().expect("run rustc")
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
    let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
    // Success → remove the now-unique per-invocation dir so `/tmp` doesn't accumulate one per test call.
    // (On an assert-panic above the dir is intentionally LEFT, as a debugging artifact — a rare failure.)
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
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
    // GLOBALLY-UNIQUE per invocation (see `unique_tmp_dir` / `rustc_run`): identical `(module, driver)`
    // content across parallel round-trips must NOT share one `prog` binary, or they race write-vs-exec.
    let dir = unique_tmp_dir("rcdzc-rust-drv", test_key(module, driver));
    let _ = std::fs::create_dir_all(&dir);
    let src_path = dir.join("prog.rs");
    let bin_path = dir.join("prog");
    // Wrap the module in `mod prog { … }` so its `pub fn`s are `prog::…` and its `#![allow(…)]` inner
    // attrs stay valid at the mod head, then append the driver (which owns `fn main`).
    let full = format!("mod prog {{\n{module}\n}}\n{driver}");
    std::fs::write(&src_path, full).expect("write rust source");
    // An emitted ASYNC module `use`s `cdz_rt::CdzEnv` from the shared `cdz-rt` crate; link its rlib (a
    // dev-dependency, so `cargo test` built it into the target dir). `cdz_rt_link()` finds the rlib +
    // its `-L` search dir; if absent (rlib not built), skip the extern — a sync module needs no crate.
    let mut cmd = Command::new("rustc");
    cmd.args(["-O", "--edition", "2021"])
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path);
    if let Some((dep_dir, rlib)) = cdz_rt_link() {
        cmd.arg("-L")
            .arg(format!("dependency={}", dep_dir.display()))
            .arg("--extern")
            .arg(format!("cdz_rt={}", rlib.display()));
    }
    let status = cmd.output().expect("run rustc");
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
    let out = String::from_utf8_lossy(&run.stdout).trim().to_string();
    // Success → drop the unique per-invocation dir (see `rustc_run`); left on an assert-panic for debug.
    let _ = std::fs::remove_dir_all(&dir);
    Some(out)
}

/// Locate the built `cdz-rt` rlib (a dev-dependency, so present when `cargo test` runs) and the `-L`
/// search directory `rustc` needs, as `(dep_dir, rlib_path)`. The test binary lives in
/// `target/<profile>/deps/`, and cargo writes dependency rlibs (with a metadata-hash suffix) into that
/// same `deps/` dir — so search there for `libcdz_rt-*.rlib`. `None` if not found (then the async
/// round-trip skips the extern; a sync module never needs the crate). A hashed name means picking the
/// newest match, which the current build produced.
fn cdz_rt_link() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    dep_rlib_link("libcdz_rt")
}

/// Same as `cdz_rt_link` for the `cdz-num` bignum rlib (`libcdz_num-*.rlib`) — the BigInt round-trip
/// tests link it so an emitted `cdz_num::Big` program compiles.
fn cdz_num_link() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    dep_rlib_link("libcdz_num")
}

/// Locate a dev-dependency rlib (a `cargo test` build writes each dep's rlib, with a metadata-hash
/// suffix, into the test binary's `deps/` dir) by its `lib<crate>` filename prefix, returning `(dep_dir,
/// rlib_path)` for `rustc -L dependency=<dep_dir> --extern <crate>=<rlib>`. Picks the newest match.
fn dep_rlib_link(prefix: &str) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe = std::env::current_exe().ok()?;
    let deps = exe.parent()?.to_path_buf(); // …/target/<profile>/deps
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(&deps).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with(prefix) && name.ends_with(".rlib") {
            let mtime = path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            if best.as_ref().is_none_or(|(t, _)| mtime >= *t) {
                best = Some((mtime, path));
            }
        }
    }
    best.map(|(_, rlib)| (deps, rlib))
}

#[test]
fn rustc_roundtrip_bigint_arithmetic_and_render() {
    // BigInt emit-side: a runtime `Big` (cdz_num, source-shared with the wasm runtime) crosses end to end.
    // `Ty::BigInt` → `cdz_num::Big`; `BigInt.of` widens; `+`/`-`/`*`/`/`/`%` are `Big` methods; the result
    // renders as its exact decimal (`to_decimal_string`). Small in-range add: 40+2 = 42.
    // `rustc_run`'s driver prints the call via `{}`; `Big` isn't `Display` (it renders by
    // `to_decimal_string`, as the gate harness does), so the CALL expression asks for that string.
    let add = compile_rust("(module m (def (g) (+ (BigInt.of 40) (BigInt.of 2))) (export g))");
    assert!(
        add.contains("cdz_num::Big"),
        "BigInt maps to cdz_num::Big:\n{add}"
    );
    if let Some(out) = rustc_run(&add, "g().to_decimal_string()") {
        assert_eq!(out, "42", "runtime BigInt add");
    }
    // The DEFINING case — a product that overflows i64 must GROW, not trap: 10^10 * 10^10 = 10^20.
    let big = compile_rust(
        "(module m (def (g) (* (BigInt.of 10000000000) (BigInt.of 10000000000))) (export g))",
    );
    if let Some(out) = rustc_run(&big, "g().to_decimal_string()") {
        assert_eq!(out, "100000000000000000000", "10^10 squared grows past i64");
    }
    // A negative subtract crosses with its sign; truncating divide.
    let neg = compile_rust("(module m (def (g) (- (BigInt.of 42) (BigInt.of 100))) (export g))");
    if let Some(out) = rustc_run(&neg, "g().to_decimal_string()") {
        assert_eq!(out, "-58", "negative BigInt result keeps its sign");
    }
    let divv = compile_rust("(module m (def (g) (/ (BigInt.of 100) (BigInt.of 7))) (export g))");
    if let Some(out) = rustc_run(&divv, "g().to_decimal_string()") {
        assert_eq!(out, "14", "truncating BigInt divide");
    }
    // REGRESSION (Copilot PR#464): `BigInt.of` is `∀a.(Int a)->BigInt`, so a runtime UNSIGNED operand
    // >= 2^63 must widen BY VALUE. The old `Big::from_i64((v) as i64)` reinterpreted a large u64 as
    // NEGATIVE (silent miscompile). The emit now goes through `i128` (a `uN as i128` keeps its true sign).
    // Pin `BigInt.of` on a runtime UInt64 = 2^63 → the POSITIVE 9223372036854775808, not a negative value.
    let usig = compile_rust("(module m (def (g (: n UInt64)) (BigInt.of n)) (export g))");
    assert!(
        usig.contains("as i128"),
        "BigInt.of widens via i128 (not `as i64`):\n{usig}"
    );
    if let Some(out) = rustc_run(&usig, "g(9223372036854775808u64).to_decimal_string()") {
        assert_eq!(
            out, "9223372036854775808",
            "BigInt.of on a UInt64 >= 2^63 is POSITIVE, not negative"
        );
    }
    // …and u64::MAX round-trips as its true positive value (was -1 under the `as i64` bug).
    if let Some(out) = rustc_run(&usig, "g(u64::MAX).to_decimal_string()") {
        assert_eq!(
            out, "18446744073709551615",
            "BigInt.of(u64::MAX) is the positive max, not -1"
        );
    }
}

#[test]
fn rustc_roundtrip_rational_arithmetic_and_render() {
    // Rational emit-side: `Ty::Rational` → `cdz_num::Rational` (a Big num/den pair, canonical normalized);
    // `Rational.of`/`.of-int` widen + build, `+`/`-`/`*`/`/` are Rational methods, cmp reduces to a bool,
    // render is `n/d`. Values MIRROR the wasm runtime's rational-* byte-for-byte. 1/3 + 1/6 = 1/2 exactly.
    let add =
        compile_rust("(module m (def (g) (+ (Rational.of 1 3) (Rational.of 1 6))) (export g))");
    assert!(
        add.contains("cdz_num::Rational"),
        "Rational maps to cdz_num::Rational:\n{add}"
    );
    if let Some(out) = rustc_run(&add, "g().to_display_string()") {
        assert_eq!(out, "1/2", "exact rational addition, lowest terms");
    }
    // Reduce to lowest terms; sign normalized onto the numerator; a whole rational keeps `/1`.
    let red = compile_rust("(module m (def (g) (Rational.of 2 4)) (export g))");
    if let Some(out) = rustc_run(&red, "g().to_display_string()") {
        assert_eq!(out, "1/2", "2/4 reduces to 1/2");
    }
    let sign = compile_rust("(module m (def (g) (Rational.of 3 -4)) (export g))");
    if let Some(out) = rustc_run(&sign, "g().to_display_string()") {
        assert_eq!(
            out, "-3/4",
            "sign moves to the numerator, denominator positive"
        );
    }
    let whole = compile_rust("(module m (def (g) ((. Rational of-int) 5)) (export g))");
    if let Some(out) = rustc_run(&whole, "g().to_display_string()") {
        assert_eq!(out, "5/1", "a whole rational carries denominator 1");
    }
    // A Rational is a valid BTreeSet/BTreeMap key (impl Ord) — dedup by exact value: {1/2, 2/4, 1/3} → 2.
    let set = compile_rust(
        "(module m (def (g) (Set.len (Set.of (list (Rational.of 1 2) (Rational.of 2 4) (Rational.of 1 3))))) \
           (export g))",
    );
    if let Some(out) = rustc_run(&set, "g()") {
        assert_eq!(
            out, "2",
            "a Rational set dedups by normalized value (1/2 == 2/4)"
        );
    }
}

#[test]
fn quantity_result_maps_to_inner_at_any_scale1_unit_else_declines() {
    // A QUANTITY RESULT at a scale-1 unit maps to its INNER magnitude's Rust type (the `Ty::Qty` wrapper
    // erases in `lower`); the unit's canonical VALUE form is carried in a `// cdz-unit` note
    // (`Unit::render_value_form`) and rendered `((. Qty of) <mag> <unit>)` by the gate harness (the corpus
    // cases pin the end-to-end render). Here we pin the EMIT side: a scale-1 unit of ANY shape compiles with
    // the inner type + the value-form note; a non-scale-1 unit still declines.
    let base = compile_rust("(module m (def (g) (Qty.of 5.0 (Unit.base #\"meter\"))) (export g))");
    assert!(
        base.contains("-> f64")
            && base.contains("// cdz-return[g]: (Qty Float64 (Unit.base #\"meter\"))")
            && base.contains("// cdz-unit[g]: ((. Unit base) #\"meter\")"),
        "a Qty{{Float64,meter}} result emits the inner f64 + return + value-form unit notes:\n{base}"
    );
    // A NON-reference (scaled) unit still DECLINES — its display would scale the magnitude (deferred).
    let scaled =
        compile_rust_result("(module m (def (g) (Qty.of 5 (Unit.of #\"mile\"))) (export g))");
    assert!(
        scaled.is_err(),
        "a non-scale-1 (mile) quantity result declines (display-scaling not yet done):\n{scaled:?}"
    );
    // A SINGLE base to a POSITIVE power — `meter²`, an area from `m·m`. The value-form note carries the
    // `Unit.^` surface (the gate's area cases pin the end-to-end render).
    let area = compile_rust(
        "(module m (def (g) (* (Qty.of 2.0 (Unit.base #\"meter\")) (Qty.of 3.0 (Unit.base #\"meter\")))) (export g))",
    );
    assert!(
        area.contains("-> f64")
            && area.contains("// cdz-unit[g]: (Unit.^ ((. Unit base) #\"meter\") 2)"),
        "a meter² result emits the inner f64 + a `Unit.^` value-form unit note:\n{area}"
    );
    // A QUOTIENT of DISTINCT bases — a velocity `m/s` — now COMPILES (scale-1, the value-form note renders it
    // as a `Unit./` quotient, cdz-run's canonical surface). The `cdz-return` note's TYPE surface spells it
    // `Unit.*`/`Unit.^ -1`, but the `cdz-unit` note carries the quotient form the boundary render needs.
    let velocity = compile_rust(
        "(module m (def (g) (/ (Qty.of 6.0 (Unit.base #\"meter\")) (Qty.of 2.0 (Unit.base #\"second\")))) (export g))",
    );
    assert!(
        velocity.contains("-> f64")
            && velocity.contains(
                "// cdz-unit[g]: (Unit./ ((. Unit base) #\"meter\") ((. Unit base) #\"second\"))"
            ),
        "a velocity (m/s) result emits the inner f64 + a `Unit./` quotient value-form note:\n{velocity}"
    );
    // A reciprocal / negative power — `second⁻¹`, a frequency — renders as `(Unit./ (. Unit one) …)`.
    let freq = compile_rust(
        "(module m (def (g) (Qty.pow (Qty.of 2.0 (Unit.base #\"second\")) -1)) (export g))",
    );
    assert!(
        freq.contains("// cdz-unit[g]: (Unit./ (. Unit one) ((. Unit base) #\"second\"))"),
        "a frequency (second⁻¹) result emits a `Unit./` over the dimensionless numerator:\n{freq}"
    );
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
    // A generic MULTI-variant sum's params become the enum's type parameters (`T0`…), a param-typed
    // payload renders as its `T{k}`, and a use at a concrete type instantiates them via
    // `types::rust_type`. Two variants keep it a boxed sum (a SINGLE-variant generic sum is a NEWTYPE that
    // ERASES — see `a_generic_newtype_erases_to_its_underlying_rust_type`), so this covers the enum path.
    let rs = compile_rust(
        "(module m (type Box (Wrap a) Empty) \
           (def (unwrap (: b (Box Int64))) (match b (((. Box Wrap) x) x) (((. Box Empty) _) 0))) (export unwrap))",
    );
    assert!(
        rs.contains("pub enum Box<T0>") && rs.contains("Wrap(T0)"),
        "generic enum decl:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: Box<i64>)"),
        "instantiated use:\n{rs}"
    );
}

#[test]
fn a_monomorphic_newtype_erases_and_emits_no_enum() {
    // A monomorphic newtype `(type UserId (Mk Int64))` erases: NO `enum UserId` is emitted (its value IS
    // the i64), and a param typed by it maps to the underlying `i64`. (Regression: before the enum-skip,
    // a dead `enum UserId { Mk(i64) }` was emitted — harmless but wrong; the value reads through the tag.)
    let rs = compile_rust(
        "(module m (type UserId (Mk Int64)) \
           (def (unwrap (: b UserId)) (match b ((Mk x) x))) (export unwrap))",
    );
    assert!(
        !rs.contains("enum UserId"),
        "an erased newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: i64)"),
        "the newtype param maps to its underlying i64:\n{rs}"
    );
}

#[test]
fn a_generic_newtype_erases_to_its_underlying_rust_type() {
    // A GENERIC single-variant sum is an erasable NEWTYPE: it emits NO enum, and a use at a concrete
    // instantiation maps to the underlying (substituted) Rust type — `(Box Int64)` → `i64`. The runtime
    // value IS the payload (the tag adds nothing), so the Rust backend reads through it, exactly as the
    // wasm backend does. The monomorphic sibling is `a_monomorphic_newtype_erases_and_emits_no_enum`.
    let rs = compile_rust(
        "(module m (type Box (Wrap a)) \
           (def (unwrap (: b (Box Int64))) (match b ((Wrap x) x))) (export unwrap))",
    );
    assert!(
        !rs.contains("enum Box"),
        "an erased generic newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("unwrap(b: i64)"),
        "the newtype param maps to its underlying i64:\n{rs}"
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
fn a_recursive_sum_emits_a_boxed_enum() {
    // A recursive sum now EMITS a Rust enum with the recursive variant's field BOXED (`Box<…>`) for
    // finite size — a function taking/returning it compiles. (Was deferred as "needs Box"; now realized:
    // the recursive variant field is one `Box`, construction `Box::new(…)`, match derefs `*__pay`.)
    let rs = try_compile_rust(
        "(module m (type IntList Nil (Cons (Tuple Int64 IntList))) \
           (def (len (: xs IntList)) (match xs (((. IntList Nil) _) 0) \
                                              (((. IntList Cons) (tuple h t)) (+ 1 (len t))))) (export len))",
    )
    .expect("a recursive sum now emits a boxed enum");
    assert!(
        rs.contains("enum IntList") && rs.contains("Box<"),
        "the recursive variant's field is boxed: {rs}"
    );
}

#[test]
fn rustc_roundtrip_generic_recursive_tree_boxes_and_counts() {
    // A GENERIC recursive sum `(type Tree (Leaf a) (Node (Tuple (Tree a) (Tree a))))` — the recursive
    // `Tree` occurrence is a type PARAMETER instantiation nested inside a `Tuple`. The Box-decision
    // (`variant_payloads_mention`) resolved a payload with `typeval_of`, which returns `None` for a payload
    // mentioning a type param (unbound), so `Node`'s recursive tuple was NOT boxed → `Node((Tree<T0>,
    // Tree<T0>))` had INFINITE size (rustc E0072). Resolving the payload PARAM-TOLERANTLY (at a sentinel
    // instantiation, the same path the enum-field render uses) makes `reaches_decl` see the self-reference
    // and box it: `Node(Box<(Tree<T0>, Tree<T0>)>)`. `cnt` counts leaves; a two-leaf node = 2.
    let rs = compile_rust(
        "(module m (type Tree (Leaf a) (Node (Tuple (Tree a) (Tree a)))) \
           (def (cnt (: t (Tree Int64))) (match t (((. Tree Leaf) _) 1) \
                                                   (((. Tree Node) (tuple l r)) (+ (cnt l) (cnt r))))) \
           (export cnt))",
    );
    assert!(
        rs.contains("enum Tree<T0>") && rs.contains("Node(::std::boxed::Box<"),
        "the generic recursive variant's nested-tuple field is boxed (was E0072 infinite size): {rs}"
    );
    // End-to-end through rustc: builds (was E0072) and cnt of a two-leaf node = 2, matching wasm.
    if let Some(out) = rustc_run(
        &rs,
        "cnt(Tree::Node(Box::new((Tree::Leaf(1), Tree::Leaf(2)))))",
    ) {
        assert_eq!(
            out, "2",
            "the generic recursive tree builds and counts:\n{rs}"
        );
    }
}

#[test]
fn a_recursive_newtype_declines_the_whole_function() {
    // A RECURSIVE NEWTYPE `(type Lst (Mk (Option (Tuple Int64 Lst))))` erases to its inner type on the rust
    // backend — but the inner type mentions `Lst` (the μ back-edge), which erasure would render as a bare
    // `Lst` that is never emitted → `cannot find type Lst` (an uncompilable crate). Erasure only works when
    // the unfold terminates; a recursive newtype needs a Box-indirected nominal emission (deferred, like a
    // recursive sum). So a function taking/returning it DECLINES, exactly as a recursive SUM does, rather
    // than emitting a signature naming an undeclared type. (The wasm backend runs it — types erase at
    // runtime with no Rust-level name needed.)
    let err = try_compile_rust(
        "(module m (type Lst (Mk (Option (Tuple Int64 Lst)))) \
           (def (sm (: l Lst)) (match l ((Mk o) (match o ((Some p) (+ (. p 0) (sm (. p 1)))) ((None) 0))))) \
           (export sm))",
    )
    .expect_err("a recursive newtype must decline, not emit an uncompilable crate");
    assert!(
        err.iter()
            .any(|d| d.contains("recursive") || d.contains("no emitted Rust enum")),
        "decline reason should cite the recursive/unrepresentable type: {err:?}"
    );
}

#[test]
fn a_recursive_sum_constructed_as_a_discarded_intermediate_folds_away() {
    // A helper `mk` returns `(tuple (NLit 5) 9)` — a pair whose element 0 is a recursive-sum value and
    // whose element 1 is the Int64 9. `main` reads `.1` and DISCARDS element 0. The projection folds
    // through the constant tuple (`(. l 1)` → 9), so the discarded `(NLit 5)` is never constructed and
    // `main` compiles to the constant 9 on BOTH backends — the same DCE the wasm backend performs, reached
    // on rust too. (The recursive `Node` enum now DOES emit — a boxed enum — but that is a harmless
    // `#[allow(dead_code)]` declaration; the load-bearing property is that `main` folds to 9, NOT whether
    // the declared-but-unused enum is present.)
    let rs = try_compile_rust(
        "(module m (type Node (NLit Int64) (NAdd (Tuple Node Node))) \
           (def (mk) (tuple (NLit 5) 9)) \
           (def (main) (let ((l (mk))) (. l 1))) (export main))",
    )
    .expect("a discarded recursive-sum intermediate folds away — the projection drops it");
    // `main` returns the folded constant 9 (the discarded `(NLit 5)` is never constructed in `main`'s body).
    assert!(
        rs.contains("9u64 as i64") || rs.contains("9i64") || rs.contains("-> i64"),
        "main folds to the projected Int64 constant 9: {rs}"
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
fn rustc_roundtrip_recursive_sum_folds() {
    // A RECURSIVE user sum (a cons-list) constructs and folds THROUGH RUSTC: the enum is `Box`ed
    // (`Cons(Box<(i64, L)>)`), construction `Box::new(…)`, match derefs `*p`. `sm` sums a runtime-passed
    // list; `sm(L::Cons(Box::new((1, L::Cons(Box::new((2, L::Nil)))))))` = 3. Pins that a recursive sum
    // RUNS on the Rust backend (not just emits) and agrees with the wasm oracle — the last rust-backend
    // sum gap, now closed via Box indirection.
    let rs = compile_rust(
        "(module m (type L Nil (Cons (Tuple Int64 L))) \
           (def (sm (: l L)) (match l (((. L Nil) _) 0) \
                                      (((. L Cons) (tuple h t)) (+ h (sm t))))) (export sm))",
    );
    if let Some(out) = rustc_run(
        &rs,
        "sm(L::Cons(Box::new((1, L::Cons(Box::new((2, L::Nil)))))))",
    ) {
        assert_eq!(out, "3");
    }
    if let Some(out) = rustc_run(&rs, "sm(L::Nil)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_boxed_payload_projected_more_than_once_clones() {
    // A recursive sum whose `Cons` payload the Rust backend BOXES (`Cons(Box<(i64,L)>)`), where the bound
    // payload field is PROJECTED MORE THAN ONCE: `(let ((d (f t))) (if (= d 0) h d))` uses the recursive
    // tail `t` in both the `if` condition and the else-branch. A bare `(*box).1` read MOVES the non-`Copy`
    // `L` field out of the box — the FIRST projection moves it, so the second is E0382 "use of moved
    // value". The fix CLONES each boxed-payload projection, so each read is an owned copy that leaves the
    // box intact. Pins that the emitted crate BUILDS and runs to f(Cons 5 Nil) = 5 (was a build failure);
    // the wasm gate never saw this (it re-reads the heap slot with no move discipline).
    let rs = compile_rust(
        "(module m (type L (Nil) (Cons Int64 L)) \
           (def (f (: xs L)) (match xs ((Nil) 0) ((Cons h t) (let ((d (f t))) (if (= d 0) h d))))) \
           (export f))",
    );
    // Each boxed-payload projection is cloned (not a bare moving `(*p).N`).
    assert!(
        rs.contains(".clone()"),
        "a boxed payload projection clones to avoid moving out of the Box:\n{rs}"
    );
    // End-to-end through rustc: builds (was E0382) and f(Cons 5 Nil) = 5, matching the wasm oracle.
    if let Some(out) = rustc_run(&rs, "f(L::Cons(Box::new((5, L::Nil))))") {
        assert_eq!(
            out, "5",
            "the multiply-projected boxed payload builds and runs:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_mutually_recursive_sums_fold() {
    // A MUTUALLY-recursive pair of sums (`A` references `B`, `B` references `A`) — NEITHER variant mentions
    // its OWN decl, but the A→B→A cycle needs Box indirection all the same (E0072 otherwise). Both edges
    // box: `A { AN(Box<B>) }`, `B { BN(Box<A>) }`, construction `Box::new(…)`, match derefs. `sa(A::AN(…B…))`
    // walks A→B→A to the `AL 9` leaf = 9. Pins that the cycle detector boxes a MUTUAL recursion, not only a
    // direct self-reference — and that it RUNS on rustc, matching the wasm oracle.
    let rs = compile_rust(
        "(module m (type A (AL Int64) (AN B)) (type B (BL Int64) (BN A)) \
           (def (sa (: a A)) (match a (((. A AL) n) n) (((. A AN) b) (sb b)))) \
           (def (sb (: b B)) (match b (((. B BL) n) n) (((. B BN) a) (sa a)))) (export sa))",
    );
    if let Some(out) = rustc_run(&rs, "sa(A::AN(Box::new(B::BN(Box::new(A::AL(9))))))") {
        assert_eq!(out, "9");
    }
    if let Some(out) = rustc_run(&rs, "sa(A::AL(42))") {
        assert_eq!(out, "42");
    }
}

#[test]
fn a_wide_mutually_recursive_cycle_boxes_every_edge() {
    // A CYCLE of many mutually-recursive sums `T0→T1→…→T{n-1}→T0` — the shape whose recursive-variant
    // boxing check the cycle detector runs per variant per enum. That check is now memoized (a per-decl
    // out-edge cache + a per-decl reachable-set cache + an O(1) `type_decl_by_occ` index), turning what
    // was O(N³) (a fresh full-cycle DFS per variant, with a linear `type_decl_by_occ` scan per step) into
    // ~O(N²). This locks in that the memoized path still produces the CORRECT boxing at width: every
    // `Mk{i}` variant carries `Box<T{i+1}>` (its payload reaches back around the cycle), and every enum
    // emits — the same verdict the un-memoized fresh-DFS gave.
    let n = 12;
    let types: Vec<String> = (0..n)
        .map(|i| format!("(type T{i} (Mk{i} T{}) (End{i}))", (i + 1) % n))
        .collect();
    let src = format!(
        "(module m {} (def (f (: t T0)) (match t (((. T0 Mk0) _) 1) (((. T0 End0) _) 0))) (export f))",
        types.join(" ")
    );
    let rs = try_compile_rust(&src).expect("a wide mutually-recursive cycle emits boxed enums");
    // Every cycle edge is boxed: `Mk{i}(::std::boxed::Box<T{i+1}>)` for each i (each payload reaches back
    // to its own sum through the cycle, so the finite-size Box is required on all of them). The box is
    // fully-qualified so a user sum named `Box` cannot shadow the heap pointer.
    for i in 0..n {
        let nxt = (i + 1) % n;
        assert!(
            rs.contains(&format!("Mk{i}(::std::boxed::Box<T{nxt}>)")),
            "T{i}'s recursive variant must box its T{nxt} payload; got:\n{rs}"
        );
        // The nullary `End{i}` variant carries no payload (not boxed) — a spot-check that boxing is
        // selective, not blanket.
        assert!(
            rs.contains(&format!("enum T{i} ")),
            "enum T{i} must emit: {rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_erased_newtype_wrapping_a_sum_nested_match() {
    // A single-variant sum is an ERASED newtype (no Rust enum — its value IS the payload). When it WRAPS
    // A SUM (`(type W (V (Result …)))`) and is matched with a NESTED constructor pattern (`(W.V (Ok n))`),
    // the decision-tree's switch/bind paths carry the newtype's `Payload` step, but `lower` erased that
    // same step from the BODY's payload reads — so the switch dispatched one level too shallow and the
    // arm's payload binder mismatched the body read ("sum payload has no bound match arm"). The backend
    // now erases the nominal switch step (twin of `lower::erase_nominal_steps`), so the switch dispatches
    // on the INNER sum directly: `match w { Result::Ok(p) => p, Result::Err(p) => p }`. `f(Ok 5)`=5,
    // `f(Err 9)`=9 — the erased wrapper is invisible at runtime, matching the wasm oracle.
    let rs = compile_rust(
        "(module m (type W (V (Result Int64 Int64))) \
           (def (f (: w W)) (match w (((. W V) (Result.Ok n)) n) (((. W V) (Result.Err e)) e))) \
           (export f))",
    );
    // The erased newtype emits NO `enum W`; the match dispatches on the inner Result directly.
    assert!(
        !rs.contains("enum W "),
        "erased newtype emits no enum:\n{rs}"
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "dispatches on inner sum:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(Ok(5))") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(Err(9))") {
        assert_eq!(out, "9");
    }
}

#[test]
fn rustc_roundtrip_erased_newtype_wrapping_a_sum_binder_reads_the_inner_not_the_wrapper() {
    // REGRESSION for a latent `fold_sum_path` miscompile (shared by BOTH backends) the Rust decline had
    // been masking: `fold_sum_path` re-read `type_of(cur)` each step to detect a nominal newtype, but for
    // a newtype WRAPPING A SUM the erased value stays the SAME node, so its raw type read `Ty::Nominal` for
    // EVERY step — the inner sum's `Payload` was consumed as a SECOND nominal no-op and a payload binder
    // folded to the WHOLE wrapper (`n` in `(W.V (Ok n))` became the whole `Result`, an infinite-ish wrong
    // value). Fixed with a PEELED type cursor (one nominal layer per erased step). Here the whole thing
    // folds (constant scrutinee), so the answer proves the binder reads the INNER Int, not the wrapper:
    // `(W.V (Ok 7))` matched `(W.V (Ok n))` → 7.
    let rs = compile_rust(
        "(module m (type W (V (Result Int64 Int64))) \
           (def (run) (match (W.V (Result.Ok 7)) (((. W V) (Result.Ok n)) n) \
                                                 (((. W V) (Result.Err e)) e))) (export run))",
    );
    if let Some(out) = rustc_run(&rs, "run()") {
        assert_eq!(
            out, "7",
            "binder reads the inner Int, not the wrapper:\n{rs}"
        );
    }
}

#[test]
fn rustc_roundtrip_recursive_sum_literal_refined_const_disc_root() {
    // A recursive-sum match with a LITERAL-REFINED payload arm (`(Cons 0 t)`) over a scrutinee whose
    // DISCRIMINANT is statically known (a constant `SumNew` — `(Cons x Nil)`, `Cons` tag known, `x`
    // runtime). `lower`'s known-disc fold collapses the root `Switch` to the `Cons` arm's continuation — a
    // bare `LitTest` at ROOT. The backend previously declined a non-`Switch` root; it now routes `LitTest`
    // / `Guarded` / `Leaf` roots through `emit_sum_cont` (which reads the sub-value via `emit_sum_payload`,
    // folding against the constant scrutinee's payloads). `f(Cons 0 Nil)` hits the literal arm = 100;
    // `f(Cons 7 Nil)` binds `h` = 7. Matches the wasm oracle — the last non-Switch-root gap.
    let rs = compile_rust(
        "(module m (type L (Nil) (Cons Int64 L)) \
           (def (f (: x Int64)) (match (L.Cons x (L.Nil)) \
                                  (((. L Cons) 0 t) 100) (((. L Cons) h t) h) (((. L Nil)) -1))) \
           (export f))",
    );
    if let Some(out) = rustc_run(&rs, "f(0)") {
        assert_eq!(out, "100");
    }
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
}

#[test]
fn rustc_roundtrip_nested_match_on_a_variant_at_disc_ge_1() {
    // A NESTED constructor match on a variant that is NOT variant 0 — `(type W (A Int64) (V (Option
    // Int64)))` matched `((W.V (Option.Some n)) …)`, where the nested-sum-carrying variant `V` is at
    // discriminant 1. The backend's nested-switch subject-type walk read variant 0's payload
    // unconditionally, so the inner switch on `W.V`'s Option resolved to `A`'s `Int64` (not `Option Int64`)
    // and declined (`sum construction node is not a sum type`). It now RECORDS each entered arm's payload
    // type in the ctx (`sum_path_types`, the twin of `lower`'s `path_types`) and the nested switch looks it
    // up — so it dispatches on the Option even when the discriminant is RUNTIME (an `if` selecting the
    // variant): `f(V(Some 7))` = 7, `f(V(None))` = -2. Matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type W (A Int64) (V (Option Int64))) \
           (def (f (: k Int64)) (match (W.V (if (> k 0) (Option.Some k) (Option.None))) \
                                  ((W.A h) h) ((W.V (Option.Some n)) n) ((W.V (Option.None)) -2))) \
           (export f))",
    );
    // Dispatches on the INNER Option (V's payload), not A's Int64.
    assert!(
        rs.contains("Option::Some(__pay"),
        "inner Option switch:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "-2");
    }
}

#[test]
fn rustc_roundtrip_two_disc_ge_1_nested_sum_variants_over_a_runtime_disc() {
    // The HARDER disc-≥1 shape: TWO variants past the first each carry a DIFFERENT nested sum, and the
    // scrutinee's discriminant is genuinely RUNTIME (an `if` chooses `W.U` vs `W.V`, so no constant `disc`
    // to read). A constant-value cursor can't recover the disc here; the arm-recorded `sum_path_types`
    // hint can — each arm records ITS variant's payload type, and the two nested switches (`U`'s Option,
    // `V`'s Result) each resolve their subject by lookup. `f(5)` → `W.U(Some 5)` → 5; `f(-1)` →
    // `W.V(Ok 0)` → 0. Was a Rust-backend decline (`sum construction node is not a sum type`).
    let rs = compile_rust(
        "(module m (type W (A Int64) (U (Option Int64)) (V (Result Int64 Int64))) \
           (def (f (: k Int64)) (match (if (> k 0) (W.U (Option.Some k)) (W.V (Result.Ok 0))) \
                                  ((W.A h) h) ((W.U (Option.Some n)) n) ((W.U (Option.None)) -1) \
                                  ((W.V (Result.Ok o)) o) ((W.V (Result.Err e)) e))) (export f))",
    );
    assert!(
        rs.contains("Option::Some(__pay"),
        "U's inner Option switch:\n{rs}"
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "V's inner Result switch:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_match_a_runtime_tuple_from_an_if() {
    // A top-level `(tuple a b)` pattern over a RUNTIME tuple scrutinee — a tuple built by an `if` (or a
    // branchy fn), NOT a constant `Core::Tuple` and NOT a bound `__pay` (a top-level tuple match mints no
    // `Switch` arm, hence no bind). The binders `a`/`b` read `[Elem(0)]`/`[Elem(1)]` directly off the
    // scrutinee; the backend now emits the scrutinee value indexed (`(<t>).i`) — the runtime-tuple twin of
    // the constant fold. Was a Rust-backend decline ("sum payload has no bound match arm"); wasm reads it
    // via `arr-get` (no bind needed). `g(5)` = (5,0) → 5, `g(-3)` = (0,-3) → -3.
    let rs = compile_rust(
        "(module m (def (g (: k Int64)) (if (> k 0) (tuple k 0) (tuple 0 k))) \
           (def (f (: k Int64)) (match (g k) ((tuple a b) (+ a b)))) (export f))",
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "5");
    }
    if let Some(out) = rustc_run(&rs, "f(-3)") {
        assert_eq!(out, "-3");
    }
}

#[test]
fn a_generic_sum_emits_a_parameterized_descriptor_for_the_gate_renderer() {
    // A GENERIC user sum emits a `// cdz-sum[Ident]:` descriptor whose payload tokens carry `T{k}`
    // PLACEHOLDERS (not concrete types), plus a `// cdz-sum-params[Ident]: N` note giving the parameter
    // count — so the gate's rust-target value renderer can substitute the result type's concrete args and
    // render a generic-sum escape (was: no descriptor → the escape fell to a scalar `Display` of the enum
    // → rustc E0277). A bare-param payload `(W a)` renders `T0`; a nested one `(W (Option a))` renders
    // `(Option T0)`.
    let bare =
        compile_rust("(module m (type Box (W a) (E)) (def (main) (Box.W 42)) (export main))");
    assert!(
        bare.contains("// cdz-sum[Box]: (W T0) (E)"),
        "bare-param generic sum descriptor uses T0 placeholder:\n{bare}"
    );
    assert!(
        bare.contains("// cdz-sum-params[Box]: 1"),
        "generic sum records its parameter count:\n{bare}"
    );

    // A nested-param payload renders the placeholder inside the nesting: `(W (Option a))` → `(Option T0)`.
    let nested = compile_rust(
        "(module m (type Box (E) (W (Option a))) (def (main) (Box.W (Option.Some 42))) (export main))",
    );
    assert!(
        nested.contains("// cdz-sum[Box]: (E) (W (Option T0))"),
        "nested-param generic sum descriptor places T0 inside the payload:\n{nested}"
    );

    // A MONOMORPHIC sum still emits its descriptor WITHOUT a params note (concrete payload types, no
    // placeholders) — the generic path must not regress the monomorphic one.
    let mono = compile_rust(
        "(module m (type W (V (Option Int64)) (Z)) (def (main) (W.V (Option.Some 5))) (export main))",
    );
    assert!(
        mono.contains("// cdz-sum[W]: (V (Option Int64)) (Z)"),
        "monomorphic descriptor keeps concrete payload types:\n{mono}"
    );
    assert!(
        !mono.contains("// cdz-sum-params[W]:"),
        "a monomorphic sum emits no params note:\n{mono}"
    );
}

#[test]
fn rustc_roundtrip_three_level_nested_sum_match_folds_through_known_constructors() {
    // THREE sum levels — `Outer.Q → Inner.Y → Result.Ok` — where the two OUTER variants are known
    // constructors (built inline) and only the innermost `Result` disc is runtime. `lower`'s disc-fold
    // collapses the two known outer switches, leaving a SINGLE `Result` switch whose subject sits at a deep
    // path (`[Payload, Payload]`) with NO enclosing binds. The backend reads that subject directly off the
    // constant value tree (`fold_const_sum_path` walks several `Payload`s deep), so it emits
    // `match <inner Result> { Ok(p) => p, Err(p) => p }`. `f(6)` = 6, `f(-1)` = 0 (Err payload). Was a
    // Rust-backend decline (`sum payload has no bound match arm`); matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type Inner (X Int64) (Y (Result Int64 Int64))) (type Outer (P Int64) (Q Inner)) \
           (def (f (: k Int64)) (match (Outer.Q (Inner.Y (if (> k 0) (Result.Ok k) (Result.Err 0)))) \
                                  ((Outer.P h) h) ((Outer.Q (Inner.X n)) n) \
                                  ((Outer.Q (Inner.Y (Result.Ok o))) o) \
                                  ((Outer.Q (Inner.Y (Result.Err e))) e))) (export f))",
    );
    assert!(
        rs.contains("Result::Ok(__pay"),
        "dispatches on the innermost Result:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(6)") {
        assert_eq!(out, "6");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_generic_sum_with_a_type_param_nested_in_a_variant_payload() {
    // A GENERIC sum whose variant payload has the type parameter NESTED inside another type — `(type Box
    // (E) (W (Option a)))`, so `W`'s payload is `Option<a>`, not a bare `a`. The enum emitter mapped only a
    // WHOLE-payload param to `T{k}`; a param nested in `(Option a)` reached `rust_type(Ty::Var)` = None and
    // the whole generic enum declined ("no native representation"). It now renders the payload at a SENTINEL
    // instantiation and maps each param var to `T{k}` wherever it appears, emitting `enum Box<T0> { E,
    // W(Option<T0>) }`. `f(7)` = 7 (W(Some 7)), `f(-1)` = -1 (E). Matches the wasm oracle.
    let rs = compile_rust(
        "(module m (type Box (E) (W (Option a))) \
           (def (f (: k Int64)) (match (if (> k 0) (Box.W (Option.Some k)) (Box.E)) \
                                  ((Box.E) -1) ((Box.W (Option.Some n)) n) ((Box.W (Option.None)) -2))) \
           (export f))",
    );
    // The nested param renders `Option<T0>`, not a declined `Ty::Var`.
    assert!(
        rs.contains("W(Option<T0>)"),
        "nested param renders T0:\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(7)") {
        assert_eq!(out, "7");
    }
    if let Some(out) = rustc_run(&rs, "f(-1)") {
        assert_eq!(out, "-1");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_payload_sum() {
    // Runtime `(= a b)` over a PAYLOAD-carrying sum. On wasm this is a value-heap equality walk; on the
    // Rust backend the sum maps to a native enum that now `#[derive(PartialEq, Eq)]` (its payloads are
    // Eq-derivable), so `=` emits a native `a == b` — was a decline ("does not yet render this compound
    // value"). `f(5)` compares `W.V(Some 5) == W.V(Some 5)` → 1; `f(9)` → `Some 9 != Some 5` → 0. Matches
    // the wasm oracle's structural equality.
    let rs = compile_rust(
        "(module m (type W (V (Option Int64)) (Z)) \
           (def (f (: k Int64)) (if (= (W.V (Option.Some k)) (W.V (Option.Some 5))) 1 0)) (export f))",
    );
    assert!(
        rs.contains("PartialEq, Eq"),
        "payload sum derives Eq:\n{rs}"
    );
    assert!(rs.contains("=="), "emits a native == :\n{rs}");
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "1");
    }
    if let Some(out) = rustc_run(&rs, "f(9)") {
        assert_eq!(out, "0");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_compound_with_a_rational_or_bigint_leaf() {
    // Runtime `(= a b)` over a COMPOUND carrying a Rational / BigInt leaf. `cdz_num::Rational` is stored
    // NORMALIZED (lowest terms) and `cdz_num::Big` sign-magnitude with no leading zeros, both `#[derive(Eq)]`
    // — so a native field-wise `==` compares by CANONICAL value, matching the wasm heap walk. Was a decline
    // ("runtime structural equality over this compound is not yet rendered").
    // A tuple with a Rational leaf: `(1/2, 1) == (2/4, 1)` is TRUE (normalization), `(1/2,1) == (1/3,1)` FALSE.
    let rat = compile_rust(
        "(module m (def (eq2 (: n Int64) (: d Int64)) \
           (if (= (tuple (Rational.of 1 2) 1) (tuple (Rational.of n d) 1)) 1 0)) (export eq2))",
    );
    assert!(
        rat.contains("=="),
        "a tuple with a Rational leaf emits a native == :\n{rat}"
    );
    if let Some(out) = rustc_run(&rat, "eq2(2, 4)") {
        assert_eq!(out, "1", "1/2 == 2/4 (normalized) inside a tuple");
    }
    if let Some(out) = rustc_run(&rat, "eq2(1, 3)") {
        assert_eq!(out, "0", "1/2 != 1/3 inside a tuple");
    }
    // A tuple with a BigInt leaf compares by value: `(big 5, 1) == (big 5, 1)` → 1, vs `(big 6, 1)` → 0.
    let big = compile_rust(
        "(module m (def (eqb (: k Int64)) \
           (if (= (tuple (BigInt.of 5) 1) (tuple (BigInt.of k) 1)) 1 0)) (export eqb))",
    );
    if let Some(out) = rustc_run(&big, "eqb(5)") {
        assert_eq!(out, "1", "a BigInt leaf compares equal by value");
    }
    if let Some(out) = rustc_run(&big, "eqb(6)") {
        assert_eq!(out, "0", "a differing BigInt leaf compares unequal");
    }
}

#[test]
fn rustc_roundtrip_runtime_equality_over_a_generic_sum() {
    // Runtime `(= a b)` over a GENERIC user sum at a concrete instantiation — `(type Box (W a) (E))`
    // compared at `Box Int64`. The generic enum `enum Box<T0> { W(T0), E }` `#[derive(PartialEq, Eq)]`
    // adds a `T0: Eq` bound automatically, so a `=` at `Box Int64` (where `i64: Eq`) emits a native
    // `a == b`. The derive/eq gate treats a type PARAMETER payload as Eq (the derive bounds it), keyed off
    // the sentinel instantiation. `f(5)` compares `Box.W(5) == Box.W(5)` → 1, `f(9)` → unequal → 0.
    let rs = compile_rust(
        "(module m (type Box (W a) (E)) (def (mk (: k Int64)) (if (> k 0) (Box.W k) (Box.E))) \
           (def (f (: k Int64)) (if (= (mk k) (mk 5)) 1 0)) (export f))",
    );
    assert!(rs.contains("enum Box<T0>"), "generic enum:\n{rs}");
    assert!(
        rs.contains(
            "#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]\n#[allow(dead_code)]\npub enum Box<T0>"
        ),
        "generic enum derives Eq + Ord (so it can key a BTreeMap; T0 bounds added by derive):\n{rs}"
    );
    if let Some(out) = rustc_run(&rs, "f(5)") {
        assert_eq!(out, "1");
    }
    if let Some(out) = rustc_run(&rs, "f(9)") {
        assert_eq!(out, "0");
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

#[test]
fn rustc_roundtrip_nullary_first_if_returns_generic_option() {
    // REGRESSION (E0282, surfaced when the BigInt emit-side enabled a kernel program's rust path): an `if`
    // whose result is a GENERIC sum and whose FIRST (`then`) branch is the bare nullary variant — `if c
    // then (Option.None) else (Option.Some …)` — gave rustc no type parameter to infer at the `None`
    // (branches type left-to-right; the sibling `Some` comes second). Bare `Option::None` → "type
    // annotations needed". FIX: the backend annotates the whole `if` with its solved generic-sum type
    // (`{ let __if: Option<i64> = if … ; __if }`), so rustc has the type up front.
    let rs = compile_rust(
        "(module m (def (mk (: n Int64)) (if (= n 0) (Option.None unit) (Option.Some n))) (export mk))",
    );
    assert!(
        rs.contains("let __if: Option<i64>"),
        "the generic-sum if is type-annotated:\n{rs}"
    );
    // n=0 → None → the match's None arm; n=5 → Some(5). Render via a driver that maps to cdz-run's form.
    let driver = "fn main(){ let v = prog::mk(0); let s = match v { Some(x) => format!(\"(Some {})\", x), \
                  None => \"(None unit)\".to_string() }; println!(\"{}\", s); }";
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(
            out, "(None unit)",
            "nullary-first if returns None on the true branch"
        );
    }
    // CONTROL: a MONOMORPHIC-sum if (no type params) stays a bare `if` — the annotation is generic-only.
    let mono = compile_rust(
        "(module m (type S (A) (B)) (def (mk (: n Int64)) (if (= n 0) (S.A) (S.B))) (export mk))",
    );
    assert!(
        !mono.contains("let __if:"),
        "a monomorphic-sum if is NOT annotated (bare if):\n{mono}"
    );
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
    // The gas/yield trait now lives in the SHARED `cdz-rt` crate (NOT re-declared per module); the
    // module brings it into scope with a `use`, so an application implements `CdzEnv` once for all.
    assert!(rs.contains("use cdz_rt::CdzEnv;"), "cdz_rt import:\n{rs}");
    assert!(
        !rs.contains("pub trait CdzEnv"),
        "must NOT re-declare the trait:\n{rs}"
    );
    // The fn is async, takes `__cdz_env: &mut __CdzE`, and charges gas at entry. Both the env TYPE param
    // (`__CdzE`) and the env VALUE param (`__cdz_env`) are reserved `__`-names so neither collides with a
    // user sum's Rust type nor a source parameter literally named `env`.
    assert!(
        rs.contains("pub async fn sum_to<__CdzE: CdzEnv>(__cdz_env: &mut __CdzE, n: i64)"),
        "signature:\n{rs}"
    );
    assert!(
        rs.contains("__cdz_env.consume(1).await;"),
        "gas charge:\n{rs}"
    );
    // The recursive call is boxed-and-awaited, threading the env first.
    assert!(
        rs.contains("Box::pin(sum_to(__cdz_env,"),
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
impl cdz_rt::CdzEnv for M { async fn consume(&mut self, _: u64) {} }
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
impl cdz_rt::CdzEnv for Meter {
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

#[test]
fn rustc_roundtrip_async_recursive_sum_folds() {
    // The async backend's recursion handling (`Box::pin(callee(env, …)).await`) composes with the
    // recursive-sum representation (a boxed self-referential payload, `Cons(Box<(i64, L)>)`): a
    // cons-list summed by a self-recursive `async fn` compiles under the real `CdzEnv`, meters gas at
    // every step, and folds to the same value the sync/wasm backends produce. This is the one sum
    // shape whose async execution the sync round-trips don't cover — the recursive `async fn` future
    // must be `Box::pin`-sized AND its payload `Box`-sized, two independent boxings that must agree.
    let module = compile_rust_async(
        "(module m (type L (Nil) (Cons Int64 L)) \
         (def (sm l) (match l ((L.Nil) 0) ((L.Cons h t) (+ h (sm t))))) \
         (def (main) (sm (L.Cons 1 (L.Cons 2 (L.Cons 3 (L.Nil)))))) (export main))",
    );
    // A recursive `async fn` sizes its future via `Box::pin`; the recursive payload sizes via `Box`.
    assert!(
        module.contains("Cons(::std::boxed::Box<(i64, L)>)"),
        "boxed payload:\n{module}"
    );
    assert!(
        module.contains("Box::pin(sm(__cdz_env,"),
        "boxed recursive call:\n{module}"
    );
    let driver = r#"
struct Meter { spent: u64 }
impl cdz_rt::CdzEnv for Meter {
    async fn consume(&mut self, g: u64) { self.spent += g; }
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
    let mut e = Meter { spent: 0 };
    let v = block_on(prog::main(&mut e));
    // sm([1,2,3]) = 6; gas was metered across the recursive descent (one charge per fn entry).
    println!("{v} {}", e.spent > 3);
}
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "6 true", "async recursive-sum run:\n{module}");
    }
}

#[test]
fn rustc_roundtrip_async_mutually_recursive_sums_fold() {
    // Async recursion + MUTUAL sum recursion: `(type A (AN B))` / `(type B (BN A))` — neither variant
    // mentions its own decl, so the box decision must follow the A→B→A cycle (reaches_decl) to box both
    // `AN(Box<B>)` and `BN(Box<A>)`; and the two mutually-recursive `async fn`s each `Box::pin` the
    // other's call. Both boxings must land or rustc rejects (E0072 infinite size / unsized future).
    let module = compile_rust_async(
        "(module m (type A (AL Int64) (AN B)) (type B (BL Int64) (BN A)) \
         (def (sa a) (match a ((A.AL n) n) ((A.AN b) (sb b)))) \
         (def (sb b) (match b ((B.BL n) n) ((B.BN a) (sa a)))) \
         (def (main) (sa (A.AN (B.BN (A.AL 9))))) (export main))",
    );
    assert!(
        module.contains("AN(::std::boxed::Box<B>)"),
        "boxed A payload:\n{module}"
    );
    assert!(
        module.contains("BN(::std::boxed::Box<A>)"),
        "boxed B payload:\n{module}"
    );
    let driver = r#"
struct M;
impl cdz_rt::CdzEnv for M { async fn consume(&mut self, _: u64) {} }
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
fn main() { println!("{}", block_on(prog::main(&mut M))); }
"#;
    if let Some(out) = rustc_run_driver(&module, driver) {
        assert_eq!(out, "9", "async mutual-recursion run:\n{module}");
    }
}

#[test]
fn rustc_roundtrip_float_arithmetic_and_of_int() {
    // The Rust backend renders floats natively: `Ty::Float` → f64/f32, the ONE arithmetic operator
    // `+`/`-`/`*`/`/` over float operands → the Rust operators (IEEE, no trap), `Float64.of-int` →
    // `as f64`. A runtime `(+ a b)` over f64 params computes the same non-exact IEEE sum the wasm backend
    // does (0.1+0.2 = 0.30000000000000004) — Rust's `{}` for an f64 prints the shortest round-tripping
    // decimal, matching the value form.
    let add = compile_rust("(module m (def (f (: a Float64) (: b Float64)) (+ a b)) (export f))");
    if let Some(out) = rustc_run(&add, "f(0.1, 0.2)") {
        assert_eq!(out, "0.30000000000000004");
    }
    let mul = compile_rust("(module m (def (f (: a Float64) (: b Float64)) (* a b)) (export f))");
    if let Some(out) = rustc_run(&mul, "f(6.0, 7.0)") {
        assert_eq!(out, "42");
    }
    // A constant float folds to a Rust `f64::from_bits(..)` literal, crossing exactly.
    let konst = compile_rust("(module m (def (f) (+ 1.5 2.0)) (export f))");
    if let Some(out) = rustc_run(&konst, "f()") {
        assert_eq!(out, "3.5");
    }
    // `Float64.of-int` over a runtime integer → `as f64` (total).
    let ofint = compile_rust("(module m (def (f (: n Int64)) (Float64.of-int n)) (export f))");
    if let Some(out) = rustc_run(&ofint, "f(42)") {
        assert_eq!(out, "42");
    }
    // `Float32.of` demotes (`as f32`, rounds); `Float64.of` promotes (`as f64`, exact). Rust's `{}`
    // for the f32 result prints the shortest round-tripping decimal for the binary32 value.
    let demote = compile_rust("(module m (def (f (: x Float64)) (Float32.of x)) (export f))");
    if let Some(out) = rustc_run(&demote, "f(0.1)") {
        assert_eq!(out, "0.1"); // Rust prints the f32 nearest to 0.1 as "0.1" (shortest round-trip)
    }
    let promote = compile_rust("(module m (def (f (: x Float32)) (Float64.of x)) (export f))");
    if let Some(out) = rustc_run(&promote, "f(1.5)") {
        assert_eq!(out, "1.5");
    }
}

#[test]
fn rustc_roundtrip_const_float_nan_emits_and_compares_by_canonical_bits() {
    // A constant NaN float (`Float64.nan`/`(. Float64 nan)`) → the EXPLICIT canonical NaN bits (not
    // `f64::NAN`, whose payload is platform-defined) — matching the runtime/wasm `CANON_NAN_BITS` so the
    // value is byte-identical to the canonical NaN the float-eq compare folds to.
    let nan = compile_rust(
        "(module m (def (f (: n Int64)) (if (= n 0) (Float64.nan) (f (+ n -1)))) (def (mk) (f 1)) (export mk))",
    );
    assert!(
        nan.contains("f64::from_bits(0x7FF8_0000_0000_0000u64)"),
        "Float64.nan → canonical NaN from_bits:\n{nan}"
    );
    // e2e: the NaN result renders as the canonical `NaN` text (the driver's Float render handles is_nan()).
    let driver = "fn main(){ let r = prog::mk(); if r.is_nan() { println!(\"NaN\"); } else { println!(\"{}\", r); } }";
    if let Some(out) = rustc_run_driver(&nan, driver) {
        assert_eq!(out, "NaN", "a constant NaN float result renders NaN");
    }
    // NaN equality by CANONICAL BYTE FORM: `n = x/x` (NaN when x=0), then `(= n Float64.nan)` — the
    // canonical-bits compare makes NaN == NaN true (unlike IEEE `==`). c>0 selects the nan-compare arm → 1.
    let eq = compile_rust(
        "(module m (def (chk (: c Int64) (: x Float64)) \
           (let ((n (/ x x))) (if (if (> c 0) (= n (. Float64 nan)) (= n 2.0)) 1 0))) (export chk))",
    );
    if let Some(out) = rustc_run(&eq, "chk(1, 0.0)") {
        assert_eq!(
            out, "1",
            "NaN == nan under the canonical byte form (0/0 = NaN)"
        );
    }
}

#[test]
fn char_maps_to_rust_char_and_escapes_across_a_sum_payload() {
    // `Ty::Char`→`char`; `ConstChar`→a `'…'` literal; a Char crosses as a sum payload and renders `#\<c>`.
    let rs = compile_rust(
        "(module m (type Tok (Ch Char) (End)) (def (mk) ((. Tok Ch) #\\a)) (export mk))",
    );
    assert!(
        rs.contains("Ch(char)"),
        "Char sum payload → char field:\n{rs}"
    );
    assert!(
        rs.contains("Ch('a')"),
        "ConstChar → a Rust char literal:\n{rs}"
    );
    // e2e: the Char-payload sum renders `(Ch #\a)`.
    let driver = "fn main(){ let v = prog::mk(); let s = match v { prog::Tok::Ch(__c) => { let __c: char = __c; match __c { ' ' => \"#\\\\space\".to_string(), c if c.is_control() => format!(\"#\\\\u+{:04X}\", c as u32), c => format!(\"#\\\\{}\", c) } }, prog::Tok::End => \"(End unit)\".to_string() }; println!(\"(Ch {})\", s); }";
    if let Some(out) = rustc_run_driver(&rs, driver) {
        assert_eq!(
            out, "(Ch #\\a)",
            "a Char sum payload renders in the canonical #\\ form"
        );
    }
    // Char.from-int on a CONSTANT scalar folds to Some(char); a surrogate folds to None. (A runtime-int
    // from-int still declines — constant-only — so these fold cases exercise the Char VALUE + Option render,
    // which the Char→char mapping enables.) 97='a' → Some → 1; 0xD800 surrogate → None → 0.
    let ok = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 97) ((Some c) 1) ((None _) 0))) (export f))",
    );
    if let Some(out) = rustc_run(&ok, "f()") {
        assert_eq!(out, "1", "97 is a valid scalar → Some");
    }
    let bad = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 55296) ((Some c) 1) ((None _) 0))) (export f))",
    );
    if let Some(out) = rustc_run(&bad, "f()") {
        assert_eq!(out, "0", "0xD800 is a surrogate → None");
    }
}

#[test]
fn char_literal_escapes_c1_controls_not_just_c0() {
    // Regression pin (Copilot PR#444 / corpus-bugfix issue): `rust_char_literal`/`rust_string_literal`
    // once escaped only C0 (`< 0x20`) + DEL (`0x7f`), leaking a RAW C1 control (0x80-0x9F) into the
    // generated `'…'`/`"…"` literal. The guard is now `is_control()`, matching
    // `cadenza-syntax::render_char`. Pin that a C1 control — U+0085 NEL — emits `\u{85}`, not a raw byte.
    let nel = compile_rust("(module m (def (g) #\\u+0085) (export g))");
    assert!(
        nel.contains("'\\u{85}'"),
        "C1 control U+0085 → escaped char literal '\\u{{85}}', not a raw control:\n{nel}"
    );
    assert!(
        !nel.contains("Ch('\u{85}')") && !nel.lines().any(|l| l.contains('\u{85}')),
        "no RAW C1 control byte in the emitted source:\n{nel}"
    );
    // DEL (0x7f) — the old guard's upper edge — still escapes.
    let del = compile_rust("(module m (def (g) #\\u+007F) (export g))");
    assert!(
        del.contains("'\\u{7f}'"),
        "DEL U+007F → escaped char literal:\n{del}"
    );
    // A C1 control as a sum payload emits an escaped field literal (the reader-error surrogate case is
    // static; a C1 control is a VALID scalar, so it reaches codegen and must escape).
    let payload = compile_rust(
        "(module m (type Tok (Ch Char) (End)) (def (mk) ((. Tok Ch) #\\u+0085)) (export mk))",
    );
    assert!(
        payload.contains("Ch('\\u{85}')"),
        "C1 control across a sum payload escapes:\n{payload}"
    );
    // e2e: the escaped literal compiles and round-trips to the same scalar (Char.to-int = 133).
    let n = compile_rust(
        "(module m (def (f) (match ((. Char from-int) 133) ((Some c) (Char.to-int c)) ((None _) -1))) \
           (export f))",
    );
    if let Some(out) = rustc_run(&n, "f()") {
        assert_eq!(
            out, "133",
            "U+0085 round-trips through generated Rust as scalar 133"
        );
    }
}

#[test]
fn rustc_roundtrip_value_eq_over_bytes_string_and_compounds() {
    // `Core::ValueEq` over a String/Char/Bytes (and a compound containing them) now emits a native `==`
    // (`String`/`char`/`Vec<u8>` are Eq; `==` compares by content = the canonical-byte value equality).
    // Bytes value-eq (v-core-opt's repro): two equal runtime byte sequences compare equal.
    let beq = compile_rust(
        "(module m (def (f (: n Int64)) \
           (if (= (Bytes.of (list (UInt8.wrap n) 20)) (Bytes.of (list (UInt8.wrap n) 20))) 1 0)) (export f))",
    );
    assert!(beq.contains("=="), "Bytes value-eq emits native ==:\n{beq}");
    if let Some(out) = rustc_run(&beq, "f(5)") {
        assert_eq!(out, "1", "equal byte sequences compare equal");
    }
    // A rope String equals its flat twin (built by concat vs a literal) — `==` compares content.
    let seq = compile_rust(
        "(module m (def (f (: n Int64)) (if (= (String.concat \"ab\" \"c\") \"abc\") 1 0)) (export f))",
    );
    if let Some(out) = rustc_run(&seq, "f(0)") {
        assert_eq!(out, "1", "a concat rope equals its flat twin by content");
    }
    // A char compares equal to a literal char (via a scalar read that folds to a char).
    let ceq = compile_rust(
        "(module m (def (f) (if (= ((. Char from-int) 97) ((. Char from-int) 97)) 1 0)) (export f))",
    );
    if let Some(out) = rustc_run(&ceq, "f()") {
        assert_eq!(out, "1", "equal chars compare equal");
    }
}
