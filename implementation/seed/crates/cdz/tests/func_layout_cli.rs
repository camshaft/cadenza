//! End-to-end tests for `cdz func-layout FILE` — the CLI face over the compiler's `FuncLayout` sidecar
//! query. The query's own logic (forcing monomorphization, laying out the boundary, the per-def
//! content-hash) is exercised by `rcdzc`'s unit tests; what is pinned HERE is what only the `cdz` binary
//! does: following the entry's IMPORT CLOSURE so a package entry lays out the whole linked program, then
//! passing the query's rows through with a loud shape-check (never a silent drop of a format-skewed row).
//!
//! The command is also the CLI surface the compile-reuse byte-identity WITNESS rides on: a def
//! byte-identical across two programs reports the SAME func-index-relative content-hash, so a shared
//! import closure can be proven to emit identically across test files. The `a_byte_identical_def_hashes_
//! the_same_across_two_programs` test pins that invariant at the CLI layer.

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

/// Write `src` to a unique temp `.sexp` file (s-expr surface) and return its path as a String.
fn temp_src(tag: &str, src: &str) -> String {
    let dir = std::env::temp_dir().join(format!("cdz-fl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

/// The `hash` column of the first emitted row whose NAME starts with `prefix` (a recursive def emits under
/// `<name>$acc` after the accumulator transform, so match by prefix). Panics if no such row.
fn hash_of(out: &str, prefix: &str) -> String {
    out.lines()
        .find(|l| l.split('\t').nth(2).is_some_and(|n| n.starts_with(prefix)))
        .and_then(|l| l.split('\t').nth(1))
        .unwrap_or_else(|| panic!("a `{prefix}*` row with a hash:\n{out}"))
        .to_string()
}

/// A recursive def stays a standalone emitted function (a non-recursive small def inlines away and never
/// appears as a row). `sumto` + `main`, `main` exported → an export-rooted layout.
const SUMTO: &str = "(module m \
    (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
    (def (main) (sumto 5)) (export main))";

#[test]
fn reports_the_defs_begin_marker_and_a_func_index_row() {
    let file = temp_src("marker", SUMTO);
    let (ok, out, err) = run(&["func-layout", &file]);
    assert!(ok, "func-layout should succeed: {err}");
    let mut lines = out.lines();
    // First row is the boundary marker `defs-begin<TAB><import-base><TAB>-`. A scalar program imports no
    // runtime op → import-base 0.
    assert_eq!(
        lines.next(),
        Some("defs-begin\t0\t-"),
        "first row is the defs-begin marker with import-base:\n{out}"
    );
    // The recursive `sumto` is a standalone emitted function (emitted under `sumto$acc` after the
    // linear-recursion accumulator transform), so match by the `sumto` prefix.
    let row = out
        .lines()
        .find(|l| l.split('\t').nth(2).is_some_and(|n| n.starts_with("sumto")))
        .unwrap_or_else(|| panic!("a `sumto*` row (recursive def stays standalone):\n{out}"));
    let cols: Vec<&str> = row.split('\t').collect();
    assert_eq!(cols.len(), 3, "row is idx<TAB>hash<TAB>name: {row:?}");
    assert!(
        cols[0].parse::<u32>().is_ok(),
        "func-index is a number: {row:?}"
    );
    assert!(
        cols[1].len() == 16 && cols[1].chars().all(|c| c.is_ascii_hexdigit()),
        "the content-hash is 16 hex digits: {row:?}"
    );
}

#[test]
fn a_byte_identical_def_hashes_the_same_across_two_programs() {
    // The prove-first invariant the compile-reuse witness rides on: a def byte-identical in two DIFFERENT
    // programs reports the SAME content-hash — a function of the def's own AST subtree, NOT its global
    // position (which shifts when other defs precede it). Program B declares an EXTRA recursive `dbl`
    // BEFORE `sumto` (shifting sumto's global id + func-index); `sumto`'s own source is identical.
    let a = temp_src("hash-a", SUMTO);
    let b = temp_src(
        "hash-b",
        "(module m \
         (def (dbl (: k Int64)) (if (= k 0) 0 (+ 2 (dbl (- k 1))))) \
         (def (sumto (: n Int64)) (if (= n 0) 0 (+ n (sumto (- n 1))))) \
         (def (main) (+ (sumto 5) (dbl 3))) (export main))",
    );
    let (ok_a, out_a, err_a) = run(&["func-layout", &a]);
    let (ok_b, out_b, err_b) = run(&["func-layout", &b]);
    assert!(ok_a, "layout A: {err_a}");
    assert!(ok_b, "layout B: {err_b}");
    assert_eq!(
        hash_of(&out_a, "sumto"),
        hash_of(&out_b, "sumto"),
        "a byte-identical `sumto` hashes the same in both programs:\nA:\n{out_a}\nB:\n{out_b}"
    );
}

#[test]
fn a_malformed_row_would_fail_loudly_but_a_wellformed_layout_passes() {
    // The shape-check exists so a sidecar output-format regression cannot slip through as a success exit
    // with a silently-short result (the Copilot-flagged silent-skip class). We cannot inject a malformed
    // row from the CLI, so this pins the positive: a well-formed layout's rows all pass the check → rc 0
    // and every non-marker row is `idx<TAB>hash16<TAB>name`.
    let file = temp_src("wellformed", SUMTO);
    let (ok, out, _err) = run(&["func-layout", &file]);
    assert!(ok, "a well-formed layout passes the shape-check");
    for (n, line) in out.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols.len(), 3, "every row has 3 TAB fields: {line:?}");
        if n == 0 {
            assert_eq!(cols[0], "defs-begin", "row 0 is the marker: {line:?}");
        } else {
            assert!(
                cols[0] == "-" || cols[0].parse::<u32>().is_ok(),
                "func-index is a number or `-`: {line:?}"
            );
            assert!(
                cols[1].len() == 16 && cols[1].chars().all(|c| c.is_ascii_hexdigit()),
                "hash is 16 hex: {line:?}"
            );
            assert!(!cols[2].is_empty(), "name is non-empty: {line:?}");
        }
    }
}

#[test]
fn an_unloadable_file_fails() {
    // A path that does not exist is a load failure — rc≠0 with a `cdz:` diagnostic, not a silent empty.
    let (ok, _out, err) = run(&["func-layout", "/no/such/file/here.sexp"]);
    assert!(!ok, "a missing file fails");
    assert!(err.contains("cdz"), "a diagnostic is printed: {err}");
}
