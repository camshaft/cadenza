//! `cdz test <dir>` — the shared-arena `EmitTestsPerFile` precompile path is BEHAVIOR-IDENTICAL to the
//! per-file compile.
//!
//! `cdz test <dir>` now compiles all target files in ONE `EmitTestsPerFile` pass (lower the shared closure
//! once, one component per file) and each file reuses its component instead of re-lowering the whole closure.
//! rcdzc's own test proves each file's shared-arena component is byte-identical to its standalone `EmitTests`
//! compile; this pins the CLI-visible contract: a directory run reports the SAME per-file PASS/FAIL, the same
//! per-file `N passed, M failed`, the same TOTAL, and a test-free file stays vacuously green — regardless of
//! whether a file's tests came from the precompile fast path or the per-file fallback.

use std::process::Command;

fn cdz() -> &'static str {
    env!("CARGO_BIN_EXE_cdz")
}

/// Write a multi-file package: `a` (1 pass + 1 fail), `b` (1 pass), `lib` (no @test). Returns the dir.
fn package(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-perfile-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("a.cdz"),
        "@test\ndef t_a_pass() =\n  if 1 + 1 == 2 then unit else trap(\"a: math\")\n\n\
         @test\ndef t_a_fail() =\n  trap(\"a: intentional fail\")\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.cdz"),
        "@test\ndef t_b_pass() =\n  if 2 * 3 == 6 then unit else trap(\"b: math\")\n",
    )
    .unwrap();
    std::fs::write(dir.join("lib.cdz"), "def helper(n: Int64) =\n  n + 1\n").unwrap();
    dir
}

/// Run `cdz test <path>`; return (exit_ok, stdout).
fn run_test(path: &std::path::Path) -> (bool, String) {
    let out = Command::new(cdz())
        .args(["test", path.to_str().unwrap()])
        .output()
        .expect("spawn cdz test");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

#[test]
fn a_directory_run_reports_the_same_per_file_and_total_results() {
    let dir = package("dir");
    let (ok, out) = run_test(&dir);
    // The suite has a failing test, so the run exits non-zero — expected.
    assert!(!ok, "a suite with a failing test exits non-zero:\n{out}");

    // Per-file results (fast path for a/b that HAVE @tests; lib is vacuously green with no output block body).
    assert!(out.contains("PASS t_a_pass"), "a's passing test:\n{out}");
    assert!(out.contains("FAIL t_a_fail"), "a's failing test:\n{out}");
    assert!(out.contains("PASS t_b_pass"), "b's passing test:\n{out}");
    // a: 1 passed, 1 failed; b: 1 passed, 0 failed — each file's own summary line.
    assert!(
        out.contains("1 passed, 1 failed"),
        "a's per-file summary:\n{out}"
    );
    assert!(
        out.contains("1 passed, 0 failed"),
        "b's per-file summary:\n{out}"
    );
    // The aggregate across the package.
    assert!(
        out.contains("TOTAL: 2 passed, 1 failed (across 3 files)"),
        "the package TOTAL aggregates all files:\n{out}"
    );
}

#[test]
fn a_single_file_run_matches_that_file_within_the_directory_run() {
    // The single-file path FALLS BACK to the per-file compile (no closure to share) — its result must match
    // what the same file produces inside the directory (fast) run: same PASS/FAIL lines + per-file summary.
    let dir = package("single");
    let (_ok, single) = run_test(&dir.join("a.cdz"));
    assert!(
        single.contains("PASS t_a_pass")
            && single.contains("FAIL t_a_fail")
            && single.contains("1 passed, 1 failed"),
        "single-file a.cdz reports the same as within the dir run:\n{single}"
    );
}

#[test]
fn same_stem_files_in_different_subdirs_do_not_cross_contaminate() {
    // PR#881 regression: a closure file's link name is its dir-BLIND stem, and the shared precompile keys the
    // union + lookup by that stem. Two same-stem files in DIFFERENT subdirs (`d1/t.cdz`, `d2/t.cdz`) must NOT
    // collapse — else a lookup fetches the WRONG dir's component and MISATTRIBUTES pass/fail. The fix gates the
    // shared precompile on a single parent dir; a multi-dir tree falls back per-file. Assert each file runs
    // its OWN test: d1/t passes (t_one), d2/t fails via its OWN trap (t_two) — not a wrong-component error.
    let root = std::env::temp_dir().join(format!("cdz-collide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2")).unwrap();
    std::fs::write(
        root.join("d1/t.cdz"),
        "@test\ndef t_one() =\n  if 1 == 1 then unit else trap(\"d1\")\n",
    )
    .unwrap();
    std::fs::write(
        root.join("d2/t.cdz"),
        "@test\ndef t_two() =\n  trap(\"d2 fails on its own\")\n",
    )
    .unwrap();
    let (_ok, out) = run_test(&root);
    assert!(
        out.contains("PASS t_one"),
        "d1/t runs its own passing test:\n{out}"
    );
    assert!(
        out.contains("FAIL t_two"),
        "d2/t runs its OWN failing test (not a wrong-component lookup):\n{out}"
    );
    // The tell of the bug was a "component exports no function `t-two`" error (fetched d1's component). Its
    // ABSENCE confirms d2 ran its own component.
    assert!(
        !out.contains("exports no function"),
        "no wrong-component reuse (the stem-collision failure mode):\n{out}"
    );
}
