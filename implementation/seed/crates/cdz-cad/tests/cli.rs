//! CLI contract tests for the `cdz-cad` binary (GH #400).
//!
//! The library tests cover `parse_solid`/`mesh`/`bounds`; these drive the actual BINARY end-to-end — the
//! arg parsing, the stdin/file input, the extension-dispatched writer, `--info`, and the error paths — so a
//! regression in the CLI wiring (not just the library) is caught. Cargo exposes the built binary's path as
//! `CARGO_BIN_EXE_cdz-cad` to an integration test in the same crate.

use std::io::Write;
use std::process::{Command, Stdio};

const CUBE: &str = "(: (Cube (: (tuple 2.0 2.0 2.0) Vec3)) Solid)";

/// Run the `cdz-cad` binary with `args`, feeding `stdin`. Returns (success, stdout, stderr).
fn run(args: &[&str], stdin: &str) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz-cad");
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz-cad");
    // Feed stdin, but TOLERATE a BrokenPipe: an arg-error path (e.g. `rejects_no_output_and_no_info`)
    // rejects + exits BEFORE reading stdin, so the write races the child's exit and intermittently hits
    // `BrokenPipe` (errno 32) — a timing flake, not a contract failure. The test cares about the exit
    // status + output, not that stdin was consumed. Any OTHER write error is still a real problem.
    {
        let mut si = child.stdin.take().unwrap();
        if let Err(e) = si.write_all(stdin.as_bytes()) {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                panic!("write stdin: {e:?}");
            }
        }
        // drop `si` → close the pipe so the child sees EOF if it IS reading.
    }
    let out = child.wait_with_output().expect("wait cdz-cad");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// A unique temp path in the OS temp dir (no external tempfile dep).
fn tmp_path(name: &str) -> std::path::PathBuf {
    // include the pid so parallel test runs don't collide.
    std::env::temp_dir().join(format!("cdz-cad-test-{}-{name}", std::process::id()))
}

#[test]
fn writes_a_binary_stl_from_stdin() {
    let out = tmp_path("cube.stl");
    let _ = std::fs::remove_file(&out);
    let (ok, _stdout, stderr) = run(&["-", "-o", out.to_str().unwrap()], CUBE);
    assert!(ok, "cdz-cad should succeed; stderr: {stderr}");
    let bytes = std::fs::read(&out).expect("output file exists");
    // binary STL: 80-byte header + u32 tri count + 50 bytes/triangle. A cube = 12 triangles.
    assert_eq!(
        bytes.len(),
        84 + 12 * 50,
        "binary STL size for a 12-triangle cube"
    );
    // header must NOT start with "solid" (that marks ASCII STL).
    assert_ne!(&bytes[..5], b"solid");
    assert!(
        stderr.contains("12 triangles"),
        "reports the triangle count"
    );
    assert!(stderr.contains("bounds"), "reports the bounding box");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn writes_ascii_stl_with_the_flag() {
    let out = tmp_path("cube-ascii.stl");
    let _ = std::fs::remove_file(&out);
    let (ok, _o, stderr) = run(&["-", "-o", out.to_str().unwrap(), "--ascii"], CUBE);
    assert!(ok, "stderr: {stderr}");
    let text = std::fs::read_to_string(&out).expect("output file");
    assert!(text.starts_with("solid cdz_cad"), "ASCII STL header");
    assert_eq!(
        text.matches("facet normal").count(),
        12,
        "12 facets for a cube"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn writes_a_glb_from_stdin() {
    let out = tmp_path("cube.glb");
    let _ = std::fs::remove_file(&out);
    let (ok, _o, stderr) = run(&["-", "-o", out.to_str().unwrap()], CUBE);
    assert!(ok, "stderr: {stderr}");
    let bytes = std::fs::read(&out).expect("output file");
    assert_eq!(&bytes[..4], b"glTF", "glb magic");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn info_reports_without_writing() {
    let (ok, _o, stderr) = run(&["-", "--info"], CUBE);
    assert!(ok, "stderr: {stderr}");
    assert!(stderr.contains("12 triangles"));
    assert!(stderr.contains("bounds"));
}

#[test]
fn rejects_no_output_and_no_info() {
    let (ok, _o, stderr) = run(&["-"], CUBE);
    assert!(!ok, "should fail without -o or --info");
    assert!(
        stderr.contains("no output") || stderr.contains("--info"),
        "stderr: {stderr}"
    );
}

#[test]
fn rejects_an_unsupported_extension() {
    let out = tmp_path("cube.obj");
    let (ok, _o, stderr) = run(&["-", "-o", out.to_str().unwrap()], CUBE);
    assert!(!ok, "should reject .obj");
    assert!(
        stderr.contains("unsupported output extension"),
        "stderr: {stderr}"
    );
}

#[test]
fn rejects_ascii_with_a_glb_target() {
    let out = tmp_path("cube-x.glb");
    let (ok, _o, stderr) = run(&["-", "-o", out.to_str().unwrap(), "--ascii"], CUBE);
    assert!(!ok, "should reject --ascii for .glb");
    assert!(stderr.contains("ascii"), "stderr: {stderr}");
}

#[test]
fn a_malformed_solid_errs_without_writing() {
    let out = tmp_path("bad.stl");
    let _ = std::fs::remove_file(&out);
    let (ok, _o, stderr) = run(&["-", "-o", out.to_str().unwrap()], "(: (Torus 1.0) Solid)");
    assert!(!ok, "should fail on an unknown constructor");
    assert!(stderr.contains("parse error"), "stderr: {stderr}");
    assert!(!out.exists(), "no output file on a parse error");
}
