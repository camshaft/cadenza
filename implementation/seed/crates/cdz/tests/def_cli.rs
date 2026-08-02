//! End-to-end tests for `cdz def FILE OFFSET` — "go to definition" (the LSP go-to-def analogue).
//!
//! `def` resolves a source BYTE OFFSET to the token there and prints the `file:line:col` of the DEFINITION
//! it references (`--json` → a `{file,line,col}` object for an editor jump). Sibling of `cdz type-at`/`cdz
//! doc-at`. The sidecar's resolution logic is unit-tested in rcdzc; what is pinned HERE is the CLI layer —
//! offset→token→definition location, the JSON shape, and the no-def / no-node / missing-file / bad-offset
//! contracts. `def` had NO test file before this (a thin-coverage gap in the query surface, the sibling of
//! the doc-at gap); an un-witnessed behavior is unprotected, so this pins the contract against a silent
//! regression in offset-resolution or go-to-def lookup. Drives the built binary over a temp `.sexp` file.
//!
//! Note the CONTRAST with `doc-at`: a valid token with NO navigable definition (a literal / builtin) is a
//! FAILURE here (exit non-zero — go-to-def has nowhere to jump), whereas doc-at's "no documentation" on a
//! valid node is a SUCCESS (empty-result-is-success). Both contracts are pinned in their respective suites.

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

/// Write `src` to a unique temp `.sexp` file; returns (dir, path, source).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("cdz-def-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).expect("write test file");
    // `.expect`, NOT `to_string_lossy`: the path is passed as a `&str` CLI arg, so it MUST be UTF-8 — a loud
    // fail-fast on a non-UTF-8 temp dir beats lossy U+FFFD corruption surfacing later as a confusing "missing
    // file".
    (
        dir,
        path.to_str()
            .expect("temp path must be valid UTF-8")
            .to_string(),
        src.to_string(),
    )
}

/// `main` references `inc`; a cursor on that reference must jump to `inc`'s DEFINITION.
const PROG: &str =
    "(module m (def (inc (: n Int64)) (+ n 1)) (def (main (: a Int64)) (inc a)) (export main))";

/// Byte offset of the first occurrence of `needle` (+ `plus`), as a decimal string — names a cursor by the
/// text it sits on, not a magic number.
fn offset_of(src: &str, needle: &str, plus: usize) -> String {
    (src.find(needle).expect("needle in source") + plus).to_string()
}

#[test]
fn def_on_a_reference_prints_the_definition_location() {
    let (dir, file, src) = temp_src("ref", PROG);
    // Cursor on the `inc` call inside `main` (`(inc a)`) — jumps to `inc`'s definition, at column 17
    // (the `inc` def name in the single-line source).
    let off = offset_of(&src, "(inc a)", 1);
    let (ok, out, err) = run(&["def", &file, &off]);
    assert!(ok, "def on a reference succeeds: {err}");
    assert!(
        out.contains(&format!("{file}:1:17")),
        "jumps to the `inc` definition at line 1 col 17: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_json_emits_a_file_line_col_object() {
    let (dir, file, src) = temp_src("json", PROG);
    let off = offset_of(&src, "(inc a)", 1);
    let (ok, out, err) = run(&["def", &file, &off, "--json"]);
    assert!(ok, "def --json succeeds: {err}");
    // A machine-readable object an editor consumes for the jump — exact fields + values.
    assert!(
        out.contains("\"line\":1") && out.contains("\"col\":17") && out.contains("\"file\":"),
        "the JSON carries file/line/col for the jump: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_on_a_token_with_no_navigable_definition_fails() {
    // A valid token that is NOT a reference to a user definition (a literal / builtin) has nowhere to jump —
    // go-to-def is a FAILURE (non-zero), NOT an empty success. This is the deliberate CONTRAST with doc-at
    // (where an undocumented valid node exits 0); pin it so the two contracts can't be conflated by a refactor.
    let (dir, file, src) = temp_src("nodef", PROG);
    // Cursor on the `1` literal in `(+ n 1)`.
    let off = offset_of(&src, "n 1)", 2);
    let (ok, _out, err) = run(&["def", &file, &off]);
    assert!(!ok, "a token with no navigable definition fails");
    assert!(
        err.contains("no definition"),
        "reports there is no definition for the token: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_an_offset_with_no_node_reports_no_node_and_fails() {
    let (dir, file, _src) = temp_src("nonode", PROG);
    let (ok, _out, err) = run(&["def", &file, "99999"]);
    assert!(!ok, "an out-of-range offset fails");
    assert!(
        err.contains("no node at byte offset"),
        "reports there is no node at the offset: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_on_a_missing_file_errors_with_the_cdz_prog_name() {
    let (ok, _out, err) = run(&["def", "/no/such/file.sexp", "5"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}

#[test]
fn def_a_non_numeric_offset_names_what_a_byte_offset_is() {
    // The shared byte-offset `value_parser` (def/scope/type-at/doc-at) must give an ACTIONABLE message, not
    // clap's bare digit error — pin it fires for `def` too.
    let (dir, file, _src) = temp_src("badoffset", PROG);
    let (ok, _out, err) = run(&["def", &file, "not-a-number"]);
    assert!(!ok, "a non-numeric offset is a usage error");
    assert!(
        err.contains("byte offset"),
        "names what a byte offset is (actionable): {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
