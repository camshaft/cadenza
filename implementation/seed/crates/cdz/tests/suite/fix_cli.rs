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

#[test]
fn ml_non_exhaustive_match_carries_the_insert_arms_fix_and_it_applies_clean() {
    // Regression pin (v-diagnostics report): a non-exhaustive `match` (CDZ0210) carries its "add the
    // missing arm" InsertArms fix on the ML surface — it was previously DROPPED (a stale insert+ml
    // suppression at the fix-assembly layer, because the textedit render printed a spliced arm as a
    // standalone application, not `| pat => body`). With v-syntax's render_child fix + the suppression
    // removed, `cdz check --json` now emits the fix, its edit is a valid `\n  | D => trap(...)` ML arm,
    // and applying it makes the match exhaustive (clean re-check). ML is the primary user surface and
    // non-exhaustive-match is the flagship actionable fix, so this end-to-end pin guards a regression.
    let dir = temp_dir("ml-insert-arms");
    let f = dir.join("nx.cdz");
    std::fs::write(
        &f,
        "type C = | A | B | D\ndef f (c: C) -> Int64 = match c with | A => 1 | B => 2\n",
    )
    .unwrap();
    let (ok, out, err) = run(&["check", f.to_str().unwrap(), "--json"]);
    // A non-exhaustive match is a compile error, so `check` exits non-zero; the DIAGNOSTIC + its fix are
    // on stdout regardless.
    let _ = ok;
    let diag = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|d| d["code"] == "CDZ0210")
        .unwrap_or_else(|| panic!("expected a CDZ0210 diagnostic: out={out} err={err}"));
    let fix = &diag["fix"];
    assert!(
        !fix.is_null(),
        "the ML CDZ0210 carries an InsertArms fix (not dropped): {diag}"
    );
    assert_eq!(fix["kind"], "insert", "the fix is an insert: {fix}");
    // The edit text is a valid ML match arm (`| pat => body`), NOT a standalone application `D(trap(…))`.
    let edits = fix["edits"].as_array().expect("edits is an array");
    assert_eq!(edits.len(), 1, "one insert edit: {fix}");
    let text = edits[0]["text"].as_str().expect("edit text");
    assert!(
        text.contains("| D =>"),
        "the inserted arm is ML arm-syntax `| D => …`, not an application: {text:?}"
    );

    // Apply the edit and re-check: the match is now exhaustive (no CDZ0210) — proving the fix is valid,
    // reparseable ML, not just present.
    let src = std::fs::read_to_string(&f).unwrap();
    let (from, to) = (
        edits[0]["from"].as_u64().unwrap() as usize,
        edits[0]["to"].as_u64().unwrap() as usize,
    );
    let fixed = format!("{}{}{}", &src[..from], text, &src[to..]);
    let ff = dir.join("nx_fixed.cdz");
    std::fs::write(&ff, &fixed).unwrap();
    let (fok, _fout, ferr) = run(&["check", ff.to_str().unwrap()]);
    assert!(
        fok,
        "applying the InsertArms fix yields exhaustive, clean-checking ML: {ferr}\n{fixed}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_preview_modes_diff_and_dry_run_do_not_mutate_the_file() {
    // PREVIEW SAFETY: `cdz fix --diff` and `--dry-run` must be READ-ONLY — a user runs them to SEE what would
    // change, so silently editing the file would be a serious surprise (and defeats the preview). The suite
    // pins the --diff OUTPUT but never that the source stays byte-identical afterward; pin it here. A file
    // with a fixable diagnostic (an unused def → `_`-prefix): capture its bytes, run --diff then --dry-run,
    // and assert the bytes are UNCHANGED each time. (A final plain `fix` then DOES mutate — the contrast that
    // proves the preview modes are the read-only ones, not that nothing was fixable.)
    let dir = temp_dir("preview-nomutate");
    let f = dir.join("p.cdz");
    let original = "def unused_thing() -> Int64 = 42\ndef main() -> Int64 = 1\nexport { main }\n";
    std::fs::write(&f, original).unwrap();

    let (dok, dout, derr) = run(&["fix", f.to_str().unwrap(), "--diff"]);
    assert!(dok, "cdz fix --diff succeeds: {derr}");
    assert!(
        dout.contains("_unused_thing"),
        "--diff previews the intended `_`-prefix rename: {dout}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "`--diff` is READ-ONLY — it must not mutate the source"
    );

    let (dryok, _dryout, dryerr) = run(&["fix", f.to_str().unwrap(), "--dry-run"]);
    assert!(dryok, "cdz fix --dry-run succeeds: {dryerr}");
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "`--dry-run` is READ-ONLY — it must not mutate the source"
    );

    // Contrast: a plain `fix` (no preview flag) DOES write — proving the file was genuinely fixable and the
    // preview modes' no-op was a deliberate read-only contract, not an empty fix set.
    let (aok, _ao, aerr) = run(&["fix", f.to_str().unwrap()]);
    assert!(aok, "plain cdz fix applies + succeeds: {aerr}");
    assert_ne!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "a plain `fix` DOES mutate (the file was fixable — the preview no-op was the read-only contract)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_on_a_fault_free_file_is_a_no_op_and_leaves_it_byte_identical() {
    // DATA-SAFETY floor: `cdz fix` WRITES to the user's source, so on a file with NO faults it must be a
    // strict no-op — never reformat, re-canonicalize, or report a phantom fix. Pin: a clean program's bytes
    // are UNCHANGED after a plain `fix`, exit 0, `--json` is `[]`. The preview-safety test proves the preview
    // modes don't mutate a FIXABLE file; this proves plain `fix` doesn't mutate a FAULT-FREE file — the
    // complementary safety direction. Both surfaces (a stray reformat could be surface-specific).
    for (name, src) in [
        (
            "clean.sexp",
            "(module m (def (f (: n Int64)) (+ n 1)) (def (main) (f 5)) (export main))",
        ),
        (
            "clean.cdz",
            "module m {\n  def f(n: Int64) = n + 1\n\n  def main() = f(5)\n\n  export { main }\n}\n",
        ),
    ] {
        let dir = temp_dir(&format!("noop-{name}"));
        let f = dir.join(name);
        std::fs::write(&f, src).unwrap();

        let (ok, _out, err) = run(&["fix", f.to_str().unwrap()]);
        assert!(ok, "cdz fix on a clean {name} succeeds (exit 0): {err}");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            src,
            "a fault-free {name} is left BYTE-IDENTICAL by `fix` (no spurious reformat/rewrite)"
        );

        // `--json` on a clean file is an empty array — zero fixes applied, not a phantom.
        let (jok, jout, jerr) = run(&["fix", f.to_str().unwrap(), "--json"]);
        assert!(jok, "cdz fix --json on a clean {name} succeeds: {jerr}");
        assert_eq!(
            jout.trim(),
            "[]",
            "no fixes on a fault-free {name} → `[]`, not a phantom fix: {jout}"
        );
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            src,
            "the --json run also leaves the clean {name} byte-identical"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
