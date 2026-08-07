//! End-to-end tests for `cdz doc-module FILE` — the TYPE-ENRICHED doc-module extraction (cadenza-docs
//! I2). Drives the built `cdz` binary over a temp module and asserts the emitted doc-AST: the structural
//! doc-item (name / structured `(sig …)` / `(kind …)` / `(visibility …)`) from
//! `cadenza_syntax::doc_item::project` (I1), MERGED with the resolved `(ty …)` from the compiler's
//! `Query::ExportedTypes` sidecar. Only the unified `cdz` binary can answer this (front-end projection +
//! compiler types in one process), so it's pinned here rather than in either sub-crate.

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
    let dir = std::env::temp_dir().join(format!("cdz-docmodule-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

#[test]
fn doc_module_emits_a_typed_doc_item_for_an_exported_def() {
    // An exported, documented, TYPED function → a doc-item carrying name/sig/doc/kind/visibility AND the
    // resolved (ty …) (from the sidecar). `--to sexpr` renders the doc-AST for inspection.
    // A top-level program (a `do` of forms, as the parser produces): the `(doc …)` attaches INSIDE the
    // def (mirroring `take_docs_here`, which splices a `///` inside its item), and the export is
    // top-level. (A `(module …)`-nested program's exports are a separate I1 follow-up — see the note in
    // the empty-module test.)
    let prog = "(do \
        (def (inc (: n Int64)) (doc \"Increments an int.\") (+ n 1)) \
        (export inc))";
    let (dir, file) = temp_src("typed", prog);
    let (ok, out, err) = run(&["doc-module", &file, "--to", "sexpr"]);
    assert!(ok, "cdz doc-module should succeed: {err}");

    assert!(out.contains("doc-module"), "emits a doc-module: {out}");
    assert!(
        out.contains("(name \"inc\")"),
        "the exported item's name: {out}"
    );
    assert!(out.contains("(kind def)"), "the item kind: {out}");
    assert!(
        out.contains("(visibility public)"),
        "exported items are public: {out}"
    );
    // The resolved type — a (ty …) carrying the arrow. `inc : Int64 -> Int64` → (-> (Int 64) (Int 64)).
    assert!(
        out.contains("(ty ") && out.contains("->"),
        "the resolved type is grafted as a (ty …) arrow: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_module_default_output_is_canonical_binary() {
    // No --to → canonical binary (the doc index IS a binary AST); the output begins with the header.
    let prog = "(do (def (f (: x Int64)) x) (export f))";
    let (dir, file) = temp_src("binary", prog);
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(["doc-module", &file])
        .output()
        .expect("spawn cdz");
    assert!(out.status.success(), "cdz doc-module (binary) succeeds");
    assert!(
        out.stdout.starts_with(b"cdzast\x00\x01"),
        "default output is a canonical cdzast\\x00\\x01 artifact"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_module_omits_ty_when_the_type_does_not_resolve() {
    // `Int` is a WIDTH CONSTRUCTOR, not a sized type — the annotation is ill-typed, so the type doesn't
    // resolve; the doc-item still emits (structural), just WITHOUT a (ty …). Graceful-degrade, no crash.
    let prog = "(do (def (g (: n Int)) n) (export g))";
    let (dir, file) = temp_src("degrade", prog);
    let (ok, out, err) = run(&["doc-module", &file, "--to", "sexpr"]);
    assert!(
        ok,
        "cdz doc-module still succeeds on an unresolved type: {err}"
    );
    assert!(
        out.contains("(name \"g\")"),
        "the item still projects: {out}"
    );
    assert!(
        !out.contains("(ty "),
        "no (ty …) when the type didn't resolve (graceful-degrade): {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_module_of_a_module_with_no_exports_is_an_empty_doc_module() {
    // A module exporting nothing → a doc-module with no doc-items (never a crash / never a hang).
    let prog = "(do (def (h (: x Int64)) x))";
    let (dir, file) = temp_src("empty", prog);
    let (ok, out, err) = run(&["doc-module", &file, "--to", "sexpr"]);
    assert!(ok, "cdz doc-module succeeds on a no-export module: {err}");
    assert!(out.contains("doc-module"), "still a doc-module: {out}");
    assert!(
        !out.contains("doc-item"),
        "nothing exported → no doc-items: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
