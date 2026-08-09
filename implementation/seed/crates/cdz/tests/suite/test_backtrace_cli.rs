//! `cdz test` — the `CDZ_WASM_BACKTRACE` diagnostic lever for a trapping test.
//!
//! When a `@test` body TRAPS, the FAIL line reports the trap reason. By default it is trimmed to the reason's
//! FIRST line (a legible one-line counterexample). wasmtime actually CAPTURES a wasm backtrace (`<wasm
//! function N>` frames) on the trap — it was just being discarded by the trim. Setting `CDZ_WASM_BACKTRACE`
//! keeps the whole reason, frames included, so a COMPILED trap (esp. self-host, where isolating the case
//! doesn't reproduce it) can be localized to a func-index. These tests pin both halves: default = one line,
//! flag = frames present.

use std::process::Command;

/// A `@test` whose body traps unconditionally (`trap(...)` lowers to a wasm `unreachable`). Tiny — no import
/// closure — so the full front-end→emit→run costs ~tens of ms, cheap enough for the default suite.
const TRAPPING_TEST: &str = "@test\ndef t_body_trap() =\n  trap(\"boom for backtrace\")\n";

/// Write `src` to a unique temp `.cdz` (ML surface) file and return its path.
fn temp_cdz(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("cdz-bt-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("t.cdz");
    std::fs::write(&path, TRAPPING_TEST).unwrap();
    path.to_str().unwrap().to_string()
}

/// Run `cdz test FILE` with optional extra env, returning combined stdout+stderr. `cdz test` exits non-zero
/// when a test fails, which is EXPECTED here (the body traps) — so we don't assert on the exit code.
fn run_test(file: &str, env: &[(&str, &str)]) -> String {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut cmd = Command::new(exe);
    cmd.args(["test", file]);
    // HERMETIC: clear CDZ_WASM_BACKTRACE from the INHERITED parent env first, so a value set in the outer
    // environment can't leak into a case that expects it unset (the "default omits frames" / off-values
    // assertions would spuriously fail otherwise). Each case then applies only its OWN override below.
    cmd.env_remove("CDZ_WASM_BACKTRACE");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn cdz test");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_trapping_test_reports_the_trap_reason_and_fails() {
    // Baseline: the trap is reported on a FAIL line with the reason, and the suite fails (0 passed, 1 failed).
    let file = temp_cdz("baseline");
    let out = run_test(&file, &[]);
    assert!(
        out.contains("FAIL t_body_trap") && out.contains("body trapped"),
        "a trapping test reports a FAIL with the trap reason:\n{out}"
    );
    assert!(
        out.contains("1 failed"),
        "the suite reports the failure:\n{out}"
    );
}

#[test]
fn default_omits_the_wasm_backtrace_frames() {
    // Without CDZ_WASM_BACKTRACE, the FAIL message is trimmed to the reason's first line — no `<wasm function
    // N>` frames — so the one-line counterexample stays legible.
    let file = temp_cdz("default");
    let out = run_test(&file, &[]);
    assert!(
        !out.contains("<wasm function"),
        "default output must NOT include backtrace frames:\n{out}"
    );
}

#[test]
fn cdz_wasm_backtrace_env_includes_the_frames() {
    // With CDZ_WASM_BACKTRACE=1, the whole trap reason is kept — including wasmtime's captured `<wasm function
    // N>` frames, the func-index locus for a compiled trap. This is the diagnostic lever (v-wasm-opt's gap:
    // the backtrace was captured but discarded by the first-line trim).
    let file = temp_cdz("frames");
    let out = run_test(&file, &[("CDZ_WASM_BACKTRACE", "1")]);
    assert!(
        out.contains("<wasm function"),
        "CDZ_WASM_BACKTRACE=1 must surface the wasm backtrace frames:\n{out}"
    );
}

#[test]
fn cdz_wasm_backtrace_disabled_values_are_off() {
    // `0` / `false` / empty are treated as OFF (the same as unset), so a stray `CDZ_WASM_BACKTRACE=0` in the
    // environment doesn't accidentally enable frames.
    let file = temp_cdz("off-values");
    for v in ["0", "false", ""] {
        let out = run_test(&file, &[("CDZ_WASM_BACKTRACE", v)]);
        assert!(
            !out.contains("<wasm function"),
            "CDZ_WASM_BACKTRACE={v:?} must be treated as OFF:\n{out}"
        );
    }
}
