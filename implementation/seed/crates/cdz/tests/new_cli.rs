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

#[test]
fn new_refuses_a_file_target_with_a_clear_error() {
    // REGRESSION (Copilot PR #420): the non-empty guard treated a `read_dir` failure — including the
    // target being a FILE — as "not empty", a misleading message. A file target now gets its own clear
    // error ("exists as a file"), distinct from a non-empty directory.
    let root = scratch("filetarget");
    std::fs::write(root.join("app"), "i am a file\n").unwrap();
    let (ok, _o, err) = run_in(&root, &["new", "app"]);
    assert!(!ok, "scaffolding onto a file should fail");
    assert!(
        err.contains("as a file"),
        "error says the target is a file (not the generic 'not empty'): {err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_escapes_the_project_name_in_the_manifest() {
    // REGRESSION (Copilot PR #420): the dir name was raw-interpolated into `def name = "…"`, so a name
    // with a `"` malformed the Project.cdz. It must be ESCAPED — and the resulting manifest must still
    // build. Uses a directory name containing a double-quote (valid on the filesystem).
    let root = scratch("escape");
    let (ok, _o, err) = run_in(&root, &["new", "q\"x"]);
    assert!(ok, "cdz new with a quote in the name failed: {err}");
    let manifest = std::fs::read_to_string(root.join("q\"x").join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("def name = \"q\\\"x\""),
        "the `\"` in the name is escaped in the manifest: {manifest}"
    );
    // The escaped manifest must still be a valid project — build it.
    let proj = root.join("q\"x");
    let (bok, _bo, be) = run_in(&proj, &["build", "-o", "."]);
    assert!(bok, "the escaped-name project must still build: {be}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_scaffolds_a_gitignore_covering_the_build_artifacts() {
    // `cdz new` writes a `.gitignore` (the `cargo new`→`/target` convention) covering the EXACT build
    // outputs of the scaffolded entry (which exports `main` → `main.{wasm,rs,dwarf}`) + `link-map.txt` +
    // the run temp — NOT broad `*.wasm`/`*.rs` globs (which would git-ignore a user's hand-written Rust
    // helper; the same over-broad extension assumption that made `cdz clean` a data-loss risk, PR #454).
    let root = scratch("gitignore");
    let (ok, _o, err) = run_in(&root, &["new", "app"]);
    assert!(ok, "cdz new failed: {err}");
    let gi = root.join("app").join(".gitignore");
    assert!(gi.is_file(), "a .gitignore is scaffolded");
    let body = std::fs::read_to_string(&gi).unwrap();
    for pat in [
        "main.wasm",
        "main.rs",
        "main.dwarf",
        "link-map.txt",
        ".cdz-run-*.wasm",
    ] {
        assert!(
            body.contains(pat),
            "the .gitignore ignores the exact output `{pat}`: {body}"
        );
    }
    // It must NOT use a broad extension glob as its OWN line — a user's hand-written `helper.rs` should
    // not be ignored. (Checked per-line so the legitimate `.cdz-run-*.wasm` temp pattern doesn't trip it.)
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    assert!(
        !lines.contains(&"*.rs") && !lines.contains(&"*.wasm") && !lines.contains(&"*.dwarf"),
        "the .gitignore does not use a broad extension glob line (would ignore user files): {body}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn new_rejects_an_in_place_name_and_points_to_init() {
    // HARDENING: `cdz new` NAMES A FRESH SUBDIRECTORY — a name that instead means the current/parent dir
    // (empty, `.`, `..`) used to silently scaffold into the CWD (empty name → a bogus "created project
    // `app` in " with an empty path). Those are now rejected with a pointer to `cdz init` (the in-place
    // command). A real name still works (covered by the other tests).
    let root = scratch("inplace");
    for bad in ["", ".", ".."] {
        let (ok, _o, err) = run_in(&root, &["new", bad]);
        assert!(
            !ok,
            "`cdz new {bad:?}` should be rejected (names no fresh subdir)"
        );
        assert!(
            err.contains("cdz init") && err.contains("NEW project directory"),
            "the error points to `cdz init` for the in-place case: {err}"
        );
    }
    // The rejection wrote nothing into the cwd.
    assert!(
        !root.join("Project.cdz").is_file() && !root.join("main.cdz").is_file(),
        "a rejected `cdz new .` must not scaffold into the current directory"
    );
    let _ = std::fs::remove_dir_all(&root);
}
