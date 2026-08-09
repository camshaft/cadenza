//! End-to-end tests for `cdz fmt` PROJECT mode — formatting a whole `Project.cdz` (the lifecycle-parity
//! addition, mirroring how `cdz build`/`test`/`check` act on a project). PROJECT mode triggers on no-arg
//! (with a manifest upward), a `Project.cdz` arg, or a directory holding one — and formats the manifest's
//! declared source set (entry + modules + tests). Explicit files, a lone `-` (stdin), and a manifest-less
//! directory pass through to the ordinary file/stdin `fmt` (tested in the cadenza-syntax crate). Drives
//! the built binary.

use std::process::Command;

#[path = "../common/mod.rs"]
mod common;
use common::write_stdin_tolerating_broken_pipe;

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

/// Count plain `//` line-comments (NOT `///` docs) in a blob. Splits on `//` but excludes the `///`
/// doc form so the two comment kinds are counted independently (a `///` contains `//` as a substring).
fn line_comment_count(s: &str) -> usize {
    s.matches("//").count() - s.matches("///").count()
}

#[test]
fn fmt_keeps_a_doc_above_an_annotation_not_between_it_and_the_def() {
    // Comment POSITION (not just count): a `///` written ABOVE an `@`-annotation must stay ABOVE it after
    // a format pass — NOT be reordered to sit BETWEEN the annotation and its def. That adjacency
    // (`@test` / `/// …` / `def`) is a latent fragility (it has produced spurious CDZ0201/CDZ0602 in
    // annotation-comment cases) and reads wrong (a section header would then annotate the first item, not
    // the section). An earlier printer bug DID reorder it (found by v-cad on a real fmt-apply); v-syntax's
    // `render_child` fix (`14ee12d7b`) prints a leading doc above the annotation. This pins the position:
    // the count-based pins above would PASS even if the `///` moved, so this is a distinct invariant — the
    // one the fleet-wide `cdz fmt` apply relied on (its safety metric forbids introducing this sandwich).
    let (dir, file) = temp_cdz(
        "doc-above-annot",
        "/// section header\n@test\ndef t () -> Bool = true\n",
    );
    let (ok, out, err) = run_in(&dir, &["fmt", &file, "--stdout"]);
    assert!(ok, "cdz fmt --stdout should succeed: {err}");
    // The doc still precedes the annotation: `/// …` appears before `@test` in the output.
    let doc_pos = out.find("/// section header");
    let at_pos = out.find("@test");
    assert!(
        matches!((doc_pos, at_pos), (Some(d), Some(a)) if d < a),
        "the `///` stays ABOVE `@test` (not moved below it): {out}"
    );
    // And there is NO `@annotation` / comment / `def` sandwich anywhere in the output.
    let has_sandwich = out.lines().collect::<Vec<_>>().windows(3).any(|w| {
        w[0].trim_start().starts_with('@')
            && w[1].trim_start().starts_with("//")
            && w[2].trim_start().starts_with("def")
    });
    assert!(
        !has_sandwich,
        "no `@` / comment / `def` sandwich is introduced: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_preserves_a_plain_line_comment_leading_a_def_body() {
    // Comment fidelity for the OTHER comment kind: a plain `//` line-comment that LEADS a def BODY (the
    // first thing inside the `=`, before the body expression) must survive a format pass. An earlier
    // reader gap dropped it (the leading-token side-table had no place to attach a comment that leads an
    // expression body, so it was stranded + discarded) — the same doc/comment-attachment family as the
    // `///`-before-`effect`/`@`-annotated-def bugs. FIXED by v-syntax (rcdzc `418bc6349`, "preserve a //
    // comment leading a def body"). This pins it: `fmt` is a faithful reprint, and a silently-eaten `//`
    // is data loss. (This gap was the bulk of what blocked the fleet-wide `cdz fmt` apply — worth a guard.)
    let (dir, file) = temp_cdz(
        "bodycomment",
        "def f () -> Int64 =\n  // interior body comment\n  1\n",
    );
    let (ok, out, err) = run_in(&dir, &["fmt", &file, "--stdout"]);
    assert!(ok, "cdz fmt --stdout should succeed: {err}");
    assert_eq!(
        line_comment_count(&out),
        1,
        "the `//` comment leading the def body survives the format pass: {out}"
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

#[test]
fn fmt_is_idempotent_a_second_pass_is_a_byte_stable_fixed_point() {
    // Core formatter invariant: `fmt` is a FIXED POINT — formatting an already-formatted file changes
    // nothing. A non-idempotent formatter churns files, defeats `fmt --check` in CI (a file it just wrote
    // would re-flag), and creates spurious diffs. Format an ugly-but-valid source with irregular spacing +
    // nested lets + a comment, capture the once-formatted bytes, format AGAIN, and assert byte-equality —
    // AND that the second pass reports `--check`-clean. Pins the fixed point so a future fmt/reader change
    // that introduces oscillation is caught. No runtime store needed (fmt is a pure text pass).
    let (dir, file) = temp_cdz(
        "idem",
        "// leading\ndef  add(a:Int64,b:Int64)->Int64=a+b\n\
         def nested()->Int64= let x=(1+2) in let y=x*3 in y\n",
    );
    // First format pass — record the result.
    let (ok1, _o1, e1) = run_in(&dir, &["fmt", &file]);
    assert!(ok1, "first fmt pass failed: {e1}");
    let once = std::fs::read_to_string(&file).expect("read after first fmt");
    // Second format pass — must not change a single byte.
    let (ok2, _o2, e2) = run_in(&dir, &["fmt", &file]);
    assert!(ok2, "second fmt pass failed: {e2}");
    let twice = std::fs::read_to_string(&file).expect("read after second fmt");
    assert_eq!(
        once, twice,
        "fmt must be idempotent — a second pass changed the bytes:\n--- once ---\n{once}\n--- twice ---\n{twice}"
    );
    // And the formatted file is `--check`-clean (the fixed point is what `--check` accepts).
    let (cok, cout, cerr) = run_in(&dir, &["fmt", &file, "--check"]);
    assert!(
        cok,
        "an already-formatted file must pass fmt --check: out={cout} err={cerr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run `cdz fmt <args…>` with `stdin` piped in, returning (exit_ok, stdout, stderr). Exercises the real
/// `std::io::stdin()` disposition path — not in-process unit-testable from cadenza-syntax, so this is the
/// e2e complement of that crate's `emits_to_stdout` predicate unit test.
fn fmt_stdin(stdin: &str, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut a = vec!["fmt"];
    a.extend_from_slice(args);
    let mut child = std::process::Command::new(exe)
        .args(&a)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz fmt (stdin)");
    write_stdin_tolerating_broken_pipe(child.stdin.take().unwrap(), stdin.as_bytes());
    let out = child.wait_with_output().expect("wait cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn fmt_stdin_check_reds_on_unformatted_and_passes_on_formatted() {
    // The stdin disposition bug (found probing `cdz`, fixed by v-syntax in cadenza-syntax cli.rs PR #1127
    // 0c7a0e134): `fmt - --check` used to hit the stdout-emit branch UNCONDITIONALLY (there's no file to
    // edit on stdin), so it printed the formatted program and exited 0 — SILENTLY IGNORING --check even for
    // UNFORMATTED input. A CI pipe (`… | cdz fmt - --from ml --check`) then got a FALSE PASS. The fix honors
    // --check on stdin. Pin it end-to-end over the REAL stdin pipe (the syntax crate can only unit-test the
    // disposition predicate in-process): unformatted stdin REDS + prints `not formatted: <stdin>`; already-
    // formatted stdin is clean (exit 0). No store — fmt is a pure text pass.
    let (ok, out, err) = fmt_stdin("def  f( )->Int64=  1\n", &["-", "--from", "ml", "--check"]);
    assert!(
        !ok,
        "fmt - --check must RED on unformatted stdin (not a silent exit-0):\nstdout:\n{out}\nstderr:\n{err}"
    );
    assert!(
        out.contains("not formatted") && out.contains("<stdin>"),
        "the --check report names the stdin source as not-formatted:\nstdout:\n{out}\nstderr:\n{err}"
    );

    // Already-formatted stdin: --check is clean → exit 0, and it must NOT re-emit the program (that's the
    // implicit stdout mode only WITHOUT --check).
    let (ok2, out2, err2) = fmt_stdin("def f() -> Int64 = 1\n", &["-", "--from", "ml", "--check"]);
    assert!(
        ok2,
        "fmt - --check exits 0 on already-formatted stdin:\nstdout:\n{out2}\nstderr:\n{err2}"
    );
    assert!(
        out2.trim().is_empty(),
        "fmt - --check writes nothing for already-formatted stdin (it inspects, doesn't emit):\nstdout:\n{out2}"
    );
}
