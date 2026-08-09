//! End-to-end tests for `cdz calc` — the calculator REPL FOLDED into the unified `cdz` binary (from
//! the `cdz-calc` lib). Completes the one-binary story for the calculator: `cdz calc` and the standalone
//! `cdz-calc` are the same code (`cdz_calc::cli`), so a single `cdz` on the PATH also gives the REPL.
//! The engine + the cli-layer are tested in the cdz-calc crate; these pin the MOUNTED `cdz calc` path
//! (arg forwarding + the `cdz:` prog name in diagnostics). Drives the built `cdz` binary.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn cdz_calc_once_evaluates_an_expression() {
    let (ok, out, err) = run(&["calc", "--once", "2 + 3"]);
    assert!(ok, "cdz calc --once failed: {err}");
    assert!(out.trim().ends_with("5"), "2 + 3 = 5: {out}");
}

#[test]
fn cdz_repl_is_an_alias_for_calc() {
    // `cdz repl` is a discoverability alias for `cdz calc` (which its own docs call "the REPL over the
    // real language") — the near-universal `repl` verb should land on the language REPL, not error as an
    // unknown subcommand. It mounts the SAME calc path: same eval, same flags.
    let (ok, out, err) = run(&["repl", "--once", "2 + 3"]);
    assert!(
        ok,
        "`cdz repl` is a valid alias of `cdz calc`, not an unknown subcommand: {err}"
    );
    assert!(
        out.trim().ends_with("5"),
        "the alias evaluates like calc: {out}"
    );
    // Exactness (a calc behavior) carries through the alias — confirms it's the same engine, not a stub.
    let (eok, eout, _e) = run(&["repl", "--once", "1/3 + 1/3 + 1/3"]);
    assert!(
        eok && eout.trim().ends_with("1"),
        "exact mode via the alias: {eout}"
    );
}

#[test]
fn cdz_calc_once_exact_mode_keeps_a_fraction_exact() {
    // Exact mode is on by default (the mounted path forwards `--no-exact` correctly): 1/3×3 = 1 exactly.
    let (ok, out, err) = run(&["calc", "--once", "1/3 + 1/3 + 1/3"]);
    assert!(ok, "exact --once failed: {err}");
    assert!(out.trim().ends_with("1"), "exact 1/3×3 = 1: {out}");
}

#[test]
fn cdz_calc_once_plain_prints_the_bare_value() {
    let (ok, out, err) = run(&["calc", "--once", "7 * 6", "--plain"]);
    assert!(ok, "--plain --once failed: {err}");
    assert_eq!(out.trim(), "42", "plain bare value: {out}");
}

#[test]
fn cdz_calc_once_error_names_cdz_not_cdz_calc() {
    // A calc error under `cdz calc` names the tool the user typed — `cdz:`, not `cdz-calc:` — because
    // the mount passes PROG through the embeddable `cli::run`. Exit non-zero.
    let (ok, _out, err) = run(&["calc", "--once", "definitely_unbound_name_zzz"]);
    assert!(!ok, "an unbound name should fail");
    assert!(
        err.contains("cdz:") && !err.contains("cdz-calc:"),
        "error names `cdz` (the mounted tool), not `cdz-calc`: {err}"
    );
}

#[test]
fn cdz_calc_sexpr_surface_evaluates() {
    let (ok, out, err) = run(&["calc", "--once", "(+ 2 3)", "--sexpr", "--plain"]);
    assert!(ok, "--sexpr --once failed: {err}");
    assert_eq!(out.trim(), "5", "(+ 2 3) in s-expr = 5: {out}");
}

#[test]
fn cdz_calc_once_accepts_a_leading_minus_expression() {
    // A leading-minus expression (`-5`) must be taken as the `--once` VALUE, not parsed as an unknown
    // flag (`allow_hyphen_values`). Reported by v-quantity: `cdz calc --once "-5"` previously errored with
    // "try --help". Both a bare negation and a later flag after the value now parse.
    let (ok, out, err) = run(&["calc", "--once", "-5", "--plain"]);
    assert!(
        ok,
        "a leading-minus --once expression should evaluate: {err}"
    );
    assert_eq!(out.trim(), "-5", "`-5` evaluates to -5: {out}");
}

#[test]
fn cdz_calc_once_accepts_a_leading_minus_subtraction() {
    // A leading-minus in a larger expression (`-3 * 4`) is also taken as the value, not a flag.
    let (ok, out, err) = run(&["calc", "--once", "-3 * 4", "--plain"]);
    assert!(
        ok,
        "a leading-minus arithmetic --once should evaluate: {err}"
    );
    assert_eq!(out.trim(), "-12", "`-3 * 4` = -12: {out}");
}

#[test]
fn cdz_calc_once_plain_renders_a_quantity_in_its_display_form() {
    // `--plain` on a QUANTITY shows the human form (`3000 meter`), not the raw constructor
    // (`Qty.of(3000, Unit.base(#"meter"))`) — the plain path uses the display render. (Regression
    // surfaced probing negative quantities; the sign is incidental — plain mode leaked the ctor for any
    // quantity.)
    let (ok, out, err) = run(&["calc", "--once", "3 kilometer", "--plain"]);
    assert!(ok, "plain quantity --once failed: {err}");
    assert_eq!(
        out.trim(),
        "3000 meter",
        "plain quantity is human-form: {out}"
    );
    // And a NEGATIVE quantity (the two fixes together): leading-minus parses AND renders in display form.
    let (nok, nout, nerr) = run(&["calc", "--once", "-3 kilometer", "--plain"]);
    assert!(nok, "plain negative quantity --once failed: {nerr}");
    assert_eq!(
        nout.trim(),
        "-3000 meter",
        "plain negative quantity: {nout}"
    );
}
