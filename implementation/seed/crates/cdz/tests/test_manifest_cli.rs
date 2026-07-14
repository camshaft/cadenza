//! End-to-end tests for `cdz test` over a FILE, a DIRECTORY, and a `Project.cdz` MANIFEST.
//!
//! `cdz test` runs a file's `@test` definitions. This suite covers the driver's file-resolution:
//! a single file runs its own tests; a directory holding a `Project.cdz` runs the manifest's declared
//! `tests`; a directory WITHOUT a manifest walks every source file; and a `Project.cdz` arg runs the
//! manifest directly. The manifest is ordinary Cadenza — well-known top-level `def`s (`name`/`entry`/
//! `modules`/`tests`) read straight from the arena, comments-and-all.
//!
//! Drives the built `cdz` binary (which shells to the sibling `cdz-run` to execute each test), over
//! temp files, so it proves the whole compile→run→report path.

use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Run `cdz <args…>` with the process's working directory set to `cwd` — for exercising the no-argument
/// "search up for Project.cdz" path, which is relative to the current directory.
fn run_in(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("cdz-test-manifest-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write");
    p.to_string_lossy().into_owned()
}

/// A module with two `@test` defs — one that passes, one whose pass/fail is caller-chosen.
fn module_src(second_passes: bool) -> String {
    let second = if second_passes { "1 == 1" } else { "1 == 2" };
    format!(
        "def one() = 1\n\
         @test def one_is_one() = if one() == 1 then unit else trap(\"one\")\n\
         @test def second() = if {second} then unit else trap(\"second failed on purpose\")\n"
    )
}

#[test]
fn a_single_file_runs_its_tests() {
    let d = dir("single");
    let f = write(&d, "m.cdz", &module_src(true));
    let (ok, stdout, _) = run(&["test", &f]);
    assert!(ok, "all tests pass → success: {stdout}");
    assert!(stdout.contains("PASS one_is_one"), "{stdout}");
    assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");
}

#[test]
fn a_failing_test_fails_the_run() {
    let d = dir("failing");
    let f = write(&d, "m.cdz", &module_src(false));
    let (ok, stdout, _) = run(&["test", &f]);
    assert!(!ok, "a failing test → non-zero exit");
    assert!(
        stdout.contains("FAIL second"),
        "the failing test is named: {stdout}"
    );
    assert!(stdout.contains("1 passed, 1 failed"), "{stdout}");
}

#[test]
fn a_directory_with_a_manifest_runs_the_declared_tests() {
    // Project.cdz names `m.cdz` as the suite; a SECOND module NOT in `tests` is ignored. Comments in the
    // manifest are tolerated (the reader wraps a leading `//` around the def; the manifest parser peels it).
    let d = dir("manifest");
    write(&d, "m.cdz", &module_src(true));
    write(
        &d,
        "ignored.cdz",
        "@test def should_not_run() = trap(\"must not run\")\n",
    );
    write(
        &d,
        "Project.cdz",
        "// the compiler-ml suite\ndef name = \"demo\"\ndef tests = [\"m.cdz\"]\n",
    );
    let (ok, stdout, stderr) = run(&["test", d.to_str().unwrap()]);
    assert!(ok, "manifest suite passes: {stdout}{stderr}");
    assert!(stdout.contains("PASS one_is_one"), "{stdout}");
    assert!(
        !stdout.contains("should_not_run"),
        "a module not in `tests` must not run: {stdout}"
    );
}

#[test]
fn the_manifest_file_can_be_named_directly() {
    let d = dir("manifest-arg");
    write(&d, "m.cdz", &module_src(true));
    write(&d, "Project.cdz", "def tests = [\"m.cdz\"]\n");
    let manifest = d.join("Project.cdz");
    let (ok, stdout, _) = run(&["test", manifest.to_str().unwrap()]);
    assert!(ok, "naming Project.cdz runs its declared tests: {stdout}");
    assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");
}

#[test]
fn a_directory_without_a_manifest_walks_every_source_file() {
    let d = dir("walk");
    write(&d, "a.cdz", &module_src(true));
    write(
        &d,
        "b.cdz",
        "@test def b_holds() = if 2 > 1 then unit else trap(\"b\")\n",
    );
    let (ok, stdout, _) = run(&["test", d.to_str().unwrap()]);
    assert!(ok, "walked suite passes: {stdout}");
    // Both files' tests ran; the combined TOTAL aggregates across files.
    assert!(
        stdout.contains("PASS one_is_one") && stdout.contains("PASS b_holds"),
        "{stdout}"
    );
    assert!(
        stdout.contains("TOTAL: 3 passed, 0 failed"),
        "combined total: {stdout}"
    );
}

#[test]
fn no_argument_searches_up_for_the_nearest_manifest() {
    // `cdz test` with NO arg finds the nearest `Project.cdz` at or above the working directory (like
    // `cargo test` finding `Cargo.toml`) and runs its suite — from the project root AND from a subdir.
    let d = dir("upward");
    write(&d, "m.cdz", &module_src(true));
    write(&d, "Project.cdz", "def tests = [\"m.cdz\"]\n");
    let sub = d.join("nested/deep");
    std::fs::create_dir_all(&sub).expect("mkdir nested");

    let (ok_root, out_root, err_root) = run_in(&d, &["test"]);
    assert!(
        ok_root,
        "no-arg test from the project root: {out_root}{err_root}"
    );
    assert!(out_root.contains("2 passed, 0 failed"), "{out_root}");

    let (ok_sub, out_sub, err_sub) = run_in(&sub, &["test"]);
    assert!(
        ok_sub,
        "no-arg test from a subdir walks up: {out_sub}{err_sub}"
    );
    assert!(out_sub.contains("2 passed, 0 failed"), "{out_sub}");
}

#[test]
fn no_argument_with_no_manifest_anywhere_errors() {
    // A directory with no `Project.cdz` at or above it → a clear error + non-zero exit (nothing to test).
    let d = dir("no-manifest");
    let (ok, _out, stderr) = run_in(&d, &["test"]);
    assert!(!ok, "no manifest found → non-zero exit");
    assert!(
        stderr.contains("no `Project.cdz` found"),
        "a clear not-found message: {stderr}"
    );
}

#[test]
fn a_filter_selects_a_subset() {
    let d = dir("filter");
    let f = write(&d, "m.cdz", &module_src(true));
    let (ok, stdout, _) = run(&["test", &f, "--filter", "one_is"]);
    assert!(ok, "{stdout}");
    assert!(stdout.contains("PASS one_is_one"), "{stdout}");
    assert!(
        !stdout.contains("second"),
        "the filter excludes `second`: {stdout}"
    );
    assert!(stdout.contains("1 passed, 0 failed"), "{stdout}");
}
