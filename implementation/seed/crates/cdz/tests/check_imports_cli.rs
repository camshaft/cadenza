//! End-to-end tests for `cdz check` FOLLOWING a file's import closure — the diagnostics loop for a
//! multi-file package. `cdz check FILE` loads FILE plus the transitive closure of the files it
//! `(import …)`s (resolved as siblings in FILE's directory), links them into one program, and reports
//! diagnostics for the whole — a cross-file reference (an imported type/definition) resolves instead of
//! surfacing as "unbound name". A diagnostic that lands in an imported library is located at THAT
//! library's own `path:line:col` (via the package `link-map` demux). A file that imports nothing is
//! checked exactly as a standalone file (byte-identical to before).
//!
//! These drive the actual built binary over temp files — the integration counterpart to the in-crate
//! logic, proving the package-load + link + demux wiring end-to-end.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// A fresh temp directory unique to this test process + a discriminator, so parallel tests don't
/// collide. Cleaned by the OS temp reaper; each test writes its own files under it.
fn pkg_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-check-imports-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir temp pkg");
    dir
}

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write file");
    path.to_string_lossy().into_owned()
}

#[test]
fn a_check_follows_an_import_and_resolves_a_cross_file_name() {
    // `app` imports `kind-of-int` from `lib` and calls it; the cross-file reference resolves +
    // type-checks, so the check is CLEAN. Before this change `cdz check app.sexp` reported `import`
    // unmodeled + `kind-of-int` unbound. `lib` OWNS the `Ast` type and builds/matches it internally
    // (a module's own constructors are always usable); the entry only calls the exported function —
    // the realistic decode shape, and one that respects constructor visibility (CDZ0214).
    let dir = pkg_dir("clean");
    write(
        &dir,
        "lib.sexp",
        "(do (type Ast (Int Int64) (Name String)) \
           (def (kind-of-int (: v Int64)) \
             (match (Ast.Int v) (((. Ast Int) _) 1) (((. Ast Name) _) 2))) \
           (export kind-of-int))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (kind-of-int)) (def (main) (kind-of-int 5)) (export main))",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(
        ok,
        "clean cross-file check should succeed: {stdout}{stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "no diagnostics expected: {stdout}"
    );
}

#[test]
fn a_diagnostic_in_an_imported_library_is_located_in_that_library() {
    // The type error is in `lib`, not `app` — the check must point at `lib.sexp`, not the entry.
    let dir = pkg_dir("lib-error");
    let lib = write(
        &dir,
        "lib.sexp",
        "(do (def (helper (: x Int64)) (+ x true)) (export helper))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app]);
    assert!(!ok, "a type error must fail the check");
    assert!(
        stdout.contains(&lib) && stdout.contains("CDZ0203"),
        "the diagnostic should be located in lib.sexp with the type code: {stdout}"
    );
}

#[test]
fn a_transitive_import_error_is_reached_and_located() {
    // app → mid → base; the error is in `base`, two hops from the entry.
    let dir = pkg_dir("transitive");
    let base = write(
        &dir,
        "base.sexp",
        "(do (def (b (: x Int64)) (+ x false)) (export b))",
    );
    write(
        &dir,
        "mid.sexp",
        "(do (import \"base\" (b)) (def (m (: x Int64)) (b x)) (export m))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"mid\" (m)) (def (main) (m 3)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app]);
    assert!(!ok, "the transitive type error must fail the check");
    assert!(
        stdout.contains(&base),
        "the diagnostic should be located in base.sexp: {stdout}"
    );
}

#[test]
fn an_unresolved_import_reports_a_precise_diagnostic() {
    // An import naming no sibling file: the LINK path gives a precise "unknown package file" error,
    // not the generic "imports are not modeled here" a bare single-file compile falls back to.
    let dir = pkg_dir("unresolved");
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"nope\" (x)) (def (main) (x 1)) (export main))",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(!ok, "an unresolved import must fail the check");
    let out = format!("{stdout}{stderr}");
    assert!(
        out.contains("nope") && out.contains("unknown package file"),
        "an unresolved import should name the missing file: {out}"
    );
}

#[test]
fn a_package_json_diagnostic_names_its_file() {
    // In `--json`, a diagnostic that belongs to an imported file carries a `file` field so an agent
    // knows which file the `from`/`to` byte offsets index.
    let dir = pkg_dir("json");
    let lib = write(
        &dir,
        "lib.sexp",
        "(do (def (helper (: x Int64)) (+ x true)) (export helper))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app, "--json"]);
    assert!(!ok);
    assert!(
        stdout.contains("\"file\":") && stdout.contains(&lib),
        "the JSON diagnostic should carry the imported file's path: {stdout}"
    );
}

#[test]
fn a_file_with_no_imports_checks_as_a_standalone_file() {
    // The single-file path is unchanged: a self-contained program with a type error fails, located in
    // itself, with no package machinery engaged.
    let dir = pkg_dir("standalone");
    let solo = write(
        &dir,
        "solo.sexp",
        "(do (def (main) (+ 1 true)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &solo]);
    assert!(!ok);
    assert!(
        stdout.contains(&solo) && stdout.contains("CDZ0203"),
        "a standalone type error is located in the file itself: {stdout}"
    );
}
