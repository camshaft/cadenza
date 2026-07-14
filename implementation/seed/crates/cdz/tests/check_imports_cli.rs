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
fn a_comment_on_an_import_is_seen_through_by_the_link_scan() {
    // A `//`/`///` comment on an `(import …)` reifies (in the ML reader) to `(comment "…" (import …))`.
    // The link scan reads imports off the RAW arena before `Db::load`'s comment-strip, so an un-peeled
    // wrapper would leave the import unrecognized → spliced as an unmodeled top-level form → "`import`
    // … not modeled" + the imported names unbound. `link_inputs` now peels comments first, so a
    // documented import resolves exactly as a bare one. (ML surface — `.cdz` — is where the reader
    // produces the wrapper.)
    let dir = pkg_dir("comment-import");
    write(
        &dir,
        "lib.cdz",
        "type Ty =\n  | Var(Int64)\n  | Con(String)\n\
         def kind(t: Ty) -> Int64 = match t with\n  | Ty.Var(n) => n\n  | Ty.Con(_) => 0\n\
         export { Ty.*, kind }\n",
    );
    let app = write(
        &dir,
        "app.cdz",
        "/// bring in the type + its reader\nimport { Ty, kind } from \"lib\"\n\
         def main() -> Int64 = kind(Ty.Var(7))\nexport { main }\n",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(
        ok && stdout.trim().is_empty(),
        "a documented import must resolve like a bare one: {stdout}{stderr}"
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

#[test]
fn an_ml_file_that_does_not_parse_fails_the_check_even_with_no_semantic_fault() {
    // A CLEAN TRUNCATION — an unclosed `(` — that the ML reader RECOVERS from by dropping the broken
    // subtree, leaving a well-formed but incomplete arena that carries NO downstream semantic fault. The
    // parse error is printed to stderr, but `check` used to exit 0 (its exit keyed only on the SEMANTIC
    // fault set), reporting a file that does not parse as clean — an editor/CI then treats a broken file
    // as passing. A parse error IS an error-severity fault; the check must FAIL.
    let dir = pkg_dir("ml-unclosed");
    let bad = write(&dir, "bad.cdz", "def main() = (1 + 2\n");
    let (ok, _stdout, stderr) = run(&["check", &bad]);
    assert!(!ok, "an unparseable ML file must fail the check");
    assert!(
        stderr.contains("expected `)`"),
        "the parse error is still surfaced: {stderr}"
    );
}

#[test]
fn a_recovered_error_placeholder_does_not_cascade_into_an_unbound_name() {
    // `if … then …` with no `else` leaves an `<error>` PLACEHOLDER node where the else-branch belongs.
    // In expression position that placeholder reduces to a bare name `<error>`, which the checker reported
    // as `unbound name `<error>`` (CDZ0101) — a spurious fault naming a token the user never wrote, piled
    // on top of the real "expected `else`" parse error. `<error>` is UNLEXABLE on the ML surface, so a
    // diagnostic naming it is ALWAYS the placeholder; it is suppressed (the parse error already says the
    // fix). The check still FAILS (the parse error), and the noise line is gone.
    let dir = pkg_dir("ml-if-no-else");
    let bad = write(&dir, "bad.cdz", "def main() = if true then 1\n");
    let (ok, stdout, stderr) = run(&["check", &bad]);
    assert!(!ok, "a missing `else` must fail the check");
    assert!(
        stderr.contains("expected `else`"),
        "the real parse error is surfaced: {stderr}"
    );
    assert!(
        !stdout.contains("`<error>`") && !stderr.contains("`<error>`"),
        "the `<error>`-placeholder cascade must be suppressed: {stdout}{stderr}"
    );
}

#[test]
fn a_legit_error_symbol_in_sexpr_is_not_suppressed_as_a_placeholder() {
    // The `<error>`-cascade suppression is GATED on the file having actually had a parse error. In s-expr
    // `<error>` is a LEGAL symbol (the reader hard-errors on a malformed program, so a well-formed one
    // that names `<error>` had zero parse errors) — so a real `unbound name `<error>`` there is NOT a
    // recovery placeholder and must still be reported.
    let dir = pkg_dir("sexpr-error-name");
    let solo = write(
        &dir,
        "solo.sexp",
        "(module m (def (main) <error>) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &solo]);
    assert!(!ok, "a genuinely unbound `<error>` name still fails");
    assert!(
        stdout.contains("unbound name `<error>`"),
        "a real `<error>` name in a parse-clean file is reported, not suppressed: {stdout}"
    );
}
