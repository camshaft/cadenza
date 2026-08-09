//! End-to-end tests for `cdz diff FILE_A FILE_B` — the STRUCTURAL AST diff (which subtrees changed).
//!
//! `diff` is a structural query only the unified `cdz` binary exposes: it reads two programs, compares
//! their binary ASTs, and reports the changed subtrees as `path: kind old => new` (human) or, under
//! `--json`, a machine-readable `[{path, kind, old?, new?}]`. The diff ALGORITHM lives in cadenza-syntax
//! and is unit-tested there; what is pinned HERE is the CLI contract only `cdz` owns: the no-change vs
//! changed rendering, `--json` shape, and the exit-code discipline (identical/changed both exit 0 — a
//! diff is a REPORT, not a failure; a missing file is an error exit 1; a missing argument is a usage
//! error exit 2). Drives the built binary over temp `.sexp` files.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_code, stdout, stderr). Uses the exit CODE (not just ok) because
/// `diff` distinguishes 0 (ran: report) from 1 (I/O error) from 2 (usage error).
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
    let dir = std::env::temp_dir().join(format!("cdz-diff-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(format!("{tag}.sexp"));
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

const PROG: &str = "(module m (def (f (: n Int64)) (+ n 1)))";

#[test]
fn diff_of_identical_programs_reports_no_change_and_exits_zero() {
    // Two byte-identical programs have no structural change: a `no structural changes` note and exit 0
    // (a diff is a REPORT — "nothing changed" is a successful run, not a failure).
    let a = temp_src("id-a", PROG);
    let b = temp_src("id-b", PROG);
    let (code, out, err) = run(&["diff", &a, &b]);
    assert_eq!(code, 0, "identical programs exit 0: stderr={err}");
    assert!(
        out.contains("no structural changes") || err.contains("no structural changes"),
        "reports no changes: out={out} err={err}"
    );
}

#[test]
fn diff_of_a_changed_subtree_reports_the_replace_and_exits_zero() {
    // Changing the `1` literal to `2` is a single subtree REPLACE. The human form names the change as
    // `replace 1 => 2`; the exit is still 0 (a reported difference is not an error).
    let a = temp_src("ch-a", PROG);
    let b = temp_src("ch-b", "(module m (def (f (: n Int64)) (+ n 2)))");
    let (code, out, err) = run(&["diff", &a, &b]);
    assert_eq!(code, 0, "a reported diff still exits 0: stderr={err}");
    assert!(
        out.contains("replace") && out.contains("1") && out.contains("2"),
        "reports the 1 => 2 replace: {out}"
    );
}

#[test]
fn diff_json_emits_a_machine_readable_change_array() {
    // `--json` emits `[{path, kind, old?, new?}]` — the shape a tool consumes. PARSE it (a substring
    // check would pass for malformed JSON) and assert the change object's fields, not the exact path
    // (the numeric path is a layout detail).
    let a = temp_src("js-a", PROG);
    let b = temp_src("js-b", "(module m (def (f (: n Int64)) (+ n 2)))");
    let (code, out, err) = run(&["diff", &a, &b, "--json"]);
    assert_eq!(code, 0, "json diff exits 0: stderr={err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    let arr = v.as_array().expect("the diff is a JSON array");
    assert!(
        !arr.is_empty(),
        "the changed program yields >=1 change: {out}"
    );
    let first = &arr[0];
    assert!(first["kind"].is_string(), "a change has a `kind`: {out}");
    assert!(
        first["path"].is_array(),
        "a change has a `path` array: {out}"
    );
}

#[test]
fn diff_of_identical_programs_emits_an_empty_json_array() {
    // No change under `--json` is the empty array `[]` (not absent output) — a machine consumer can
    // distinguish "ran, no changes" from an error by the exit code (0) + valid empty array.
    let a = temp_src("emptyjson-a", PROG);
    let b = temp_src("emptyjson-b", PROG);
    let (code, out, _err) = run(&["diff", &a, &b, "--json"]);
    assert_eq!(code, 0, "identical --json exits 0");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    assert_eq!(
        v.as_array().map(|a| a.len()),
        Some(0),
        "no change is the empty array: {out}"
    );
}

#[test]
fn diff_on_a_missing_file_is_an_io_error_exit_one() {
    // A missing input is an I/O ERROR (exit 1), NOT "no changes" (exit 0) — a tool must be able to tell
    // "the files are the same" from "I couldn't read a file". The error names the tool (`cdz`).
    let a = temp_src("miss-a", PROG);
    let (code, _out, err) = run(&["diff", &a, "/no/such/file.sexp"]);
    assert_eq!(
        code, 1,
        "a missing file exits 1 (I/O error), not 0: stderr={err}"
    );
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
}

#[test]
fn diff_with_a_missing_argument_is_a_usage_error_exit_two() {
    // `diff` needs TWO files; one argument is a clap USAGE error (exit 2) — distinct from an I/O error.
    let a = temp_src("arg-a", PROG);
    let (code, _out, err) = run(&["diff", &a]);
    assert_eq!(
        code, 2,
        "a missing second file is a usage error (exit 2): stderr={err}"
    );
    assert!(
        err.contains("error") || err.contains("Usage") || err.contains("required"),
        "clap usage error on stderr: {err}"
    );
}

#[test]
fn diff_is_structural_not_textual_across_surfaces() {
    // `cdz diff` compares the AST, not the source text — so the SAME program written in DIFFERENT surfaces
    // (s-expr vs ML) has NO structural change. The `--from` per-file inference (by extension) means diffing
    // an `.sexp` against its `.cdz` equivalent must report `no structural changes` + exit 0. A regression
    // that made diff surface-sensitive (comparing tokens/text) would report spurious changes here — this
    // pins the surface-independence that is the whole point of a STRUCTURAL diff. (All the other tests use a
    // single surface, so none witnesses this.) The `.cdz` form is produced by `cdz convert` so the two are
    // guaranteed to be the same program in two spellings, not a hand-transliteration that could drift.
    let dir = std::env::temp_dir().join(format!("cdz-diff-xsurface-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let sexpr = dir.join("p.sexp");
    std::fs::write(&sexpr, PROG).unwrap();

    // Convert the s-expr program to the ML surface (a .cdz file) — the exact same AST, different spelling.
    let exe = env!("CARGO_BIN_EXE_cdz");
    let conv = Command::new(exe)
        .args(["convert", sexpr.to_str().unwrap(), "--to", "ml"])
        .output()
        .expect("spawn cdz convert");
    assert!(
        conv.status.success(),
        "convert to ml succeeds: {}",
        String::from_utf8_lossy(&conv.stderr)
    );
    let ml = dir.join("p.cdz");
    std::fs::write(&ml, &conv.stdout).unwrap();

    let (code, out, err) = run(&["diff", sexpr.to_str().unwrap(), ml.to_str().unwrap()]);
    assert_eq!(
        code, 0,
        "diffing a program against its own other-surface form exits 0: out={out} err={err}"
    );
    assert!(
        out.contains("no structural changes") || err.contains("no structural changes"),
        "the same program in two surfaces has NO structural change (diff is structural, not textual): \
         out={out} err={err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
