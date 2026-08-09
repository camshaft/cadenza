//! X5b run-side witness — a variable-length `List` HANDLE crosses a peer-interface edge and is dereferenced
//! correctly through the ONE shared value-heap runtime instance, end-to-end via the `cdz` binary.
//!
//! Context: cross-component peer linking (`cdz run --peer`) composes consumer + peers over one wasmtime store
//! and one shared runtime instance, so a heap `value` handle one component produces is meaningful to another
//! (they index the same heap). rcdzc's `u6_*` library test already proves this for a fixed-arity TUPLE handle.
//! This pins the specific case Option C (shared-closure-as-imported-component) rides on: a VARIABLE-LENGTH
//! `List`/rope handle (CHAMP/RRB), returned by a provider op and consumed across the peer edge. `cdz-run`'s
//! doc noted the peer path as "scalar peer ops today; a value-handle op RIDES this shared instance"
//! (lib.rs:450) — this witness confirms the handle-crossing works on the run side, so X5b needs no run-side
//! work and Option C's cross-component call edges can carry List handles.
//!
//! Drives the built `cdz`: `compile` the provider (with a published interface name) + the consumer, then
//! `run --peer`. Needs the content-addressed runtime store (the value-heap runtime the components import);
//! if that can't be resolved in this checkout, the run is SKIPPED (not failed) — the same discretion the
//! run-dependent suites use.

use std::process::Command;

fn cdz() -> &'static str {
    env!("CARGO_BIN_EXE_cdz")
}

/// Whether the value-heap runtime STORE is present. CI's bare `test` job runs `cargo test` with NO
/// `cargo xtask build`, so there is no store — and `cdz run --peer` resolves the content-addressed
/// runtime (to run the composed module) BEFORE it can produce output, failing "no runtime of content
/// address … in the store … refusing to run" + exiting non-zero storeless. Skip when absent; the
/// store-having `gate` + `@test suites` jobs exercise it fully. Resolve the store dir the SAME way the
/// runtime resolver does (`CADENZA_STORE` env first, else `<target>/cadenza-store`), so this guard
/// AGREES with the storeless-rerun mechanism (which sets `CADENZA_STORE` to an empty temp dir).
/// Mirrors the `store_present()` guard in `run_emitted_cli.rs` / `normalize_cli.rs`.
///
/// HASH-AWARE (not merely presence): the store must hold the CURRENT runtime, `<store>/<hash>.wasm` for
/// `REQUIRED_RUNTIME_HASH` — NOT just be a non-empty dir. A runner's rust-cache can restore a STALE
/// `cadenza-store` holding an OLDER runtime hash (after a runtime-hash bump with no fresh `cargo xtask
/// build`); a presence/non-empty check passes on that stale store, the peer run then resolves the CURRENT
/// hash, misses it, and fails "no runtime of content address <hash> … refusing to run" (this red the
/// REQUIRED `test (macos-latest)` job trunk-wide). Checking the exact `<hash>.wasm` makes a stale store
/// read as absent → the test SKIPS (its documented storeless behavior) instead of running against the
/// wrong/missing runtime.
fn store_present() -> bool {
    let required = rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH;
    let store_dir = std::env::var_os("CADENZA_STORE")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::path::PathBuf::from(env!("CARGO_BIN_EXE_cdz"))
                .parent()
                .and_then(|d| d.parent())
                .map(|t| t.join("cadenza-store"))
        });
    store_dir
        .map(|d| d.join(format!("{required}.wasm")).is_file())
        .unwrap_or(false)
}

/// A unique temp dir for one test.
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-x5b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// Compile `src` to a component in `dir`; returns the produced `.wasm` path. `cdz compile -o <dir>` names the
/// output after the EXPORTED def, and each source here writes its file + exports a single def under the SAME
/// `name` (so `name.sexp` → `name.wasm`) — so the output path is `dir/<name>.wasm` DETERMINISTICALLY (no
/// mtime scan, which would flake on coarse mtime / a stray wasm). `component_name` (if any) publishes the
/// provider's interface. Panics on failure.
fn compile(
    dir: &std::path::Path,
    name: &str,
    src: &str,
    component_name: Option<&str>,
) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.sexp"));
    std::fs::write(&path, src).unwrap();
    let mut args = vec![
        "compile".to_string(),
        path.to_str().unwrap().to_string(),
        "-o".to_string(),
        dir.to_str().unwrap().to_string(),
    ];
    if let Some(cn) = component_name {
        args.push("--component-name".to_string());
        args.push(cn.to_string());
    }
    let out = Command::new(cdz())
        .args(&args)
        .output()
        .expect("spawn cdz compile");
    assert!(
        out.status.success(),
        "compile {name} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The emitted component is named after the exported entry, which here == `name` (each source exports a
    // single def of that name). So the path is deterministic — no dir scan / mtime pick.
    let wasm = dir.join(format!("{name}.wasm"));
    assert!(
        wasm.is_file(),
        "expected `cdz compile -o` to emit `{name}.wasm` (named after the exported def); dir contents: {:?}",
        std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
            .unwrap_or_default()
    );
    wasm
}

#[test]
fn a_list_handle_crosses_a_peer_edge_and_reads_the_right_length() {
    // SKIP (don't fail) storeless: `cdz run --peer` resolves the value-heap runtime before it can produce
    // output, so with no store (CI's bare `test` job) it exits non-zero "no runtime … refusing to run".
    // The store-having gate/@test jobs witness the run fully. (Checked up front so the guard can't miss the
    // resolver's error string.)
    if !store_present() {
        eprintln!(
            "[x5b] skipping: no cadenza-store (storeless test job) — cdz run --peer needs the runtime"
        );
        return;
    }

    // PROVIDER: `mklist n` builds a runtime List `[n, n+1, n+2]` and publishes it as `cadenza:closure/api` —
    // the shape a shared-closure component (Option C) exports. CONSUMER: imports that interface as a
    // peer-bound effect, calls `mklist n` across the edge, and reads `List.len` of the returned handle.
    let dir = temp_dir("listlen");
    let provider = compile(
        &dir,
        "mklist",
        "(do (def (mklist (: n Int64)) (list n (+ n 1) (+ n 2))) (export mklist))",
        Some("cadenza:closure/api"),
    );
    let consumer = compile(
        &dir,
        "main",
        "(do (effect C (op mklist (-> Int64 (List Int64)))) (bind C \"cadenza:closure/api\") \
             (def (main (: n Int64)) (host (C) (List.len (C.mklist n)))) (export main))",
        None,
    );

    let out = Command::new(cdz())
        .args([
            "run",
            consumer.to_str().unwrap(),
            "--peer",
            &format!("cadenza:closure/api={}", provider.to_str().unwrap()),
            "--arg",
            "5",
        ])
        .output()
        .expect("spawn cdz run --peer");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "cdz run --peer should succeed (a List handle crosses the shared runtime): {stderr}"
    );
    // main(5) = List.len([5,6,7]) = 3. If the List HANDLE failed to cross the peer edge (or indexed a
    // different heap), this would trap or print a wrong length — the X5b failure mode.
    assert_eq!(
        stdout.trim(),
        "3",
        "the List handle crossed the peer edge and List.len read the right length through the shared \
         runtime; got stdout={stdout:?} stderr={stderr:?}"
    );
}
