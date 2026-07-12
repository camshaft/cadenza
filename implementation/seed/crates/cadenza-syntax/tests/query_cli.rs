//! End-to-end tests for the `cdz-syntax query` / `rewrite` codemod subcommands.
//!
//! These drive the actual built binary over stdin/stdout — the integration counterpart to the
//! `query` module's unit tests, proving the CLI wiring (arg parsing, format resolution, the
//! validated-transaction rewrite, exit codes) end-to-end.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `cdz-syntax <args…>` feeding `stdin`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str], stdin: &str) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz-syntax");
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz-syntax");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn query_reports_a_match_with_span_and_binding() {
    let (ok, stdout, _) = run(&["query", "(+ ,x 0)", "--from", "ml"], "f(a + 0, b * 1)");
    assert!(ok);
    assert!(stdout.contains("(+ a 0)"), "matched form: {stdout}");
    assert!(stdout.contains("$x = a"), "binding reported: {stdout}");
    assert!(stdout.contains("byte "), "span reported: {stdout}");
}

#[test]
fn query_count_prints_the_number() {
    let (ok, stdout, _) = run(
        &["query", "(+ ,e 0)", "--from", "ml", "--count"],
        "g(x + 0) + (y + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn query_no_match_is_empty_and_succeeds() {
    let (ok, stdout, _) = run(&["query", "(* ,x ,y)", "--from", "ml"], "1 + 2");
    assert!(ok);
    assert!(stdout.trim().is_empty(), "no output: {stdout:?}");
}

#[test]
fn rewrite_additive_identity_ml_to_ml() {
    let (ok, stdout, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "ml"],
        "f(a + 0, b + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "f(a, b)");
    assert!(stderr.contains("rewrote 2 site(s)"), "count on stderr: {stderr}");
}

#[test]
fn rewrite_splice_template_wraps_a_call() {
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(risky ,@args)",
            "(log (risky ,@args))",
            "--from",
            "sexpr",
            "--to",
            "sexpr",
        ],
        "(risky a b)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(log (risky a b))");
}

#[test]
fn rewrite_crosses_surfaces_ml_to_sexpr() {
    let (ok, stdout, _) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "sexpr"],
        "compute(x + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(compute x)");
}

#[test]
fn query_over_broken_input_still_works_and_warns() {
    // The recovering parser yields a usable tree even here; the query runs and the CLI warns on
    // stderr about the recoverable parse error. (Exercises the parser-recovery + codemod combo.)
    let (ok, stdout, stderr) = run(&["query", "(f ,@args)", "--from", "ml"], "f(a, @, c)");
    assert!(ok, "query still succeeds over recovered input");
    assert!(stdout.contains("(f a"), "match reported: {stdout}");
    assert!(
        stderr.contains("parse warning"),
        "recovery warning on stderr: {stderr}"
    );
}

#[test]
fn bad_pattern_two_splices_is_rejected() {
    let (ok, _, stderr) = run(&["query", "(f ,@a ,@b)", "--from", "ml"], "x");
    assert!(!ok, "exit failure");
    assert!(stderr.contains("at most one"), "reason: {stderr}");
}

#[test]
fn rewrite_no_match_is_a_no_op_that_reprints() {
    let (ok, stdout, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "ml"],
        "a - 1",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "a - 1");
    assert!(stderr.contains("rewrote 0 site(s)"), "0 sites: {stderr}");
}
