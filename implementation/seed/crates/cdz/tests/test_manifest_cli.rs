//! End-to-end tests for `cdz test` over a FILE, a DIRECTORY, and a `Project.cdz` MANIFEST.
//!
//! `cdz test` runs a file's `@test` definitions. This suite covers the driver's file-resolution:
//! a single file runs its own tests; a directory holding a `Project.cdz` runs the manifest's declared
//! `tests`; a directory WITHOUT a manifest walks every source file; and a `Project.cdz` arg runs the
//! manifest directly. The manifest is ordinary Cadenza — well-known top-level `def`s (`name`/`entry`/
//! `modules`/`tests`) read straight from the arena, comments-and-all.
//!
//! Drives the built `cdz` binary, over temp files, so it proves the whole compile→run→report path. `cdz
//! test` runs each test IN-PROCESS (wasmtime + the runner are linked into `cdz` via the `cdz-run`
//! library), so — unlike the earlier subprocess design — NO sibling `cdz-run` binary needs to be built
//! for these tests. That is exactly the one-binary win: a bare `cargo test --workspace` on a fresh
//! checkout can run `cdz test` without first building any other workspace binary.

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

/// Whether the value-heap runtime STORE is present beside the `cdz` binary (`<target>/cadenza-store`,
/// where `cdz test` looks by default). CI's bare `test (ubuntu+macos)` job runs `cargo test --workspace`
/// with NO `cargo xtask build`, so there is no store — and a `@test` whose body touches a HEAP value
/// (a runtime `List`/`Set`/`Map`/`Record` — every property test over a generated collection) TRAPS
/// without it (`staging-sync-loop-harness-trap`). A heap-dependent test SKIPS (returns green) when this
/// is false, so the storeless `test` job cannot red on it; the store-having `gate` + `@test suites` jobs
/// still exercise it fully. This mirrors the `let Some(store) else return` guard the heap-value CLI
/// tests already use.
fn store_present() -> bool {
    // Resolve the store dir the SAME way the runtime resolver does (`CADENZA_STORE` first, else the
    // physical `<target>/cadenza-store`), so this guard AGREES with a storeless rerun's mechanism: xtask's
    // storeless-CI rerun sets `CADENZA_STORE=<empty temp dir>`, and this guard must then report ABSENT so
    // the runtime-driving test SKIPS — exactly as it does in the real storeless CI job. Checking only the
    // physical dir would ignore that env and wrongly report PRESENT (the physical store still exists),
    // running the test → the resolver finds no runtime → a false-positive red. `CARGO_BIN_EXE_cdz` =
    // `<target>/<profile>/cdz`; the physical store sits at `<target>/cadenza-store` (two parents up).
    if let Ok(dir) = std::env::var("CADENZA_STORE") {
        return std::path::Path::new(&dir).is_dir()
            && std::fs::read_dir(&dir)
                .map(|mut e| e.next().is_some())
                .unwrap_or(false);
    }
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
        .parent()
        .and_then(|d| d.parent())
        .map(|t| t.join("cadenza-store").exists())
        .unwrap_or(false)
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
fn cdz_test_runs_in_process_with_no_sibling_runner_on_path() {
    // The one-binary guarantee for `cdz test`: it runs each test IN-PROCESS (wasmtime linked in via the
    // `cdz-run` library), so it must work with NO `cdz-run` binary discoverable — not beside `cdz`, not on
    // `PATH`. Drive `cdz` with an EMPTIED `PATH` from a scratch cwd: were it still shelling out to a
    // sibling `cdz-run`, the spawn would fail to locate it and the run would error. A scalar test (no heap
    // value) needs no runtime store, so this holds even on a storeless bare checkout.
    let d = dir("in-process");
    let f = write(
        &d,
        "m.cdz",
        "@test def scalar_ok() = if 1 + 1 == 2 then unit else trap(\"math\")\n",
    );
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(["test", &f])
        .env("PATH", "") // no `cdz-run` findable anywhere — proves in-process execution
        .current_dir(&d)
        .output()
        .expect("spawn cdz");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "cdz test runs in-process with no cdz-run on PATH: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS scalar_ok") && stdout.contains("1 passed, 0 failed"),
        "the in-process run reports the test result: {stdout}"
    );
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
fn a_parse_broken_sibling_def_fails_the_run_not_a_false_green() {
    // SYSTEMIC gate fix (v-syntax's 76-min-block post-mortem, routed by concierge): a def that fails to
    // PARSE is RECOVERED by the reader (errors printed, truncated arena), so the defs that DID parse still
    // run and the suite reported "N passed, 0 failed" — the parse-broken sibling silently absent, landing
    // GREEN. `cdz test` now gates on `cdz check` clean FIRST: any parse/check error → FAIL (non-zero), the
    // suite does NOT run, and its stderr says so (not a green summary). Here a good `@test` sits beside a
    // def with an unclosed paren; the run must be RED and must NOT print a passed-count.
    let d = dir("parse-broken");
    let f = write(
        &d,
        "m.cdz",
        "@test def good() = if 1 == 1 then unit else trap(\"g\")\n\
         def broken() -> Int64 = (1 + 2\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f]);
    assert!(
        !ok,
        "a parse error in a sibling def must FAIL the run: {stdout}{stderr}"
    );
    assert!(
        stderr.contains("NOT running the suite"),
        "the run is gated on check-clean and says it didn't run the suite: {stderr}"
    );
    assert!(
        !stdout.contains("passed,"),
        "must NOT report a green `N passed` summary for a parse-broken project: {stdout}"
    );
}

/// A single explicit `cdz test <file>` that contributes ZERO tests must PRINT a "0 tests found" hint, not
/// exit silently — otherwise a file whose only marker is an UNRECOGNIZED test-ish annotation (`@property`,
/// which is silently stripped, so its def is NOT a `@test`) is dead + "green" by omission (breaker's
/// silent-no-op finding). The hint points at `@test` (the property spelling; `@property` is not a supported
/// annotation — operator ruling). Still exit 0 (an empty file is not a failure). No store needed — the file
/// has no test to run.
#[test]
fn a_single_file_with_zero_tests_prints_a_hint_not_silence() {
    let d = dir("zero-tests");
    // `@property` is silently stripped (not in KNOWN_ANNOTATIONS), so this file has NO `@test` → 0 tests.
    let f = write(
        &d,
        "m.cdz",
        "@property def add_comm(a: Int64, b: Int64) = if a + b == b + a then unit else trap(\"nc\")\n",
    );
    let (ok, stdout, _) = run(&["test", &f]);
    assert!(ok, "a zero-test file is not a failure (exit 0): {stdout}");
    assert!(
        stdout.contains("0 tests found"),
        "a single file with no tests prints a hint, not silence: {stdout:?}"
    );
    // The hint names `@test` as the fix (a parameterized @test is the property spelling).
    assert!(
        stdout.contains("@test"),
        "the 0-tests hint points at the @test annotation: {stdout}"
    );
}

#[test]
fn a_test_compile_decline_reports_the_source_location() {
    // A `cdz test` compile DECLINE (here: an invalid-kebab `@test` name — `small-5`'s `-5` segment is a
    // digit-led boundary segment, CDZ0201) must report `file:line:col: error [CODE]: …` — the SAME located
    // shape `cdz check` uses — not a bare `cdz: error [CODE]: …` that drops the anchor. `cdz test` holds the
    // program's source + span table, and the diagnostic carries a node, so the reporter maps it to a
    // location (report_errors_located). Pins the fix for the dropped-span gap v-diagnostics flagged.
    let d = dir("decline-loc");
    let f = write(&d, "m.cdz", "@test def small-5() = unit\n");
    let (ok, _stdout, stderr) = run(&["test", &f]);
    assert!(
        !ok,
        "an invalid-kebab @test name declines (non-zero exit): {stderr}"
    );
    assert!(
        stderr.contains("[CDZ0201]"),
        "the decline carries its code: {stderr}"
    );
    // The located shape: the diagnostic is anchored at the file with a line:col, not the bare `cdz:` prefix.
    assert!(
        stderr.contains(&format!("{f}:")) && !stderr.trim_start().starts_with("cdz: error"),
        "the decline reports file:line:col (not a span-dropped `cdz: error …`): {stderr}"
    );
    let _ = std::fs::remove_dir_all(&d);
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

    // An unknown tag selects nothing — a vacuously green run (0 tests, still exit 0) — but the hint must
    // point at the TAG (not a missing `@test`): the file is full of tests the filter excluded, so blaming
    // `@test` would send a user who typo'd `--tag` to the wrong fix.
    let (ok, stdout, _) = run(&["test", &f, "--tag", "nope"]);
    assert!(ok, "no matching tag → vacuously green: {stdout}");
    assert!(
        !stdout.contains("PASS "),
        "no test runs under an unknown tag: {stdout}"
    );
    assert!(
        stdout.contains("--tag nope") && !stdout.contains("needs the `@test` annotation"),
        "an unmatched --tag names the tag as the cause, NOT a missing @test: {stdout}"
    );

    // Symmetrically, an unmatched `--filter` blames the FILTER, not a missing `@test`.
    let (ok, stdout, _) = run(&["test", &f, "--filter", "zzznomatch"]);
    assert!(ok, "no matching filter → vacuously green: {stdout}");
    assert!(
        stdout.contains("--filter zzznomatch") && !stdout.contains("needs the `@test` annotation"),
        "an unmatched --filter names the filter as the cause, NOT a missing @test: {stdout}"
    );

    // BOTH selectors present with an empty intersection: `--tag slow` MATCHES (slow-one/both-one carry it)
    // but `--filter zzz` misses. The hint must NOT falsely claim no `@test` carries "slow" (one does) — it
    // names BOTH selectors and their empty intersection, so a user isn't misdirected to the wrong cause.
    let (ok, stdout, _) = run(&["test", &f, "--tag", "slow", "--filter", "zzznomatch"]);
    assert!(
        ok,
        "empty tag∩filter intersection → vacuously green: {stdout}"
    );
    assert!(
        stdout.contains("--tag slow")
            && stdout.contains("--filter zzznomatch")
            && !stdout.contains("no `@test` carries that `@tag(\"slow\")`"),
        "a matching-tag + missing-filter run names BOTH selectors, not a false 'no @test carries slow': \
         {stdout}"
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
     \x20 | gen-int : Unit -> Int64\n\
     \x20 | fail : String -> Unit\n\
     def assert(cond, msg: String) =\n\
     \x20 if cond then unit else host Test in (Test.fail(msg); trap(\"assertion failed\"))\n\
     @test def refl() = host Test in (let n = Test.gen-int() in assert(n == n, \"int equals itself\"))\n\
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
        \x20 | gen-int : Unit -> Int64\n\
        \x20 | fail : String -> Unit\n\
        def assert(cond, msg: String) =\n\
        \x20 if cond then unit else host Test in (Test.fail(msg); trap(\"assertion failed\"))\n\
        @test def always_positive() = host Test in (let n = Test.gen-int() in assert(n > 0, \"n should be positive\"))\n";
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

/// When a property's BODY TRAPS (rather than returning false), the FAIL message names WHY — it surfaces the
/// trap reason ("body trapped: …") so an author can tell a trapped body apart from a genuinely false
/// property. The load-bearing case (breaker 2026-07-18): a mathematically-TRUE property over full-domain
/// `Int64` whose unguarded `+` OVERFLOWS on two large samples reports an integer-overflow trap, NOT a
/// commutativity failure. Before this, a trapping body reported a bare `FAIL name` with no reason (the
/// runner recovered a message only from a `Test.fail` host op; a raw trap's reason was discarded). No store
/// needed — the trap fires in scalar arithmetic before any heap value.
#[test]
fn a_property_whose_body_traps_reports_the_trap_reason_not_a_bare_fail() {
    let d = dir("proptest-body-trap");
    // Commutativity is always true, but a full-domain Int64 generator samples large values whose SUM
    // overflows Int64 — the checked `+` traps before `==`. The FAIL must name the overflow, not imply the
    // property is false. (No @requires bound here on purpose — that is exactly the overflow-prone shape.)
    let src = "@test def add_commutes(a: Int64, b: Int64) = if a + b == b + a then unit else trap(\"noncomm\")\n\
               @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n";
    let f = write(&d, "m.cdz", src);
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        !ok,
        "the overflow-trapping property fails the run: {stdout}{stderr}"
    );
    // The FAIL names the trap reason (an integer overflow), distinguishing a trapped body from a false property.
    assert!(
        stdout.contains("FAIL add_commutes")
            && stdout.contains("body trapped")
            && stdout.contains("overflow"),
        "a trapping property body reports the trap reason (overflow), not a bare FAIL: {stdout}"
    );
    // The sibling still runs (a trapping property is per-test, not a file abort).
    assert!(
        stdout.contains("PASS anchor"),
        "a sibling test runs despite the trapping property: {stdout}"
    );
}

#[test]
fn a_non_fail_string_op_before_the_failure_is_not_reported_as_the_assertion_message() {
    // REGRESSION (Copilot PR #481): the runner reads the failure message from the OBSERVED host-op list,
    // but `run_capturing` records EVERY string-carrying host call as `<op>\t<msg>` — not just the failing
    // assertion's. A test that performs a NON-fail string op (a log/note line) before it fails would, under
    // the old "first tab-carrying entry" extractor, get that benign line misreported as its failure
    // message. The extractor must match only a REPORTING op (dotted name ends in `.fail`). Here the test
    // performs `Test.note("…")` THEN fails via `Test.fail("the real reason")`; the reported FAIL message
    // must be the latter. (Both ops live on ONE effect — the compiler emits a single host interface per
    // envelope, so two separate effects can't both be delegated here.)
    let d = dir("notethenfail");
    let src = "effect Test =\n\
        \x20 | note : String -> Unit\n\
        \x20 | fail : String -> Unit\n\
        @test def notes_then_fails() =\n\
        \x20 host Test in (Test.note(\"a benign note line\"); Test.fail(\"the real reason\"); trap(\"assertion failed\"))\n";
    let f = write(&d, "m.cdz", src);
    let (ok, stdout, stderr) = run(&["test", &f]);
    assert!(!ok, "the test fails → non-zero exit: {stdout}{stderr}");
    assert!(
        stdout.contains("FAIL notes_then_fails: the real reason"),
        "the reported message is the Test.fail text, NOT the earlier note line: {stdout}"
    );
    assert!(
        !stdout.contains("a benign note line"),
        "the non-fail note line must not be reported as the failure message: {stdout}"
    );
}

/// F2 (`@exhaustive`): a property test marked `@exhaustive` is driven over its ENTIRE finite input domain
/// (every combination of its bounded scalar parameters) rather than by random sampling — a pass is a PROOF
/// over the domain, and a failure names the exact case. An UNBOUNDED domain (a wide int / float) declines
/// with a narrow-the-type message.
#[test]
fn an_exhaustive_property_is_driven_over_its_whole_domain() {
    // NOTE: even a scalar-only `@exhaustive`/`@test` here EXECUTES its body under `cdz-run`, which resolves
    // the value-heap runtime by content-address from the store — so the `anchor` sibling (`if 1==1 then
    // unit`) FAILS storeless (giving a spurious "0 passed, N failed"). CI's `test` job builds NO store,
    // so guard like the heap-property tests: skip when the store is absent (the store-having `gate` +
    // `@test suites` jobs exercise this fully). See `store_present`.
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — @test bodies execute under the runtime"
        );
        return;
    }
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

    // A COMPOUND param under `@exhaustive` (a collection domain is unbounded) declines cleanly — and does
    // NOT abort the whole file (the sibling `anchor` still runs). Before the fix, `@exhaustive` was not
    // recognized by the generator-synthesis pass, so the compound param hit the export boundary and killed
    // the entire compile.
    let compound = write(
        &d,
        "compound.cdz",
        "@exhaustive def clist(xs: List(Bool)) = if List.len(xs) >= 0 then unit else trap(\"x\")\n\
         @test def anchor4() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &compound]);
    assert!(
        !ok,
        "a compound @exhaustive domain → non-zero exit (declines): {stdout}"
    );
    assert!(
        stdout.contains("FAIL clist-gen")
            && stdout.contains("not supported for a compound parameter"),
        "a compound @exhaustive declines because its generator samples (cannot enumerate a domain): {stdout}"
    );
    assert!(
        stdout.contains("PASS anchor4"),
        "a compound @exhaustive decline does NOT abort the file — the sibling test still runs: {stdout}"
    );

    // A small user-SUM enum is a FINITE domain, but `@exhaustive` still declines it — the decline is not
    // about boundedness (the message must NOT claim the enum is "unbounded") but about the compound
    // generator sampling the pool rather than enumerating. Sexpr form (a user `(type …)` sum). Pins the
    // accurate diagnostic for a bounded-but-compound domain.
    let enom = write(
        &d,
        "enom.sexp",
        "(do (type Color (Red) (Green) (Blue)) \
           (@ exhaustive (def (ce (: c Color)) unit)) (def (anchor6) 1))",
    );
    let (ok, stdout, _) = run(&["test", &enom]);
    assert!(
        !ok,
        "a user-sum @exhaustive declines → non-zero exit: {stdout}"
    );
    assert!(
        stdout.contains("FAIL ce-gen")
            && stdout.contains("not supported for a compound parameter")
            && !stdout.contains("unbounded"),
        "a bounded user-sum @exhaustive declines with the compound-generator reason, NOT a false \
         'unbounded' claim: {stdout}"
    );

    // A MULTI-scalar `@exhaustive` enumerates the full Cartesian PRODUCT of its parameters' domains:
    // `Bool` × `UInt8` = 2 × 256 = 512 cases. Pins that the domain is the product, not a per-parameter
    // sum, and that a modest multi-scalar signature stays within the enumeration cap.
    let multi = write(
        &d,
        "multi.cdz",
        "@exhaustive def bp(a: Bool, v: UInt8) = if v == v then unit else trap(\"x\")\n\
         @test def anchor5() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &multi]);
    assert!(ok, "a multi-scalar @exhaustive passes: {stdout}{stderr}");
    assert!(
        stdout.contains("PASS bp (exhaustive, 512 cases)"),
        "a multi-scalar @exhaustive enumerates the full Bool×UInt8 product (512 cases): {stdout}"
    );

    // A multi-scalar `@exhaustive` whose parameters are EACH individually bounded but whose PRODUCT exceeds
    // MAX_EXHAUSTIVE_CASES declines — a DIFFERENT path from the single-unbounded-param decline above (there
    // `scalar_domain` returns None; here every `scalar_domain` returns Some, and the running `product >
    // MAX_EXHAUSTIVE_CASES` bail fires). `UInt16 × UInt16` = 65536² ≈ 4.29e9 ≫ 100k. Pins that the product
    // accumulator declines a combinatorial blowup rather than trying to build billions of cases (a DoS).
    let product_blowup = write(
        &d,
        "blowup.cdz",
        "@exhaustive def wide2(x: UInt16, y: UInt16) = if x == x then unit else trap(\"x\")\n\
         @test def anchor7() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, _) = run(&["test", &product_blowup]);
    assert!(
        !ok,
        "a product-exceeds-MAX @exhaustive domain → non-zero exit (declines): {stdout}"
    );
    assert!(
        stdout.contains("FAIL wide2") && stdout.contains("BOUNDED input domain"),
        "a UInt16×UInt16 @exhaustive (product ≫ MAX) declines with the narrow-the-type message: {stdout}"
    );
    // NB: the product-exceeds path declines INSTANTLY (it bails before building any cases), so this pin is
    // cheap. A wide-but-under-cap proof (e.g. `UInt16` alone = 65536 cases) would exercise the "accumulator
    // does not prematurely bail" side, but ENUMERATING 65536 real trials costs ~35s wall-clock — too heavy
    // for the gated suite (v-compiler-perf's wall-clock gate). The existing Bool×UInt8=512 proof already
    // covers that the accumulator threads the running product correctly under the cap.

    // `@exhaustive` composes with `@tag`: a `@tag("fast") @exhaustive` test is selected by `--tag fast`
    // and runs exhaustively. Pins that the two annotations stack (independent metadata + run mode).
    let tagged = write(
        &d,
        "tagged.cdz",
        "@tag(\"fast\") @exhaustive def be(a: Bool) = if a then unit else unit\n\
         @test def slowanchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &tagged, "--tag", "fast"]);
    assert!(
        ok,
        "a @tag+@exhaustive test passes under --tag: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS be (exhaustive, 2 cases)") && !stdout.contains("slowanchor"),
        "@exhaustive composes with @tag — --tag fast selects only the exhaustive test: {stdout}"
    );
}

/// `@exhaustive` over a BOUNDED `@invariant` NEWTYPE is a PROOF over its whole in-domain set — not a decline.
/// `Small = S(Int64)` with `@invariant [0,3]` has a 4-value domain; its `-gen` wrapper's param is a
/// single-variant `Sum` whose payload is `IntRange{0,3}`, and the runner ENUMERATES it (driving the wrapper
/// over each `v in 0..=3` via the inverse of the IntRange pool→value map) rather than sampling/declining. A
/// property true across [0,3] PASSES as `(exhaustive, 4 cases)`; a property false inside [0,3] FAILS naming
/// the in-domain value `S(v)`. Store-guarded — constructing the nominal newtype value needs the heap runtime.
#[test]
fn exhaustive_over_a_bounded_invariant_newtype_is_a_proof_over_its_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    // A property TRUE across the whole [0,3] domain → a PROOF over 4 cases.
    let d = dir("exhaustive-invariant-newtype");
    let proof = write(
        &d,
        "proof.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 3))) (type Small (S Int64))) \
           (@ exhaustive (def (p (: x Small)) \
             (match x (((. Small S) v) (if (and (>= v 0) (<= v 3)) unit (trap \"out\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &proof]);
    assert!(
        ok,
        "an @exhaustive over a bounded @invariant newtype proves over its domain: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS p-gen (exhaustive, 4 cases)"),
        "the [0,3] @invariant newtype domain is enumerated as a 4-case proof: {stdout}"
    );
    // A property FALSE inside the domain (traps for v >= 2) → FAIL naming the in-domain value S(2 or 3).
    let fail = write(
        &d,
        "fail.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 3))) (type Small (S Int64))) \
           (@ exhaustive (def (p (: x Small)) \
             (match x (((. Small S) v) (if (< v 2) unit (trap \"big\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &fail]);
    assert!(!ok, "a property false inside the domain fails: {stdout}");
    assert!(
        stdout.contains("FAIL p-gen") && stdout.contains("counterexample: p-gen(S("),
        "a failing exhaustive newtype names the in-domain value S(v), not a raw int: {stdout}"
    );
    // A NON-bounded (one-sided) invariant → the window [0, 1_000_000] exceeds MAX_EXHAUSTIVE_CASES → DECLINE
    // (not a false proof over a truncated domain). Confirms the cap guard on the enumeration path.
    let wide = write(
        &d,
        "wide.sexp",
        "(do (@ (invariant (>= self 0)) (type Nat (N Int64))) \
           (@ exhaustive (def (p (: x Nat)) \
             (match x (((. Nat N) v) (if (>= v 0) unit (trap \"neg\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &wide]);
    assert!(
        !ok,
        "a too-wide invariant domain declines (non-zero): {stdout}"
    );
    assert!(
        stdout.contains("not supported for a compound parameter"),
        "a too-wide @invariant newtype domain declines rather than enumerating millions of cases: {stdout}"
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

    // A FALSE property over a generated list fails with a counterexample + a replay seed. The generator
    // draws a variable-length (0..=3) list, so some trial hits length 3 → `len == 3` traps → the run fails.
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
    // The counterexample shows the CONCRETE shrunk VALUE — a rendered list `[…]` reported as a call to the
    // ORIGINAL test name (`never_three([…])`), NOT the opaque raw driver ints (`generated ints [big64, …]`).
    // This is the operator-visible payoff of shrinking: the minimal failing input, printed. The property
    // `List.len(xs) == 3` only trapped because the generated list is length 3, so the render is a 3-element
    // list of the shrunk element values (each shrunk toward 0).
    assert!(
        stdout.contains("counterexample: never_three([")
            && !stdout.contains("counterexample: generated ints"),
        "the counterexample shows the rendered list VALUE via the original test name, not raw ints: {stdout}"
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

    // G5-COVERAGE: the sum generator must actually REACH a payload-carrying variant, not just always pick
    // the nullary one — a "never a `Var`" property MUST fail within a modest trial count (mirrors the
    // never-true/never-empty coverage cases for Bool/List). This pins the discriminating direction: the
    // variant selection has power, and a FALSE sum property is reported by its `-gen` wrapper name with a
    // replayable seed + a counterexample.
    let sumfail = write(
        &d,
        "sumfail.sexp",
        "(do (type Ty (Var Int64) (Con Bool) (Nil)) \
           (@ test (def (never-var (: t Ty)) \
             (match t (((. Ty Var) n) (trap \"saw Var\")) (((. Ty Con) b) unit) (((. Ty Nil)) unit)))) \
           (def (anchor6) 1))",
    );
    let (ok, stdout, _) = run(&["test", &sumfail, "--seed", "0", "--trials", "30"]);
    assert!(
        !ok && stdout.contains("FAIL never-var-gen"),
        "the sum generator must reach the Var variant so a never-a-Var property fails: {stdout}"
    );
    assert!(
        stdout.contains("counterexample") && stdout.contains("seed 0"),
        "a failing sum property reports a counterexample + replay seed: {stdout}"
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

    // A LONE single-form file — ONE @test def with a compound param and NOTHING else — has no enclosing
    // `do`-block (it parses as the bare annotated def AS the root). The synthesis must still fire. Before
    // this, such a file declined at the compound param's boundary (the pass only handled a `(do …)` root).
    let lone = write(
        &d,
        "lone.cdz",
        "@test def solo(xs: List(Int64)) = if List.len(xs) >= 0 then unit else trap(\"neg\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &lone, "--trials", "6"]);
    assert!(
        ok,
        "a lone single-form compound test passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS solo-gen (6 trials)"),
        "a lone single-form file (no do-block) is property-tested via the synthesized wrapper: {stdout}"
    );
}

/// A `@test` whose compound param has a NON-GENERATABLE LEAF (`(List Char)` — `Char` unsupported) DECLINES
/// CLEANLY PER-TEST and does NOT abort the whole file — a SIBLING test in the same file still runs. Before
/// this, no `-gen` wrapper was synthesized for a non-generatable-leaf compound, so the compound param hit the
/// export boundary and produced a file-level compile error (`type (List Char) has no component boundary
/// representation`, exit 1) that killed every sibling. The fix synthesizes a DECLINING wrapper (a trapping
/// nullary def) so the runner reports a per-test `FAIL charlist-gen` while the sibling PASSES — test
/// isolation (concierge ruling 2026-07-18). No store needed (the declining wrapper traps before any heap use).
#[test]
fn a_nongeneratable_leaf_compound_test_declines_cleanly_and_siblings_run() {
    let d = dir("nongeneratable-leaf-decline");
    let f = write(
        &d,
        "m.cdz",
        "@test def charlist(cs: List(Char)) = if List.len(cs) >= 0 then unit else trap(\"x\")\n\
         @test def sibling_runs() = if 1 == 1 then unit else trap(\"sibling should run\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f]);
    // The run FAILS overall (the un-generatable test declines), but it must NOT be a file-level compile abort.
    assert!(
        !ok,
        "the non-generatable test declines → non-zero exit: {stdout}{stderr}"
    );
    assert!(
        !stderr.contains("no component boundary representation"),
        "the compound param must NOT abort the file at the boundary (declining wrapper intercepts): {stderr}"
    );
    // The KEY property — test isolation: the sibling still RUNS and passes.
    assert!(
        stdout.contains("PASS sibling_runs"),
        "a sibling test still runs despite the un-generatable compound test: {stdout}"
    );
    // The un-generatable test is reported as a clean per-test FAIL (not a silent drop, not a file abort),
    // and NAMES its cause — the declining wrapper performs `Test.fail("… has no property-test-generatable
    // form yet (Char/…, or a compound with such a leaf) — not property-testable …")`, so the author gets an
    // actionable reason, not a bare `body trapped`. (The message covers BOTH the compound-leaf case here and
    // the bare-name-scalar case; it names `Char` in the non-generatable-type list.)
    assert!(
        stdout.contains("FAIL charlist-gen")
            && stdout.contains("not property-testable")
            && stdout.contains("Char"),
        "the non-generatable-leaf compound test declines with an ACTIONABLE per-test FAIL (names the \
         Char-leaf cause): {stdout}"
    );
}

/// The counterexample-VALUE render covers a user SUM parameter: a failing property over a `(type Res (Ok
/// Int64) (Err Int64))` reports the concrete failing VALUE (`never_ok(Ok(0))` — the variant name + its
/// decoded payload) rather than the raw driver ints. The runner classifies the wrapper's param via
/// `proptest_gen::gen_ty_of_wrapper_param` (the SAME `GenTy` the generator was built from), so the sum's
/// variant selection + payload decode mirror the wrapper exactly. Complements the structural List/Tuple/
/// Record renders (pinned in the list-param test). Written in sexpr (a user sum type).
#[test]
fn a_failing_sum_property_reports_the_decoded_variant_value() {
    // The counterexample's SUM payload is decoded by running the `@test` body under the runtime, which
    // needs the content-addressed store; skip when it's absent (the storeless `test` job) — the
    // store-having `gate` + `@test suites` jobs exercise this fully. See `store_present`.
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — @test bodies execute under the runtime"
        );
        return;
    }
    let d = dir("sum-counterexample");
    // `never_ok` traps whenever the generated `Res` is an `Ok` — so a counterexample IS an `Ok(_)`. The
    // render must show `never_ok(Ok(<int>))`, NOT `generated ints [..]`.
    let f = write(
        &d,
        "m.sexp",
        "(do (type Res (Ok Int64) (Err Int64)) \
           (@ test (def (never-ok (: r Res)) \
             (match r (((. Res Ok) v) (trap \"ok\")) (((. Res Err) v) unit)))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "0"]);
    assert!(!ok, "a false sum property fails: {stdout}");
    // The counterexample renders the decoded SUM VALUE via the original test name — a variant ctor with its
    // payload — NOT the opaque raw driver ints.
    assert!(
        stdout.contains("counterexample: never-ok(Ok(")
            && !stdout.contains("counterexample: generated ints"),
        "a failing sum property shows the decoded variant value (Ok(<payload>)), not raw ints: {stdout}"
    );
}

/// The counterexample-VALUE render for a TUPLE and a RECORD parameter — the two positional/named product
/// shapes. A failing property over a `(Tuple Int64 Int64)` renders `p((<a>, <b>))`; over a `(Record (x
/// Int64) (y Bool))` renders `p({x: <a>, y: <b>})` — NOT the raw driver ints. The List + Sum renders are
/// pinned elsewhere (the list-param test / the sum test); this pins Tuple + Record so a regression in either
/// GenTy decode arm is caught. Store-guarded — a compound `-gen` builds a runtime heap value.
#[test]
fn a_failing_tuple_or_record_property_reports_the_decoded_value() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a compound generator builds a heap value"
        );
        return;
    }
    let d = dir("tuple-record-counterexample");
    // A Tuple(Int64, Int64) property that traps when the first element is large — the counterexample must
    // render the concrete pair `p((<a>, <b>))`.
    let tup = write(
        &d,
        "tup.sexp",
        "(do (@ test (def (p (: t (Tuple Int64 Int64))) (if (< (. t 0) 50) unit (trap \"big\")))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &tup, "--seed", "0"]);
    assert!(!ok, "a false tuple property fails: {stdout}");
    assert!(
        stdout.contains("counterexample: p((") && !stdout.contains("generated ints"),
        "a failing tuple property shows the decoded pair p((a, b)), not raw ints: {stdout}"
    );
    // A Record(x: Int64, y: Bool) property — the counterexample must render `p({x: <a>, y: <b>})`.
    let rec = write(
        &d,
        "rec.sexp",
        "(do (@ test (def (q (: r (Record (x Int64) (y Bool)))) (if (< (. r x) 50) unit (trap \"big\")))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &rec, "--seed", "0"]);
    assert!(!ok, "a false record property fails: {stdout}");
    assert!(
        stdout.contains("counterexample: q({x: ") && !stdout.contains("generated ints"),
        "a failing record property shows the decoded record q({{x: …}}), not raw ints: {stdout}"
    );
}

/// END-TO-END: `@invariant`-CONSTRAINED generation. A type-level `@invariant` with a recognized integer
/// RANGE over `it` makes the generator draw ONLY invariant-satisfying values — no wasted reject cycle
/// (operator directive: "invariants inform how random values are generated"). `Percent = Pct(Int64)` with
/// `@invariant(0 <= self <= 100)`: every generated `Percent` has its `Pct` payload in `[0, 100]`, so a property
/// asserting that range PASSES all trials. Before constrained-gen it drew any Int64 (e.g. Pct(195)) and the
/// body trapped. Store-guarded — building the nominal heap value needs the runtime store.
#[test]
fn a_range_invariant_constrains_generation_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-gen");
    // The body asserts the invariant `0 <= v <= 100`; it can only PASS if generation is constrained to it.
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) \
           (@ test (def (p (: x Percent)) \
             (match x (((. Percent Pct) v) (if (and (>= v 0) (<= v 100)) unit (trap \"out of invariant range\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "100"]);
    assert!(
        ok && stdout.contains("PASS p-gen (100 trials)"),
        "a range @invariant constrains generation in-domain so 100 trials pass: {stdout}{stderr}"
    );
}

/// A ONE-SIDED lower-bound `@invariant (>= self 0)` also constrains generation in-domain. REGRESSION: a
/// one-sided bound used to fall through to unconstrained generation (`invariant_int_range` required BOTH
/// ends), so the generator drew negatives; the construct-site `@invariant` trap then rejected each as a
/// spurious out-of-domain counterexample (every seed reported `NN(-1)`). The fix closes a one-sided bound
/// with a generation window (`[0, 1_000_000]`), so every drawn value satisfies `>= 0`. The body asserts
/// `v >= 0` and can only PASS if generation stayed in-domain across all trials.
#[test]
fn a_one_sided_lower_bound_invariant_constrains_generation_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-one-sided");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (>= self 0)) (type NonNeg (NN Int64))) \
           (@ test (def (p (: x NonNeg)) \
             (match x (((. NonNeg NN) v) (if (>= v 0) unit (trap \"negative leaked past the one-sided invariant\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok && stdout.contains("PASS p-gen (100 trials)"),
        "a one-sided lower-bound @invariant constrains generation to [0, WINDOW] so 100 trials pass \
         (regression: it drew negatives → a spurious NN(-1) counterexample): {stdout}{stderr}"
    );
}

/// A FAILING @invariant-constrained property renders its counterexample IN-DOMAIN — the payload decodes
/// through the same IntRange the generator used, NOT the raw driver int. Regression pin: the counterexample
/// DECODER runs at `cdz test` time AFTER strip_annotations removed the `(@ (invariant …) …)` wrapper, so it
/// must re-source the invariant from `db.invariants` (via invariant_of); otherwise a Percent [0,100] property
/// failure rendered a wild raw int like `Pct(-7167677685577955866)` instead of an in-range value. The body
/// traps for v >= 50, so a failing draw is an in-range `Pct(50..100)`.
#[test]
fn a_failing_range_invariant_property_renders_the_counterexample_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-counterexample");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Percent (Pct Int64))) \
           (@ test (def (p (: x Percent)) (match x (((. Percent Pct) v) (if (< v 50) unit (trap \"big\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "3", "--trials", "50"]);
    assert!(
        !ok,
        "the property fails (a draw >= 50 exists in [0,100]): {stdout}"
    );
    // The counterexample must be an IN-RANGE Pct — extract the int and assert it's in [0, 100], NOT a raw
    // out-of-domain driver int. (Regression: pre-fix it rendered Pct(<huge negative>).)
    let ce = stdout
        .lines()
        .find(|l| l.contains("counterexample: p(Pct("))
        .unwrap_or("");
    let n: i64 = ce
        .split("Pct(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);
    assert!(
        (0..=100).contains(&n),
        "the @invariant counterexample decodes IN-DOMAIN (0..=100), not a raw driver int: {stdout}"
    );
}

/// A failing `@invariant`-newtype property shrinks its counterexample to the MINIMAL in-domain failing value
/// (the boundary), not just any in-domain value. `Pct[0,100]`, body traps for `v >= 50` → the minimal failing
/// value is exactly 50. Before decoded-space shrinking the runner reported a coarse witness (e.g. Pct(67/73/88))
/// because the generic shrinker halves the RAW pool int, and the IntRange decode `v = lo + (pool & MAX) % span`
/// is non-monotonic in the raw int — so it could not converge in value space. The decoded-space pass bisects
/// the VALUE toward `lo` (via the invertible pool map), converging to the boundary. Store-guarded.
#[test]
fn a_failing_invariant_property_shrinks_to_the_minimal_in_domain_value() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-shrink-minimal");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64))) \
           (@ test (def (p (: x Pct)) (match x (((. Pct P) v) (if (< v 50) unit (trap \"big\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "0", "--trials", "50"]);
    assert!(
        !ok,
        "the property fails (a draw >= 50 exists in [0,100]): {stdout}"
    );
    // The counterexample must be EXACTLY 50 — the minimal failing value — not a coarse in-domain witness.
    let n: i64 = stdout
        .lines()
        .find(|l| l.contains("counterexample: p(P("))
        .and_then(|l| l.split("P(").nth(1))
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    assert_eq!(
        n, 50,
        "the failing @invariant counterexample shrinks to the MINIMAL in-domain value (the boundary 50), \
         not a coarse witness: {stdout}"
    );
}

/// A refined newtype NESTED inside a compound (`(Tuple Pct Bool)`) also decodes its counterexample IN-DOMAIN.
/// REGRESSION: `reapply_recorded_invariant` (the post-strip decoder-side re-source) used to handle only a
/// TOP-LEVEL `GenTy::Sum` — a refined newtype in a Tuple slot / List element / Record field was left
/// unconstrained on the decode side, so its counterexample rendered a raw out-of-domain driver int (e.g.
/// `P(-2332…)`) even though the GENERATOR (which recurses) drew it in-domain. The fix makes the re-apply
/// RECURSE into compound GenTy shapes, mirroring the generator's `classify_sum` recursion. Here `Pct` is
/// `[0,100]` and the property traps for `x >= 50`, so a failing draw is an in-range `Pct(50..100)`.
#[test]
fn a_failing_property_over_a_nested_refined_newtype_renders_the_counterexample_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-nested-counterexample");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64))) \
           (@ test (def (p (: pair (Tuple Pct Bool))) \
             (match pair ((tuple a b) (match a (((. Pct P) x) (if (< x 50) unit (trap \"big\")))))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "3", "--trials", "50"]);
    assert!(
        !ok,
        "the property fails (a draw >= 50 exists in [0,100]): {stdout}"
    );
    // Extract the nested `P(N)` from the tuple counterexample and assert N is in [0,100], NOT a raw int.
    let ce = stdout
        .lines()
        .find(|l| l.contains("counterexample: p((P("))
        .unwrap_or("");
    let n: i64 = ce
        .split("P(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    assert!(
        (0..=100).contains(&n),
        "a NESTED @invariant newtype counterexample decodes IN-DOMAIN (0..=100), not a raw driver int: {stdout}"
    );
}

/// A refined newtype nested as ANOTHER SUM's PAYLOAD (`type Box (B Pct)`, `Pct = P(Int64) @invariant
/// [0,100]`) decodes its counterexample IN-DOMAIN. This guards the SUM-PAYLOAD recursion arm of
/// `reapply_recorded_invariant` (it recurses into each variant's payload) — a face breaker verified on
/// trunk (`p(B(P(84)))`) that the top-level nested-Tuple pin does not cover. The property traps for x >= 50,
/// so a failing draw is an in-range `B(P(50..100))`.
#[test]
fn a_failing_property_over_a_newtype_wrapped_in_a_sum_renders_the_counterexample_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-sum-payload-counterexample");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64))) \
           (type Box (B Pct)) \
           (@ test (def (p (: bx Box)) \
             (match bx (((. Box B) pc) (match pc (((. Pct P) x) (if (< x 50) unit (trap \"big\")))))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "2", "--trials", "50"]);
    assert!(
        !ok,
        "the property fails (a draw >= 50 exists in [0,100]): {stdout}"
    );
    // Extract the `P(N)` from `p(B(P(N)))` — the lowercase `p(` does not match the uppercase `P(` split, so
    // the single `P(` is `nth(1)` — and assert N is in [0,100].
    let n: i64 = stdout
        .lines()
        .find(|l| l.contains("counterexample: p(B(P("))
        .and_then(|l| l.split("P(").nth(1))
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    assert!(
        (0..=100).contains(&n),
        "a sum-wrapped @invariant newtype counterexample decodes IN-DOMAIN (0..=100): {stdout}"
    );
}

/// A DOUBLY-nested refined newtype — `(Tuple (List Pct) Bool)`, so `Pct` sits under a List under a Tuple —
/// decodes its counterexample IN-DOMAIN, exercising `reapply_recorded_invariant`'s recursion to DEPTH 2.
/// A face breaker verified on trunk (`p(([P(0), P(0), P(99)], true))`) beyond the single-level Tuple pin.
/// The property traps if a list element's `x >= 50`, so a failing element is an in-range `P(50..100)`.
#[test]
fn a_failing_property_over_a_doubly_nested_refined_newtype_renders_the_counterexample_in_domain() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-doubly-nested-counterexample");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (>= self 0) (<= self 100))) (type Pct (P Int64))) \
           (@ test (def (p (: pair (Tuple (List Pct) Bool))) \
             (match pair ((tuple xs b) \
               (match xs ((list) unit) \
                 ((list h .. t) (match h (((. Pct P) x) (if (< x 50) unit (trap \"big\")))))))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "3", "--trials", "50"]);
    assert!(
        !ok,
        "the property fails (a list element >= 50 exists in [0,100]): {stdout}"
    );
    // The counterexample renders `p(([P(N), …], b))`; extract the FIRST in-range element's int and check it.
    let n: i64 = stdout
        .lines()
        .find(|l| l.contains("counterexample: p(([P("))
        .and_then(|l| l.split("P(").nth(1))
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    assert!(
        (0..=100).contains(&n),
        "a doubly-nested @invariant newtype counterexample decodes IN-DOMAIN (0..=100) at depth 2: {stdout}"
    );
}

/// END-TO-END: a MIN-LENGTH `@invariant` constrains a newtype-List to non-empty generation. `NEList = Mk
/// (List Int64)` with `@invariant(< 0 (List.len self))`: every generated `NEList` wraps a NON-EMPTY list, so
/// a property asserting `List.len > 0` PASSES all trials (before the constraint the generator drew the empty
/// list and the body trapped). Store-guarded — building the nominal heap value needs the store.
#[test]
fn a_min_length_invariant_constrains_a_list_non_empty() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-nonempty");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (< 0 (List.len self))) (type NEList (Mk (List Int64)))) \
           (@ test (def (p (: x NEList)) \
             (match x (((. NEList Mk) xs) (if (< 0 (List.len xs)) unit (trap \"generated an empty list\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "50"]);
    assert!(
        ok && stdout.contains("PASS p-gen (50 trials)"),
        "a min-length @invariant generates non-empty lists so 50 trials pass: {stdout}{stderr}"
    );
}

/// END-TO-END: a min-length `@invariant` inside a CONJUNCTION still floors the list length. `NEList = Mk
/// (List Int64)` with `@invariant(and (< 0 (List.len self)) (<= (List.len self) 10))`: the non-empty conjunct
/// must floor generation at length 1 even though it sits in an `(and …)`. REGRESSION: `min_len_for_param`
/// matched only a BARE comparison and missed the conjunction, so generation drew the empty list and every
/// seed reported a spurious `Mk([])` counterexample (the construct-site @invariant trap firing on an
/// out-of-domain draw). The fix descends the conjunction (mirroring the int-range recognizer). Store-guarded.
#[test]
fn a_conjunction_min_length_invariant_constrains_a_list_non_empty() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — building a nominal value needs the store"
        );
        return;
    }
    let d = dir("invariant-conj-nonempty");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (invariant (and (< 0 (List.len self)) (<= (List.len self) 10))) (type NEList (Mk (List Int64)))) \
           (@ test (def (p (: x NEList)) \
             (match x (((. NEList Mk) xs) (if (< 0 (List.len xs)) unit (trap \"generated an empty list past the conjunction\")))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "50"]);
    assert!(
        ok && stdout.contains("PASS p-gen (50 trials)"),
        "a conjunction min-length @invariant floors generation non-empty so 50 trials pass \
         (regression: it drew the empty list → a spurious Mk([]) counterexample): {stdout}{stderr}"
    );
}

/// END-TO-END: a COMPOUND-param `@test` that is ALSO `@requires`/`@ensures`-annotated must SYNTHESIZE its
/// `-gen` wrapper and RUN (before the peel fix, the compound param under the verification wrapper declined at
/// the export boundary — a hard compile error). A PERMISSIVE precondition/postcondition over a `List` param
/// (always true) passes the trial count. This pins the compound `-gen` synthesis under a `@requires`/`@ensures`
/// wrapper end-to-end (the proptest_gen unit test only checks the wrapper appears in the db; this checks it
/// actually runs through `cdz test`). Store-guarded — a compound `-gen` builds a runtime heap value.
#[test]
fn a_compound_param_test_under_a_requires_or_ensures_wrapper_runs() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a compound generator builds a heap value"
        );
        return;
    }
    let d = dir("compound-verified");
    // A permissive @requires (0 <= len, always true) over a List param: synthesizes `f-gen` and passes.
    let req = write(
        &d,
        "req.sexp",
        "(do (@ test (@ (requires (<= 0 (List.len xs))) \
           (def (f (: xs (List Int64))) (if (<= 0 (List.len xs)) unit (trap \"neg\"))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &req, "--trials", "8"]);
    assert!(
        ok && stdout.contains("PASS f-gen (8 trials)"),
        "a compound-param @requires def synthesizes + runs its -gen wrapper: {stdout}{stderr}"
    );
    // A permissive @ensures (0 <= len of the returned list, always true): same — synthesizes + passes.
    let ens = write(
        &d,
        "ens.sexp",
        "(do (@ test (@ (ensures (<= 0 (List.len ret))) (def (g (: xs (List Int64))) xs))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &ens, "--trials", "8"]);
    assert!(
        ok && stdout.contains("PASS g-gen (8 trials)"),
        "a compound-param @ensures def synthesizes + runs its -gen wrapper: {stdout}{stderr}"
    );
}

/// A compound-param `@test` with `@requires` written in the NATURAL precondition-first order — `@requires`
/// OUTER, `@test` inner (`(@ (requires Q) (@ test (def…)))`) — must ALSO synthesize its `-gen` wrapper and
/// run. REGRESSION: `plan_for_item` used to require the OUTERMOST annotation be `test`/`exhaustive`, so an
/// outer `@requires` returned None → no wrapper → the compound param hit the export boundary and ABORTED THE
/// WHOLE FILE (`type (List Int64) has no component boundary representation`, killing sibling tests). The peel
/// now scans the whole annotation stack for a `test`/`exhaustive` marker in ANY order. A permissive
/// `@requires(0 <= len)` (always true) → the property passes its trials. Store-guarded (compound heap value).
#[test]
fn a_requires_outer_test_inner_compound_synthesizes_and_runs() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a compound generator builds a heap value"
        );
        return;
    }
    let d = dir("requires-outer-order");
    // `@requires` OUTER, `@test` inner — the natural precondition-first spelling. Permissive pre (always true).
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (requires (<= 0 (List.len xs))) \
               (@ test (def (p (: xs (List Int64))) (if (<= 0 (List.len xs)) unit (trap \"neg\"))))) \
           (@ test (def (sibling) (if (= 1 1) unit (trap \"s\")))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "8"]);
    assert!(
        ok,
        "a @requires-OUTER compound test synthesizes its wrapper + runs (no boundary abort): {stdout}{stderr}"
    );
    assert!(
        !stderr.contains("no component boundary representation"),
        "the outer-@requires ordering must NOT abort the file at the compound boundary: {stderr}"
    );
    assert!(
        stdout.contains("PASS p-gen (8 trials)") && stdout.contains("PASS sibling"),
        "the outer-@requires compound property runs AND its sibling runs (no file abort): {stdout}"
    );
}

/// A param-level `@requires` MIN-LENGTH on a `(List …)` param CONSTRAINS generation: `@requires(<= 2
/// (List.len xs))` floors the drawn list at length 2, so every trial satisfies the precondition and the
/// enforced (D) body-entry pre never spuriously trips. Before this, `plan_for_item` classified the List with
/// `min_len` 0, so generation drew length-0/1 lists that violated the pre → a spurious `p([])` FAIL. The
/// property asserts `List.len(xs) >= 2` and can only PASS if every drawn list is long enough. Store-guarded
/// (a compound generator builds a heap value).
#[test]
fn a_param_requires_min_length_constrains_list_generation() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a compound generator builds a heap value"
        );
        return;
    }
    let d = dir("requires-min-len-gen");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (requires (<= 2 (List.len xs))) \
               (@ test (def (p (: xs (List Int64))) \
                 (if (<= 2 (List.len xs)) unit (trap \"too short despite requires\"))))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "50"]);
    assert!(
        ok && stdout.contains("PASS p-gen (50 trials)"),
        "a param-level @requires min-length floors List generation so all trials pass (no spurious \
         too-short draw): {stdout}{stderr}"
    );
}

/// A GENUINELY-failing param-level `@requires` min-length property renders a counterexample of the correct
/// IN-DOMAIN LENGTH — replayable, not a bogus `p([])`. The wrapper draws a floored-length list; the shrunk
/// counterexample must DECODE with the same floor. This works because `synthesize` leaves the `(@ (requires
/// …) …)` wrapper INTACT on the neutralized def (neutralizing only the test/exhaustive markers), so strip
/// records the predicate into `db.requires` and `gen_ty_of_wrapper_param` re-applies the min_len floor on the
/// decode side. Property: `@requires(len >= 2)` on a body that ALWAYS traps → the failing value must be a
/// length-≥2 list (renderable + replayable), not the empty list (which violates the requires and can't
/// replay). Store-guarded (compound heap value).
#[test]
fn a_failing_requires_min_length_property_renders_an_in_domain_length_counterexample() {
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a compound generator builds a heap value"
        );
        return;
    }
    let d = dir("requires-min-len-render");
    let f = write(
        &d,
        "m.sexp",
        "(do (@ (requires (<= 2 (List.len xs))) \
               (@ test (def (p (: xs (List Int64))) (trap \"always fails\")))) \
           (def (anchor) 1))",
    );
    let (ok, stdout, _) = run(&["test", &f, "--seed", "0", "--trials", "20"]);
    assert!(!ok, "the always-trapping property fails: {stdout}");
    // The counterexample must be a list of length >= 2 (in-domain), NOT `p([])` — extract + count elements.
    let ce = stdout
        .lines()
        .find(|l| l.contains("counterexample: p(["))
        .unwrap_or("");
    let inner = ce
        .split("p([")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .unwrap_or("");
    let len = if inner.trim().is_empty() {
        0
    } else {
        inner.split(',').count()
    };
    assert!(
        len >= 2,
        "a failing @requires(len>=2) property renders an IN-DOMAIN (len>=2) counterexample, not p([]): {stdout}"
    );
}

// NOTE: the two TESTED-tier `@test @ensures` integration tests (a true postcondition passes over trials; a
// false one fails with a counterexample) MOVED OUT of this file in the `@ensures`-ownership lockstep. `@ensures`
// enforcement — bare AND `@test`-stacked — is now v-verification's `verify_enforce::enforce` pass (it rewrites a
// def body to `(let ((it BODY)) (if Q it (trap)))` BEFORE the test runner sees it). This vertical no longer owns
// the `@test @ensures` postcondition rewrite (its `rewrite_ensures_stacked_tests` pre-pass was deleted here), so
// the postcondition-behavior coverage belongs on v-verification's side once their skip-lift lands. What THIS file
// still covers is the property-GENERATION half (the `-gen` wrapper, scalar/compound params, `@requires` constrained
// generation) — orthogonal to `@ensures` enforcement.

/// CONSTRAINED GENERATION for a `@requires` precondition: a `@test` over a `@requires`-constrained def must
/// generate only IN-DOMAIN inputs, so the (D) body-entry precondition trap never fires and the test is not
/// spuriously failed. `@requires(x >= 0)` on `f` — the generator draws only `x >= 0` (the negative half of
/// the `Int64` range is clamped away), so every trial satisfies the pre and the property passes. Before this,
/// the generator drew a negative `x`, the enforced pre trapped, and the runner reported a spurious `f(-1)`.
/// (v-verification keeps the pre a HARD trap for production callers; the harness generates in-domain so it
/// never trips under test — the agreed division of the @requires seam.)
#[test]
fn a_test_over_a_requires_constrained_def_generates_in_domain_and_passes() {
    let d = dir("requires-gen");
    // `@requires(x >= 0)` on a total-on-its-domain function. With constrained generation every drawn `x` is
    // >= 0, so the precondition holds on every trial and the property passes the full trial count. A second
    // def keeps the ML top level a do-block.
    let f = write(
        &d,
        "m.cdz",
        "@requires(x >= 0)\n\
         def f(x: Int64) = x + 1\n\
         @test def drive() = if f(3) == 4 then unit else trap(\"body\")\n\
         @requires(x >= 0)\n\
         @test def prop(x: Int64) = if x >= 0 then unit else trap(\"generated a negative x despite @requires\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "100"]);
    assert!(
        ok,
        "a @requires-constrained property generates in-domain and passes (no spurious pre-trap): {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS prop (100 trials)"),
        "the @requires(x >= 0) property passes all trials — generation stayed in-domain: {stdout}"
    );
    assert!(
        !stdout.contains("FAIL prop"),
        "the @requires'd property must NOT spuriously fail on a generated out-of-domain draw: {stdout}"
    );
}

/// A RANGE `@requires` (a conjunction) constrains generation to the bounded window: `@requires(x >= 0 and
/// x < 100)` draws only `x` in `[0, 99]`, so a body that traps outside that window never fires. Pins that the
/// conjunction is distilled into both bounds (lo AND hi), not just one.
#[test]
fn a_range_requires_constrains_generation_to_the_window() {
    let d = dir("requires-range");
    let f = write(
        &d,
        "m.cdz",
        "@requires(x >= 0 and x < 100)\n\
         @test def inwindow(x: Int64) = if x >= 0 and x < 100 then unit else trap(\"out of the required window\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--trials", "100"]);
    assert!(
        ok,
        "a range @requires constrains generation to [0,99] so the window property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS inwindow (100 trials)"),
        "the range-@requires property passes all trials — generation stayed within [0,99]: {stdout}"
    );
}

/// The scalar `@requires` recognizer distills the MIRRORED comparison spelling (`lit OP param`, e.g.
/// `0 <= x` / `50 >= x`) as well as the direct `param OP lit` form — `apply_cmp` flips the operator for the
/// mirrored case. `@requires(0 <= x and 50 >= x)` must constrain generation to `[0, 50]` so a body trapping
/// outside that window never fires. Pins the mirrored branch end-to-end (the other tests use the direct
/// spelling only); no store needed (scalar Int param, no heap value).
#[test]
fn a_mirrored_spelling_requires_constrains_generation_to_the_window() {
    let d = dir("requires-mirrored");
    let f = write(
        &d,
        "m.cdz",
        "@requires(0 <= x and 50 >= x)\n\
         @test def inwindow(x: Int64) = if x >= 0 and x <= 50 then unit else trap(\"out of the required window\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "a mirrored-spelling @requires constrains generation to [0,50] so the window property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS inwindow (100 trials)"),
        "the mirrored-@requires property passes all trials — generation stayed within [0,50]: {stdout}"
    );
}

/// A RELATIONAL two-parameter `@requires (< a b)` constrains generation by REJECTION SAMPLING, not a
/// per-param clamp: a relation between two params cannot be satisfied by clamping either in isolation, so the
/// generator re-draws until `a < b` holds. Before this, the two params were drawn independently, `a >= b`
/// occurred, the (D) body-entry precondition trap fired, and the runner reported a spurious `f(0, 0)`. The
/// body always returns, so the ONLY way it can fail is a pre-trap on an out-of-domain draw — a PASS proves
/// every drawn pair satisfied `a < b`. No store needed (scalar Int params). Pins the relational analogue of
/// the single-param constrained-gen fix.
#[test]
fn a_relational_two_param_requires_constrains_generation_by_rejection_sampling() {
    let d = dir("requires-relational");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a < b)\n\
         @test def rel(a: Int64, b: Int64) = if a < b then unit else trap(\"generated a >= b despite @requires(a < b)\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "a relational @requires(a < b) generates only in-domain pairs so the property passes (no spurious pre-trap): {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS rel (100 trials)"),
        "the relational-@requires property passes all trials — every drawn pair satisfied a < b: {stdout}"
    );
    assert!(
        !stdout.contains("FAIL rel"),
        "the relational @requires'd property must NOT spuriously fail on an out-of-domain (a >= b) draw: {stdout}"
    );
}

/// A GENUINELY-failing property over a relational `@requires (< a b)` def must still FAIL — and its
/// counterexample must stay IN-DOMAIN (`a < b`), because the shrink step preserves the relation (it must not
/// shrink `b` toward 0 in a way that makes `a >= b`, which would masquerade the pre-trap as "still fails").
/// The body traps whenever `a < b` (always true in-domain), so it fails on the first trial; the reported
/// counterexample must satisfy `a < b`. Pins that the relation is enforced through generation AND shrinking.
#[test]
fn a_failing_relational_requires_property_reports_an_in_domain_counterexample() {
    let d = dir("requires-relational-fail");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a < b)\n\
         @test def rel(a: Int64, b: Int64) = if a < b then trap(\"always trips in domain\") else unit\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        !ok,
        "a body that traps for every in-domain (a < b) pair must FAIL the property: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("FAIL rel"),
        "the relational property reports a genuine failure: {stdout}"
    );
    // The counterexample line is `rel(A, B)` — parse A and B and assert A < B (the relation held through
    // shrinking, so the reported witness is in-domain, not a spurious out-of-domain pre-trap).
    let cx = stdout
        .lines()
        .find(|l| l.contains("counterexample: rel("))
        .unwrap_or_else(|| panic!("no counterexample line: {stdout}"));
    let inside = cx
        .split("rel(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .unwrap_or_else(|| panic!("malformed counterexample: {cx}"));
    let mut parts = inside.split(',').map(|p| p.trim().parse::<i64>());
    let a = parts
        .next()
        .unwrap()
        .unwrap_or_else(|_| panic!("bad a in {cx}"));
    let b = parts
        .next()
        .unwrap()
        .unwrap_or_else(|_| panic!("bad b in {cx}"));
    assert!(
        a < b,
        "the reported counterexample rel({a}, {b}) must stay IN-DOMAIN (a < b) — shrink preserved the relation: {cx}"
    );
}

/// A CONJUNCTION mixing a single-param RANGE bound with a two-param RELATION — `@requires(x >= 0 and x < y)`
/// — must satisfy BOTH: the range clamps `x >= 0` (per-param `ParamBound`) AND rejection sampling ensures
/// `x < y` (a `Relation`). The two mechanisms compose through the conjunction descent (the range test and the
/// relation test each exercise ONE mechanism; this pins them TOGETHER). A body that traps outside the joint
/// domain never fires, so a PASS proves every drawn pair satisfied both. No store needed (scalar Int params).
#[test]
fn a_requires_mixing_a_range_bound_and_a_relation_satisfies_both() {
    let d = dir("requires-mixed");
    let f = write(
        &d,
        "m.cdz",
        "@requires(x >= 0 and x < y)\n\
         @test def m(x: Int64, y: Int64) = if x >= 0 and x < y then unit else trap(\"out of the joint domain\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "a @requires mixing a range bound (x >= 0) and a relation (x < y) satisfies both so the property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS m (100 trials)"),
        "the mixed range+relation @requires property passes all trials — generation stayed in the joint domain: {stdout}"
    );
}

/// A CHAINED @requires with TWO coupled relations — `@requires(a < b and b < c)` — must satisfy the whole
/// chain at once: rejection sampling re-draws until `a < b` AND `b < c` hold simultaneously (a single relation
/// is easy; two coupled ones are the tighter joint domain, ~1/6 of draws). A body trapping outside the chain
/// never fires, so a PASS proves every drawn triple was strictly increasing. Pins that multiple relations
/// compose (the fuel budget is sufficient for a realistic chain). No store needed (scalar Int params).
#[test]
fn a_chained_requires_with_two_relations_satisfies_the_whole_chain() {
    let d = dir("requires-chain");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a < b and b < c)\n\
         @test def ch(a: Int64, b: Int64, c: Int64) = if a < b and b < c then unit else trap(\"chain violated\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "a chained @requires(a < b and b < c) satisfies both relations by rejection sampling so the property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS ch (100 trials)"),
        "the chained-relation @requires property passes all trials — every drawn triple was strictly increasing: {stdout}"
    );
}

/// An EQUALITY relation between two params — `@requires(a == b)` — is satisfied by PROPAGATION, not rejection
/// (two independent draws are ~never equal, so rejection would exhaust fuel). The generator copies the LEFT
/// param's draw onto the RIGHT so `a == b` holds BY CONSTRUCTION. Before this, `=` was dropped by the
/// recognizer, the two params drew independently, `a != b`, the (D) pre-trap fired, and the runner reported a
/// spurious `e(0, 1)`. A PASS proves every drawn pair was equal. No store needed (scalar Int params).
#[test]
fn an_equality_requires_is_satisfied_by_propagation_not_rejection() {
    let d = dir("requires-eq");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a == b)\n\
         @test def e(a: Int64, b: Int64) = if a == b then unit else trap(\"a != b despite @requires(a == b)\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"x\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "an equality @requires(a == b) propagates a==b so the property passes (no spurious pre-trap): {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS e (100 trials)"),
        "the equality-@requires property passes all trials — every drawn pair was equal by propagation: {stdout}"
    );
    assert!(
        !stdout.contains("FAIL e"),
        "the equality @requires'd property must NOT spuriously fail on an unequal draw: {stdout}"
    );
}

/// A CHAIN of equalities — `@requires(a == b and b == c)` — propagates to a FIXPOINT: all three params take
/// the leftmost's value, so the whole chain holds by construction regardless of the order the relations were
/// recorded. Pins the fixpoint iteration in `propagate_equalities`. No store needed (scalar Int params).
#[test]
fn a_chain_of_equality_requires_propagates_to_a_fixpoint() {
    let d = dir("requires-eq-chain");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a == b and b == c)\n\
         @test def c3(a: Int64, b: Int64, c: Int64) = if a == b and b == c then unit else trap(\"chain not equal\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"x\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok,
        "a chained equality @requires(a == b and b == c) propagates to a fixpoint so the property passes: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS c3 (100 trials)"),
        "the equality-chain @requires property passes all trials — all three params equal by propagation: {stdout}"
    );
}

/// A GENUINELY-failing property under an equality `@requires(a == b)` must still FAIL — and its counterexample
/// must stay IN-DOMAIN (`a == b`), because propagation is re-applied through shrinking (shrinking the left `=`
/// param carries to the right, so the pair stays equal; the (D) pre-trap can't masquerade as "still fails").
/// The body traps whenever `a == b` (always true in-domain), so it fails on the first trial; the reported
/// counterexample must satisfy `a == b`. Pins equality enforcement through generation AND shrinking.
#[test]
fn a_failing_equality_requires_property_reports_an_in_domain_counterexample() {
    let d = dir("requires-eq-fail");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a == b)\n\
         @test def f(a: Int64, b: Int64) = if a == b then trap(\"always trips in domain\") else unit\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"x\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        !ok,
        "a body that traps for every in-domain (a == b) pair must FAIL the property: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("FAIL f"),
        "the equality property reports a genuine failure: {stdout}"
    );
    // The counterexample `f(A, B)` must satisfy A == B — propagation held through shrinking.
    let cx = stdout
        .lines()
        .find(|l| l.contains("counterexample: f("))
        .unwrap_or_else(|| panic!("no counterexample line: {stdout}"));
    let inside = cx
        .split("f(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .unwrap_or_else(|| panic!("malformed counterexample: {cx}"));
    let mut parts = inside.split(',').map(|p| p.trim().parse::<i64>());
    let a = parts
        .next()
        .unwrap()
        .unwrap_or_else(|_| panic!("bad a in {cx}"));
    let b = parts
        .next()
        .unwrap()
        .unwrap_or_else(|_| panic!("bad b in {cx}"));
    assert_eq!(
        a, b,
        "the reported counterexample f({a}, {b}) must stay IN-DOMAIN (a == b) — propagation held through shrink: {cx}"
    );
}

/// The two relation-enforcement strategies COMPOSE in one predicate: `@requires(a == b and b < c)` needs
/// EQUALITY propagation (b := a) AND an ORDER relation (b < c, i.e. a < c after propagation). The generator
/// propagates equalities on every draw BEFORE the order-relation rejection check, so the order test sees the
/// post-propagation values and rejection re-draws until `b < c` also holds. A PASS proves every drawn triple
/// satisfied both. Also checks conjunction-order-independence (the equality recorded after the order relation
/// still propagates first). No store needed (scalar Int params).
#[test]
fn an_equality_and_an_order_relation_compose_in_one_requires() {
    let d = dir("requires-eq-order");
    // Equality first, then order.
    let f = write(
        &d,
        "m.cdz",
        "@requires(a == b and b < c)\n\
         @test def m(a: Int64, b: Int64, c: Int64) = if a == b and b < c then unit else trap(\"eq+order violated\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"x\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        ok && stdout.contains("PASS m (100 trials)"),
        "equality propagation (b := a) composes with order rejection (b < c) so the property passes: {stdout}{stderr}"
    );
    // Order first, then equality — the conjunction order must not matter (propagation always runs first).
    let d2 = dir("requires-order-eq");
    let f2 = write(
        &d2,
        "m.cdz",
        "@requires(b < c and a == b)\n\
         @test def m2(a: Int64, b: Int64, c: Int64) = if a == b and b < c then unit else trap(\"order+eq violated\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"x\")\n",
    );
    let (ok2, stdout2, stderr2) = run(&["test", &f2, "--seed", "0", "--trials", "100"]);
    assert!(
        ok2 && stdout2.contains("PASS m2 (100 trials)"),
        "the same eq+order constraint recorded in reverse conjunction order still passes (propagation runs first): {stdout2}{stderr2}"
    );
}

/// TERMINATION under an UNSATISFIABLE relation: `@requires(a < b and b < a)` can NEVER hold (it implies
/// `a < a`), so rejection sampling can never find an in-domain draw. The generator must NOT loop forever —
/// it is fuel-bounded (`RELATION_FUEL`), so after exhausting fuel it returns the last draw and lets the (D)
/// body-entry precondition trap report honestly. The property under test FAILS (the pre trap fires on the
/// out-of-domain draw), but crucially: (a) the run TERMINATES, and (b) a SIBLING `@test` in the same file
/// still RUNS (the unsatisfiable one does not hang or abort the suite). Pins that fuel exhaustion degrades
/// gracefully rather than wedging the runner. No store needed (scalar Int params).
#[test]
fn an_unsatisfiable_relational_requires_terminates_by_fuel_and_spares_siblings() {
    let d = dir("requires-unsat");
    let f = write(
        &d,
        "m.cdz",
        "@requires(a < b and b < a)\n\
         @test def u(a: Int64, b: Int64) = unit\n\
         @test def sibling() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    // If the generator looped on the unsatisfiable relation, this would hang; the harness `run` wrapper
    // returns, proving termination. The unsatisfiable property FAILs (pre-trap), the sibling PASSes.
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "20"]);
    assert!(
        !ok,
        "an unsatisfiable @requires makes its property FAIL via the honest pre-trap: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("PASS sibling"),
        "a sibling @test still RUNS — the unsatisfiable relation did not hang or abort the suite: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("FAIL u"),
        "the unsatisfiable-relation property reports its (honest) failure: {stdout}"
    );
}

#[test]
fn cdz_test_on_a_compiled_wasm_gives_an_actionable_diagnostic_not_zero_tests_found() {
    // `cdz test foo.wasm` (a COMPILED component passed by mistake) must NOT surface the misleading
    // "0 tests found — add `@test`" (a .wasm has no source to scan) — it should point at the real path:
    // pass the SOURCE file, and `cdz run` is for the compiled component. The inverse of `cdz run` on a
    // source file. Exit non-zero (a usage mistake).
    let d = dir("test-on-wasm");
    let src = write(&d, "m.sexp", "(do (def (main) 0) (export main))");
    let wasm = d.join("m.wasm");
    let (cok, _co, cerr) = run(&["compile", &src, "-o", wasm.to_str().unwrap()]);
    assert!(cok, "compile the fixture: {cerr}");
    let (ok, _out, err) = run(&["test", wasm.to_str().unwrap()]);
    assert!(!ok, "testing a compiled .wasm is a usage error (non-zero)");
    assert!(
        err.contains("is a COMPILED component") && err.contains("SOURCE file"),
        "the diagnostic names the compiled-artifact mistake + the source-file fix: {err}"
    );
    assert!(
        !err.contains("0 tests found"),
        "must NOT leak the misleading '0 tests found' message: {err}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A FAILING scalar `@requires` property must SHRINK to an IN-DOMAIN counterexample — the shrink search
/// (`shrink` in main.rs) must not descend a param BELOW its `@requires` lower bound. Here the property
/// `x >= 100` is FALSE across the whole required window `[10, 99]`, so every trial fails; the shrinker
/// then halves `x` toward 0. The candidate-skip guard must reject every halved candidate below the bound
/// (`< 10`), so the reported counterexample stays in `[10, 99]`. WITHOUT the guard, shrink would descend
/// to values `< 10`; the (D) `@requires` body-entry trap on those out-of-domain inputs would read as
/// "still fails", and the counterexample would be reported as a spurious out-of-domain value (e.g. `0`).
/// This is the shrink-side twin of `a_failing_range_invariant_property_renders_the_counterexample_in_domain`
/// (which pins the compound `-gen` path); this pins the SCALAR `@requires` path's guard.
#[test]
fn a_failing_requires_property_shrinks_to_an_in_domain_counterexample() {
    let d = dir("requires-shrink-in-domain");
    let f = write(
        &d,
        "m.cdz",
        "@requires(x >= 10 and x < 100)\n\
         @test def p(x: Int64) = if x >= 100 then unit else trap(\"x below 100\")\n\
         @test def anchor() = if 1 == 1 then unit else trap(\"a\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "1", "--trials", "50"]);
    assert!(
        !ok,
        "the property is false across the required window [10,99] so it fails: {stdout}{stderr}"
    );
    assert!(stdout.contains("FAIL p"), "the property fails: {stdout}");
    // Extract the shrunk counterexample `p(N)` and assert N stayed IN-DOMAIN [10, 99] — the shrink guard
    // must not have descended below the @requires lower bound (10). Pre-guard it would report N < 10.
    let ce = stdout
        .lines()
        .find(|l| l.contains("counterexample: p("))
        .unwrap_or("");
    let n: i64 = ce
        .split("p(")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(-1);
    assert!(
        (10..=99).contains(&n),
        "the shrunk @requires counterexample stays IN-DOMAIN [10,99], not below the bound: {stdout}"
    );
}

/// A property with THREE parameters is generated and checked across all three. This pins the 3-plus-param
/// property path, which sits adjacent to a NEIGHBORING parser boundary: trunk `fd23c9d09` declines a
/// 3-plus-param s-expr `(def (f a b c) …)`. A property `@test` — ML surface (scalar params) AND the
/// synthesized compound `-gen` wrapper — must NOT be caught by that decline: every param is generated,
/// and a false property yields a full three-tuple counterexample. This covers two faces. Face one:
/// three scalar params yield a genuine overflow counterexample naming all three arguments. Face two:
/// three compound (List) params yield a synthesized `-gen` wrapper that draws all three and runs clean.
#[test]
fn a_three_parameter_property_generates_and_checks_every_parameter() {
    // Running a property (`cdz test --trials`) executes it, which resolves the value-heap runtime; CI's
    // storeless `cargo test --workspace` (no `cargo xtask build`) has no store, so the run declines
    // instead of producing the FAIL/PASS verdict this pins → skip storeless (the store-having gate +
    // `@test suites` jobs exercise it fully). Same guard the sibling runtime-driving tests use.
    if !store_present() {
        eprintln!(
            "skipping: no cadenza-store (storeless test job) — a property run resolves the runtime"
        );
        return;
    }
    let d = dir("proptest-three-params");
    // (a) Three scalar params: commutativity is always true, but a full-domain triple overflows the
    // checked `+`, so it fails with a counterexample that names all three arguments.
    let f = write(
        &d,
        "scalar.cdz",
        "@test def three(a: Int64, b: Int64, c: Int64) = if a + b + c == c + b + a then unit else trap(\"noncomm\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &f, "--seed", "0", "--trials", "100"]);
    assert!(
        !ok,
        "the overflow-prone three-param property fails on a large triple: {stdout}{stderr}"
    );
    assert!(
        stdout.contains("FAIL three")
            && stdout.contains("body trapped")
            && stdout.contains("overflow"),
        "a three-scalar-param property is generated across all three and reports the overflow: {stdout}"
    );
    // The counterexample names a three-argument call `three(_, _, _)` — proof all three were generated,
    // not just the first two (the >2-param decline would have dropped or mis-parsed the third).
    assert!(
        stdout
            .lines()
            .any(|l| l.contains("counterexample: three(") && l.matches(',').count() == 2),
        "the counterexample carries all THREE generated arguments: {stdout}"
    );

    // (b) Three compound (List) params: the synthesized `-gen` wrapper draws all three; the property
    // (len >= 0) is always true, so it passes 100 trials — proving the >2-param compound path synthesizes
    // a wrapper rather than declining.
    let f2 = write(
        &d,
        "lists.cdz",
        "@test def three_lists(xs: List(Int64), ys: List(Int64), zs: List(Int64)) = if List.len(xs) >= 0 then unit else trap(\"neg\")\n",
    );
    let (ok2, stdout2, stderr2) = run(&["test", &f2, "--trials", "100"]);
    assert!(
        ok2,
        "a three-compound-param property synthesizes its `-gen` wrapper and passes: {stdout2}{stderr2}"
    );
    assert!(
        stdout2.contains("PASS three_lists-gen"),
        "the three-List-param property runs via a synthesized generator: {stdout2}"
    );
}

/// A property whose parameter is annotated at an UNRECOGNIZED type declines cleanly, rather than
/// synthesizing a generator that runs to a fabricated value. This pins the property path against trunk
/// `64346a8ca`, which enforces param type annotations: a typed param at a type not in scope declines
/// (CDZ0101) instead of silently running. A property `@test` routes its params through `-gen` synthesis,
/// so the enforcement must reach that path too — an unknown-type param must NOT be fabricated and tested.
/// The sibling with a recognized type still runs, proving the enforcement did not over-reject.
#[test]
fn a_property_parameter_at_an_unrecognized_type_declines_rather_than_fabricating_a_value() {
    let d = dir("proptest-unknown-param-type");
    // A property param at an undeclared type `Nonexistent` — the compiler must decline (CDZ0101), NOT
    // synthesize a generator and run to a fabricated value.
    let bogus = write(
        &d,
        "bogus.cdz",
        "@test def prop_bogus(x: Nonexistent) = if x == x then unit else trap(\"neq\")\n",
    );
    let (ok, stdout, stderr) = run(&["test", &bogus]);
    let out = format!("{stdout}{stderr}");
    assert!(
        !ok,
        "a property param at an unrecognized type declines, it does not run: {out}"
    );
    assert!(
        out.contains("CDZ0101") && out.contains("Nonexistent"),
        "the decline names the unknown type (CDZ0101), not a fabricated-value run: {out}"
    );

    // The recognized-type sibling still runs 100 trials — the enforcement did not over-reject a valid param.
    // Running a property (`cdz test --trials`) executes it, which resolves the value-heap runtime; CI's
    // storeless `cargo test --workspace` (no `cargo xtask build`) has no store, so the run declines instead
    // of producing the PASS verdict. The decline assertion above is a COMPILE-time check (no store), so it
    // stays live everywhere; only this run half is skipped storeless (the store-having gate + `@test suites`
    // jobs exercise it fully). Same guard the sibling runtime-driving tests use.
    if !store_present() {
        eprintln!(
            "skipping the recognized-type run half: no cadenza-store (storeless test job) — a property run resolves the runtime"
        );
        return;
    }
    let okf = write(
        &d,
        "ok.cdz",
        "@test def prop_ok(x: Int64) = if x == x then unit else trap(\"neq\")\n",
    );
    let (ok2, stdout2, stderr2) = run(&["test", &okf]);
    assert!(
        ok2 && stdout2.contains("PASS prop_ok"),
        "a property at a recognized type still runs after the annotation-enforcement change: {stdout2}{stderr2}"
    );
}

/// A `@test` over a BARE-NAME non-generatable concrete scalar (`Rational`/`Char`/`BigInt`/`String`/`Symbol`
/// — a heap/non-boundary scalar with no host boundary form) DECLINES CLEANLY PER-TEST rather than aborting
/// the whole `cdz test` file. Before this, such a param fell through to the export boundary — `Char` at
/// layout, the others at serialize — and the hard `cdz: error: … has no component boundary representation`
/// KILLED THE SIBLING TESTS in the file. Now the runner reports a per-test `FAIL <name>-gen: … not
/// property-testable` and the sibling `anchor` still PASSES. This is the bare-name-scalar twin of the
/// compound-leaf declining path (`a_nongeneratable_leaf_compound_gets_a_declining_wrapper` in proptest_gen).
#[test]
fn a_property_over_a_bare_name_nongeneratable_scalar_declines_per_test_not_a_file_abort() {
    // This RUNS properties (the declining wrapper + the anchor both resolve the runtime via `cdz test`),
    // so it must skip storeless: CI's `cargo test --workspace` has no store → the run declines → the
    // PASS/FAIL verdict asserts would red. The store-having gate + `@test suites` jobs exercise it fully.
    if !store_present() {
        eprintln!("skipping: no cadenza-store (storeless test job) — this test runs properties");
        return;
    }
    let d = dir("proptest-bare-nongeneratable-scalar");
    // `anchor` FIRST so a file-abort (the pre-fix behavior) would visibly prevent its PASS from appearing.
    let src = "@test def anchor() = if 1 == 1 then unit else trap(\"a\")\n\
               @test def prop_rat(r: Rational) = if r == r then unit else trap(\"neq\")\n";
    let f = write(&d, "m.cdz", src);
    let (ok, stdout, stderr) = run(&["test", &f]);
    let out = format!("{stdout}{stderr}");
    // The run is non-zero (the declining property is reported as a FAIL), but it did NOT hard-abort: the
    // sibling ran and the decline is a per-test FAIL naming its cause, not a `cdz: error:` boundary abort.
    assert!(
        !ok,
        "the declining property is reported as a per-test FAIL (non-zero run): {out}"
    );
    assert!(
        stdout.contains("PASS anchor"),
        "the SIBLING test still runs — the non-generatable scalar did NOT abort the whole file: {out}"
    );
    assert!(
        stdout.contains("FAIL prop_rat-gen") && stdout.contains("not property-testable"),
        "the non-generatable scalar param declines per-test with a named reason, not a boundary abort: {out}"
    );
    assert!(
        !out.contains("has no component boundary representation"),
        "the hard boundary-abort error must NOT surface — it is replaced by the per-test decline: {out}"
    );
}
