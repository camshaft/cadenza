//! End-to-end tests for `cdz doc-at FILE OFFSET` — "documentation at cursor" (the LSP hover-doc analogue).
//!
//! `doc-at` resolves a source BYTE OFFSET to a node, then to the definition it IS or REFERENCES, and prints
//! that definition's `(doc "…")` text. It's the doc companion of `cdz type-at`/`cdz def`. The sidecar's
//! DocAt logic is unit-tested in rcdzc; what is pinned HERE is the CLI layer — resolving the offset, printing
//! the doc across a reference, and the no-doc / no-node / missing-file / bad-offset contracts. `doc-at` had NO
//! test file before this (a thin-coverage gap in the query surface); an un-witnessed behavior is unprotected,
//! so this pins the contract so a future change to the offset-resolution or doc-lookup can't silently regress.
//! Drives the built binary over a temp `.sexp` file.

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
    let dir = std::env::temp_dir().join(format!("cdz-docat-{tag}-{}", std::process::id()));
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

/// `inc` carries a `(doc …)`; `main` does not. `main` REFERENCES `inc`, so a cursor on that reference must
/// resolve to `inc`'s doc — the "hover the call site, see the callee's doc" contract.
const PROG: &str = "(module m \
    (def (inc (: n Int64)) (doc \"Adds one to n.\") (+ n 1)) \
    (def (main (: a Int64)) (inc a)) (export main))";

/// Byte offset of the first occurrence of `needle` (+ `plus`), as a decimal string — names a cursor by the
/// text it sits on, not a magic number.
fn offset_of(src: &str, needle: &str, plus: usize) -> String {
    (src.find(needle).expect("needle in source") + plus).to_string()
}

#[test]
fn doc_at_a_reference_prints_the_referenced_definitions_doc() {
    let (dir, file, src) = temp_src("ref", PROG);
    // Cursor on the `inc` call inside `main` (`(inc a)`) — resolves to `inc`, whose doc is printed.
    let off = offset_of(&src, "(inc a)", 1);
    let (ok, out, err) = run(&["doc-at", &file, &off]);
    assert!(ok, "doc-at on a documented reference succeeds: {err}");
    assert!(
        out.contains("Adds one to n."),
        "the referenced definition's doc is printed: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_at_the_definition_itself_prints_its_doc() {
    let (dir, file, src) = temp_src("def", PROG);
    // Cursor on the `inc` DEFINITION name.
    let off = offset_of(&src, "(inc (: n", 1);
    let (ok, out, err) = run(&["doc-at", &file, &off]);
    assert!(ok, "doc-at on the definition itself succeeds: {err}");
    assert!(
        out.contains("Adds one to n."),
        "the definition's own doc is printed: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_at_an_undocumented_node_reports_no_documentation_and_succeeds() {
    // A valid node whose definition has NO `(doc …)` is not an error — an empty result is SUCCESS (exit 0),
    // like `uses`/`symbols` (distinct from `type-at`'s no-NODE, which IS a failure). Pins that contract so a
    // "hover on an undocumented thing" stays a clean exit-0 note, not a spurious non-zero.
    let (dir, file, src) = temp_src("nodoc", PROG);
    let off = offset_of(&src, "(main (: a", 1);
    let (ok, out, _err) = run(&["doc-at", &file, &off]);
    assert!(
        ok,
        "an undocumented-but-valid node is not a failure (exit 0)"
    );
    assert!(
        out.contains("no documentation"),
        "reports there is no documentation at the offset: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_at_an_offset_with_no_node_reports_no_node_and_fails() {
    // An out-of-range offset resolves to NO node — there is nothing to document there, so this is a FAILURE
    // (non-zero), distinct from the undocumented-valid-node case above. Pins the no-node contract on stderr.
    let (dir, file, _src) = temp_src("nonode", PROG);
    let (ok, _out, err) = run(&["doc-at", &file, "99999"]);
    assert!(!ok, "an offset with no node fails (nothing to document)");
    assert!(
        err.contains("no node at byte offset"),
        "reports there is no node at the offset: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_at_on_a_missing_file_errors_with_the_cdz_prog_name() {
    let (ok, _out, err) = run(&["doc-at", "/no/such/file.sexp", "5"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}

#[test]
fn doc_at_a_non_numeric_offset_names_what_a_byte_offset_is() {
    // A non-numeric OFFSET (a stale/mis-typed editor arg) must get an ACTIONABLE message naming what the
    // argument is — a 0-based byte offset — not clap's bare `invalid digit found in string`. The custom
    // `value_parser` is shared by def/scope/type-at/doc-at; pin it fires for doc-at too.
    let (dir, file, _src) = temp_src("badoffset", PROG);
    let (ok, _out, err) = run(&["doc-at", &file, "not-a-number"]);
    assert!(!ok, "a non-numeric offset is a usage error");
    assert!(
        err.contains("byte offset"),
        "names what a byte offset is (actionable, not clap's bare digit error): {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
