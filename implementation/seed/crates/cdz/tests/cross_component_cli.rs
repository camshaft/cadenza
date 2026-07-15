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

#[test]
fn cdz_run_peer_reports_a_signature_mismatch_clearly_not_a_trap() {
    // The CONSUMER-run half through the MOUNTED `cdz run --peer` binary (the peer-signature check lives
    // in cdz-run, surfaced by the unified `cdz` binary). A consumer binding `Math.add` as a 2-arg op
    // composed with a peer exporting a 1-ARG `add` is an arity mismatch: without the compose-time check
    // it traps opaquely deep in the callee; the check rejects it BEFORE instantiation naming the op +
    // both arities. Both peers are scalar (no value-heap runtime), so no runtime store is needed — the
    // mismatch is caught at compose time regardless. This pins the mismatch diagnostic all the way out
    // to the real CLI (the library `rcdzc` test pins the `run_with_peers` API; this pins `cdz run`).
    let dir = temp_dir("peer-mismatch");
    // Provider: `add` taking ONE argument, published as cadenza:math/api.
    let prov = dir.join("prov.sexp");
    std::fs::write(&prov, "(do (def (add (: x Int64)) (+ x 1)) (export add))").unwrap();
    let (ok, _o, err) = run(&[
        "compile",
        prov.to_str().unwrap(),
        "--component-name",
        "cadenza:math/api",
        "-o",
        dir.join("prov.wasm").to_str().unwrap(),
    ]);
    assert!(ok, "provider compile failed: {err}");
    // Consumer: binds `Math.add` as a TWO-argument op — arity mismatch with the peer.
    let cons = dir.join("cons.sexp");
    std::fs::write(
        &cons,
        "(do (effect Math (op add (-> Int64 Int64 Int64))) (bind Math \"cadenza:math/api\") \
         (def (main (: x Int64)) (host (Math) (Math.add x x))) (export main))",
    )
    .unwrap();
    let (ok, _o, err) = run(&[
        "compile",
        cons.to_str().unwrap(),
        "-o",
        dir.join("cons.wasm").to_str().unwrap(),
    ]);
    assert!(ok, "consumer compile failed: {err}");
    // `cdz run --peer` must REJECT the mismatch with an actionable message, not run to a trap.
    let peer_arg = format!(
        "cadenza:math/api={}",
        dir.join("prov.wasm").to_str().unwrap()
    );
    let (ok, _out, err) = run(&[
        "run",
        dir.join("cons.wasm").to_str().unwrap(),
        "--peer",
        &peer_arg,
        "--call",
        "main",
        "--arg",
        "5",
    ]);
    assert!(
        !ok,
        "an arity-mismatched peer run must FAIL, not succeed: {err}"
    );
    assert!(
        err.contains("signature mismatch")
            && err.contains("add")
            && err.contains("2 argument(s)")
            && err.contains("1 argument(s)"),
        "the CLI must surface the compose-time signature-mismatch diagnostic, not an opaque trap: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
