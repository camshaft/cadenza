//! End-to-end tests for `cdz compile` producing DWARF debug output DIRECTLY FROM A SOURCE FILE — the
//! ergonomic payoff of `cdz` holding both the front-end and the compiler: a debug target auto-supplies
//! the `spans` artifact (parsed in-process), so a user needn't hand-build one. Drives the built binary.

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

/// A unique temp dir for one test (avoids cross-test collisions without an extra dep).
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-compile-dbg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

const PROG: &str = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";

#[test]
fn compile_a_source_file_to_a_component() {
    // `cdz compile` accepts a SOURCE file (not just a pre-built binary AST) — parsed in-process.
    let dir = temp_dir("plain");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "compile failed: {err}");
    assert!(
        dir.join("add.wasm").is_file(),
        "no component produced: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wasm_debug_from_source_auto_supplies_spans() {
    // The payoff: `--target wasm-debug` on a SOURCE file needs NO explicit `spans:` input — cdz parses
    // with spans and injects the artifact. The component is produced (and carries debug sections).
    let dir = temp_dir("modee");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--target",
        "wasm-debug",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "wasm-debug compile failed: {err}");
    let comp = dir.join("add.wasm");
    assert!(comp.is_file(), "no debug component: {err}");
    // The embedded core module carries `.debug_*` custom sections (a plain component would not). Assert
    // the bytes contain the `.debug_info` section name — a cheap, dependency-free check.
    let bytes = std::fs::read(&comp).unwrap();
    assert!(
        contains(&bytes, b".debug_info") && contains(&bytes, b".debug_line"),
        "the wasm-debug component must embed DWARF sections"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dwarf_sidecar_from_source_auto_supplies_spans() {
    // `--target dwarf` on a source file produces a detached `<name>.dwarf` sidecar (no explicit spans).
    let dir = temp_dir("modes");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--target",
        "dwarf",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "dwarf compile failed: {err}");
    let sidecar = dir.join("add.dwarf");
    assert!(sidecar.is_file(), "no dwarf sidecar: {err}");
    let bytes = std::fs::read(&sidecar).unwrap();
    // A bare core module carrying the debug sections: the `\0asm` header + the section names + the
    // source function name (`add`) proving the DWARF describes the program.
    assert_eq!(&bytes[..4], b"\0asm", "not a wasm module");
    assert!(
        contains(&bytes, b".debug_info") && contains(&bytes, b"add"),
        "the sidecar must carry DWARF naming the source function"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Substring search over bytes (no external dep).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
