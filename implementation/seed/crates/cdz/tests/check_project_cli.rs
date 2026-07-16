//! End-to-end tests for `cdz check` over a PROJECT (a directory / `Project.cdz` / no-arg upward search)
//! — the project-wide lint, mirroring how `cdz test`/`cdz build` treat a project. A single-file `cdz
//! check FILE` still checks just that file (its import closure); a project target checks every source
//! file and fails if ANY has an error. Drives the built binary.

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

/// A two-file project (manifest + entry importing a module). `util_body` is the module's source, so a
/// test can inject an error into the MODULE (not the entry). Returns the project dir.
fn temp_project(tag: &str, util_body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-checkproj-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"chk\"\ndef entry = \"app.cdz\"\ndef modules = [\"util.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(dir.join("util.cdz"), util_body).unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "import { helper } from \"util\"\ndef main(a: Int64) -> Int64 = helper(a)\nexport { main }\n",
    )
    .unwrap();
    dir
}

const CLEAN_UTIL: &str = "def helper(n: Int64) -> Int64 = n + 1\nexport { helper }\n";

#[test]
fn check_a_clean_project_via_directory_succeeds() {
    let dir = temp_project("dir", CLEAN_UTIL);
    let (ok, out, err) = run_in(&dir, &["check", "."]);
    assert!(ok, "a clean project checks clean: {err}{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_a_project_via_manifest_arg_and_no_arg_agree() {
    let dir = temp_project("resolve", CLEAN_UTIL);
    // Manifest-path arg.
    let (m_ok, _o, me) = run_in(&dir, &["check", "Project.cdz"]);
    assert!(m_ok, "check Project.cdz (clean) failed: {me}");
    // No-arg → upward search finds the same Project.cdz.
    let (n_ok, _o, ne) = run_in(&dir, &["check"]);
    assert!(n_ok, "no-arg project check (clean) failed: {ne}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_a_project_fails_when_a_module_has_an_error() {
    // The project-wide payoff: an error in an imported module (not the entry) fails the project check
    // and is reported at the module's own path — a single-entry-file check would miss it. Diagnostics
    // print to stdout; the failure is the non-zero exit.
    let dir = temp_project(
        "moderr",
        "def helper(n: Int64) -> Int64 = undefined_xyz\nexport { helper }\n",
    );
    let (ok, out, err) = run_in(&dir, &["check", "."]);
    assert!(
        !ok,
        "an error in a module must fail the project check: {err}"
    );
    assert!(
        out.contains("util.cdz") && out.contains("undefined_xyz"),
        "the module's error is reported at util.cdz: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_a_single_file_still_checks_only_that_file() {
    // A single-file target is unchanged: `cdz check app.cdz` checks that file (+ its closure), not the
    // whole directory. With a clean project, it passes.
    let dir = temp_project("single", CLEAN_UTIL);
    let (ok, _o, err) = run_in(&dir, &["check", "app.cdz"]);
    assert!(ok, "single-file check still works: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}
