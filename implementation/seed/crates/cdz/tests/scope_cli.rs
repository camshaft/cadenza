//! End-to-end tests for `cdz scope FILE OFFSET` — the visible-BINDINGS query (the LSP scope/completion
//! analogue).
//!
//! `scope` is a semantic query only the unified `cdz` binary can answer (compiler scope resolution + the
//! front-end span table in one process): given a source BYTE OFFSET, it lists the bindings visible there
//! as `file:line:col: name : type` (or, under `--json`, one structured object per binding an editor
//! consumes for a completion view). The scope-resolution logic is unit-tested in rcdzc; what is pinned
//! HERE is the CLI-RENDERING layer only `cdz` does: the human `file:line:col: name : type` line, the
//! `--json` object shape at row-parity, and the exit-code discipline (a listed scope exits 0; a missing
//! file / an offset with no node is an error exit 1; a missing OFFSET arg is a usage error exit 2).
//! Drives the built binary over a temp `.sexp` file.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_code, stdout, stderr). The exit CODE matters: `scope`
/// distinguishes 0 (listed) from 1 (I/O / no-node) from 2 (usage).
fn run(args: &[&str]) -> (i32, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// Write `src` to a unique temp `.sexp` file and return its path.
fn temp_src(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("cdz-scope-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

/// A module with a sibling `g` and an `f` whose body `(+ n 1)` references the param `n`.
const PROG: &str = "(module m (def (g) 5) (def (f (: n Int64)) (+ n 1)))";

/// The byte offset of the `(` opening `f`'s body `(+ n 1)` — a point where the param `n` is in scope.
fn body_offset() -> usize {
    PROG.find("(+ n 1)").expect("body present") + 1
}

#[test]
fn scope_lists_a_param_visible_at_a_body_offset_with_its_type_and_locus() {
    let file = temp_src("param", PROG);
    let off = body_offset().to_string();
    let (code, out, err) = run(&["scope", &file, &off]);
    assert_eq!(code, 0, "a scope query at a valid offset exits 0: {err}");
    // The param `n : Int64` is visible in `f`'s body, rendered `file:line:col: n : Int64`.
    assert!(
        out.contains("n : Int64"),
        "the in-scope param `n` is listed with its type: {out}"
    );
    // Every non-empty line carries a `file:line:col:` locus prefix.
    for l in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            l.starts_with(&file) && l[file.len()..].starts_with(':'),
            "binding line is `file:line:col: name : type`: {l}"
        );
    }
}

#[test]
fn scope_json_emits_one_structured_object_per_binding_at_row_parity() {
    // `--json` emits `{file,name,type,(line,col)}` per binding (the completion-view shape) — PARSED (a
    // substring check would pass for malformed JSON), at 1:1 row-parity with the human form (both from
    // the same resolved binding set, so they can't drift). Assert object SHAPE + parity, not line/col.
    let file = temp_src("json", PROG);
    let off = body_offset().to_string();
    let (code, out, err) = run(&["scope", &file, &off, "--json"]);
    assert_eq!(code, 0, "json scope exits 0: {err}");
    let rows: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    let (hcode, human, herr) = run(&["scope", &file, &off]);
    assert_eq!(hcode, 0, "baseline (non-json) scope exits 0: {herr}");
    let human_rows = human.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        rows.len(),
        human_rows,
        "one JSON row per human row (parity): {out}"
    );
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
        names.contains("n"),
        "the in-scope param `n` is emitted as a structured object: {names:?}"
    );
}

#[test]
fn scope_on_a_missing_file_is_an_error_exit_one() {
    // A missing input is an error (exit 1), and the message names the tool (`cdz`).
    let (code, _out, err) = run(&["scope", "/no/such/file.sexp", "10"]);
    assert_eq!(code, 1, "a missing file exits 1: {err}");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
}

#[test]
fn scope_at_an_offset_past_eof_is_an_error_exit_one() {
    // An offset with no node under it (past EOF) is an error (exit 1) with a clear "no node at byte
    // offset" message — NOT a silent empty success.
    let file = temp_src("eof", PROG);
    let (code, _out, err) = run(&["scope", &file, "999999"]);
    assert_eq!(code, 1, "an out-of-range offset exits 1: {err}");
    assert!(
        err.contains("no node at byte offset"),
        "clear no-node error: {err}"
    );
}

#[test]
fn scope_with_a_missing_offset_argument_is_a_usage_error_exit_two() {
    // `scope` needs FILE and OFFSET; omitting the offset is a clap usage error (exit 2), distinct from
    // an I/O error (1).
    let file = temp_src("arg", PROG);
    let (code, _out, err) = run(&["scope", &file]);
    assert_eq!(code, 2, "a missing offset is a usage error (exit 2): {err}");
    assert!(
        err.contains("error") || err.contains("Usage") || err.contains("required"),
        "clap usage error on stderr: {err}"
    );
}
