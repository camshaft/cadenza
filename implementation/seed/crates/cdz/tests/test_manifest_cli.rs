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

/// Whether the value-heap runtime STORE is present beside the `cdz` binary (`<target>/cadenza-store`,
/// where `cdz test` looks by default). CI's bare `test (ubuntu+macos)` job runs `cargo test --workspace`
/// with NO `cargo xtask build`, so there is no store — and a `@test` whose body touches a HEAP value
/// (a runtime `List`/`Set`/`Map`/`Record` — every property test over a generated collection) TRAPS
/// without it (`staging-sync-loop-harness-trap`). A heap-dependent test SKIPS (returns green) when this
/// is false, so the storeless `test` job cannot red on it; the store-having `gate` + `@test suites` jobs
/// still exercise it fully. This mirrors the `let Some(store) else return` guard the heap-value CLI
/// tests already use.
fn store_present() -> bool {
    // `CARGO_BIN_EXE_cdz` = `<target>/<profile>/cdz`; the store sits at `<target>/cadenza-store` (two
    // parents up, then `cadenza-store`) — the same path `cdz test`'s `default_store` computes.
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
        .parent()
        .and_then(|d| d.parent())
        .map(|t| t.join("cadenza-store").exists())
        .unwrap_or(false)
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

/// `cdz test --tag <t>` runs only the tests whose def carries the `@tag("t")` string tag, AND-composed
/// with `--filter`. Written in the SEXPR canonical form `(@ (tag "t") (@ test (def …)))` — the shape the
/// ML surface `@tag("t")` reifies to (the ML parser change for the call-style annotation is the sibling
/// v-syntax slice; the rcdzc recognition + the runner filter, exercised here, land independently against
/// the canonical form). Covers: no `--tag` runs every test; `--tag` selects only the matching tag; a
/// second tag selects the other; multiple tags on one def are each selectable; `--tag` AND `--filter`
/// intersect; an unknown tag selects nothing (a vacuously green run).
#[test]
fn a_tag_selects_a_subset_and_composes_with_filter() {
    let d = dir("tag");
    // Three tests: two tagged ("slow"/"fast"), one untagged; plus a def carrying TWO tags.
    let f = write(
        &d,
        "m.sexp",
        "(do \
           (@ (tag \"slow\") (@ test (def (slow-one) (if (= 1 1) unit (trap \"s\"))))) \
           (@ (tag \"fast\") (@ test (def (fast-one) (if (= 2 2) unit (trap \"f\"))))) \
           (@ (tag \"slow\") (@ (tag \"net\") (@ test (def (both-one) (if (= 3 3) unit (trap \"b\")))))) \
           (@ test (def (untagged) (if (= 4 4) unit (trap \"u\")))) \
           (export slow-one))",
    );

    // No `--tag`: every test runs (four).
    let (ok, stdout, stderr) = run(&["test", &f]);
    assert!(ok, "untagged run passes all: {stdout}{stderr}");
    assert!(stdout.contains("4 passed, 0 failed"), "{stdout}");

    // `--tag slow`: slow-one + both-one (both carry "slow"); NOT fast-one / untagged.
    let (ok, stdout, _) = run(&["test", &f, "--tag", "slow"]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("PASS slow-one") && stdout.contains("PASS both-one"),
        "both slow-tagged tests run: {stdout}"
    );
    assert!(
        !stdout.contains("fast-one") && !stdout.contains("untagged"),
        "the untagged + differently-tagged tests are skipped: {stdout}"
    );
    assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");

    // `--tag net`: only the multiply-tagged def (a second tag on the same def is selectable).
    let (ok, stdout, _) = run(&["test", &f, "--tag", "net"]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("PASS both-one") && stdout.contains("1 passed, 0 failed"),
        "a second tag on the same def selects it: {stdout}"
    );

    // `--tag slow --filter both`: AND — only both-one (slow-tagged AND name contains "both").
    let (ok, stdout, _) = run(&["test", &f, "--tag", "slow", "--filter", "both"]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("PASS both-one")
            && !stdout.contains("slow-one")
            && stdout.contains("1 passed, 0 failed"),
        "--tag AND --filter intersect: {stdout}"
    );

    // An unknown tag selects nothing — a vacuously green run (0 tests, still exit 0).
    let (ok, stdout, _) = run(&["test", &f, "--tag", "nope"]);
    assert!(ok, "no matching tag → vacuously green: {stdout}");
    assert!(
        !stdout.contains("PASS "),
        "no test runs under an unknown tag: {stdout}"
    );
}

/// The ML SURFACE of test tagging: `@tag("slow")` written in `.cdz` (ML) source — the call-style
/// annotation the front-end reifies to `(@ (tag "slow") def)`, which the runner's `--tag` filter reads.
/// The sibling test above exercises the sexpr canonical form directly; this pins the whole ML path
/// (parser reifies `@tag("…")` → the rcdzc recognition records it → `--tag` selects it), so the surface
/// the operator asked for works end-to-end and stays working.
#[test]
fn the_ml_tag_annotation_surface_selects_a_subset() {
    let d = dir("ml-tag");
    let f = write(
        &d,
        "m.cdz",
        "@tag(\"slow\")\n\
         @test def slow_one() = if 1 == 1 then unit else trap(\"s\")\n\
         @tag(\"fast\")\n\
         @test def fast_one() = if 2 == 2 then unit else trap(\"f\")\n\
         @test def untagged() = if 3 == 3 then unit else trap(\"u\")\n",
    );

    // No `--tag`: all three run.
    let (ok, stdout, stderr) = run(&["test", &f]);
    assert!(ok, "ML-tagged file passes all: {stdout}{stderr}");
    assert!(stdout.contains("3 passed, 0 failed"), "{stdout}");

    // `--tag slow`: only the `@tag("slow")` test — proves the ML surface reifies + is filtered.
    let (ok, stdout, _) = run(&["test", &f, "--tag", "slow"]);
    assert!(ok, "{stdout}");
    assert!(
        stdout.contains("PASS slow_one")
            && !stdout.contains("fast_one")
            && !stdout.contains("untagged")
            && stdout.contains("1 passed, 0 failed"),
        "the `@tag(\"slow\")` ML surface selects only the slow test: {stdout}"
    );
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

/// F2 (`@exhaustive`): a property test marked `@exhaustive` is driven over its ENTIRE finite input domain
/// (every combination of its bounded scalar parameters) rather than by random sampling — a pass is a PROOF
/// over the domain, and a failure names the exact case. An UNBOUNDED domain (a wide int / float) declines
/// with a narrow-the-type message. Scalar-only params, so no value-heap store is needed (like the sampled
/// scalar property tests above — no `store_present` guard).
#[test]
fn an_exhaustive_property_is_driven_over_its_whole_domain() {
    let d = dir("exhaustive");
    // A TRUE property over Bool×Bool (4 cases) — `@exhaustive` reports the case count, not a trial count.
    // (The body is trivially true over the whole domain, exercising the enumeration + report, not a bug.)
    let ok_src = write(
        &d,
        "ok.cdz",
        "@exhaustive def band(a: Bool, b: Bool) = if a then unit else unit\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &ok_src]);
    assert!(
        ok,
        "a true exhaustive Bool×Bool property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS band (exhaustive, 4 cases)"),
        "an @exhaustive Bool×Bool property reports its full 4-case domain: {stdout}"
    );

    // A FALSE property over UInt8 (256 cases) that traps for a specific value → FAIL naming the case.
    // (Name avoids a digit-led kebab segment — `not_200` → `not-200` fails extern-name validation.)
    let bad = write(
        &d,
        "bad.cdz",
        "@exhaustive def avoids_ten(v: UInt8) = if v == 10 then trap(\"hit ten\") else unit\n\
         @test def anchor2() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &bad]);
    assert!(!ok, "a false exhaustive property → non-zero exit: {stdout}");
    assert!(
        stdout.contains("FAIL avoids_ten") && stdout.contains("avoids_ten(10)"),
        "the exhaustive run names the exact failing case (10): {stdout}"
    );

    // An UNBOUNDED domain (Int64) cannot be exhaustively enumerated → FAIL with a narrow-the-type message.
    let unbounded = write(
        &d,
        "unbounded.cdz",
        "@exhaustive def wide(n: Int64) = if n == n then unit else trap(\"x\")\n\
         @test def anchor3() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &unbounded]);
    assert!(
        !ok,
        "an unbounded exhaustive domain → non-zero exit: {stdout}"
    );
    assert!(
        stdout.contains("FAIL wide") && stdout.contains("BOUNDED input domain"),
        "an unbounded @exhaustive domain declines with a narrow-the-type message: {stdout}"
    );
}

/// F1 (compiler-directed collection generators): a `@test` whose parameter is a `(List Int64)` is
/// property-tested by a COMPILER-SYNTHESIZED wrapper — the compiler builds a list from `Test.gen` and
/// calls the test, so a property over a data structure runs over `--trials` generated inputs and shrinks
/// a counterexample, exactly like a scalar property. Before this, a compound param declined at the
/// boundary ("no component boundary representation"). Two files (so the ML root is a `do`-block the pass
/// rewrites): a passing list property + a failing one that reports a counterexample.
#[test]
fn a_list_parameter_test_is_property_tested_by_a_synthesized_generator() {
    // A property over a generated collection builds a runtime HEAP value, which traps without the
    // value-heap store; the storeless CI `test` job has none, so skip there (the store-having `gate` +
    // `@test suites` jobs cover it). See `store_present`.
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — heap property tests need the store"
        );
        return;
    }
    let d = dir("f1-listgen");
    // A TRUE property over any generated `List Int64` (length is non-negative), plus a second def so the
    // ML file's top level is a `do`-block. The synthesized `<name>-gen` wrapper is what runs.
    let f = write(
        &d,
        "m.cdz",
        "@test def len_nonneg(xs: List(Int64)) = if List.len(xs) >= 0 then unit else trap(\"neg\")\n\
         @test def plain() = if 1 == 1 then unit else trap(\"p\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "12"]);
    assert!(
        ok,
        "a true list property + a unit test pass: {stdout}{stderr}"
    );
    // The list property runs as the synthesized generator wrapper over the trial count.
    assert!(
        stdout.contains("PASS len_nonneg-gen (12 trials)"),
        "the List parameter is property-tested via the synthesized wrapper: {stdout}"
    );
    assert!(stdout.contains("PASS plain"), "{stdout}");
    assert!(stdout.contains("2 passed, 0 failed"), "{stdout}");

    // A FALSE property over a generated list fails with a counterexample + a replay seed. The wrapper
    // builds a fixed-length-3 list, so `len == 3` is always true → this asserts-false and fails.
    let bad = write(
        &d,
        "bad.cdz",
        "@test def never_three(xs: List(Int64)) = if List.len(xs) == 3 then trap(\"was three\") else unit\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &bad, "--seed", "0"]);
    assert!(!ok, "a false list property → non-zero exit: {stdout}");
    assert!(
        stdout.contains("FAIL never_three-gen"),
        "the failing list property is reported by its wrapper name: {stdout}"
    );
    assert!(
        stdout.contains("counterexample") && stdout.contains("seed 0"),
        "a counterexample + replay seed are reported: {stdout}"
    );

    // G2: the element type need not be an integer — a `(List Bool)` is generated too (each element from
    // a `Test.gen` int read as a boolean). Pins that the generator recurses over the element kind.
    let bools = write(
        &d,
        "bools.cdz",
        "@test def bools_len(bs: List(Bool)) = if List.len(bs) >= 0 then unit else trap(\"neg\")\n\
         @test def anchor2() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &bools, "--trials", "10"]);
    assert!(ok, "a List Bool property passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS bools_len-gen (10 trials)"),
        "a List Bool parameter is property-tested via the synthesized wrapper: {stdout}"
    );

    // Bool DISTRIBUTION (PR #408): the generated Bool must actually produce `true` values, not almost
    // always `false`. A property that traps the moment it sees a `true` element MUST fail within a modest
    // trial count — with the old `(= gen 0)` mapping `true` appeared only for the exact int 0 (near-never)
    // and this would spuriously pass; the parity mapping `(= (% gen 2) 0)` makes it ~50/50 so `true` shows.
    let boolcov = write(
        &d,
        "boolcov.sexp",
        "(do (@ test (def (never-true (: bs (List Bool))) \
           (match bs ((list) unit) ((list h .. t) (if h (trap \"saw true\") unit))))) \
           (def (anchor4) 1))",
    );
    let (ok, stdout, _) = run(&["test", &boolcov, "--seed", "0", "--trials", "30"]);
    assert!(
        !ok && stdout.contains("FAIL never-true-gen"),
        "the Bool generator must produce `true` (parity, not = 0) so a see-a-true property fails: {stdout}"
    );

    // G3: a `(Tuple …)` parameter is generated too, and nesting composes — a `(List (Tuple Int64 Bool))`
    // is property-tested. Pins the recursive `<gen:T>` over tuple slots + arbitrary nesting.
    let tup = write(
        &d,
        "tup.cdz",
        "@test def pair_ok(p: Tuple(Int64, Bool)) = if p.0 == p.0 then unit else trap(\"p\")\n\
         @test def nested_ok(xs: List(Tuple(Int64, Bool))) = if List.len(xs) >= 0 then unit else trap(\"n\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &tup, "--trials", "8"]);
    assert!(
        ok,
        "a Tuple + nested List(Tuple) property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS pair_ok-gen (8 trials)")
            && stdout.contains("PASS nested_ok-gen (8 trials)"),
        "a Tuple param and a nested List(Tuple) param are both property-tested: {stdout}"
    );

    // G4: a `(Record …)` parameter is generated too (`(record (f <gen>) …)`). Written in the SEXPR
    // canonical form (the record-TYPE ML surface `Record(x: T, …)` is a separate v-syntax concern; the
    // generator recursion over record fields is what this pins).
    let rec = write(
        &d,
        "rec.sexp",
        "(do \
           (@ test (def (rec-ok (: v (Record (x Int64) (y Bool)))) \
             (if (= (. v x) (. v x)) unit (trap \"r\")))) \
           (def (anchor3) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &rec, "--trials", "8"]);
    assert!(ok, "a Record property passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS rec-ok-gen (8 trials)"),
        "a Record parameter is property-tested via the synthesized wrapper: {stdout}"
    );

    // G5: a `@test` over a USER SUM is property-tested — the generator picks a variant by `Test.gen % k`
    // and builds its payload (a mix of payload'd + nullary variants). Sexpr form (a user sum type).
    let sum = write(
        &d,
        "sum.sexp",
        "(do (type Ty (Var Int64) (Con Bool) (Nil)) \
           (@ test (def (ty-ok (: t Ty)) \
             (match t (((. Ty Var) n) unit) (((. Ty Con) b) unit) (((. Ty Nil)) unit)))) \
           (def (anchor5) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &sum, "--trials", "12"]);
    assert!(ok, "a user-sum property passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS ty-ok-gen (12 trials)"),
        "a user-sum parameter is property-tested via the synthesized wrapper: {stdout}"
    );

    // G6: `(Set …)` and `(Map …)` params are generated too (a `Set.of (list …)` / a `Map.insert` fold).
    let setmap = write(
        &d,
        "setmap.cdz",
        "@test def set_ok(s: Set(Int64)) = if Set.len(s) >= 0 then unit else trap(\"s\")\n\
         @test def map_ok(m: Map(Int64, Bool)) = if Map.len(m) >= 0 then unit else trap(\"m\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &setmap, "--trials", "8"]);
    assert!(ok, "Set + Map properties pass: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS set_ok-gen (8 trials)")
            && stdout.contains("PASS map_ok-gen (8 trials)"),
        "a Set and a Map parameter are both property-tested via synthesized wrappers: {stdout}"
    );

    // G7: generated lists are VARIABLE-length (0..=3), so the EMPTY list is reachable — a "never empty"
    // property MUST fail within a modest trial count. With the old fixed-length-3 list this would
    // spuriously PASS (never empty). Pins that the generator exercises the empty + short-list cases.
    let varlen = write(
        &d,
        "varlen.cdz",
        "@test def never_empty(xs: List(Int64)) = if List.len(xs) > 0 then unit else trap(\"empty\")\n\
         @test def anchor6() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &varlen, "--seed", "0", "--trials", "20"]);
    assert!(
        !ok && stdout.contains("FAIL never_empty-gen"),
        "a variable-length list generator reaches the empty list (a never-empty property fails): {stdout}"
    );

    // Multi-parameter: a `@test` with a compound + a scalar param generates BOTH args. The wrapper builds
    // the list and the int and calls `p(xs, n)`.
    let multi = write(
        &d,
        "multi.cdz",
        "@test def two(xs: List(Int64), n: Int64) = if List.len(xs) >= 0 then unit else trap(\"x\")\n\
         @test def anchor7() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &multi, "--trials", "8"]);
    assert!(ok, "a multi-param property passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS two-gen (8 trials)"),
        "a multi-param @test (compound + scalar) is property-tested via a synthesized wrapper: {stdout}"
    );

    // G9: a `Float64`/`Float32` leaf is generated when COMPOUND — a `(List Float64)` param is
    // property-tested (each element `Float64.of-int(Test.gen)`, an integer-valued float that shrinks with
    // the int pool). Before G9 a compound float declined at the boundary. A LONE float already crosses the
    // boundary (the runner generates the scalar), so that path is unchanged — this pins the NESTED case.
    let floats = write(
        &d,
        "floats.cdz",
        "@test def flen(xs: List(Float64)) = if List.len(xs) >= 0 then unit else trap(\"f\")\n\
         @test def tfloat(p: Tuple(Float32, Int64)) = if p.1 == p.1 then unit else trap(\"t\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &floats, "--trials", "8"]);
    assert!(
        ok,
        "a List(Float64) + Tuple(Float32,_) property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS flen-gen (8 trials)")
            && stdout.contains("PASS tfloat-gen (8 trials)"),
        "a compound Float param is property-tested via the synthesized wrapper: {stdout}"
    );
}
