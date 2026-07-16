//! End-to-end test that `cdz compile <single-file>` FOLLOWS the file's import closure — the same
//! resolution `cdz check`/`cdz test` do — so a lone source file that `(import …)`s a sibling compiles
//! as a package instead of dropping the import.
//!
//! REGRESSION (v-compiler-ml stress finding): a single-file `cdz compile app.cdz` compiled the file
//! ALONE, so the `(import …)` was an unmodeled module form AND a bare imported name fell back to a
//! BUILT-IN (e.g. `Ast` → the metaprog Ast) — flipping the program's meaning vs `cdz check app.cdz` and
//! yielding a misleading "variant not covered" cascade. Now `cdz compile` loads the closure (entry = the
//! file), resolving the import exactly as check does.

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

/// A two-file package: `lib.sexp` exports `bump`, `app.sexp` imports + uses it. Returns the dir.
fn temp_pkg(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("cdz-compile-closure-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(do (def (bump (: x Int64)) (+ x 1)) (export bump))\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.sexp"),
        "(do (import \"lib\" (bump)) (def (main (: x Int64)) (bump x)) (export main))\n",
    )
    .unwrap();
    dir
}

#[test]
fn compile_a_single_file_follows_its_import_closure() {
    // `cdz compile app.sexp` (the lone file, NO --entry) must resolve `(import "lib" …)` by loading the
    // closure — compiling the package, NOT dropping the import. It writes the entry's component.
    let dir = temp_pkg("follow");
    let app = dir.join("app.sexp");
    let (ok, _out, err) = run(&[
        "compile",
        app.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(
        ok,
        "single-file compile should follow the import and succeed, not drop it: {err}"
    );
    // The import must NOT surface as an unmodeled-module error, nor a spurious not-covered cascade.
    assert!(
        !err.contains("import` is a module form this compiler does not yet model"),
        "the import is followed, not reported as unmodeled: {err}"
    );
    assert!(
        dir.join("main.sexp.wasm").is_file() || dir.join("main.wasm").is_file(),
        "the entry's component is written: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_a_single_file_matches_check_on_the_same_file() {
    // The consistency the finding is about: `cdz compile` and `cdz check` on the SAME single file agree
    // — both follow the import, so both succeed (no meaning flip between the two tool paths).
    let dir = temp_pkg("agree");
    let app = dir.join("app.sexp");
    let (check_ok, _co, ce) = run(&["check", app.to_str().unwrap()]);
    let (compile_ok, _o, cce) = run(&[
        "compile",
        app.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(check_ok, "cdz check follows the import (clean): {ce}");
    assert!(
        compile_ok,
        "cdz compile agrees with check — both follow the import: {cce}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
