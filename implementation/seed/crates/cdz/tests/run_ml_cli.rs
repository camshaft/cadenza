//! End-to-end tests for `cdz run-ml [FILE]` — the CADENZA-IN-CADENZA (ML) compiler conformance seam the
//! `cargo xtask gate` `cadenza-ml` target invokes.
//!
//! These pin the EXIT-CODE CONTRACT the subcommand's doc promises (and that the gate wiring relies on):
//! any RUN OUTCOME — a value, a `declined` (not-yet-supported), or an `error <msg>` — exits 0 (a verdict,
//! not a shell failure), so the gate reads stdout for the per-case outcome; the ONE non-zero exit is a
//! HARNESS failure that produced no verdict (a file/stdin READ error). (`run-ml` is currently a
//! declines-all stub; the ML front-end lands behind it — so the value/error verdicts aren't exercised
//! here yet, but the decline + read-error exits, which the gate contract hinges on, are.)

use std::io::Write;
use std::process::{Command, Stdio};

/// Run `cdz run-ml <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut a = vec!["run-ml"];
    a.extend_from_slice(args);
    let out = Command::new(exe).args(&a).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Write `src` to a unique temp `.sexp` file; return (dir, path).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-runml-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

#[test]
fn run_ml_exits_zero_for_a_run_outcome() {
    // A well-formed program yields a VERDICT to stdout and exits 0 — a decline is a verdict, not a shell
    // failure (the gate reads stdout, not the exit code, for the per-case outcome). Currently the stub
    // declines every program, so the verdict is `declined`; the invariant under test is the ZERO exit.
    let (dir, path) = temp_src("verdict", "(do (def (main) 0) (export main))");
    let (ok, out, err) = run(&[&path]);
    assert!(
        ok,
        "a run outcome exits 0 (a verdict is not a shell failure): {err}{out}"
    );
    assert!(
        out.contains("declined") || out.starts_with("value ") || out.starts_with("error "),
        "stdout carries one verdict line: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ml_from_stdin_also_exits_zero_with_a_verdict() {
    // No FILE arg → read the program from stdin (the same path the gate can drive); still a verdict + exit 0.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["run-ml"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"(do (def (main) 0) (export main))\n")
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "stdin run-ml exits 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("declined") || stdout.starts_with("value ") || stdout.starts_with("error "),
        "stdin run-ml prints a verdict: {stdout:?}"
    );
}

#[test]
fn run_ml_leaves_no_driver_file_behind() {
    // `cdz run-ml` writes a pid-stamped `zz-run-ml-driver-<pid>.cdz` INTO the compiler-ml src tree (its
    // `import "sread-eval"` resolves entry-dir-relative, so it can't live in a temp dir). An RAII guard
    // must remove it on every in-process exit path — else a screenful of stray drivers accumulates in the
    // shared worktree's `git status` (the fleet-pollution issue this pins).
    //
    // Assert on OUR child's OWN pid-stamped driver, not a directory COUNT: the sibling run-ml tests run
    // concurrently against the same shared src dir, so their in-flight drivers make a count-delta racy
    // (observed before=0/after=2). Spawn the child ourselves, capture its pid, let it fully exit, then
    // assert exactly `zz-run-ml-driver-<child-pid>.cdz` is absent — a leak of THIS run is unambiguous and
    // independent of any concurrent invocation.
    let mut root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = loop {
        let cand = root.join("implementation/compiler-ml/src");
        if cand.is_dir() {
            break Some(cand);
        }
        if !root.pop() {
            break None;
        }
    };
    let Some(src_dir) = src_dir else {
        // The compiler-ml src tree isn't present in this checkout (shouldn't happen in-repo) — nothing to
        // pin; don't fail the harness over a layout we can't see.
        return;
    };
    let exe = env!("CARGO_BIN_EXE_cdz");
    let (dir, path) = temp_src("cleanup", "(do (def (main) 0) (export main))");
    // Spawn so we can read the child's pid — the driver is named after the CHILD `cdz`'s process id.
    let child = Command::new(exe)
        .args(["run-ml", &path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz run-ml");
    let child_pid = child.id();
    let out = child.wait_with_output().expect("wait run-ml");
    let _ = std::fs::remove_dir_all(&dir);
    // The child has fully exited, so its RAII guard has run — its own driver must be gone.
    let leaked = src_dir.join(format!("zz-run-ml-driver-{child_pid}.cdz"));
    assert!(
        !leaked.exists(),
        "run-ml leaked its driver {} (child exited status {:?}) — the RAII cleanup guard regressed",
        leaked.display(),
        out.status
    );
}

#[test]
fn run_ml_on_an_unreadable_file_exits_non_zero() {
    // The ONE non-zero exit: a HARNESS failure that produced no verdict (a file READ error). A script /
    // the gate distinguishes "the ML compiler judged this program" (exit 0 + a verdict) from "the input
    // couldn't be read at all" (non-zero + no verdict).
    let dir = std::env::temp_dir().join(format!("cdz-runml-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("nope.sexp");
    let (ok, out, err) = run(&[missing.to_str().unwrap()]);
    assert!(
        !ok,
        "an unreadable input is a non-zero (harness) exit, not a verdict: {out}"
    );
    assert!(
        err.contains("cannot read"),
        "the read failure is named on stderr: {err}"
    );
    assert!(
        !out.contains("declined") && !out.starts_with("value "),
        "no verdict line is printed for a read error: {out:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
