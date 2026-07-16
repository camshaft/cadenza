//! End-to-end tests for `cdz doctor` — the toolchain-health preflight (a `cargo`-doctor analogue).
//!
//! `cdz doctor` reports the `cdz` version + path, whether the standalone `cdz-run` runner is present
//! (INFORMATIONAL — `cdz run`/`cdz test` run in-process, so it never affects the verdict), and whether
//! the value-heap runtime store holds the runtime `cdz` compiles against. It exits non-zero ONLY when the
//! runtime STORE is missing/stale — the sole toolchain fault that breaks `cdz run`/`cdz test` — so a
//! setup/CI script can gate on it. Drives the built binary. The `cdz-run`/store presence is
//! ENVIRONMENT-dependent (built locally, absent on CI's bare `cargo test`), so the always-on tests assert
//! the doctor REPORTS its checks (not a specific verdict); the verdict-under-a-controlled-input tests
//! drive an explicit `--store <missing>` override.

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
    // the standalone runner reports `present` or `not present` is ENVIRONMENT-dependent — a local
    // `cargo test` after a full build has `cdz-run` beside `cdz` (→ present), but CI's bare `cargo test
    // --workspace` does NOT build `cdz-run` first (→ not present). Either way it is INFORMATIONAL: `cdz
    // run`/`cdz test` run IN-PROCESS, so the standalone binary is optional and never a doctor failure. So
    // assert the doctor REPORTS the runner check, not a specific verdict.
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
        out.contains("cdz-run: present") || out.contains("cdz-run: not present"),
        "reports the standalone-runner check (present or not present per environment): {out}"
    );
}

#[test]
fn doctor_help_describes_cdz_run_as_informational_not_required() {
    // `cdz doctor --help` must describe the standalone `cdz-run` check as INFORMATIONAL/optional — NOT
    // "needed to run/test". Regression: after `cdz run`/`cdz test` moved in-process, the help text (and
    // the doctor's own verdict) stopped depending on `cdz-run`, but the `--help` prose lagged, telling
    // users a missing `cdz-run` breaks run/test. Pin the corrected wording so it can't silently regress.
    let (ok, out, err) = run(&["doctor", "--help"]);
    assert!(ok, "cdz doctor --help should succeed: {err}");
    let help = format!("{out}{err}"); // clap may route --help to stdout
    assert!(
        help.contains("in-process"),
        "help explains run/test are in-process (so cdz-run is optional): {help}"
    );
    assert!(
        !help.contains("needed to run/`test`") && !help.contains("needed to run/test"),
        "help must NOT claim cdz-run is needed to run/test (it's informational): {help}"
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
    // `--json` emits one JSON object with the version/path/cdz_run/runtime_store/ok keys — the CI/setup
    // shape. Assert the OBJECT SHAPE (keys present, well-formed), NOT the `ok` verdict: whether the
    // toolchain is healthy is ENVIRONMENT-dependent — a local `cargo test` after a full build has cdz-run
    // beside cdz + a store (→ ok:true), but CI's bare `cargo test --workspace` builds neither (→ ok:false,
    // non-zero exit). The verdict-under-a-controlled-input is covered by the `--store <missing>` test below.
    let (_ok, out, _err) = run(&["doctor", "--json"]);
    assert!(
        out.trim_start().starts_with('{'),
        "emits a JSON object: {out}"
    );
    for key in ["\"version\"", "\"cdz_run\"", "\"runtime_store\"", "\"ok\":"] {
        assert!(out.contains(key), "json has {key}: {out}");
    }
    // `ok` is a boolean (true when the env is fully built, false on a bare checkout) — pin only that the
    // field is a well-formed bool, not which value this environment produces.
    assert!(
        out.contains("\"ok\":true") || out.contains("\"ok\":false"),
        "json has a boolean ok verdict: {out}"
    );
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
