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
use std::sync::OnceLock;

/// `cdz test` shells out to the sibling `cdz-run` binary (kept separate — it carries wasmtime + the
/// runtime store). `cargo test --workspace` builds the crate under test's OWN bin (`CARGO_BIN_EXE_cdz`)
/// but NOT a sibling workspace bin, so `target/<profile>/cdz-run` may be absent under CI's bare
/// `cargo test --workspace` (no artifact-dependency without nightly `-Z bindeps`). Build it ONCE, into
/// the same directory as the `cdz` test binary, before the first spawn — idempotent and cheap when the
/// binary is already current.
fn ensure_cdz_run() {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        // `CARGO_BIN_EXE_cdz` = `<target>/<profile>/cdz`; its grandparent is `<target>`, the dir
        // `cargo build` writes into. `cdz test`'s `locate_cdz_run` looks for `cdz-run` beside `cdz`.
        let cdz = std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"));
        let profile_dir = cdz.parent().expect("cdz exe has a parent dir");
        if profile_dir
            .join(if cfg!(windows) {
                "cdz-run.exe"
            } else {
                "cdz-run"
            })
            .exists()
        {
            return;
        }
        let profile = profile_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("debug");
        // `-p cdz-run`: the test's working directory is the `cdz` crate, so an unqualified
        // `--bin cdz-run` looks for the bin in `cdz` (not found). Select the owning package explicitly.
        let mut cmd = Command::new(env!("CARGO"));
        cmd.args(["build", "-p", "cdz-run", "--bin", "cdz-run"]);
        if profile == "release" {
            cmd.arg("--release");
        }
        let status = cmd.status().expect("spawn cargo build -p cdz-run");
        assert!(
            status.success(),
            "building the sibling cdz-run binary failed"
        );
    });
}

fn run(args: &[&str]) -> (bool, String, String) {
    ensure_cdz_run();
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
    ensure_cdz_run();
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
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("mkdir parent");
    }
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
fn a_manifest_glob_expands_to_matching_files() {
    // `tests = ["src/*.cdz"]` picks up every source file under src/ — a new module drops in with no
    // manifest edit. A single-segment `src/*.cdz` matches src/ only (not a nested dir), and never the
    // manifest itself.
    let d = dir("glob");
    write(&d, "src/a.cdz", &module_src(true));
    write(
        &d,
        "src/b.cdz",
        "@test def b_ok() = if 1 == 1 then unit else trap(\"b\")\n",
    );
    write(
        &d,
        "src/nested/deep.cdz",
        "@test def deep() = trap(\"nested must not match src/*.cdz\")\n",
    );
    write(&d, "Project.cdz", "def tests = [\"src/*.cdz\"]\n");
    let (ok, stdout, stderr) = run(&["test", d.to_str().unwrap()]);
    assert!(ok, "glob suite passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS one_is_one") && stdout.contains("PASS b_ok"),
        "src/*.cdz picks up src/a.cdz + src/b.cdz: {stdout}"
    );
    assert!(
        !stdout.contains("deep"),
        "a single-segment glob must not match src/nested/deep.cdz: {stdout}"
    );
    assert!(stdout.contains("TOTAL: 3 passed, 0 failed"), "{stdout}");
}

#[test]
fn a_double_star_glob_matches_recursively() {
    // `**/*.cdz` matches at any depth.
    let d = dir("doublestar");
    write(&d, "src/a.cdz", &module_src(true));
    write(
        &d,
        "src/nested/deep.cdz",
        "@test def deep_ok() = if 2 > 1 then unit else trap(\"d\")\n",
    );
    write(&d, "Project.cdz", "def tests = [\"**/*.cdz\"]\n");
    let (ok, stdout, _) = run(&["test", d.to_str().unwrap()]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("PASS one_is_one") && stdout.contains("PASS deep_ok"),
        "**/*.cdz reaches a nested file: {stdout}"
    );
}

#[test]
fn an_exclude_removes_a_globbed_file() {
    // A glob sweeps two files; `exclude` drops one (a demo/fixture) — literal AND glob exclude both work.
    let d = dir("exclude");
    write(&d, "src/keep.cdz", &module_src(true));
    write(
        &d,
        "src/skip.cdz",
        "@test def should_skip() = trap(\"must be excluded\")\n",
    );
    write(
        &d,
        "Project.cdz",
        "def tests = [\"src/*.cdz\"]\ndef exclude = [\"src/skip.cdz\"]\n",
    );
    let (ok, stdout, stderr) = run(&["test", d.to_str().unwrap()]);
    assert!(
        ok,
        "excluded file's failing test never runs: {stdout}{stderr}"
    );
    assert!(stdout.contains("PASS one_is_one"), "{stdout}");
    assert!(
        !stdout.contains("should_skip"),
        "the excluded file must not run: {stdout}"
    );

    // The same exclude expressed as a GLOB.
    write(
        &d,
        "Project.cdz",
        "def tests = [\"src/*.cdz\"]\ndef exclude = [\"**/skip.cdz\"]\n",
    );
    let (ok2, stdout2, _) = run(&["test", d.to_str().unwrap()]);
    assert!(
        ok2 && !stdout2.contains("should_skip"),
        "glob exclude: {stdout2}"
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

/// `cdz test` FOLLOWS the entry file's import closure — a test in a module that imports a sibling type
/// links against it and runs, the same closure `cdz check` walks. Before this, `cdz test` compiled the
/// single file alone → `import` was "not modeled" and the imported name was unbound.
#[test]
fn a_test_follows_an_import_and_uses_a_cross_file_type() {
    let d = dir("xfile-import");
    // `lib` owns a sum type + a function over it, exports the type WITH its constructors (`.*`).
    write(
        &d,
        "lib.sexp",
        "(do (type Ty (Var Int64) (Con String)) \
           (def (name-of (: t Ty)) (match t (((. Ty Var) _) \"v\") (((. Ty Con) n) n))) \
           (export (. Ty *)) (export name-of))",
    );
    // `app` imports the type + fn, and its `@test` constructs an imported constructor + calls the fn.
    let app = write(
        &d,
        "app.sexp",
        "(do (import \"lib\" (Ty name-of)) \
           (@ test (def (uses-imported) (if (= (name-of ((. Ty Con) \"z\")) \"z\") unit (trap \"x\")))) \
           (export uses-imported))",
    );
    let (ok, stdout, stderr) = run(&["test", &app]);
    assert!(ok, "cross-file test should pass: {stdout}{stderr}");
    assert!(stdout.contains("PASS uses-imported"), "{stdout}");
    assert!(stdout.contains("1 passed, 0 failed"), "{stdout}");
}

/// In a DIRECTORY run, a library's OWN `@test` runs exactly once (when that library is the entry), not
/// AGAIN through an importer — the entry-file filter keeps each file's tests to that file's run, so the
/// combined total is not inflated by a shared imported module's tests.
#[test]
fn an_imported_library_test_is_not_double_counted() {
    let d = dir("xfile-nodup");
    write(
        &d,
        "lib.sexp",
        "(do (type Ty (Con String)) \
           (def (name-of (: t Ty)) (match t (((. Ty Con) n) n))) \
           (@ test (def (lib-own) (if (= (name-of ((. Ty Con) \"a\")) \"a\") unit (trap \"l\")))) \
           (export (. Ty *)) (export name-of))",
    );
    write(
        &d,
        "app.sexp",
        "(do (import \"lib\" (Ty name-of)) \
           (@ test (def (app-own) (if (= (name-of ((. Ty Con) \"b\")) \"b\") unit (trap \"a\")))) \
           (export app-own))",
    );
    let (ok, stdout, stderr) = run(&["test", d.to_str().unwrap()]);
    assert!(ok, "directory run should pass: {stdout}{stderr}");
    // `lib-own` runs once (lib as entry) and `app-own` once (app as entry): two tests, not three.
    assert!(stdout.contains("PASS lib-own"), "{stdout}");
    assert!(stdout.contains("PASS app-own"), "{stdout}");
    assert!(
        stdout.contains("TOTAL: 2 passed, 0 failed"),
        "lib-own must not be double-counted via app: {stdout}"
    );
}

/// A PROPERTY test — a nullary `@test` that pulls random ints from the runner via the well-known
/// `Test.gen : Unit -> Int64` op — is detected (by the `Test.gen` calls it makes) and run over many
/// trials with a seeded, reproducible int pool. The generator builds its own input from the int stream
/// (bolero's Driver model). A test that pulls NO generated int is a plain unit test (one run). This
/// exercises the whole driver: gen-detection, the seeded pool, and the let-bound-perform single-eval fix
/// (each `Test.gen` binding evaluates exactly once).
fn proptest_src() -> String {
    // `refl` is a true property over any generated int; `plain` pulls no gen (a unit test).
    "effect Test =\n\
     \x20 | gen : Unit -> Int64\n\
     \x20 | fail : String -> Unit\n\
     def assert(cond, msg: String) =\n\
     \x20 if cond then unit else host Test in (Test.fail(msg); trap(\"assertion failed\"))\n\
     @test def refl() = host Test in (let n = Test.gen() in assert(n == n, \"int equals itself\"))\n\
     @test def plain() = assert(1 + 1 == 2, \"1+1 is 2\")\n"
        .to_string()
}

#[test]
fn a_property_test_runs_many_trials_and_a_plain_test_runs_once() {
    let d = dir("proptest");
    let f = write(&d, "p.cdz", &proptest_src());
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "20"]);
    assert!(
        ok,
        "a true property + a passing unit test → success: {stdout}{stderr}"
    );
    // The property test is detected (pulls Test.gen ints) and run over the trial count.
    assert!(
        stdout.contains("PASS refl (20 trials)"),
        "the property test runs the trial count: {stdout}"
    );
    // The gen-less test is a plain unit test — one run, no trial count.
    assert!(
        stdout.contains("PASS plain\n") || stdout.contains("PASS plain "),
        "the non-generating test runs once as a plain unit test: {stdout}"
    );
    assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");
}

#[test]
fn a_false_property_fails_with_a_shrunk_counterexample_and_a_seed() {
    let d = dir("proptest-fail");
    // `n > 0` is false for a generated 0 / negatives — the property fails and shrinks toward 0.
    let src = "effect Test =\n\
        \x20 | gen : Unit -> Int64\n\
        \x20 | fail : String -> Unit\n\
        def assert(cond, msg: String) =\n\
        \x20 if cond then unit else host Test in (Test.fail(msg); trap(\"assertion failed\"))\n\
        @test def always_positive() = host Test in (let n = Test.gen() in assert(n > 0, \"n should be positive\"))\n";
    let f = write(&d, "p.cdz", src);
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0"]);
    assert!(!ok, "a false property → non-zero exit: {stdout}{stderr}");
    assert!(stdout.contains("FAIL always_positive"), "{stdout}");
    // The reported failure message rides through, and the seed is printed to replay.
    assert!(stdout.contains("n should be positive"), "{stdout}");
    assert!(
        stdout.contains("seed 0"),
        "the replay seed is reported: {stdout}"
    );
    // The counterexample is the shrunk int pool — 0 is the minimal failing int for `n > 0`.
    assert!(
        stdout.contains("generated ints [0]"),
        "shrinks to the minimal failing int: {stdout}"
    );
}
