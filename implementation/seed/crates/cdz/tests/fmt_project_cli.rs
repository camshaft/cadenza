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

/// Write `src` to a fresh temp `.cdz` file and return (dir, path) — for the comment-fidelity checks
/// (which need a specific comment shape, not the `ugly_project` spacing fixture).
fn temp_cdz(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-fmtdoc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.cdz");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// Count `///` doc-comment markers in a blob (the fidelity metric: fmt must never DROP or DOWNGRADE a
/// doc-comment to a plain `//`, so the `///` count must survive a format pass).
fn triple_slash_count(s: &str) -> usize {
    s.matches("///").count()
}

#[test]
fn fmt_preserves_a_doc_comment_on_a_plain_def() {
    // Comment fidelity: `cdz fmt` must NOT drop or downgrade a `///` doc-comment. The common case — a
    // `///` before a plain `def` — round-trips its doc through a format pass with the `///` count intact.
    // (This pins the invariant against a future reader/printer regression; the whole point of `fmt` is a
    // faithful reprint, and a silently-eaten doc-comment is a data-loss bug, not a formatting choice.)
    let (dir, file) = temp_cdz(
        "plaindef",
        "/// A documented function.\n/// Second doc line.\ndef answer () -> Int64 = 42\n",
    );
    let (ok, out, err) = run_in(&dir, &["fmt", &file, "--stdout"]);
    assert!(ok, "cdz fmt --stdout should succeed: {err}");
    assert_eq!(
        triple_slash_count(&out),
        2,
        "both `///` doc lines survive the format pass (not downgraded to `//`): {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// A `///` before an `@`-annotated def (e.g. `@test`) must survive a format pass — the annotation sits
// between the doc and the def, and an earlier reader doc-attachment bug (same class as the `///`-before-
// `effect` one) dropped the doc to a `//` because `def_expr` ran after the `@name` slot never carried the
// docs across. FIXED by v-syntax (rcdzc `aa1d8a75b`, "carry docs across the annotation"); this pins it as
// a green regression guard (was `#[ignore]`d while the fix was in flight). NOTE: a `///` file-HEADER
// before the FIRST `import` is a SEPARATE, still-open gap (v-syntax's import doc-drain slice) — the
// fleet-wide `cdz fmt` apply phases around it per-file (apply iff the file's `///` count round-trips).
#[test]
fn fmt_preserves_a_doc_comment_on_an_annotated_def() {
    let (dir, file) = temp_cdz(
        "annotdef",
        "def a () -> Int64 = 1\n\n/// Doc before an annotated def.\n@test\ndef b () -> Bool = Int64.eq(a(), 1)\n",
    );
    let (ok, out, err) = run_in(&dir, &["fmt", &file, "--stdout"]);
    assert!(ok, "cdz fmt --stdout should succeed: {err}");
    assert_eq!(
        triple_slash_count(&out),
        1,
        "the `///` before the @test def survives (not downgraded to `//`): {out}"
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
