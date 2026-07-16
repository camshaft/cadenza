//! End-to-end tests for `cdz fmt` PROJECT mode — formatting a whole `Project.cdz` (the lifecycle-parity
//! addition, mirroring how `cdz build`/`test`/`check` act on a project). PROJECT mode triggers on no-arg
//! (with a manifest upward), a `Project.cdz` arg, or a directory holding one — and formats the manifest's
//! declared source set (entry + modules + tests). Explicit files, a lone `-` (stdin), and a manifest-less
//! directory pass through to the ordinary file/stdin `fmt` (tested in the cadenza-syntax crate). Drives
//! the built binary.

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

/// A project with an entry + a module + a test, each written with UGLY spacing so `fmt --check` flags it.
fn ugly_project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-fmtproj-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"demo\"\ndef entry = \"app.cdz\"\ndef modules = [\"util.cdz\"]\n\
         def tests = [\"*_test.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def  main( )->Int64=  0\nexport{main}\n",
    )
    .unwrap();
    std::fs::write(dir.join("util.cdz"), "def u( )->Int64=1\nexport{u}\n").unwrap();
    std::fs::write(dir.join("app_test.cdz"), "def  t( )->Int64= 3\nexport{t}\n").unwrap();
    dir
}

#[test]
fn fmt_no_arg_checks_the_whole_project() {
    // A bare `cdz fmt --check` inside a project formats-checks the manifest's declared sources (entry +
    // modules + tests) — the lifecycle-parity payoff — and fails (non-zero) listing each unformatted file.
    let dir = ugly_project("noarg");
    let (ok, out, err) = run_in(&dir, &["fmt", "--check"]);
    assert!(!ok, "unformatted project sources → non-zero: {err}");
    for f in ["app.cdz", "util.cdz", "app_test.cdz"] {
        assert!(
            out.contains(f),
            "the project fmt --check covers `{f}`: {out}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_writes_the_whole_project_and_rechecks_clean() {
    // `cdz fmt` (write) formats every declared source in place; a re-check is then clean.
    let dir = ugly_project("write");
    let (ok, _o, err) = run_in(&dir, &["fmt"]);
    assert!(ok, "cdz fmt (project write) failed: {err}");
    let (cok, _co, _ce) = run_in(&dir, &["fmt", "--check"]);
    assert!(cok, "after formatting, --check is clean");
    // The manifest itself is NOT reformatted (only declared sources) — it's left byte-identical.
    let manifest = std::fs::read_to_string(dir.join("Project.cdz")).unwrap();
    assert!(
        manifest.contains("def entry = \"app.cdz\""),
        "the Project.cdz manifest is not touched by fmt"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_a_project_cdz_arg_and_a_dir_arg_both_enter_project_mode() {
    let dir = ugly_project("args");
    // Manifest-path arg.
    let manifest = dir.join("Project.cdz");
    let (m_ok, m_out, _e) = run_in(&dir, &["fmt", manifest.to_str().unwrap(), "--check"]);
    assert!(
        !m_ok && m_out.contains("app.cdz"),
        "Project.cdz arg → project mode: {m_out}"
    );
    // Directory arg.
    let (d_ok, d_out, _e) = run_in(&dir, &["fmt", ".", "--check"]);
    assert!(
        !d_ok && d_out.contains("util.cdz"),
        "dir arg → project mode: {d_out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_an_explicit_file_list_is_not_project_mode() {
    // Passing explicit files formats exactly those — NOT the whole project. Only `app.cdz` is checked here.
    let dir = ugly_project("explicit");
    let (ok, out, _e) = run_in(&dir, &["fmt", "app.cdz", "--check"]);
    assert!(!ok, "the one unformatted file is flagged");
    assert!(out.contains("app.cdz"), "app.cdz checked: {out}");
    assert!(
        !out.contains("util.cdz") && !out.contains("app_test.cdz"),
        "an explicit file list does NOT pull in the rest of the project: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_a_manifest_with_no_source_errors() {
    // A `Project.cdz` declaring no entry/modules/tests has no source to format — a clear error.
    let dir = std::env::temp_dir().join(format!("cdz-fmtproj-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def name = \"empty\"\n").unwrap();
    let (ok, _o, err) = run_in(&dir, &["fmt", "--check"]);
    assert!(!ok, "an empty manifest fmt should fail");
    assert!(
        err.contains("no `entry`/`modules`/`tests` source to format"),
        "clear no-source error: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
