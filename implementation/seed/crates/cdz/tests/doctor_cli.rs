//! End-to-end tests for `cdz doctor` — the toolchain-health preflight (a `cargo`-doctor analogue).
//!
//! `cdz doctor` reports the `cdz` version + path, whether the sibling `cdz-run` runner is present, and
//! whether the value-heap runtime store holds the runtime `cdz` compiles against. It exits non-zero if a
//! component that would break `cdz run`/`cdz test` is missing — so a setup/CI script can gate on it.
//! Drives the built binary. (The `cdz-run` sibling is built alongside `cdz` in the test target, so the
//! runner check is `ok` here; the store checks drive the `--store` override.)

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn doctor_reports_version_and_the_runner() {
    // Always-present facts: the `cdz` version line, its path, and a `cdz-run` runner check LINE. Whether
    // the runner reports `ok` or `MISSING` is ENVIRONMENT-dependent — a local `cargo test` after a full
    // build has `cdz-run` beside `cdz` (→ ok), but CI's bare `cargo test --workspace` does NOT build
    // `cdz-run` first (→ MISSING). So assert the doctor REPORTS the runner check, not a specific verdict.
    let (_ok, out, _err) = run(&["doctor"]);
    assert!(
        out.starts_with("cdz "),
        "reports the cdz version line: {out}"
    );
    assert!(
        out.contains("path:"),
        "reports the cdz executable path: {out}"
    );
    assert!(
        out.contains("cdz-run: ok") || out.contains("cdz-run: MISSING"),
        "reports the sibling-runner check (ok or MISSING per environment): {out}"
    );
}

#[test]
fn doctor_flags_a_missing_runtime_store() {
    // `--store` at a nonexistent path → the runtime store is MISSING and doctor exits non-zero.
    let missing = std::env::temp_dir().join(format!("cdz-doctor-nostore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let (ok, out, err) = run(&["doctor", "--store", missing.to_str().unwrap()]);
    assert!(
        !ok,
        "a missing runtime store is an error (non-zero exit): {out}"
    );
    assert!(
        out.contains("runtime store: MISSING"),
        "reports the store as missing: {out}"
    );
    assert!(
        err.contains("doctor found problem"),
        "summary on stderr: {err}"
    );
}

#[test]
fn doctor_flags_a_store_without_the_required_runtime() {
    // A store DIRECTORY that exists but lacks the required `<hash>.wasm` runtime → STALE, non-zero.
    let empty = std::env::temp_dir().join(format!("cdz-doctor-emptystore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    // A decoy .wasm that is NOT the required runtime hash — must not satisfy the check.
    std::fs::write(empty.join("deadbeef.wasm"), b"not the runtime").unwrap();
    let (ok, out, _err) = run(&["doctor", "--store", empty.to_str().unwrap()]);
    assert!(
        !ok,
        "a store missing the required runtime is an error: {out}"
    );
    assert!(
        out.contains("runtime store: STALE"),
        "reports the store as stale (present, wrong/no runtime): {out}"
    );
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn doctor_json_emits_a_machine_readable_health_object() {
    // `--json` emits one JSON object (version/path/cdz_run/runtime_store/ok) — the CI/setup shape. Here the
    // toolchain is healthy (cdz-run built beside cdz), so `ok` is true and the store status is "ok".
    let (ok, out, _err) = run(&["doctor", "--json"]);
    assert!(ok, "healthy toolchain → success: {out}");
    // Well-formed single-line-ish object with the expected keys.
    assert!(
        out.trim_start().starts_with('{'),
        "emits a JSON object: {out}"
    );
    for key in [
        "\"version\"",
        "\"cdz_run\"",
        "\"runtime_store\"",
        "\"ok\":true",
    ] {
        assert!(out.contains(key), "json has {key}: {out}");
    }
    assert!(out.contains("\"ok\":true"), "healthy → ok:true: {out}");
}

#[test]
fn doctor_json_reports_a_missing_store_as_not_ok() {
    // `--json` with a missing store: the object reports the store status and `ok:false`, and exits non-zero
    // — a CI script can parse the verdict AND gate on the exit code.
    let missing =
        std::env::temp_dir().join(format!("cdz-doctor-json-nostore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let (ok, out, _err) = run(&["doctor", "--json", "--store", missing.to_str().unwrap()]);
    assert!(!ok, "missing store → non-zero exit: {out}");
    assert!(out.contains("\"ok\":false"), "json reports ok:false: {out}");
    assert!(
        out.contains("\"status\":\"missing\""),
        "json reports the store status as missing: {out}"
    );
}
