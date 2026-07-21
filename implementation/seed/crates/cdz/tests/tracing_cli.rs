//! End-to-end tests for the `RUST_LOG`-gated trace subscriber in `cdz` main.
//!
//! rcdzc's lib emits rich `trace!` events (lower/eval/resolve decisions) but installs NO sink — a bin
//! must install one for them to fire. `cdz` (the user-facing entry) installs a `RUST_LOG`-gated
//! subscriber writing to STDERR, so `RUST_LOG=rcdzc=trace cdz compile …` surfaces the compiler's
//! decision flow. These pin the gating contract: (1) with `RUST_LOG` UNSET a normal compile is silent
//! (no trace, and stdout/the written artifact is uncorrupted); (2) with `RUST_LOG=rcdzc=trace` the
//! compile emits TRACE events on stderr; (3) trace never leaks onto stdout. Drives the built binary.

use std::process::Command;

/// A minimal, valid, compilable program (has an `(export …)`, so the compile succeeds cleanly).
const PROG: &str = "(module m (def (main) (+ 1 2)) (export main))";

/// Write PROG to a unique temp `.sexp` and return (dir, src_path, out_wasm_path).
fn temp_project(tag: &str) -> (std::path::PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("cdz-tracing-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let src = dir.join("m.sexp");
    std::fs::write(&src, PROG).unwrap();
    let out = dir.join("m.wasm");
    (
        dir.clone(),
        src.to_str().unwrap().to_string(),
        out.to_str().unwrap().to_string(),
    )
}

/// Run `cdz compile <src> -o <out>` with an optional `RUST_LOG` value, returning (ok, stdout, stderr).
fn compile_with_rust_log(src: &str, out: &str, rust_log: Option<&str>) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut cmd = Command::new(exe);
    cmd.args(["compile", src, "-o", out]);
    // Ensure a clean baseline: remove any inherited RUST_LOG, then set it only when the case wants it.
    cmd.env_remove("RUST_LOG");
    if let Some(v) = rust_log {
        cmd.env("RUST_LOG", v);
    }
    let o = cmd.output().expect("spawn cdz");
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).to_string(),
        String::from_utf8_lossy(&o.stderr).to_string(),
    )
}

#[test]
fn without_rust_log_a_compile_emits_no_trace() {
    // The gate: with RUST_LOG unset, no subscriber is installed and every `trace!` is a runtime no-op —
    // a normal run pays nothing and its stderr carries only the tool's own messages, no TRACE lines.
    let (dir, src, out) = temp_project("off");
    let (ok, _stdout, stderr) = compile_with_rust_log(&src, &out, None);
    assert!(ok, "a clean compile succeeds: {stderr}");
    assert!(
        !stderr.contains("TRACE"),
        "no trace output when RUST_LOG is unset: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_rust_log_a_compile_emits_trace_events_on_stderr() {
    // With RUST_LOG=rcdzc=trace, the subscriber installs and rcdzc's compile/resolve/lower `trace!`
    // events surface on stderr — the whole point (unblocks trace-based compiler debugging via `cdz`).
    let (dir, src, out) = temp_project("on");
    let (ok, _stdout, stderr) = compile_with_rust_log(&src, &out, Some("rcdzc=trace"));
    assert!(ok, "the compile still succeeds with tracing on: {stderr}");
    assert!(
        stderr.contains("TRACE") && stderr.contains("rcdzc"),
        "RUST_LOG=rcdzc=trace surfaces rcdzc TRACE events on stderr: {}",
        &stderr[..stderr.len().min(400)]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn trace_output_goes_to_stderr_not_stdout() {
    // Trace must NEVER corrupt stdout — that channel carries the tool's real output (emitted artifacts,
    // verdicts). Even with tracing fully on, stdout stays free of TRACE lines.
    let (dir, src, out) = temp_project("chan");
    let (ok, stdout, _stderr) = compile_with_rust_log(&src, &out, Some("rcdzc=trace"));
    assert!(ok, "compile succeeds");
    assert!(
        !stdout.contains("TRACE"),
        "trace is on stderr, never stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
