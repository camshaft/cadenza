//! End-to-end tests for `cdz doc <name> FILE` — the documentation query.
//!
//! Focus: the EXIT CODE distinguishes an UNRESOLVABLE name from a real-but-undocumented one. The `DocOf`
//! sidecar query is total — it returns a defined line for the doc text, for "no documentation for `X`" (a
//! real def with no doc), and for "no such definition `X`" (a typo). `cdz doc` maps only the last (an
//! unresolvable name) to a non-zero exit, so a script can tell "you misspelled it" from "this exists but
//! is undocumented". Drives the built binary.

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

/// Write a module with a documented def, an undocumented def, and export both. Returns the file path.
fn temp_module(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-doc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("m.sexp");
    std::fs::write(
        &f,
        "(module m (def (documented (: n Int64)) (doc \"the doc text\") n) \
         (def (plain (: n Int64)) n) (export documented plain))\n",
    )
    .unwrap();
    f
}

#[test]
fn doc_of_a_documented_definition_prints_the_doc_and_succeeds() {
    let f = temp_module("documented");
    let (ok, out, err) = run(&["doc", "documented", f.to_str().unwrap()]);
    assert!(ok, "cdz doc of a documented def should succeed: {err}");
    assert!(out.contains("the doc text"), "prints the doc text: {out}");
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn doc_of_a_real_but_undocumented_definition_succeeds() {
    // A REAL definition that carries no doc is a legitimate answer ("no documentation for `X`") — success,
    // NOT an error. Asking for the doc of something that exists is valid even when there's no doc.
    let f = temp_module("plain");
    let (ok, out, err) = run(&["doc", "plain", f.to_str().unwrap()]);
    assert!(
        ok,
        "cdz doc of a real undocumented def should still SUCCEED: {err}"
    );
    assert!(
        out.contains("no documentation for `plain`"),
        "reports the undocumented def: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn doc_of_an_unresolvable_name_fails() {
    // A name that resolves to NOTHING (a typo) is a FAILURE — distinct from a real-but-undocumented def —
    // so a script can tell the two apart. The message is the sidecar's "no such definition `X`".
    let f = temp_module("unknown");
    let (ok, out, _err) = run(&["doc", "totally_unknown_zzz", f.to_str().unwrap()]);
    assert!(
        !ok,
        "cdz doc of an unresolvable name must FAIL (non-zero exit)"
    );
    assert!(
        out.contains("no such definition `totally_unknown_zzz`"),
        "reports the unresolvable name distinctly: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}

#[test]
fn doc_of_a_misspelled_name_fails_with_a_did_you_mean() {
    // A near-miss of a real name still FAILS (unresolvable) but carries a "did you mean `Y`?" hint.
    let f = temp_module("typo");
    let (ok, out, _err) = run(&["doc", "documentd", f.to_str().unwrap()]); // near "documented"
    assert!(!ok, "a misspelled (unresolvable) name must fail");
    assert!(
        out.contains("no such definition") && out.contains("did you mean `documented`"),
        "the failure suggests the near name: {out}"
    );
    let _ = std::fs::remove_dir_all(f.parent().unwrap());
}
