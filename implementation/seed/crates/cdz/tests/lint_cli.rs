//! End-to-end tests for `cdz lint` — the structural anti-pattern linter.
//!
//! `lint` matches user-supplied lint rules (`--rule '(lint PATTERN "message" [severity])'`, repeatable,
//! and/or a `--rules FILE`) against a program's AST and reports each hit as `file:line:col: severity:
//! message` (or, under `--json`, a structured diagnostic array). The pattern-matching engine lives in
//! cadenza-syntax; what is pinned HERE is the CLI contract only `cdz` owns: a rule match renders the
//! diagnostic + the severity→exit-code discipline (an `error` match fails with exit 1; a `warning`-only
//! match still exits 0; a clean file exits 0), the `--json` diagnostic shape, and that a run with NO
//! explicit rule uses the built-in `idiomatic` catalog (the Tier-A pack) rather than erroring or being a
//! silent no-op. Drives the built binary.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_code, stdout, stderr). Exit CODE matters: lint distinguishes 0
/// (clean / warning-only) from 1 (an `error` match, a missing file, or no rules).
fn run(args: &[&str]) -> (i32, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Write `src` to a unique temp `.sexp` file and return its path.
fn temp_src(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("cdz-lint-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

/// A program containing a `(deprecated-op 1)` application a lint rule can target.
const PROG: &str = "(module m (def (f) (deprecated-op 1)) (export f))";
const RULE_ERR: &str = "(lint (deprecated-op ,@_) \"avoid deprecated-op\" error)";

#[test]
fn lint_reports_an_error_match_and_exits_nonzero() {
    let file = temp_src("err", PROG);
    let (code, out, err) = run(&["lint", &file, "--rule", RULE_ERR]);
    assert_eq!(code, 1, "an `error`-severity match fails (exit 1): {err}");
    assert!(
        out.contains("error:") && out.contains("avoid deprecated-op"),
        "the diagnostic renders `severity: message`: {out}"
    );
    // Locus prefix `file:line:col:`.
    let first = out.lines().find(|l| l.contains("error:")).unwrap_or("");
    assert!(
        first.starts_with(&file) && first[file.len()..].starts_with(':'),
        "the diagnostic carries a `file:line:col:` locus: {first}"
    );
}

#[test]
fn lint_a_warning_only_match_still_exits_zero() {
    // Severity discipline: only an `error` match fails the exit. A `warning` match is REPORTED but the
    // run still exits 0 (so `cdz lint --rule '(… warning)'` is advisory, not a CI-failing gate).
    let file = temp_src("warn", PROG);
    let (code, out, err) = run(&[
        "lint",
        &file,
        "--rule",
        "(lint (deprecated-op ,@_) \"soft\" warning)",
    ]);
    assert_eq!(code, 0, "a warning-only match still exits 0: {err}");
    assert!(
        out.contains("warning:") && out.contains("soft"),
        "the warning is still reported: {out}"
    );
}

#[test]
fn lint_a_clean_file_exits_zero_with_no_diagnostics() {
    // A rule that matches nothing: no diagnostics, exit 0.
    let file = temp_src("clean", PROG);
    let (code, out, _err) = run(&[
        "lint",
        &file,
        "--rule",
        "(lint (nonexistent-op ,@_) \"x\" error)",
    ]);
    assert_eq!(code, 0, "a no-match lint exits 0");
    assert!(
        !out.contains("error:") && !out.contains("warning:"),
        "no diagnostics on a clean file: {out}"
    );
}

#[test]
fn lint_json_emits_a_structured_diagnostic_array() {
    // `--json` emits `[{file?, line, col, severity, message, matched}]` — PARSED (a substring check would
    // pass for malformed JSON), asserting the diagnostic's fields incl the matched-subtree text.
    let file = temp_src("json", PROG);
    let (code, out, err) = run(&["lint", &file, "--rule", RULE_ERR, "--json"]);
    assert_eq!(code, 1, "an error match under --json still exits 1: {err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    let arr = v.as_array().expect("the diagnostics are a JSON array");
    assert_eq!(arr.len(), 1, "one diagnostic for the single match: {out}");
    let d = &arr[0];
    assert_eq!(
        d["severity"].as_str(),
        Some("error"),
        "severity is `error`: {out}"
    );
    assert_eq!(
        d["message"].as_str(),
        Some("avoid deprecated-op"),
        "message round-trips: {out}"
    );
    assert!(
        d["matched"]
            .as_str()
            .is_some_and(|m| m.contains("deprecated-op")),
        "`matched` carries the matched subtree text: {out}"
    );
    assert!(
        d["line"].is_number() && d["col"].is_number(),
        "line/col are numbers: {out}"
    );
}

#[test]
fn lint_with_no_rule_runs_the_builtin_idiomatic_catalog() {
    // `cdz lint FILE` with NO `--rule`/`--rules` runs the BUILT-IN `idiomatic` catalog (the Tier-A
    // pack) — the tool is useful out of the box (DESIGN-cadenza-lint §2), not a usage error. On a
    // program with no idiomatic issue (PROG uses `deprecated-op`, which no built-in lint targets) it
    // exits 0 with no diagnostics; a program WITH an idiomatic shape flags it (checked below).
    let file = temp_src("norule", PROG);
    let (code, out, _err) = run(&["lint", &file]);
    assert_eq!(
        code, 0,
        "no explicit rules runs the built-in catalog, not an error: {out}"
    );
    // A program with a built-in-lint shape (`if b true false` → idiomatic/if-bool) is flagged.
    let idiomatic = temp_src(
        "idiom",
        "(module m (def (f (: b Bool)) (if b true false)) (export f))",
    );
    let (icode, iout, _ierr) = run(&["lint", &idiomatic]);
    assert_eq!(icode, 0, "a warning-level lint does not fail the run");
    assert!(
        iout.contains("idiomatic") || iout.contains("if"),
        "the built-in if-bool lint fires by default: {iout}"
    );
    // `--deny` on the built-in lint promotes it to an error (exit non-zero).
    let (dcode, _dout, _derr) = run(&["lint", &idiomatic, "--deny", "idiomatic/if-bool"]);
    assert_ne!(dcode, 0, "--deny on a built-in lint fails the run");
}

#[test]
fn lint_on_a_missing_file_errors() {
    let (code, _out, err) = run(&["lint", "/no/such/file.sexp", "--rule", RULE_ERR]);
    assert_ne!(code, 0, "a missing file is an error");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
}
