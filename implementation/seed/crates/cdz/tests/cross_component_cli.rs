//! End-to-end tests for the CROSS-COMPONENT provider CLI on the effects-unified surface (X4b-4, U2) —
//! `cdz compile <provider> --component-name cadenza:pkg/iface`. This is the provider half of the
//! cross-component delivery: a Cadenza source file compiled with a published interface name, so a peer
//! consumer's `(effect …)` `(bind "cadenza:pkg/iface")` can import it. Drives the built `cdz` binary.
//!
//! X4b-4's CLI delivery was originally hand-verified on the removed `(extern …)` surface; U4 unified
//! cross-component interop with effects. These tests lock in that the `--component-name` flag path still
//! produces the right component after U4 — a real user-facing surface that otherwise had no coverage.
//!
//! The consumer-run half (`cdz-run --peer`) needs wasmtime + the content-addressed runtime store, which
//! lives in `cdz-run` (deliberately kept out of `cdz`); the library test `u6_*` in `rcdzc` proves the
//! full both-sides-from-source run over one shared runtime. Here we assert the provider component's SHAPE
//! by dependency-free byte inspection (it validates, publishes the named interface, imports the runtime).

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

/// A unique temp dir for one test.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-xcomp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Substring search over bytes (no external dep).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn compile_a_scalar_provider_with_a_component_name_publishes_the_interface() {
    // A SCALAR provider: `neg` publishes under `cadenza:math/api`. The component must carry the interface
    // name (the named instance export) — a bare top-level `neg` would not embed `cadenza:math/api`.
    let dir = temp_dir("scalar");
    let src = dir.join("neg.sexp");
    std::fs::write(&src, "(do (def (neg (: x Int64)) (- 0 x)) (export neg))").unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--component-name",
        "cadenza:math/api",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "provider compile failed: {err}");
    let comp = dir.join("neg.wasm");
    assert!(comp.is_file(), "no provider component produced: {err}");
    let bytes = std::fs::read(&comp).unwrap();
    assert_eq!(&bytes[..4], b"\0asm", "not a wasm component");
    assert!(
        contains(&bytes, b"cadenza:math/api"),
        "the provider must publish its exports under the --component-name interface"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_a_compound_provider_with_a_component_name_imports_the_runtime() {
    // A COMPOUND-returning provider: `pair x = (tuple x x)` publishes under `cadenza:pairs/api`. Because it
    // BUILDS a runtime value, it takes the provider+runtime envelope (assemble_provider_runtime) — so the
    // component both publishes the interface AND imports the value-heap runtime (`cadenza:runtime/heap`).
    // This is the source-provider path the U6 library test runs end-to-end; here we prove the CLI drives it.
    let dir = temp_dir("compound");
    let src = dir.join("pair.sexp");
    std::fs::write(
        &src,
        "(do (def (pair (: x Int64)) (tuple x x)) (export pair))",
    )
    .unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--component-name",
        "cadenza:pairs/api",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "compound provider compile failed: {err}");
    let comp = dir.join("pair.wasm");
    assert!(comp.is_file(), "no compound provider component: {err}");
    let bytes = std::fs::read(&comp).unwrap();
    assert!(
        contains(&bytes, b"cadenza:pairs/api"),
        "the compound provider must publish its interface"
    );
    assert!(
        contains(&bytes, b"cadenza:runtime/heap"),
        "a compound-building provider must import the value-heap runtime (it mints a handle)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_component_name_the_export_stays_top_level() {
    // The CONTROL: the SAME provider source, compiled WITHOUT --component-name, publishes `neg` at top
    // level — the interface name is absent (the flag is what wraps the exports as a named instance).
    let dir = temp_dir("plain");
    let src = dir.join("neg.sexp");
    std::fs::write(&src, "(do (def (neg (: x Int64)) (- 0 x)) (export neg))").unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "plain compile failed: {err}");
    let bytes = std::fs::read(dir.join("neg.wasm")).unwrap();
    assert!(
        !contains(&bytes, b"cadenza:math/api"),
        "without --component-name the interface name must not appear"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
