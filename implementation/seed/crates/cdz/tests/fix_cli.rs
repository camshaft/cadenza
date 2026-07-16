//! End-to-end tests for `cdz fix` — apply verified quick-fixes and write the repaired program back.
//!
//! Focus: the fix must land on the RIGHT node. A regression (found by v-lsp) had `cdz fix` mis-anchor a
//! quick-fix on a MULTI-form s-expr program — the per-iteration span rebuild (`reparse_spans`) did not
//! canonicalize the s-expr arm, so the canonical fix-node id indexed a NEIGHBOUR's span and the edit
//! rewrote the wrong node (a param's TYPE, or the whole param list — DESTROYING the function). These
//! drive the built binary over a temp file and assert the edit renames the intended node.

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

/// A unique temp dir for one test.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-fix-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn fix_a_multiform_sexpr_renames_the_param_not_the_type() {
    // REGRESSION (found + verified by v-lsp): a MULTI-form s-expr program where the 2nd def has an unused
    // param. The CDZ0306 fix must rename the PARAM (`(: _y Int64)`), NOT rewrite the type
    // (`(: y _y)`) — the mis-anchoring bug. Applying the fix must yield a program that re-checks CLEAN.
    let dir = temp_dir("multiform-typed");
    let f = dir.join("mf.sexp");
    std::fs::write(
        &f,
        "(def (a (: x Int64)) x)\n(def (b (: y Int64)) 5)\n(export a b)\n",
    )
    .unwrap();
    // `--diff` shows the intended edit: the param binder, not its type.
    let (ok, out, err) = run(&["fix", f.to_str().unwrap(), "--diff"]);
    assert!(ok, "cdz fix --diff failed: {err}");
    assert!(
        out.contains("(: _y Int64)"),
        "the fix renames the PARAM to `_y`, keeping its type: {out}"
    );
    assert!(
        !out.contains("(: y _y)"),
        "the fix must NOT rewrite the type to `_y`: {out}"
    );
    // Applying it in place yields a program that re-checks clean (a correct repair, not a destroyed one).
    let (aok, _ao, aerr) = run(&["fix", f.to_str().unwrap()]);
    assert!(aok, "applying the fix failed: {aerr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(def (b (: _y Int64)) 5)"),
        "the repaired 2nd def renames the param: {repaired}"
    );
    let (cok, _co, _ce) = run(&["check", f.to_str().unwrap()]);
    assert!(cok, "the repaired program re-checks clean: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_a_multiform_sexpr_untyped_param_does_not_destroy_the_function() {
    // The worse shape: an UNTYPED param `(def (b y) 5)`. The mis-anchoring rewrote the whole `(b y)`
    // param list to `_y` → `(def _y 5)`, DESTROYING the function. The fix must rename just the param
    // binder → `(def (b _y) 5)`.
    let dir = temp_dir("multiform-untyped");
    let f = dir.join("mf.sexp");
    std::fs::write(&f, "(def (a (: x Int64)) x)\n(def (b y) 5)\n(export a b)\n").unwrap();
    let (ok, out, err) = run(&["fix", f.to_str().unwrap(), "--diff"]);
    assert!(ok, "cdz fix --diff failed: {err}");
    assert!(
        out.contains("(def (b _y) 5)"),
        "the fix renames the param binder, not the whole param list: {out}"
    );
    assert!(
        !out.contains("(def _y 5)"),
        "the function must NOT be destroyed: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_a_single_form_sexpr_still_renames_the_param() {
    // A LONE s-expr form was already correct (identity canon map) — assert the shared canonicalize+remap
    // did not regress it.
    let dir = temp_dir("single");
    let f = dir.join("one.sexp");
    std::fs::write(&f, "(def (b (: y Int64)) 5)\n(export b)\n").unwrap();
    let (ok, out, err) = run(&["fix", f.to_str().unwrap(), "--diff"]);
    assert!(ok, "cdz fix --diff failed: {err}");
    assert!(
        out.contains("(: _y Int64)"),
        "single-form s-expr still renames the param: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
