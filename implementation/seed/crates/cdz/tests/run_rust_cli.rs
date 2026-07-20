//! End-to-end tests for `cdz run-rust [FILE] [--call NAME]` — the RUST-backend runner the fuzzer's
//! rust-vs-wasm differential oracle shells out to.
//!
//! Contract (mirrors `cdz run-ml`, plus a DISTINCT `trap`/`error`): any run OUTCOME exits 0 with one
//! verdict line on stdout — `value <sexpr>` | `declined` | `trap <msg>` | `error <msg>`; the one non-zero
//! exit is a HARNESS read failure. The rendered value must match what `cdz run` (wasm) prints for the same
//! program, so the oracle compares like-for-like.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `cdz run-rust` with `program` on stdin; return (exit_ok, stdout, stderr).
fn run_stdin(program: &str) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .arg("run-rust")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-rust");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn run_rust_renders_a_scalar_value_and_exits_zero() {
    // A nullary main returning an int → `value <n>` (the same bare scalar cdz-run prints), exit 0.
    let (ok, out, err) = run_stdin("(do (def (main) (+ 20 22)) (export main))");
    assert!(ok, "a run outcome exits 0: {err}{out}");
    assert_eq!(out, "value 42", "scalar renders as the bare value: {out:?}");
}

#[test]
fn run_rust_an_explicit_dash_reads_stdin_not_a_file_named_dash() {
    // REGRESSION: `run-rust -` (the stdin MARKER the other pipe commands use) used to fall through to
    // `read_to_string("-")`, leaking the raw `cannot read -: No such file or directory (os error 2)` errno.
    // Now `-` reads stdin like a missing arg: a verdict + exit 0, NOT the errno leak.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-rust", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-rust -");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 42) (export main))")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "run-rust - exits 0 (a verdict)");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "value 42",
        "run-rust - reads the program from stdin"
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("cannot read -"),
        "must NOT leak the `cannot read -` errno — `-` is the stdin marker"
    );
}

#[test]
fn run_rust_renders_a_bool_value() {
    let (ok, out, _err) = run_stdin("(do (def (main) (< 3 5)) (export main))");
    assert!(ok);
    assert_eq!(out, "value true", "bool renders like cdz-run: {out:?}");
}

#[test]
fn run_rust_renders_a_tuple_the_same_bare_form_cdz_run_prints() {
    // The key differential case: a compound result must render byte-identically to cdz-run's inner form
    // (cdz-run wraps it `(: (tuple 3 true) (Tuple …))`; run-rust prints the bare `(tuple 3 true)` the
    // oracle compares against). So the bare `(tuple …)` must match exactly.
    let (ok, out, _err) = run_stdin("(do (def (main) (tuple 3 (< 1 2))) (export main))");
    assert!(ok);
    assert_eq!(
        out, "value (tuple 3 true)",
        "tuple renders bare like cdz-run: {out:?}"
    );
}

#[test]
fn run_rust_a_diverging_main_is_a_trap_not_an_error() {
    // REGRESSION (breaker + corpus-bugfix): a program whose `main` DIVERGES — a run-time provable trap like
    // `Option.expect(None, …)` — lowers to `pub fn main() -> ! { panic!(…) }`. The run-rust DRIVER used to
    // emit post-call render code after `prog::main()`, which is unreachable under `-> !`; rustc's
    // `-D warnings` elevated the unreachable-statement warning to a HARD ERROR, so run-rust reported the
    // `error` (miscompile) verdict for what is actually a TRAP — spuriously flagging EVERY diverging program
    // in the fuzzer's rust-vs-wasm differential. The driver now special-cases a `!`-return export (just CALL
    // it, no render), so it compiles, panics at run time, and maps to `trap` — matching wasm's clean
    // `trap unreachable`. Assert `trap …`, and specifically NOT the `error` verdict.
    let (ok, out, err) = run_stdin("(do (def (main) (Option.expect (None) \"m\")) (export main))");
    assert!(ok, "a run outcome exits 0: {err}{out}");
    assert!(
        out.starts_with("trap"),
        "a diverging main is a TRAP verdict (matching wasm), not `error`/`value`: {out:?}"
    );
    assert!(
        !out.starts_with("error"),
        "must NOT be the `error` (miscompile) verdict — a diverging main is a trap: {out:?}"
    );
}

#[test]
fn run_rust_on_a_constant_trapping_program_declines_not_errors() {
    // `(/ 1 0)` is a COMPILE-TIME trap (CDZ0304 constant divide-by-zero) — the front-end rejects it, so
    // the rust emit fails → `declined` (coverage/reject), NOT `error` (which is reserved for an emitted
    // `.rs` that fails rustc — a real miscompile). Exit 0 either way.
    let (ok, out, _err) = run_stdin("(do (def (main) (/ 1 0)) (export main))");
    assert!(ok);
    assert_eq!(
        out, "declined",
        "a front-end reject is a decline, not an error: {out:?}"
    );
}

#[test]
fn run_rust_bad_call_name_is_a_usage_error_not_an_error_verdict() {
    // A `--call` naming an export that doesn't exist is a USAGE mistake, NOT a rust-backend miscompile:
    // it must be a clean non-zero harness error, NOT the `error` verdict (which the fuzzer files as a
    // miscompile). Without the pre-rustc arity check it fell through to rustc as `error error[E0425]`.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-rust", "--call", "nope"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 5) (export main))")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(
        !out.status.success(),
        "a bad --call is a non-zero usage error"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.starts_with("error "),
        "must NOT be the `error` (miscompile) verdict — it's a usage error: {stdout:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no exported `nope`"),
        "names the missing export: {stderr}"
    );
}

#[test]
fn run_rust_arg_taking_export_is_a_usage_error_not_an_error_verdict() {
    // An arg-taking export invoked with the nullary driver (no `--arg` passthrough yet) is a USAGE limit,
    // NOT a miscompile — a clean non-zero error naming the arity, NOT the `error` verdict (which fell
    // through as rustc `error[E0061]` before the pre-rustc arity guard).
    let (ok, out, err) = run_stdin("(do (def (main (: n Int64)) (+ n 1)) (export main))");
    assert!(
        !ok,
        "an arg-taking export with no args is a non-zero usage error"
    );
    assert!(
        !out.starts_with("error "),
        "must NOT be the `error` (miscompile) verdict: {out:?}"
    );
    assert!(
        err.contains("takes 1 argument") && err.contains("NULLARY"),
        "explains the nullary-only limit: {err}"
    );
}

#[test]
fn run_rust_multiple_exports_requires_call_not_an_arbitrary_pick() {
    // With >1 exported fn, run-rust must NOT guess (splitting on the first `pub fn` could run the wrong
    // one — Copilot PR #547): a no-`--call` invocation is a clean non-zero usage error naming the exports,
    // and `--call main` then runs the chosen one.
    let (ok, out, err) =
        run_stdin("(do (def (main) 1) (def (other) 2) (export main) (export other))");
    assert!(
        !ok,
        "an ambiguous multi-export needs --call (non-zero usage error)"
    );
    assert!(
        !out.starts_with("error "),
        "must NOT be the `error` (miscompile) verdict — it's a usage error: {out:?}"
    );
    assert!(
        err.contains("exports 2 functions") && err.contains("--call"),
        "names the exports + points at --call: {err}"
    );
    // Selecting one runs it.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-rust", "--call", "main"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 1) (def (other) 2) (export main) (export other))")
        .unwrap();
    let sel = child.wait_with_output().expect("wait");
    assert!(sel.status.success());
    assert_eq!(
        String::from_utf8_lossy(&sel.stdout).trim(),
        "value 1",
        "--call main runs main"
    );
}

#[test]
fn run_rust_on_an_unreadable_file_exits_non_zero() {
    // The ONE non-zero exit: a harness READ failure (no verdict). Distinguishes "the compiler judged it"
    // from "the input couldn't be read".
    let exe = env!("CARGO_BIN_EXE_cdz");
    let dir = std::env::temp_dir().join(format!("cdz-runrust-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("nope.sexp");
    let out = Command::new(exe)
        .arg("run-rust")
        .arg(&missing)
        .output()
        .expect("spawn");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !out.status.success(),
        "an unreadable input is a non-zero harness exit"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot read"),
        "names the read failure: {stderr}"
    );
}
