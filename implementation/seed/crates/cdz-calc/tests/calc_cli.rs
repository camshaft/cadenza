//! End-to-end tests for the `cdz-calc` command surface — now the embeddable `cdz_calc::cli` (an
//! `Args` group + `run`) that the standalone `cdz-calc` bin shims over and the unified `cdz` binary
//! will mount as `cdz calc`. The engine (`Calculator`/`Eval`) is unit-tested in the lib; these pin the
//! CLI LAYER: `--once` one-shot evaluation, `--plain`/`--no-exact`/`--sexpr` flags, and the diagnostic
//! prefix (a trap/error names the invoking tool). Drives the built `cdz-calc` binary. There was no
//! integration test over the binary before this — the refactor that lifted the CLI into the lib now has
//! a behavior-preserving witness.

use std::process::Command;

/// Run `cdz-calc <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz-calc");
    let out = Command::new(exe)
        .args(args)
        .output()
        .expect("spawn cdz-calc");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn once_evaluates_an_expression_and_prints_the_value() {
    // `--once EXPR` compiles + runs one expression and prints its value (the launcher/script hook).
    let (ok, out, err) = run(&["--once", "2 + 3"]);
    assert!(ok, "cdz-calc --once failed: {err}");
    assert!(out.trim().ends_with("5"), "2 + 3 evaluates to 5: {out}");
}

#[test]
fn once_exact_mode_keeps_a_fraction_exact() {
    // Exact mode is ON by default: `1/3 + 1/3 + 1/3` is exact rational arithmetic → exactly 1, not a
    // floating drift. Pins that the CLI defaults `exact` on (it passes `!no_exact` to the engine).
    let (ok, out, err) = run(&["--once", "1/3 + 1/3 + 1/3"]);
    assert!(ok, "exact --once failed: {err}");
    assert!(out.trim().ends_with("1"), "exact 1/3×3 = 1: {out}");
}

#[test]
fn once_plain_strips_the_type_suffix() {
    // `--plain` prints the bare value (no `: Type` suffix) — the launcher-facing shape.
    let (ok, out, err) = run(&["--once", "2 + 3", "--plain"]);
    assert!(ok, "--plain --once failed: {err}");
    assert_eq!(out.trim(), "5", "plain output is the bare value: {out}");
}

#[test]
fn once_on_an_error_names_the_tool_and_exits_nonzero() {
    // An unbound name is an error: the CLI prints `cdz-calc: error: …` on stderr (the prog name threads
    // through the embeddable `run`) and exits non-zero.
    let (ok, _out, err) = run(&["--once", "definitely_unbound_name_zzz"]);
    assert!(!ok, "an unbound name should fail");
    assert!(
        err.contains("cdz-calc:") && err.contains("error"),
        "error names the tool `cdz-calc`: {err}"
    );
}

#[test]
fn sexpr_surface_evaluates_s_expressions() {
    // `--sexpr` reads/renders in the s-expression surface, so an s-expr expression evaluates.
    let (ok, out, err) = run(&["--once", "(+ 2 3)", "--sexpr", "--plain"]);
    assert!(ok, "--sexpr --once failed: {err}");
    assert_eq!(out.trim(), "5", "(+ 2 3) in s-expr surface = 5: {out}");
}
