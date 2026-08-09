//! End-to-end tests for `cdz uses NAME FILE` — the "find all references" semantic query.
//!
//! `uses` is one of the in-process queries that only the unified `cdz` binary can answer (it needs the
//! COMPILER's resolution AND the front-end's span table in ONE process — the cross-process CLI throws
//! the spans away). The sidecar's reference-finding logic is unit-tested in rcdzc; what is pinned HERE
//! is the CLI-RENDERING layer only `cdz` does: mapping each referencing node id back to a source
//! `file:line:col` through the span table, one line per reference (or, under `--json`, one structured
//! object per reference for an editor's find-all-references), and the empty-result contract. Drives the
//! built binary over a temp `.sexp` file.

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

/// Write `src` to a unique temp `.sexp` file and return its path.
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-uses-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// `inc` is defined once and CALLED three times in `main` — so `uses inc` reports its call-site
/// references (the resolution the compiler provides, mapped to source positions).
const PROG: &str = "(module m \
    (def (inc (: n Int64)) (+ n 1)) \
    (def (main (: a Int64)) (+ (inc a) (inc (inc a)))) \
    (export main))";

#[test]
fn uses_reports_each_reference_as_file_line_col() {
    let (dir, file) = temp_src("refs", PROG);
    let (ok, out, err) = run(&["uses", "inc", &file]);
    assert!(ok, "cdz uses should succeed: {err}");
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    // `inc` is called three times — three referencing occurrences.
    assert_eq!(lines.len(), 3, "three call-site references to `inc`: {out}");
    // Every line is a `file:line:col` locus (the span-map payoff — the CLI's job, not the sidecar's).
    for l in &lines {
        assert!(
            l.starts_with(&file),
            "reference line is prefixed with the file path: {l}"
        );
        let rest = &l[file.len()..];
        let cols: Vec<&str> = rest.trim_start_matches(':').split(':').collect();
        assert!(
            cols.len() == 2 && cols[0].parse::<u32>().is_ok() && cols[1].parse::<u32>().is_ok(),
            "reference renders as file:LINE:COL with numeric line+col: {l}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_json_emits_one_structured_object_per_reference() {
    // `--json` emits one machine-readable object per reference — {file,line,col} — the shape an editor
    // consumes for a find-all-references result without re-parsing the `file:line:col` text. Both output
    // shapes are computed from the SAME resolved reference ids, so they keep row parity.
    let (dir, file) = temp_src("json", PROG);
    let (ok, out, err) = run(&["uses", "inc", &file, "--json"]);
    assert!(ok, "cdz uses --json should succeed: {err}");
    let rows: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    // `inc` is called three times — three JSON objects, matching the human row count.
    assert_eq!(rows.len(), 3, "one JSON object per reference: {out}");
    // PARSE each row (serde_json is in-crate) — a substring check would pass for MALFORMED JSON; parsing
    // rejects it. Every reference here has a span, so line/col are present + numeric.
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(
            v["line"].is_number() && v["col"].is_number(),
            "`line`/`col` are numbers (each reference has a span): {row}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_of_an_exported_name_finds_the_export_reference() {
    // `main` is referenced by `(export main)` — `uses main` finds that occurrence (a reference is any
    // resolved occurrence, not only a call). One locus, exit 0.
    let (dir, file) = temp_src("export", PROG);
    let (ok, out, err) = run(&["uses", "main", &file]);
    assert!(ok, "cdz uses main should succeed: {err}");
    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty() && lines.iter().all(|l| l.starts_with(&file)),
        "the export reference is reported as a file locus: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_of_an_unknown_name_reports_no_references_on_stderr_and_succeeds() {
    // An unknown name is TOTAL: empty stdout, a `no references to` note on stderr, SUCCESS exit (the
    // query ran; the name just names nothing). Pins the exact empty-result CLI contract.
    let (dir, file) = temp_src("ghost", PROG);
    let (ok, out, err) = run(&["uses", "ghost", &file]);
    assert!(ok, "an unknown name still exits successfully: {err}");
    assert!(
        out.trim().is_empty(),
        "stdout is empty for no references: {out}"
    );
    assert!(
        err.contains("no references to `ghost`"),
        "the not-found note is on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn uses_on_a_missing_file_errors_with_the_cdz_prog_name() {
    // A load error names the tool (`cdz`) and exits non-zero — the tool-level diagnostic contract.
    let (ok, _out, err) = run(&["uses", "inc", "/no/such/file.sexp"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}
