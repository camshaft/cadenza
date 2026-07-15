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
