//! End-to-end tests for `cdz remove PATH` — the manifest dependency-editor (the `cargo remove` analogue,
//! the inverse of `cdz add`).
//!
//! `cdz remove ../lib` drops a PATH dependency from the project's `Project.cdz` `def deps`, so a user
//! doesn't hand-edit the manifest. These drive the built binary over real project dirs and assert: a
//! middle/first/last entry is removed cleanly (no dangling comma); removing the only entry leaves `[]`;
//! removing a path that isn't declared is an idempotent no-op; and the edited manifest stays valid
//! (re-parses — proven by `cdz tree`) throughout.

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

/// A workspace root with an `app` project carrying `deps_line` (e.g. `def deps = ["../lib1", "../lib2"]\n`)
/// plus sibling `lib1`/`lib2` projects. Returns the app dir path (where the manifest lives).
fn app_with_deps(tag: &str, deps_line: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("cdz-remove-{tag}-{}", std::process::id()));
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
    root.join("app")
}

#[test]
fn remove_drops_one_entry_and_keeps_the_others_and_reparses() {
    // Removing a MIDDLE entry drops just it (no dangling comma), the rest survive, and the manifest still
    // re-parses (proven by `cdz tree` — `cdz remove` re-parses internally too and would have reverted a
    // broken edit).
    let app = app_with_deps("mid", "def deps = [\"../lib1\", \"../lib2\"]\n");
    let (ok, _o, err) = run(&["remove", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "remove should succeed: {err}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        !manifest.contains("\"../lib1\"") && manifest.contains("\"../lib2\""),
        "the removed dep is gone, the other survives: {manifest}"
    );
    // No dangling/leading comma left in the list.
    assert!(
        manifest.contains("def deps = [\"../lib2\"]"),
        "the list is a clean canonical form after removal: {manifest}"
    );
    let (tok, tout, _e) = run(&["tree", app.to_str().unwrap()]);
    assert!(
        tok && tout.contains("lib2 (../lib2)") && !tout.contains("lib1 (../lib1)"),
        "tree sees only the surviving dep + the manifest re-parses: {tout}"
    );
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn remove_the_only_entry_leaves_an_empty_list() {
    let app = app_with_deps("only", "def deps = [\"../lib1\"]\n");
    let (ok, _o, err) = run(&["remove", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "remove should succeed: {err}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("def deps = []"),
        "removing the only dep leaves an empty list: {manifest}"
    );
    // The empty-deps manifest still re-parses.
    let (tok, _tout, _e) = run(&["tree", app.to_str().unwrap()]);
    assert!(tok, "the emptied manifest re-parses");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn remove_a_path_that_is_not_a_dep_is_an_idempotent_no_op() {
    // Removing a path that isn't declared is a SUCCESS no-op (a notice), not an error — and the manifest
    // is left untouched.
    let app = app_with_deps("noop", "def deps = [\"../lib1\"]\n");
    let before = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    let (ok, _o, err) = run(&["remove", "../nope", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "removing a non-dep is a no-op success: {err}");
    assert!(
        err.contains("does not declare") && err.contains("nothing to remove"),
        "the no-op is reported: {err}"
    );
    let after = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert_eq!(
        before, after,
        "a no-op remove leaves the manifest byte-identical"
    );
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn add_then_remove_round_trips_back_to_the_original_deps() {
    // `cdz remove` is the inverse of `cdz add`: add a dep then remove it, and the `def deps` list is back
    // to where it started (here: a single `../lib1`).
    let app = app_with_deps("roundtrip", "def deps = [\"../lib1\"]\n");
    let m = app.to_str().unwrap();
    let (aok, _o, aerr) = run(&["add", "../lib2", "--manifest", m]);
    assert!(aok, "add: {aerr}");
    let (rok, _o2, rerr) = run(&["remove", "../lib2", "--manifest", m]);
    assert!(rok, "remove: {rerr}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("def deps = [\"../lib1\"]") && !manifest.contains("../lib2"),
        "add then remove round-trips to the original single dep: {manifest}"
    );
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}
