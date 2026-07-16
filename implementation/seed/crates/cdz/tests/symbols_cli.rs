//! End-to-end tests for `cdz symbols FILE` — the document-OUTLINE query (the LSP `documentSymbol`
//! analogue).
//!
//! `symbols` is a semantic query only the unified `cdz` binary can answer (compiler classification +
//! the front-end span table in one process). The sidecar's classification logic is unit-tested in
//! rcdzc; what is pinned HERE is the CLI-RENDERING layer only `cdz` does: emitting each declaration as
//! `file:line:col: KIND NAME` (or, under `--json`, one structured object per declaration for an
//! editor/tool), listing EVERY declaration (private ones included — the superset of `cdz exports`), and
//! the empty-module contract. Drives the built binary over a temp `.sexp` file.

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
    let dir = std::env::temp_dir().join(format!("cdz-symbols-{tag}-{}", std::process::id()));
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
fn symbols_lists_every_declaration_classified_by_kind() {
    let (dir, file) = temp_src("outline", PROG);
    let (ok, out, err) = run(&["symbols", &file]);
    assert!(ok, "cdz symbols should succeed: {err}");
    // Each declaration is `file:line:col: KIND NAME`. A value is `value`, a function is `function`.
    assert!(
        out.contains(": value pi"),
        "the value `pi` is classified as a value: {out}"
    );
    assert!(
        out.contains(": function inc"),
        "the private function `inc` is listed and classified: {out}"
    );
    assert!(
        out.contains(": function main"),
        "the exported function `main` is listed: {out}"
    );
    // Every non-empty line carries a `file:line:col:` locus prefix.
    for l in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            l.starts_with(&file) && l[file.len()..].starts_with(':'),
            "declaration line is `file:line:col: KIND NAME`: {l}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn symbols_is_the_superset_of_exports_including_private_declarations() {
    // The point of `symbols` vs `exports`: `exports` lists only the exported subset; `symbols` lists
    // EVERY declaration, so the private `inc` (not exported) appears in `symbols` but NOT in `exports`.
    let (dir, file) = temp_src("superset", PROG);
    let (sym_ok, sym, se) = run(&["symbols", &file]);
    let (exp_ok, exp, ee) = run(&["exports", &file]);
    assert!(sym_ok, "symbols failed: {se}");
    assert!(exp_ok, "exports failed: {ee}");
    assert!(
        sym.contains("inc"),
        "the private `inc` appears in symbols: {sym}"
    );
    assert!(
        !exp.contains("inc"),
        "the private `inc` does NOT appear in exports (it is not exported): {exp}"
    );
    assert!(
        sym.contains("main") && exp.contains("main"),
        "the exported `main` appears in both"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn symbols_json_emits_one_structured_object_per_declaration() {
    // `--json` emits one machine-readable object per declaration (the `documentSymbol` payload an editor
    // consumes without re-parsing the `file:line:col: kind name` text). Each object has file/line/col/
    // kind/name, and the JSON rows correspond 1:1 with the human rows (both computed from the same
    // resolved declaration set, so they can't drift). Assert the object SHAPE + row parity, not the exact
    // line/col numbers (those are surface-layout details).
    let (dir, file) = temp_src("json", PROG);
    let (ok, out, err) = run(&["symbols", &file, "--json"]);
    assert!(ok, "cdz symbols --json should succeed: {err}");
    let rows: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    // One object per declaration (pi, inc, main) — same count as the human form. Assert the baseline run
    // SUCCEEDED (else human_rows would be 0 and the parity check would pass hollowly).
    let (hok, human, herr) = run(&["symbols", &file]);
    assert!(
        hok,
        "baseline (non-json) symbols run should succeed: {herr}"
    );
    let human_rows = human.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        rows.len(),
        human_rows,
        "one JSON row per human row (parity): {out}"
    );
    // PARSE each row (serde_json is in-crate) — a substring check would pass for MALFORMED JSON; parsing
    // rejects it. Collect the (kind, name) pairs off the parsed values to assert the classification.
    let mut decls = std::collections::BTreeSet::new();
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(
            v["line"].is_number() && v["col"].is_number(),
            "`line`/`col` are numbers: {row}"
        );
        let kind = v["kind"].as_str().expect("`kind` is a string");
        let name = v["name"].as_str().expect("`name` is a string");
        decls.insert((kind.to_string(), name.to_string()));
    }
    // The classified declarations are present with their kinds (parsed, not substring).
    assert!(
        decls.contains(&("value".to_string(), "pi".to_string())),
        "the value `pi` is emitted as a structured object: {decls:?}"
    );
    assert!(
        decls.contains(&("function".to_string(), "inc".to_string())),
        "the private function `inc` is emitted (superset, private included): {decls:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn symbols_of_an_empty_module_reports_declares_nothing_and_succeeds() {
    // A module with no declarations is TOTAL: empty stdout, a `declares nothing` note on stderr, and a
    // SUCCESS exit (the query ran; there is just nothing to outline).
    let (dir, file) = temp_src("empty", "(module m)");
    let (ok, out, err) = run(&["symbols", &file]);
    assert!(ok, "an empty module still exits successfully: {err}");
    assert!(out.trim().is_empty(), "stdout is empty: {out}");
    assert!(
        err.contains("declares nothing"),
        "the empty-module note is on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn symbols_on_a_missing_file_errors_with_the_cdz_prog_name() {
    // A load error names the tool (`cdz`) and exits non-zero.
    let (ok, _out, err) = run(&["symbols", "/no/such/file.sexp"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}
