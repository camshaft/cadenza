//! End-to-end tests for `cdz param-manifest FILE` — the `@param` WIDGET MANIFEST query (the data a HOST
//! reads to render a control per program parameter).
//!
//! The sidecar half (`Query::ParamManifest` — scan + declared-type render) is unit-tested in rcdzc; what
//! is pinned HERE is the CLI-RENDERING layer only `cdz` does: mapping the wire answer's value NODE IDS to
//! rendered source forms (range/options/default via the shared-`StructId` arena) + the name node to
//! `file:line:col` (via the span table), the human `file:line:col: name : type [widget=… …]` line, the
//! `--json` object (parsed, not substring-checked — null-not-omitted for absent config so a host gets a
//! stable schema), and the empty (no-@param) contract. Drives the built binary over a temp `.sexp` file.
//!
//! NOTE the s-expr surface spells a range list `(list 0 100)` — the `[0 100]` bracket sugar is ML-surface
//! only (the s-expr reader reads `[0`/`100]` as atoms), so an s-expr fixture uses `(list …)`.

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
    let dir = std::env::temp_dir().join(format!("cdz-parammanifest-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// A module with two `@param` sites: `width` (a slider with a range) and `mirror` (a toggle, no range).
const PROG: &str = "(module m \
    (: (@ (param (: widget slider) (: range (list 0 100))) width) Int64) \
    (: (@ (param (: widget toggle)) mirror) Bool) \
    (def (main) 0) (export main))";

#[test]
fn param_manifest_renders_each_site_human_readable() {
    let (dir, path) = temp_src("human", PROG);
    let (ok, out, err) = run(&["param-manifest", &path]);
    assert!(ok, "param-manifest should succeed: {out}{err}");
    // `width` — a slider with a rendered range; the declared type is the checker's `Int64`.
    assert!(
        out.lines().any(|l| l.contains("width : Int64")
            && l.contains("widget=slider")
            && l.contains("range=[0,100]")),
        "width row carries type + widget + rendered range: {out}"
    );
    // `mirror` — a toggle, Bool, NO range clause in the human summary.
    let mirror = out
        .lines()
        .find(|l| l.contains("mirror"))
        .expect("a mirror row");
    assert!(
        mirror.contains("mirror : Bool") && mirror.contains("widget=toggle"),
        "mirror row: {mirror}"
    );
    assert!(
        !mirror.contains("range="),
        "mirror has no range clause: {mirror}"
    );
    // The location points at the param name occurrence (a real file:line:col).
    assert!(
        out.lines().all(|l| l.starts_with(&format!("{path}:"))),
        "each row is anchored at file:line:col: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn param_manifest_json_is_a_stable_schema_per_param() {
    let (dir, path) = temp_src("json", PROG);
    let (ok, out, err) = run(&["param-manifest", &path, "--json"]);
    assert!(ok, "param-manifest --json should succeed: {out}{err}");
    let rows: Vec<serde_json::Value> = out
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {l}")))
        .collect();
    assert_eq!(rows.len(), 2, "two @param sites → two objects: {out}");

    let width = rows
        .iter()
        .find(|v| v["name"] == "width")
        .expect("width object");
    assert_eq!(
        width["type"], "Int64",
        "width type via the type column: {width}"
    );
    assert_eq!(width["widget"], "slider", "width widget: {width}");
    // range is a two-element array of the rendered element nodes (stable schema, not omitted).
    assert_eq!(width["range"][0], "0", "range lo: {width}");
    assert_eq!(width["range"][1], "100", "range hi: {width}");
    assert!(
        width["options"].is_null(),
        "absent options → null, not omitted: {width}"
    );
    assert!(width["default"].is_null(), "absent default → null: {width}");
    assert!(
        width["line"].is_number(),
        "name node maps to a line: {width}"
    );

    // mirror — the absent config fields are JSON null (the stable-schema contract a host relies on).
    let mirror = rows
        .iter()
        .find(|v| v["name"] == "mirror")
        .expect("mirror object");
    assert_eq!(mirror["type"], "Bool", "mirror type: {mirror}");
    assert_eq!(mirror["widget"], "toggle", "mirror widget: {mirror}");
    assert!(
        mirror["range"].is_null(),
        "mirror has no range → null: {mirror}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn param_manifest_on_a_paramless_program_reports_none() {
    // Total: a program with no `@param` sites succeeds and produces no manifest rows.
    let (dir, path) = temp_src("none", "(module m (def (main) 0) (export main))");
    let (ok, out, err) = run(&["param-manifest", &path]);
    assert!(ok, "a paramless program still succeeds: {out}{err}");
    assert!(
        out.trim().is_empty(),
        "no rows for a paramless program: {out}"
    );
    assert!(
        err.contains("no @param"),
        "a note names the absence rather than failing: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
