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
fn remove_the_only_entry_drops_the_deps_line_entirely() {
    // Removing the LAST dep drops the whole `def deps` line rather than leaving a dangling `def deps = []`
    // (the `cargo remove` behavior — an emptied section is removed, not left as an empty stub). The other
    // manifest fields are untouched and no blank line is left behind.
    let app = app_with_deps("only", "def deps = [\"../lib1\"]\n");
    let (ok, _o, err) = run(&["remove", "../lib1", "--manifest", app.to_str().unwrap()]);
    assert!(ok, "remove should succeed: {err}");
    let manifest = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    assert!(
        !manifest.contains("def deps"),
        "removing the only dep drops the `def deps` line entirely (no `[]` stub): {manifest}"
    );
    assert!(
        manifest.contains("def name = \"app\"") && manifest.contains("def entry = \"main.sexp\""),
        "the other manifest fields are preserved: {manifest}"
    );
    assert!(
        !manifest.contains("\n\n"),
        "no blank line is left where the deps line was: {manifest:?}"
    );
    // The manifest still re-parses.
    let (tok, _tout, _e) = run(&["tree", app.to_str().unwrap()]);
    assert!(tok, "the manifest re-parses after dropping the deps line");
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

#[test]
fn remove_refuses_to_edit_a_scalar_deps_manifest_leaving_it_intact() {
    // The inverse of `cdz add`'s scalar-deps refusal: `def deps` READS as accept-both (a bare scalar string
    // coerces to a one-element list), but `cdz remove` EDITS the `[...]` list a scalar has no brackets for.
    // Rather than corrupt the manifest, `remove` must REFUSE cleanly — exit non-zero, name the malformed
    // clause, and leave the manifest byte-for-byte unchanged (the text-editor never writes an edit it can't
    // do cleanly). Note the scalar `../foo` is the very path being removed, so this isn't a "not a dep"
    // no-op — the refusal is specifically about the non-list SHAPE.
    let app = app_with_deps("scalar", "def deps = \"../foo\"\n");
    let before = std::fs::read_to_string(app.join("Project.cdz")).unwrap();
    let (ok, _o, err) = run(&["remove", "../foo", "--manifest", app.to_str().unwrap()]);
    assert!(
        !ok,
        "remove on a scalar (non-list) deps must be refused: {err}"
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
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}
