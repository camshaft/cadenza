//! End-to-end tests for `cdz init [dir]` — scaffold a project INTO an existing directory (the `cargo
//! init` analogue). Unlike `cdz new` (which makes a fresh `<name>/`), `init` adopts the directory it's
//! given (default: the current one): it writes a `Project.cdz` + a minimal buildable entry and leaves
//! any other files alone, refusing only when a `Project.cdz` already exists. These drive the built
//! binary and assert the scaffold is BUILDABLE.

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

/// A fresh scratch dir to `init` into.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-init-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

#[test]
fn init_scaffolds_the_current_directory_into_a_buildable_project() {
    // The whole point: `cdz init` (no arg) writes a manifest + entry into the CURRENT directory (no new
    // subdir), then `cdz build` compiles it — the scaffold is buildable in place.
    let dir = scratch("cwd");
    let (ok, out, err) = run_in(&dir, &["init"]);
    assert!(ok, "cdz init failed: {err}");
    assert!(
        out.contains("initialized project"),
        "reports what it made: {out}"
    );
    assert!(dir.join("Project.cdz").is_file(), "manifest scaffolded");
    assert!(dir.join("main.cdz").is_file(), "entry scaffolded");
    // No SUBdirectory was created — init adopts the dir in place.
    assert!(
        !dir.join(dir.file_name().unwrap()).exists(),
        "init does not create a nested project subdir"
    );
    let (bok, _bo, be) = run_in(&dir, &["build", "-o", "."]);
    assert!(bok, "the initialized project must build: {be}");
    assert!(
        dir.join("main.wasm").is_file(),
        "the build produced a component: {be}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_adopts_a_directory_that_already_has_other_files() {
    // Unlike `cdz new`, `init` does NOT refuse a non-empty directory — it ADOPTS it, adding the manifest +
    // entry beside whatever is already there (a README, sources, etc.), leaving those untouched.
    let dir = scratch("adopt");
    std::fs::write(dir.join("README.md"), "# my project\n").unwrap();
    std::fs::write(dir.join("notes.txt"), "keep me\n").unwrap();
    let (ok, _o, err) = run_in(&dir, &["init"]);
    assert!(ok, "init should adopt a non-empty directory: {err}");
    assert!(dir.join("Project.cdz").is_file(), "manifest added");
    // The pre-existing files are untouched.
    assert_eq!(
        std::fs::read_to_string(dir.join("README.md")).unwrap(),
        "# my project\n",
        "an existing file is left untouched"
    );
    assert!(dir.join("notes.txt").is_file(), "existing files preserved");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_refuses_to_overwrite_an_existing_manifest() {
    // `init` must never clobber an existing `Project.cdz` — a directory already a project is refused with a
    // clear error, and the original manifest is preserved byte-for-byte.
    let dir = scratch("existing");
    let original = "def name = \"keep\"\ndef entry = \"other.cdz\"\n";
    std::fs::write(dir.join("Project.cdz"), original).unwrap();
    let (ok, _o, err) = run_in(&dir, &["init"]);
    assert!(
        !ok,
        "init into a dir that already has a manifest should fail"
    );
    assert!(
        err.contains("already") && err.contains("project"),
        "error explains the directory is already a project: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Project.cdz")).unwrap(),
        original,
        "the existing manifest is preserved unchanged"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_creates_a_named_missing_directory() {
    // `cdz init <dir>` on a NAMED directory that doesn't exist creates it (like `cargo init <dir>`), then
    // scaffolds into it — so `cdz init sub && cd sub && cdz build` works.
    let root = scratch("named");
    let (ok, _o, err) = run_in(&root, &["init", "sub"]);
    assert!(ok, "init of a named missing dir failed: {err}");
    let proj = root.join("sub");
    assert!(
        proj.join("Project.cdz").is_file(),
        "manifest in the named dir"
    );
    assert!(proj.join("main.cdz").is_file(), "entry in the named dir");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn init_sexpr_scaffolds_the_s_expression_surface() {
    // `--sexpr` scaffolds a `.sexp` entry (and it builds too).
    let dir = scratch("sexpr");
    let (ok, _o, err) = run_in(&dir, &["init", "--sexpr"]);
    assert!(ok, "cdz init --sexpr failed: {err}");
    assert!(dir.join("main.sexp").is_file(), "s-expr entry scaffolded");
    let (bok, _bo, be) = run_in(&dir, &["build", "-o", "."]);
    assert!(bok, "the s-expr scaffold must build: {be}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_writes_a_gitignore_when_absent() {
    // `cdz init` scaffolds a `.gitignore` covering the build artifacts (like `cdz new`) when the directory
    // has none.
    let dir = scratch("gi-new");
    let (ok, _o, err) = run_in(&dir, &["init"]);
    assert!(ok, "cdz init failed: {err}");
    let gi = dir.join(".gitignore");
    assert!(gi.is_file(), "a .gitignore is written");
    let body = std::fs::read_to_string(&gi).unwrap();
    assert!(
        body.contains("main.wasm") && body.contains("link-map.txt"),
        "the .gitignore covers the exact build artifacts: {body}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn init_does_not_clobber_an_existing_gitignore() {
    // `cdz init` ADOPTS a directory — it must NEVER overwrite a `.gitignore` the user maintains (the same
    // non-destructive spirit as refusing an existing Project.cdz). The original content is preserved.
    let dir = scratch("gi-keep");
    let original = "# my rules\nsecret.key\n/build-cache\n";
    std::fs::write(dir.join(".gitignore"), original).unwrap();
    let (ok, _o, err) = run_in(&dir, &["init"]);
    assert!(
        ok,
        "cdz init should still succeed with an existing .gitignore: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        original,
        "an existing .gitignore is preserved unchanged"
    );
    // The project was still scaffolded.
    assert!(
        dir.join("Project.cdz").is_file(),
        "manifest still scaffolded"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
