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
