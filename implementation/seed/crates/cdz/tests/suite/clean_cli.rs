//! End-to-end tests for `cdz clean [dir]` — remove a project's build artifacts (the `cargo clean`
//! analogue). Safe by construction: it sweeps the manifest directory for build OUTPUTS (`.wasm`/`.rs`/
//! `.dwarf` + `link-map.txt`) — none of which is a Cadenza source extension — and never touches a
//! `.cdz`/`.ml`/`.sexp` source or the `Project.cdz` manifest. These drive the built binary over a real
//! project (build, then clean) and assert the sources survive and the artifacts are gone.

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

/// A two-file project (manifest + entry importing a module). Returns the project dir.
fn temp_project(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-clean-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("Project.cdz"),
        "def name = \"demo\"\ndef entry = \"app.cdz\"\ndef modules = [\"util.cdz\"]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app.cdz"),
        "def go(a: Int64) -> Int64 = a\nexport { go }\n",
    )
    .unwrap();
    std::fs::write(dir.join("util.cdz"), "def u() -> Int64 = 1\nexport { u }\n").unwrap();
    dir
}

#[test]
fn clean_removes_build_artifacts_and_keeps_sources() {
    // Build (wasm), then clean: the produced artifacts (`<export>.wasm`, `link-map.txt`) are removed, and
    // every source + the manifest survives.
    let dir = temp_project("basic");
    let (bok, _bo, be) = run_in(&dir, &["build"]);
    assert!(bok, "build failed: {be}");
    // A component + link-map were produced (named after the export, not the entry stem).
    let wasm_before = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().ends_with(".wasm"));
    assert!(wasm_before, "build produced a .wasm");
    let (ok, out, err) = run_in(&dir, &["clean"]);
    assert!(ok, "cdz clean failed: {err}");
    assert!(
        out.contains("removed"),
        "clean reports what it removed: {out}"
    );
    // Artifacts gone.
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !leftovers.iter().any(|n| n.ends_with(".wasm")
            || n.ends_with(".rs")
            || n.ends_with(".dwarf")
            || n == "link-map.txt"),
        "no build artifacts remain: {leftovers:?}"
    );
    // Sources + manifest survive.
    assert!(dir.join("Project.cdz").is_file(), "manifest kept");
    assert!(dir.join("app.cdz").is_file(), "entry source kept");
    assert!(dir.join("util.cdz").is_file(), "module source kept");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_removes_the_rust_target_output() {
    // A `--target rust` build writes `<export>.rs`; clean removes it (it's a build output, `.rs` is not a
    // Cadenza source extension).
    let dir = temp_project("rust");
    let (bok, _bo, be) = run_in(&dir, &["build", "--target", "rust"]);
    assert!(bok, "rust build failed: {be}");
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".rs")),
        "rust build produced a .rs"
    );
    let (ok, _o, err) = run_in(&dir, &["clean"]);
    assert!(ok, "clean failed: {err}");
    assert!(
        !std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".rs")),
        "the .rs output is removed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_dry_run_lists_without_deleting() {
    // `--dry-run` previews the removals but deletes nothing.
    let dir = temp_project("dryrun");
    let (bok, _bo, be) = run_in(&dir, &["build"]);
    assert!(bok, "build failed: {be}");
    let (ok, out, err) = run_in(&dir, &["clean", "--dry-run"]);
    assert!(ok, "clean --dry-run failed: {err}");
    assert!(out.contains("would remove"), "dry-run lists targets: {out}");
    // The .wasm is still there.
    assert!(
        std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().ends_with(".wasm")),
        "dry-run did not delete the artifact"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_a_project_with_no_artifacts_reports_nothing_to_clean() {
    // A fresh (un-built) project has no artifacts — clean succeeds and says so, deleting nothing.
    let dir = temp_project("empty");
    let (ok, out, err) = run_in(&dir, &["clean"]);
    assert!(ok, "clean of a clean project should succeed: {err}");
    assert!(
        out.contains("nothing to clean"),
        "reports nothing to clean: {out}"
    );
    assert!(dir.join("app.cdz").is_file(), "sources untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_works_on_a_manifest_with_no_entry() {
    // HARDENING: `cdz clean` must NOT require an `entry` — a manifest still being authored (or a
    // library-only layout) has build cruft (link-map.txt, a leftover cdz-run temp) that clean should
    // still remove. Previously `clean` went through the entry-requiring resolver and errored
    // "declares no entry", leaving the cruft un-cleanable. Now it cleans the unambiguous artifacts and
    // still never touches a user file.
    let dir = std::env::temp_dir().join(format!("cdz-clean-noentry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.cdz"), "def name = \"x\"\n").unwrap(); // NO entry
    std::fs::write(dir.join("link-map.txt"), "stale demux").unwrap();
    std::fs::write(dir.join(".cdz-run-foo-1.wasm"), b"leftover temp").unwrap();
    std::fs::write(dir.join("helper.rs"), "fn h() {}\n").unwrap(); // user file — must survive
    let (ok, out, err) = run_in(&dir, &["clean"]);
    assert!(
        ok,
        "clean of an entry-less manifest should succeed, not error: {err}{out}"
    );
    assert!(
        !dir.join("link-map.txt").is_file() && !dir.join(".cdz-run-foo-1.wasm").is_file(),
        "the link-map + run temp are removed even with no entry"
    );
    assert!(
        dir.join("helper.rs").is_file() && dir.join("Project.cdz").is_file(),
        "a user file + the manifest survive"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clean_never_deletes_user_authored_rs_or_wasm_files() {
    // DATA-LOSS REGRESSION (Copilot PR #451): `cdz clean` must remove only THIS project's own emitted
    // outputs (by the compiler's export-derived NAME) + `link-map.txt` + `.cdz-run-*` temps — NEVER a
    // blanket `.rs`/`.wasm`/`.dwarf` sweep, which silently destroyed a user's hand-authored `helper.rs`
    // or a checked-in `asset.wasm`. Build (so the project's own `<export>.wasm` exists), drop user files
    // beside it, clean, and assert the user files SURVIVE while the project's output is gone.
    let dir = temp_project("userfiles"); // entry app.cdz exports `go` → build emits `go.wasm`
    let (bok, _bo, be) = run_in(&dir, &["build"]);
    assert!(bok, "build failed: {be}");
    // User-authored files that must survive (unrelated to the project's `go` output).
    std::fs::write(dir.join("helper.rs"), "fn helper() {}\n").unwrap();
    std::fs::write(dir.join("asset.wasm"), b"a checked-in wasm asset").unwrap();
    std::fs::write(dir.join("notes.dwarf"), b"not really dwarf").unwrap();
    let (ok, _o, err) = run_in(&dir, &["clean"]);
    assert!(ok, "clean failed: {err}");
    // The user files SURVIVE — this is the whole point.
    assert!(
        dir.join("helper.rs").is_file(),
        "a user-authored helper.rs must NOT be deleted"
    );
    assert!(
        dir.join("asset.wasm").is_file(),
        "a checked-in asset.wasm must NOT be deleted"
    );
    assert!(
        dir.join("notes.dwarf").is_file(),
        "an unrelated .dwarf must NOT be deleted"
    );
    // The project's OWN output (go.wasm) + link-map are gone.
    assert!(
        !dir.join("go.wasm").is_file(),
        "the project's own component output is removed"
    );
    assert!(
        !dir.join("link-map.txt").is_file(),
        "link-map.txt is removed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
