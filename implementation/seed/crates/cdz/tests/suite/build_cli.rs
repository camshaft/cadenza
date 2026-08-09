//! End-to-end tests for `cdz build` — the manifest-driven compile (the `cargo build` analogue).
//!
//! `cdz build` resolves a project's `Project.cdz` (a directory arg, a manifest-path arg, or — with no
//! arg — an upward search from the cwd, like `cargo build` finding `Cargo.toml`) and compiles the
//! manifest's `entry` file plus its `modules` into one wasm component, with NO per-run flags. These
//! drive the built binary over a temp project (a cross-file package: `app.cdz` imports `util.cdz`).

use std::process::Command;

/// Run `cdz <args…>` (optionally from `cwd`), returning (exit_ok, stdout, stderr).
fn run_in(cwd: Option<&std::path::Path>, args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(d) = cwd {
        cmd.current_dir(d);
    }
    let out = cmd.output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn run(args: &[&str]) -> (bool, String, String) {
    run_in(None, args)
}

/// Write a small cross-file project (manifest + entry `app.cdz` importing module `util.cdz`) into a
/// unique temp dir; returns the dir.
fn temp_project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-build-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"demo\"\ndef entry = \"app.cdz\"\ndef modules = [\"util.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("util.cdz"),
        "def inc(n: Int64) -> Int64 = n + 1\nexport { inc }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "import { inc } from \"util\"\ndef main(a: Int64) -> Int64 = inc(a)\nexport { main }\n",
    )
    .unwrap();
    dir
}

#[test]
fn build_a_project_from_a_directory_arg() {
    // `cdz build <dir>` compiles the manifest's entry + modules into `<entry-stem>.wasm`.
    let dir = temp_project("dir");
    let (ok, _out, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(ok, "cdz build failed: {err}");
    assert!(
        dir.join("main.wasm").is_file(),
        "the entry (app.cdz → main) produces main.wasm: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_from_a_manifest_path_arg() {
    // `cdz build path/to/Project.cdz` builds that project.
    let dir = temp_project("manifest");
    let manifest = dir.join("Project.cdz");
    let out = dir.join("out.wasm");
    let (ok, _o, err) = run(&[
        "build",
        manifest.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert!(ok, "build from manifest path failed: {err}");
    assert!(out.is_file(), "component written to the -o path: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_with_no_arg_searches_up_for_the_manifest() {
    // With no arg, `cdz build` searches up from the cwd for the nearest `Project.cdz` (cargo-style).
    let dir = temp_project("upward");
    let out = dir.join("out.wasm");
    let (ok, _o, err) = run_in(Some(&dir), &["build", "-o", out.to_str().unwrap()]);
    assert!(ok, "no-arg build (upward search) failed: {err}");
    assert!(out.is_file(), "component produced via upward search: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_with_no_manifest_errors() {
    // A directory without a `Project.cdz` is a build error naming the missing manifest, non-zero exit.
    let dir = std::env::temp_dir().join(format!("cdz-build-nomani-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "a dir with no manifest should fail");
    assert!(
        err.contains("Project.cdz") && err.contains("cdz:"),
        "error names the missing manifest: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_build_writes_no_partial_artifact() {
    // Like `cargo build`, a FAILED build must leave NO output — not even the `link-map.txt` companion the
    // compiler produces alongside the (absent) component. Before the fix, an errored directory-mode build
    // still wrote `link-map.txt`, leaving a stray sidecar with no `.wasm` beside it (a confusing partial
    // state). Here the entry exports an invalid-kebab name (`small-5`'s `-5` is a digit-led boundary
    // segment → CDZ0201), so the build fails; assert non-zero exit, the located error, and a CLEAN dir:
    // no `.wasm`, no `link-map.txt`.
    let dir = std::env::temp_dir().join(format!("cdz-build-fail-clean-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"p\"\ndef entry = \"main.sexp\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.sexp"),
        "(module m (def (small-5) unit) (export small-5))",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "the invalid-kebab export must fail the build: {err}");
    assert!(
        err.contains("[CDZ0201]"),
        "the failure reports its diagnostic: {err}"
    );
    assert!(
        !dir.join("link-map.txt").exists(),
        "a FAILED build must not leave the link-map companion behind"
    );
    assert!(
        !dir.join("main.wasm").exists(),
        "a FAILED build must not leave a component behind"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_a_named_manifest_that_does_not_exist_errors_not_dir_walks() {
    // Sibling-consistency with `cdz check`/`cdz test` (PR #422): `cdz build path/to/Project.cdz` when
    // that manifest DOESN'T EXIST must error clearly ("no such file") naming the arg — not resolve to the
    // parent dir and report the confusing "no `Project.cdz` in <parent>". The dir HAS another manifest
    // absent, so this is purely the missing-named-file case.
    let dir = std::env::temp_dir().join(format!("cdz-build-nomani-named-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("Project.cdz");
    let (ok, _o, err) = run(&["build", missing.to_str().unwrap()]);
    assert!(!ok, "naming a non-existent manifest must fail");
    assert!(
        err.contains("no such file"),
        "the error names the missing manifest as a missing file: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_a_manifest_without_an_entry_errors() {
    // A manifest with no `entry` cannot build a component — a clear, actionable error.
    let dir = std::env::temp_dir().join(format!("cdz-build-noentry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def name = \"x\"\n").unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "a manifest with no entry should fail");
    assert!(
        err.contains("entry"),
        "error tells the author to add an `entry`: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_a_manifest_with_a_non_string_entry_says_entry_must_be_a_string() {
    // A manifest that HAS a `def entry` whose value is NOT a string (`def entry = 42`) must NOT report
    // "declares no `entry`" — that misleadingly tells the author to add an entry they already wrote. The
    // real fault is the wrong TYPE, so the error must say `entry` must be a string. Regression: the parser
    // drops a non-string value silently, making `entry` None (indistinguishable from absent) — the
    // `entry_malformed` flag restores the distinction.
    let dir = std::env::temp_dir().join(format!("cdz-build-badentry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"x\"\ndef entry = 42\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "a non-string entry should fail");
    assert!(
        err.contains("`entry` must be a string"),
        "names the wrong-type fault, not a missing entry: {err}"
    );
    assert!(
        !err.contains("declares no `entry`"),
        "must NOT misreport a present-but-mistyped entry as absent: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_with_a_non_string_opt_level_warns_and_uses_the_default() {
    // A `def opt-level` with a non-string value (`def opt-level = 42`) is silently dropped by the parser
    // (manifest_strings yields nothing → opt_level None). Unlike `entry` (required → hard error), opt-level
    // has a safe default, so the build must WARN the setting was ignored (not silently build at the
    // default with no feedback) yet still SUCCEED. Regression: pre-fix it dropped the setting with zero
    // signal — the `opt_level_malformed` flag drives the warning.
    let dir = std::env::temp_dir().join(format!("cdz-build-badopt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"p\"\ndef entry = \"main.cdz\"\ndef opt-level = 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.cdz"),
        "def main() -> Int64 = 1\nexport { main }\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(
        ok,
        "a non-string opt-level must still BUILD (default tier): {err}"
    );
    assert!(
        err.contains("warning") && err.contains("opt-level") && err.contains("not a string"),
        "the build WARNS that opt-level was ignored: {err}"
    );
    assert!(
        dir.join("main.wasm").is_file(),
        "the component is still produced at the default tier: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_with_a_non_string_name_warns_and_falls_back_to_the_directory_name() {
    // A `def name` with a non-string value (`def name = 42`) is silently dropped by the parser
    // (manifest_strings yields nothing → name None). Unlike `entry` (required → hard error), `name` has a
    // safe fallback (the manifest's DIRECTORY name, used for the published `cadenza:<name>/api` interface),
    // so the build must WARN the declared name was ignored (not silently drop it with zero feedback) yet
    // still SUCCEED. Regression: `name` was the one known manifest key with NO malformed-detection — `entry`
    // and `opt-level` had it, `name` didn't; the new `name_malformed` flag drives this warning, matching the
    // opt-level pattern. A standalone build doesn't NEED the name, so this is warn-not-fail.
    let dir = std::env::temp_dir().join(format!("cdz-build-badname-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = 42\ndef entry = \"main.cdz\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.cdz"),
        "def main() -> Int64 = 1\nexport { main }\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(
        ok,
        "a non-string name must still BUILD (dir-name fallback): {err}"
    );
    assert!(
        err.contains("warning") && err.contains("`name`") && err.contains("not a string"),
        "the build WARNS that name was ignored: {err}"
    );
    assert!(
        dir.join("main.wasm").is_file(),
        "the component is still produced under the fallback name: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_with_a_duplicate_manifest_key_warns_last_wins() {
    // A manifest that declares a known key TWICE (`def entry` twice) is last-wins in the parser — the
    // earlier value is silently discarded, which can quietly change WHAT builds. The build must WARN
    // (naming the duplicated key + that the last wins) yet still succeed, building the LAST value. Two
    // entry files exist; the manifest names `first.cdz` then `main.cdz` → builds `main.wasm` + warns.
    let dir = std::env::temp_dir().join(format!("cdz-build-dupkey-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("first.cdz"),
        "def first() -> Int64 = 1\nexport { first }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.cdz"),
        "def main() -> Int64 = 2\nexport { main }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"p\"\ndef entry = \"first.cdz\"\ndef entry = \"main.cdz\"\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(ok, "a duplicate key warns but still builds: {err}");
    assert!(
        err.contains("warning") && err.contains("`entry`") && err.contains("more than once"),
        "warns that `entry` is declared more than once (last-wins): {err}"
    );
    assert!(
        dir.join("main.wasm").is_file() && !dir.join("first.wasm").is_file(),
        "the LAST entry (main.cdz) wins — main.wasm is built, not first.wasm: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_a_glob_entry_matching_one_file_uses_the_resolved_files_name() {
    // REGRESSION (Copilot PR #413): a GLOB `entry` (`app*.cdz`) must derive the compiler entry NAME from
    // the RESOLVED file (`app_main.cdz` → `app_main`), NOT the glob pattern (which would pass an invalid
    // name like `*` and fail package linking). A glob matching exactly one file builds fine.
    let dir = std::env::temp_dir().join(format!("cdz-build-globentry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"app*.cdz\"\n").unwrap();
    std::fs::write(
        dir.join("app_main.cdz"),
        "def main(a: Int64) -> Int64 = a + 1\nexport { main }\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(
        ok,
        "a glob entry matching one file builds (name from the resolved file, not `*`): {err}"
    );
    // The component links (the `main` export → main.wasm); a `*` entry name would have failed linking.
    assert!(
        dir.join("main.wasm").is_file(),
        "the resolved entry compiles + links: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_a_multi_match_entry_glob_is_rejected() {
    // REGRESSION (Copilot PR #413): an entry glob matching MULTIPLE files is ambiguous — a component has
    // ONE boundary — so it must fail with a clear error, not implicitly pick/compile several.
    let dir = std::env::temp_dir().join(format!("cdz-build-multientry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def entry = \"*.cdz\"\n").unwrap();
    std::fs::write(
        dir.join("a.cdz"),
        "def main() -> Int64 = 1\nexport { main }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.cdz"),
        "def helper() -> Int64 = 2\nexport { helper }\n",
    )
    .unwrap();
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "a multi-match entry glob should fail");
    assert!(
        err.contains("matched") && err.to_lowercase().contains("single file"),
        "error explains the entry must be a single file: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A minimal single-file project (manifest naming `app.cdz` + the entry) with an optional extra
/// manifest line (e.g. an `opt-level` field); returns the dir.
fn temp_opt_project(tag: &str, extra_manifest_line: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-build-opt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.cdz"),
        format!("def entry = \"app.cdz\"\n{extra_manifest_line}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def main(a: Int64) -> Int64 = a + 1\nexport { main }\n",
    )
    .unwrap();
    dir
}

#[test]
fn build_release_and_opt_level_flags_build() {
    // `--release` (O2) and `--opt-level O3` are accepted and build (the level threads to the compiler;
    // today the pass pipeline is empty so bytes match, but the flag must be wired + valid).
    let dir = temp_opt_project("flags", "");
    for args in [
        vec!["build", dir.to_str().unwrap(), "--release", "-o"],
        vec!["build", dir.to_str().unwrap(), "--opt-level", "O3", "-o"],
    ] {
        let out = dir.join("o.wasm");
        let mut a = args.clone();
        a.push(out.to_str().unwrap());
        let (ok, _o, err) = run(&a);
        assert!(ok, "build with {args:?} failed: {err}");
        assert!(out.is_file(), "component produced for {args:?}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_reads_the_manifest_opt_level_field() {
    // A `def opt-level = "O2"` manifest field is accepted (parsed via OptLevel::FromStr) and builds.
    let dir = temp_opt_project("manifest", "def opt-level = \"O2\"\n");
    let out = dir.join("o.wasm");
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert!(ok, "build with a manifest opt-level failed: {err}");
    assert!(out.is_file(), "component produced: {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_a_bad_opt_level_flag_naming_the_set() {
    // A bogus `--opt-level` is a clear error naming the valid set — not a silent fallback.
    let dir = temp_opt_project("badflag", "");
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "--opt-level", "O9"]);
    assert!(!ok, "a bad --opt-level should fail");
    assert!(
        err.contains("O9") && err.contains("O0, O1, O2, O3"),
        "error names the valid set: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_a_bad_manifest_opt_level() {
    // A bogus `def opt-level` in the manifest is a clear error (not silently ignored) naming the manifest.
    let dir = temp_opt_project("badmanifest", "def opt-level = \"fast\"\n");
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap()]);
    assert!(!ok, "a bad manifest opt-level should fail");
    assert!(
        err.contains("opt-level") && err.contains("fast"),
        "error names the bad manifest opt-level: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_target_rust_emits_a_rust_module() {
    // `cdz build --target rust` emits a `.rs` module instead of a wasm component; default is wasm.
    let dir = temp_opt_project("targetrust", "");
    let (ok, _o, err) = run(&[
        "build",
        dir.to_str().unwrap(),
        "--target",
        "rust",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "cdz build --target rust failed: {err}");
    assert!(
        dir.join("main.rs").is_file(),
        "the rust target emits main.rs: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_defaults_to_the_wasm_target() {
    // With no --target, `cdz build` emits a wasm component (the default).
    let dir = temp_opt_project("targetwasm", "");
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "-o", dir.to_str().unwrap()]);
    assert!(ok, "default build failed: {err}");
    assert!(
        dir.join("main.wasm").is_file(),
        "default target is wasm: {err}"
    );
    assert!(
        !dir.join("main.rs").is_file(),
        "no rust module without --target rust"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_rejects_an_unknown_target_naming_the_choices() {
    // A bogus --target is a clap value error listing the valid targets.
    let dir = temp_opt_project("targetbad", "");
    let (ok, _o, err) = run(&["build", dir.to_str().unwrap(), "--target", "elf"]);
    assert!(!ok, "a bad --target should fail");
    assert!(
        err.contains("invalid value") && err.contains("wasm") && err.contains("rust"),
        "error lists the valid targets: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
