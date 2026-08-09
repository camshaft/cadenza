//! End-to-end tests for `cdz highlight FILE` — SEMANTIC syntax highlighting (the LSP `semanticTokens`
//! analogue): every token classified by the ROLE it plays (type vs function vs param vs local vs
//! keyword vs literal), not by spelling.
//!
//! An existing test in `compile_debug_cli.rs` pins the POSITION mapping (each token's line:col via the
//! shared LineIndex) and that `keyword`/`number` kinds appear. What is pinned HERE is the SEMANTIC
//! CLASSIFICATION itself — the payoff of the feature: a type name is `type`, a called function is
//! `function`, a parameter is `param`, a numeric literal is `number`, the surface keyword is `keyword`.
//! A regression that mis-coloured (e.g. classified a type as a variable) would slip past the
//! position-only test but fails here. Drives the built binary; `file:line:col: kind` per token (or, under
//! `--json`, one structured `{file,line,col,kind}` object per token — the machine-readable semanticTokens
//! payload).

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
    let dir = std::env::temp_dir().join(format!("cdz-hl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

/// The set of `kind` labels present in `cdz highlight` output (the third `:`-field after `file:line:col`).
fn kinds(out: &str) -> std::collections::BTreeSet<String> {
    out.lines()
        .filter_map(|l| l.rsplit(": ").next())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect()
}

/// A program exercising several token ROLES: a called function (`inc`), a parameter (`n`/`a`), a type
/// annotation (`Int64`), numeric literals, and the surface keywords.
const PROG: &str = "(module m \
    (def (inc (: n Int64)) (+ n 1)) \
    (def (main (: a Int64)) (inc a)) \
    (export main))";

#[test]
fn highlight_classifies_tokens_by_semantic_role() {
    let (dir, file) = temp_src("kinds", PROG);
    let (ok, out, err) = run(&["highlight", &file]);
    assert!(ok, "cdz highlight should succeed: {err}");
    let ks = kinds(&out);
    // The semantic payoff: a TYPE annotation is `type`, a called function is `function`, a parameter is
    // `param`, a literal is `number`, the surface form head is `keyword`. Each role must be represented
    // (a mis-classification — e.g. `Int64` coloured as a plain variable — drops the `type` kind).
    for role in ["keyword", "type", "function", "param", "number"] {
        assert!(
            ks.contains(role),
            "expected a `{role}` token in the classification; got kinds {ks:?}\n{out}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn highlight_marks_the_type_annotation_as_a_type() {
    // Pin the specific classification most prone to regression: the `Int64` in `(: n Int64)` is a TYPE,
    // not a value/variable. Find the line for `Int64`'s column and assert its kind is `type`.
    let (dir, file) = temp_src("type", PROG);
    let (ok, out, err) = run(&["highlight", &file]);
    assert!(ok, "highlight failed: {err}");
    // At least one token classified `type` (the two `Int64` annotations). The position-only test never
    // checks a kind is `type`, so this is the new guard.
    assert!(
        out.lines().any(|l| l.trim_end().ends_with(": type")),
        "a type annotation is classified `type`: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn highlight_every_line_is_file_line_col_kind() {
    // The render contract: each emitted line is `file:LINE:COL: kind` with numeric line+col. A malformed
    // render (missing the locus, or the kind) fails here.
    let (dir, file) = temp_src("shape", PROG);
    let (ok, out, err) = run(&["highlight", &file]);
    assert!(ok, "highlight failed: {err}");
    let mut count = 0;
    for l in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            l.starts_with(&file),
            "line prefixed with the file path: {l}"
        );
        // `file:LINE:COL: kind` — after the path, `:LINE:COL: kind`.
        let rest = &l[file.len()..];
        let parts: Vec<&str> = rest.splitn(4, ':').collect();
        // parts = ["", LINE, COL, " kind"]
        assert!(
            parts.len() == 4
                && parts[1].trim().parse::<u32>().is_ok()
                && parts[2].trim().parse::<u32>().is_ok()
                && !parts[3].trim().is_empty(),
            "token line is file:LINE:COL: kind: {l}"
        );
        count += 1;
    }
    assert!(
        count >= 5,
        "a non-trivial program yields several tokens: {count}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn highlight_json_emits_one_structured_object_per_token() {
    // `--json` emits one machine-readable object per classified token — {file,line,col,kind} — the
    // `semanticTokens` payload an editor consumes without re-parsing the `file:line:col: kind` text. Both
    // output shapes come from the SAME resolved token set (span-less nodes are skipped in both), so they
    // keep row-for-row parity.
    let (dir, file) = temp_src("json", PROG);
    let (ok, out, err) = run(&["highlight", &file, "--json"]);
    assert!(ok, "cdz highlight --json should succeed: {err}");
    let rows: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    // Same token count as the human form (parity — both skip span-less nodes identically). Assert the
    // baseline run SUCCEEDED — otherwise human_rows would be 0 and the parity check would pass hollowly.
    let (hok, human, herr) = run(&["highlight", &file]);
    assert!(
        hok,
        "baseline (non-json) highlight run should succeed: {herr}"
    );
    let human_rows = human.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(
        rows.len(),
        human_rows,
        "one JSON row per human token: {out}"
    );
    assert!(
        rows.len() >= 5,
        "a non-trivial program yields several tokens"
    );
    // PARSE each row as JSON (serde_json is in-crate) — a substring check would pass for MALFORMED JSON
    // (a missing comma / bad escaping); parsing rejects it. Assert the typed fields on the parsed value.
    let mut kinds = std::collections::BTreeSet::new();
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(
            v["line"].is_number() && v["col"].is_number(),
            "`line`/`col` are numbers (every token has a span): {row}"
        );
        let kind = v["kind"].as_str().expect("`kind` is a string");
        kinds.insert(kind.to_string());
    }
    // The semantic classification rides through as a structured `kind` field (parsed, not substring).
    assert!(
        kinds.contains("type") && kinds.contains("function"),
        "the type + function classifications are emitted as structured kinds: {kinds:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn highlight_on_a_missing_file_errors_with_the_cdz_prog_name() {
    let (ok, _out, err) = run(&["highlight", "/no/such/file.sexp"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}
