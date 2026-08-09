//! End-to-end tests for `cdz corpus` — the executable-semantics corpus tool FOLDED into the unified
//! `cdz` binary (from the `cdz-corpus` lib). Part of the one-binary story: the corpus maintenance tool
//! needn't be a separate binary on the PATH. The standalone `cdz-corpus` bin remains a thin shim over
//! the same code (xtask shells out to it); this pins the mounted `cdz corpus` path. Drives the built
//! `cdz` binary over a tiny corpus file written to a temp dir.

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

/// Write `src` to a unique temp `.sexp` corpus file and return its path.
fn temp_corpus(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-corpus-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(format!("{tag}.sexp"));
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// A minimal two-case corpus file (the test-DSL `(case …)` vocabulary).
const CORPUS: &str = "\
(case \"a decimal literal\" (input 42) (output (: 42 Int64)))
(case \"addition\" (input (+ 1 2)) (output (: 3 Int64)))
";

#[test]
fn cdz_corpus_records_emits_the_normalized_record_stream() {
    // `cdz corpus records` reads a corpus file and prints one normalized record per case — the flat
    // stream the xtask gate consumes. Pins the mounted path produces the expected record shape.
    let (dir, file) = temp_corpus("records", CORPUS);
    let (ok, out, err) = run(&["corpus", "records", &file]);
    assert!(ok, "cdz corpus records failed: {err}");
    // Each case becomes a `case\t…` + `program\t…` + `expect\t…` record, `---`-separated. The bare
    // input `42` normalizes to the runnable export shape.
    assert!(out.contains("case\ta decimal literal"), "case line: {out}");
    assert!(
        out.contains("program\t(do (def (main) 42) (export main))"),
        "normalized program: {out}"
    );
    assert!(
        out.contains("expect\toutput (: 42 Int64)"),
        "expect line: {out}"
    );
    assert!(out.contains("case\taddition"), "second case present: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_corpus_migrate_projects_to_markdown() {
    // `cdz corpus migrate` (no --write) prints literate markdown to stdout for a `.sexp` corpus.
    let (dir, file) = temp_corpus("migrate", CORPUS);
    let (ok, out, err) = run(&["corpus", "migrate", &file]);
    assert!(ok, "cdz corpus migrate failed: {err}");
    // A migrated document carries the case descriptions as prose and the programs in tagged fences.
    assert!(
        out.contains("a decimal literal") && out.contains("addition"),
        "both cases in the markdown: {out}"
    );
    assert!(out.contains("```"), "markdown has code fences: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_corpus_check_passes_for_an_unmigrated_sexp_file() {
    // `cdz corpus check` verifies a migration preserves the record stream; an `.sexp` file trivially
    // round-trips through migrate→read. Exit 0 + an "ok" report line.
    let (dir, file) = temp_corpus("check", CORPUS);
    let (ok, out, err) = run(&["corpus", "check", &file]);
    assert!(ok, "cdz corpus check failed: {err}\n{out}");
    assert!(
        out.contains("ok") || out.contains("preserve the record stream"),
        "check reports success: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cdz_corpus_records_on_a_missing_file_errors_with_the_cdz_prog_name() {
    // A read error names the tool the user typed (`cdz`), not `cdz-corpus` — the prog name threads
    // through the mounted entry point. Non-zero exit.
    let (ok, _out, err) = run(&["corpus", "records", "/no/such/corpus.sexp"]);
    assert!(!ok, "a missing corpus file should fail");
    assert!(
        err.contains("cdz:") && err.to_lowercase().contains("reading"),
        "error names `cdz` and mentions the read failure: {err}"
    );
}
