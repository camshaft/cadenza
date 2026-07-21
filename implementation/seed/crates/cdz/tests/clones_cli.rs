//! End-to-end tests for `cdz clones` — the duplicated-subtree (clone) detector.
//!
//! `clones` finds repeated subtrees within/across programs: EXACT clones by default, or NEAR-clones
//! (`--near`, same shape / differing leaves, reported as a `,mK`-metavariable pattern feedable into
//! `rewrite`). The clone-detection engine lives in cadenza-syntax; what is pinned HERE is the CLI
//! contract only `cdz` owns: the human `clone: N occurrences, M nodes: <exemplar>` + loci rendering,
//! the exact-vs-near `--json` shapes (`{exemplar,size,sites}` vs `{pattern,size,holes,sites}`), the
//! `--min-size` filter's no-clone note + exit 0, and a missing-file error. Drives the built binary.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_code, stdout, stderr).
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
    let dir = std::env::temp_dir().join(format!("cdz-clones-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

/// Two defs sharing the exact subtree `(* 2 3)`, and — with differing trailing leaves (1 vs 9) — a
/// NEAR-clone `(+ (* 2 3) ,m0)`.
const PROG: &str = "(module m (def (a) (+ (* 2 3) 1)) (def (b) (+ (* 2 3) 9)) (export a))";

#[test]
fn clones_reports_an_exact_duplicated_subtree_with_loci() {
    let file = temp_src("exact", PROG);
    let (code, out, err) = run(&["clones", &file]);
    assert_eq!(code, 0, "a clone report is a REPORT, exits 0: {err}");
    assert!(
        out.contains("(* 2 3)"),
        "the duplicated subtree `(* 2 3)` is reported: {out}"
    );
    // The two occurrence sites are listed with a `file:line:col` locus each.
    let loci = out.lines().filter(|l| l.contains(&file)).count();
    assert!(loci >= 2, "both occurrence sites are listed: {out}");
}

#[test]
fn clones_json_emits_exemplar_size_and_sites() {
    // Exact `--json`: `[{exemplar, size, sites:[{file,line,col}]}]` — PARSED, asserting fields.
    let file = temp_src("json", PROG);
    let (code, out, err) = run(&["clones", &file, "--json"]);
    assert_eq!(code, 0, "json clones exits 0: {err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    let arr = v.as_array().expect("clone classes are a JSON array");
    assert!(!arr.is_empty(), "at least one clone class: {out}");
    let c = &arr[0];
    assert!(
        c["exemplar"].is_string(),
        "a class has an `exemplar`: {out}"
    );
    assert!(c["size"].is_number(), "a class has a `size`: {out}");
    let sites = c["sites"].as_array().expect("`sites` is an array");
    assert!(sites.len() >= 2, "the clone has >=2 sites: {out}");
    assert!(
        sites[0]["file"].is_string() && sites[0]["line"].is_number(),
        "each site has file+line: {out}"
    );
}

#[test]
fn clones_near_reports_a_metavariable_pattern() {
    // `--near`: same shape, differing leaves → a `,mK`-hole pattern (feedable into `rewrite`). The JSON
    // shape differs from exact: `{pattern, size, holes, sites}`.
    let file = temp_src("near", PROG);
    let (code, out, err) = run(&["clones", &file, "--near", "--json"]);
    assert_eq!(code, 0, "near-clones exits 0: {err}");
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("valid JSON ({e}): {out}"));
    let arr = v.as_array().expect("near-clone classes are a JSON array");
    assert!(!arr.is_empty(), "at least one near-clone class: {out}");
    let c = &arr[0];
    assert!(
        c["pattern"].as_str().is_some_and(|p| p.contains(",m")),
        "a near-clone reports a `,mK`-metavariable pattern: {out}"
    );
    assert!(
        c["holes"].is_number(),
        "a near-clone has a `holes` count: {out}"
    );
}

#[test]
fn clones_with_min_size_above_any_clone_reports_none_and_exits_zero() {
    // The `--min-size` filter suppresses small clones. Above every clone's size: a clear `no clones`
    // note (naming the min-size) and exit 0 — a REPORT of "nothing", not an error.
    let file = temp_src("minsize", PROG);
    let (code, out, err) = run(&["clones", &file, "--min-size", "99"]);
    assert_eq!(code, 0, "no-clone run exits 0");
    assert!(
        out.contains("no clones") || err.contains("no clones"),
        "a clear no-clones note: out={out} err={err}"
    );
}

#[test]
fn clones_on_a_missing_file_errors() {
    let (code, _out, err) = run(&["clones", "/no/such/file.sexp"]);
    assert_ne!(code, 0, "a missing file is an error");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
}
