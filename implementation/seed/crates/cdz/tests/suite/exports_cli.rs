//! End-to-end tests for `cdz exports FILE` — the public-INTERFACE query (a module's exported surface).
//!
//! `exports` is a semantic query only the unified `cdz` binary can answer (compiler export-resolution +
//! the front-end span table in one process). The sidecar's export-resolution is unit-tested in rcdzc;
//! what is pinned HERE is the CLI-RENDERING layer only `cdz` does: emitting each export as
//! `file:line:col: name : type` (or, under `--json`, one structured object per export for a tool that
//! reads a module's public interface without re-parsing the text), listing ONLY the exported subset (the
//! private-vs-exported distinction from `cdz symbols`), and the exports-nothing / missing-file contracts.
//! The human and `--json` shapes are computed from the SAME resolved `(name, type, line, col)` so they
//! can't drift — this pins that parity. Drives the built binary over a temp `.sexp` file.

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

/// Write `src` to a unique temp `.sexp` file and return (dir, path).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-exports-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// A module with a PRIVATE function `inc`, a value `pi`, and an EXPORTED function `main`.
const PROG: &str = "(module m \
    (def pi 3) \
    (def (inc (: n Int64)) (+ n 1)) \
    (def (main (: a Int64)) (inc a)) \
    (export main))";

#[test]
fn exports_lists_the_exported_name_with_its_type_and_locus() {
    let (dir, file) = temp_src("iface", PROG);
    let (ok, out, err) = run(&["exports", &file]);
    assert!(ok, "cdz exports should succeed: {err}");
    // Each export is `file:line:col: name : type`. `main` is the sole export; its type is a function arrow.
    assert!(out.contains("main"), "the exported `main` is listed: {out}");
    assert!(
        out.contains(" : "),
        "an export carries its ` : type` rendering: {out}"
    );
    // Every non-empty line carries a `file:line:col:` locus prefix (the export's def has a known span).
    for l in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            l.starts_with(&file) && l[file.len()..].starts_with(':'),
            "export line is `file:line:col: name : type`: {l}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_lists_only_the_exported_subset_not_private_declarations() {
    // The point of `exports` vs `symbols`: `exports` lists ONLY the exported subset, so the private `inc`
    // and the un-exported value `pi` do NOT appear — only `main` does.
    let (dir, file) = temp_src("subset", PROG);
    let (ok, out, err) = run(&["exports", &file]);
    assert!(ok, "exports failed: {err}");
    assert!(out.contains("main"), "the exported `main` appears: {out}");
    assert!(
        !out.contains("inc"),
        "the private `inc` does NOT appear (not exported): {out}"
    );
    assert!(
        !out.contains("pi"),
        "the un-exported value `pi` does NOT appear: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_json_emits_one_structured_object_per_export_at_row_parity() {
    // `--json` emits one machine-readable object per export (the shape a tool consumes to read a module's
    // public interface without re-parsing the `file:line:col: name : type` text). Each object has
    // file/name/type (+ line/col when the def has a span), and the JSON rows correspond 1:1 with the human
    // rows (both computed from the same resolved export set, so they can't drift). Assert object SHAPE +
    // row parity, not the exact line/col numbers (those are surface-layout details).
    let (dir, file) = temp_src("json", PROG);
    let (ok, out, err) = run(&["exports", &file, "--json"]);
    assert!(ok, "cdz exports --json should succeed: {err}");
    let rows: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    // Baseline (non-json) run must SUCCEED, else human_rows would be 0 and the parity check passes hollowly.
    let (hok, human, herr) = run(&["exports", &file]);
    assert!(
        hok,
        "baseline (non-json) exports run should succeed: {herr}"
    );
    let human_rows = human.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        rows.len(),
        human_rows,
        "one JSON row per human row (parity): {out}"
    );
    // PARSE each row (a substring check would pass for MALFORMED JSON; parsing rejects it). Collect the
    // export names off the parsed values to assert the interface.
    let mut names = std::collections::BTreeSet::new();
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(v["type"].is_string(), "`type` is a string: {row}");
        let name = v["name"].as_str().expect("`name` is a string");
        names.insert(name.to_string());
    }
    assert!(
        names.contains("main"),
        "the exported `main` is emitted as a structured object: {names:?}"
    );
    assert!(
        !names.contains("inc") && !names.contains("pi"),
        "only the exported subset is emitted (no private `inc`/`pi`): {names:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_of_a_module_exporting_nothing_reports_it_and_succeeds() {
    // A module that exports nothing is TOTAL: empty stdout, an `exports nothing` note on stderr, and a
    // SUCCESS exit (the query ran; there is just nothing to report). `pi`/`inc` exist but nothing is
    // exported.
    let (dir, file) = temp_src(
        "nothing",
        "(module m (def pi 3) (def (inc (: n Int64)) (+ n 1)))",
    );
    let (ok, out, err) = run(&["exports", &file]);
    assert!(
        ok,
        "a module exporting nothing still exits successfully: {err}"
    );
    assert!(out.trim().is_empty(), "stdout is empty: {out}");
    assert!(
        err.contains("exports nothing"),
        "the exports-nothing note is on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_on_a_missing_file_errors_with_the_cdz_prog_name() {
    // A load error names the tool (`cdz`) and exits non-zero.
    let (ok, _out, err) = run(&["exports", "/no/such/file.sexp"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}
