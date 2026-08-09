//! End-to-end tests for `cdz add PATH` — the manifest dependency-editor (the `cargo add` analogue).
//!
//! `cdz add ../lib` appends a PATH dependency to the project's `Project.cdz` `def deps` (creating the line
//! when absent), so a user doesn't hand-edit the manifest. These drive the built binary over real project
//! dirs and assert: adding to a deps-less manifest creates the line; a second add appends (not replaces);
//! a duplicate is an idempotent no-op; an unresolvable path warns but is still added; and the edited
//! manifest stays valid (re-parses — proven by `cdz tree` seeing the new dep) and fmt-clean.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A workspace root with an `app` project (given `deps_line`, e.g. "" or `def deps = ["../x"]\n`) plus
/// sibling `lib1`/`lib2` projects to depend on. Returns the root.
fn workspace(tag: &str, deps_line: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("cdz-add-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for lib in ["lib1", "lib2"] {
        std::fs::create_dir_all(root.join(lib)).unwrap();
        std::fs::write(
            root.join(lib).join("Project.cdz"),
            format!("def name = \"{lib}\"\ndef entry = \"l.sexp\"\n"),
        )
        .unwrap();
        std::fs::write(root.join(lib).join("l.sexp"), "(do (def (l) 0) (export l))").unwrap();
    }
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(
        root.join("app/Project.cdz"),
        format!("def name = \"app\"\ndef entry = \"main.sexp\"\n{deps_line}"),
    )
    .unwrap();
    std::fs::write(
        root.join("app/main.sexp"),
        "(do (def (main) 0) (export main))",
    )
    .unwrap();
    root
}

#[test]
fn add_creates_the_deps_line_when_absent_and_the_result_parses() {
    let root = workspace("create", "");
    let app = root.join("app");
    let (ok, _o, err) = run(&["add", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "add should succeed: {err}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("def deps = [\"../lib1\"]"),
        "a `def deps` line is created with the path: {manifest}"
    );
    // The edited manifest RE-PARSES as a valid project — `cdz tree` sees the new dep (proves validity,
    // not just text; `cdz add` re-parses internally too and would have reverted a broken edit).
    let (tok, tout, _e) = run(&["tree", app.to_str().unwrap()]);
    assert!(
        tok && tout.contains("lib1 (../lib1)"),
        "tree sees the added dep + the manifest re-parses: {tout}"
    );
    // The `def deps` line `cdz add` writes is the canonical manifest form (`def deps = ["…"]`), matching
    // how `cdz new`/`metadata` render it — so a subsequent hand-edit or fmt sees no surprise.
    assert!(
        manifest
            .lines()
            .any(|l| l.trim() == "def deps = [\"../lib1\"]"),
        "the deps line is a clean canonical `def deps = [...]`: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_appends_to_an_existing_deps_list() {
    // A second add extends the list, preserving the first entry (not a replace).
    let root = workspace("append", "def deps = [\"../lib1\"]\n");
    let app = root.join("app");
    let (ok, _o, err) = run(&["add", "../lib2", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "append should succeed: {err}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("\"../lib1\"") && manifest.contains("\"../lib2\""),
        "both deps present after append: {manifest}"
    );
    let (_t, tout, _e) = run(&["tree", app.to_str().unwrap()]);
    assert!(
        tout.contains("lib1 (../lib1)") && tout.contains("lib2 (../lib2)"),
        "tree sees both deps: {tout}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_is_idempotent_for_an_already_declared_dep() {
    // Adding a path already in `def deps` is a no-op notice, not a duplicate entry.
    let root = workspace("dup", "def deps = [\"../lib1\"]\n");
    let app = root.join("app");
    let (ok, _o, err) = run(&["add", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "a duplicate add still exits 0: {err}");
    assert!(
        err.contains("already declares"),
        "it reports the no-op: {err}"
    );
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert_eq!(
        manifest.matches("../lib1").count(),
        1,
        "the dep appears exactly once (no duplicate): {manifest}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_warns_but_adds_an_unresolvable_path() {
    // A path with no `Project.cdz` yet is a warning, not a refusal — the dep may not exist yet (same
    // tolerance as `cdz tree`'s `*unresolved*`).
    let root = workspace("unresolved", "");
    let app = root.join("app");
    let (ok, _o, err) = run(&["add", "../ghost", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "an unresolvable path is still added (exit 0): {err}");
    assert!(
        err.contains("warning") && err.contains("../ghost"),
        "it warns: {err}"
    );
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("\"../ghost\""),
        "the path is added anyway: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_refuses_a_self_dependency() {
    // `cdz add .` (or a roundabout `../app` pointing back at the project) is a SELF-dependency — a project
    // cannot depend on itself (a `def deps` entry names ANOTHER project). It must be refused (non-zero) with
    // a clear message, and NOT written to the manifest. Both the literal `.` and the roundabout `../app`
    // form (which canonicalizes to the same dir) are caught.
    let root = workspace("selfdep", "");
    let app = root.join("app");

    let (ok, _o, err) = run(&["add", ".", "--manifest", app.to_str().unwrap()]);
    assert!(!ok, "`cdz add .` (self-dep) must be refused: {err}");
    assert!(
        err.contains("own directory") && err.contains("cannot depend on itself"),
        "the refusal names the self-dependency: {err}"
    );

    let (ok2, _o2, err2) = run(&["add", "../app", "--manifest", app.to_str().unwrap()]);
    assert!(
        !ok2,
        "a roundabout self-dep (`../app`) must also be refused: {err2}"
    );

    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        !manifest.contains("def deps"),
        "no `def deps` was written for the refused self-dependency: {manifest}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn add_refuses_to_edit_a_scalar_deps_manifest_leaving_it_intact() {
    // `def deps` READS as accept-both — a bare scalar string coerces to a one-element list (so `metadata`/
    // `tree` see `["../foo"]`). But `cdz add` EDITS the manifest TEXT by splicing into the `[...]` list,
    // which a scalar has no brackets for. Rather than corrupt the manifest (e.g. produce `def deps =
    // "../foo""../bar"`), `add` must REFUSE cleanly — exit non-zero, name the malformed clause, and leave
    // the manifest byte-for-byte unchanged. This is the data-loss-safety floor of the text-editor: it
    // never writes an edit it can't do cleanly.
    let root = workspace("scalar", "def deps = \"../foo\"\n");
    let app = root.join("app");
    let before = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    let (ok, _o, err) = run(&["add", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(
        !ok,
        "add onto a scalar (non-list) deps must be refused: {err}"
    );
    assert!(
        err.contains("malformed `def deps`"),
        "the refusal names the malformed clause: {err}"
    );
    let after = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert_eq!(
        before, after,
        "the manifest is left byte-for-byte unchanged on a refused edit"
    );
    let _ = std::fs::remove_dir_all(&root);
}
