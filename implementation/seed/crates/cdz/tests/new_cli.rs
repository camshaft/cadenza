//! End-to-end tests for `cdz new <name>` — scaffold a new project (the `cargo new` analogue).
//!
//! `cdz new my-app` creates `my-app/` with a `Project.cdz` manifest + a minimal buildable entry, so
//! `cd my-app && cdz build` works immediately — the last piece of the new→build→run→test project loop.
//! These drive the built binary and assert the scaffold is BUILDABLE (not just present).

use std::process::Command;

/// Run `cdz <args…>` from `cwd`, returning (exit_ok, stdout, stderr).
fn run_in(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// A fresh empty scratch dir to scaffold projects INTO.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-new-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn new_scaffolds_a_buildable_project() {
    // The whole point: `cdz new app` then `cdz build` (in app/) compiles — the scaffold is buildable,
    // not just files on disk.
    let root = scratch("build");
    let (ok, out, err) = run_in(&root, &["new", "app"]);
    assert!(ok, "cdz new failed: {err}");
    assert!(
        out.contains("created project"),
        "reports what it made: {out}"
    );
    let proj = root.join("app");
    assert!(proj.join("Project.cdz").is_file(), "manifest scaffolded");
    assert!(proj.join("main.cdz").is_file(), "entry scaffolded");
    // Build the scaffold — this is the acid test that `new` produced a coherent project.
    let (bok, _bo, be) = run_in(&proj, &["build", "-o", "."]);
    assert!(bok, "the scaffolded project must build: {be}");
    assert!(
        proj.join("main.wasm").is_file(),
        "the build produced a component: {be}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_sexpr_scaffolds_the_s_expression_surface() {
    // `--sexpr` scaffolds a `.sexp` entry (and it builds too).
    let root = scratch("sexpr");
    let (ok, _o, err) = run_in(&root, &["new", "app", "--sexpr"]);
    assert!(ok, "cdz new --sexpr failed: {err}");
    let proj = root.join("app");
    assert!(proj.join("main.sexp").is_file(), "s-expr entry scaffolded");
    let (bok, _bo, be) = run_in(&proj, &["build", "-o", "."]);
    assert!(bok, "the s-expr scaffold must build: {be}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_refuses_to_clobber_a_non_empty_directory() {
    // `cdz new` must never destroy existing work — a non-empty target is refused with a clear error.
    let root = scratch("clobber");
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app").join("keep.txt"), "important\n").unwrap();
    let (ok, _o, err) = run_in(&root, &["new", "app"]);
    assert!(!ok, "scaffolding into a non-empty dir should fail");
    assert!(
        err.contains("not empty") || err.contains("already exists"),
        "error explains the refusal: {err}"
    );
    // The pre-existing file is untouched.
    assert!(
        root.join("app").join("keep.txt").is_file(),
        "existing files are preserved"
    );
    let _ = std::fs::remove_dir_all(&root);
}
