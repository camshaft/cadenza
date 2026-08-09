//! End-to-end tests for `cdz convert FILE` — surface conversion (binary/sexpr/ml + debug/flat views).
//!
//! The CONVERSION itself (the codec, and its round-trip fidelity across every surface pair) is v-syntax's
//! `cadenza-syntax` territory and is exhaustively unit-tested there. What is pinned HERE is the thin `cdz
//! convert` CLI WRAPPER layer, which had no dedicated e2e test: `--from`/`--to` wiring, the default `--to`,
//! reading a FILE vs stdin (`--from` required for stdin), and clean read/parse errors. Drives the built
//! binary over temp files — so it proves the CLI plumbs the codec correctly, without re-testing the codec.

use std::process::Command;

#[path = "../common/mod.rs"]
mod common;
use common::write_stdin_tolerating_broken_pipe;

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

/// Write `src` to a unique temp file with the given name; returns (dir, path).
fn temp(tag: &str, name: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-convert-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(name);
    std::fs::write(&path, src).expect("write test file");
    // `.expect`, NOT `to_string_lossy`: the path is passed as a `&str` CLI arg, so it MUST be UTF-8 — a
    // loud fail-fast on the vanishingly-rare non-UTF-8 temp dir beats lossy U+FFFD corruption that would
    // surface later as a confusing "missing file".
    (
        dir,
        path.to_str()
            .expect("temp path must be valid UTF-8")
            .to_string(),
    )
}

const SEXPR: &str = "(module m (def (main) (+ 1 2)) (export main))";

#[test]
fn convert_sexpr_to_ml_produces_ml_surface() {
    let (dir, file) = temp("s2ml", "p.sexp", SEXPR);
    let (ok, out, err) = run(&["convert", &file, "--to", "ml"]);
    assert!(ok, "sexpr→ml converts: {err}");
    assert!(
        out.contains("def main()") && out.contains("1 + 2"),
        "the ML surface renders the def + infix body: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_defaults_to_sexpr_when_to_is_omitted() {
    // Per `--to`'s help: inferred from FILE's extension, else defaults to `sexpr`. A `.sexp` input with no
    // `--to` round-trips to itself (sexpr→sexpr). Pins the default-output contract of the CLI wrapper.
    let (dir, file) = temp("default", "p.sexp", SEXPR);
    let (ok, out, err) = run(&["convert", &file]);
    assert!(ok, "convert with no --to succeeds: {err}");
    assert!(
        out.trim() == SEXPR,
        "default --to reproduces the sexpr: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_round_trips_sexpr_through_ml_via_the_cli() {
    // The CLI plumbs the codec faithfully: sexpr →(ml)→ sexpr recovers the original. (The codec's round-trip
    // is proven in cadenza-syntax; this proves the `cdz convert` wrapper feeds `--from`/`--to` through
    // correctly — a wrapper bug, e.g. swapping from/to, would break this even with a correct codec.)
    let (dir, file) = temp("rt", "p.sexp", SEXPR);
    let (ok1, ml, err1) = run(&["convert", &file, "--to", "ml"]);
    assert!(ok1, "sexpr→ml: {err1}");
    // Feed the ml back in via stdin with an explicit --from, convert back to sexpr.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["convert", "-", "--from", "ml", "--to", "sexpr"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz convert (stdin)");
    write_stdin_tolerating_broken_pipe(child.stdin.take().unwrap(), ml.as_bytes());
    let out = child.wait_with_output().expect("wait cdz");
    let back = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        out.status.success() && back == SEXPR,
        "sexpr→ml→sexpr round-trips through the CLI: got {back:?} (stderr={})",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn convert_reading_stdin_requires_from() {
    // stdin has no extension to infer the surface, so `--from` is mandatory — a missing one is a clean,
    // actionable error (not a guess or a panic).
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(["convert", "-", "--to", "ml"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn cdz convert (stdin)");
    // cdz errors on the missing --from and EXITS before reading stdin, so this write races the pipe close.
    write_stdin_tolerating_broken_pipe(child.stdin.take().unwrap(), SEXPR.as_bytes());
    let out = child.wait_with_output().expect("wait cdz");
    assert!(!out.status.success(), "stdin without --from must fail");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("requires --from"),
        "the error tells the user --from is required for stdin: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn convert_a_missing_file_errors_cleanly() {
    let (ok, _out, err) = run(&["convert", "/no/such/file.sexp", "--to", "ml"]);
    assert!(!ok, "a missing file fails");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}

#[test]
fn convert_a_malformed_input_errors_cleanly_not_a_panic() {
    let (dir, file) = temp("bad", "bad.sexp", "(module m (def");
    let (ok, _out, err) = run(&["convert", &file, "--to", "ml"]);
    assert!(!ok, "a malformed input fails");
    assert!(
        err.contains("parse error") && !err.contains("panicked"),
        "a clean parse error, not a panic: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
