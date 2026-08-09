//! End-to-end tests for the `cdz query` / `rewrite` codemod subcommands (the front-end surface,
//! now served by the unified `cdz` binary — same code as the retired `cdz-syntax`, via `cadenza_syntax::cli`).
//!
//! These drive the actual built binary over stdin/stdout — the integration counterpart to the
//! `query` module's unit tests, proving the CLI wiring (arg parsing, format resolution, the
//! validated-transaction rewrite, exit codes) end-to-end.

use std::process::{Command, Stdio};

#[path = "../common/mod.rs"]
mod common;
use common::write_stdin_tolerating_broken_pipe;

/// Run `cdz <args…>` feeding `stdin`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str], stdin: &str) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cdz");
    write_stdin_tolerating_broken_pipe(child.stdin.take().unwrap(), stdin.as_bytes());
    let out = child.wait_with_output().expect("wait");
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn query_reports_a_match_with_span_and_binding() {
    let (ok, stdout, _) = run(&["query", "(+ ,x 0)", "--from", "ml"], "f(a + 0, b * 1)");
    assert!(ok);
    assert!(stdout.contains("(+ a 0)"), "matched form: {stdout}");
    assert!(stdout.contains("$x = a"), "binding reported: {stdout}");
    assert!(stdout.contains("byte "), "span reported: {stdout}");
}

#[test]
fn query_count_prints_the_number() {
    let (ok, stdout, _) = run(
        &["query", "(+ ,e 0)", "--from", "ml", "--count"],
        "g(x + 0) + (y + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn query_no_match_is_empty_and_succeeds() {
    let (ok, stdout, _) = run(&["query", "(* ,x ,y)", "--from", "ml"], "1 + 2");
    assert!(ok);
    assert!(stdout.trim().is_empty(), "no output: {stdout:?}");
}

#[test]
fn rewrite_additive_identity_ml_to_ml() {
    let (ok, stdout, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "ml"],
        "f(a + 0, b + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "f(a, b)");
    assert!(
        stderr.contains("rewrote 2 site(s)"),
        "count on stderr: {stderr}"
    );
}

#[test]
fn rewrite_splice_template_wraps_a_call() {
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(risky ,@args)",
            "(log (risky ,@args))",
            "--from",
            "sexpr",
            "--to",
            "sexpr",
        ],
        "(risky a b)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(log (risky a b))");
}

#[test]
fn rewrite_crosses_surfaces_ml_to_sexpr() {
    let (ok, stdout, _) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "sexpr"],
        "compute(x + 0)",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(compute x)");
}

#[test]
fn query_over_broken_input_still_works_and_warns() {
    // The recovering parser yields a usable tree even here; the query runs and the CLI warns on
    // stderr about the recoverable parse error. (Exercises the parser-recovery + codemod combo.)
    let (ok, stdout, stderr) = run(&["query", "(f ,@args)", "--from", "ml"], "f(a, @, c)");
    assert!(ok, "query still succeeds over recovered input");
    assert!(stdout.contains("(f a"), "match reported: {stdout}");
    assert!(
        stderr.contains("parse warning"),
        "recovery warning on stderr: {stderr}"
    );
}

#[test]
fn bad_pattern_adjacent_splices_is_rejected() {
    // Two ADJACENT splices are still rejected (no anchor between them); non-adjacent is now allowed.
    let (ok, _, stderr) = run(&["query", "(f ,@a ,@b)", "--from", "ml"], "x");
    assert!(!ok, "exit failure");
    assert!(stderr.contains("adjacent"), "reason: {stderr}");
}

#[test]
fn rewrite_no_match_is_a_no_op_that_reprints() {
    let (ok, stdout, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--to", "ml"],
        "a - 1",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "a - 1");
    assert!(stderr.contains("rewrote 0 site(s)"), "0 sites: {stderr}");
}

// ---- matcher enrichment: guards, relational context, multi-rule, strategy ----

#[test]
fn query_guard_filters_by_structure() {
    // `(+ ,(x is-literal) ,y)` matches only the site whose first operand is a literal.
    let (ok, stdout, _) = run(
        &[
            "query",
            "(+ ,(x is-literal) ,y)",
            "--from",
            "sexpr",
            "--count",
        ],
        "(do (+ 1 a) (+ b c))",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "1");
}

#[test]
fn query_unknown_guard_is_rejected() {
    let (ok, _, stderr) = run(&["query", "(f ,(x is-bogus))", "--from", "sexpr"], "(f a)");
    assert!(!ok);
    assert!(stderr.contains("unknown guard"), "reason: {stderr}");
}

#[test]
fn query_inside_restricts_to_ancestor() {
    let (ok, stdout, _) = run(
        &["query", "x", "--from", "sexpr", "--inside", "(danger ,@_)"],
        "(do (safe x) (danger (g x)))",
    );
    assert!(ok);
    // exactly one `x` line (the one under danger).
    assert_eq!(
        stdout.lines().filter(|l| l.contains(": x")).count(),
        1,
        "{stdout}"
    );
}

#[test]
fn query_has_requires_descendant() {
    let (ok, stdout, _) = run(
        &[
            "query",
            "(fn ,@_)",
            "--from",
            "sexpr",
            "--has",
            "(raise ,_)",
        ],
        "(do (fn a (raise e)) (fn b (return c)))",
    );
    assert!(ok);
    assert!(stdout.contains("raise"), "the fn with raise: {stdout}");
    assert!(!stdout.contains("return"), "not the other fn: {stdout}");
}

#[test]
fn rewrite_with_a_rules_file_applies_a_peephole_set() {
    // Write a 3-rule peephole set to a temp file and apply it.
    let dir = std::env::temp_dir();
    let path = dir.join("cdz_cli_peephole.rules");
    std::fs::write(
        &path,
        "(rule (+ ,x 0) ,x)\n(rule (* ,x 1) ,x)\n(rule (* ,_ 0) 0)\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = run(
        &[
            "rewrite",
            "--rules",
            path.to_str().unwrap(),
            "--from",
            "sexpr",
            "--to",
            "sexpr",
        ],
        "(f (+ a 0) (* b 1) (* c 0))",
    );
    let _ = std::fs::remove_file(&path);
    assert!(ok, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "(f a b 0)");
    assert!(stderr.contains("rewrote 3 site(s)"), "count: {stderr}");
}

#[test]
fn rewrite_top_down_does_a_single_pass_unwrap() {
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(wrap ,x)",
            ",x",
            "--from",
            "sexpr",
            "--to",
            "sexpr",
            "--top-down",
        ],
        "(wrap (wrap a))",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(wrap a)");
}

#[test]
fn rewrite_requires_a_pattern_or_rules() {
    // Neither positional PATTERN/TEMPLATE nor --rules given.
    let (ok, _, stderr) = run(&["rewrite", "--from", "sexpr"], "(f a)");
    assert!(!ok);
    assert!(stderr.contains("required"), "reason: {stderr}");
}

// ---- multi-file, --write, --diff, --json ----

/// A throwaway directory unique to `tag`, cleaned by the caller. (No external tempfile dep.)
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz_cli_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn query_over_a_directory_reports_each_file() {
    let dir = scratch_dir("qdir");
    std::fs::write(dir.join("a.ml"), "f(x + 0)\n").unwrap();
    std::fs::write(dir.join("b.sexp"), "(g (+ y 0))\n").unwrap();
    let (ok, stdout, _) = run(&["query", "(+ ,e 0)", dir.to_str().unwrap()], "");
    assert!(ok);
    // per-file headers and both matches present.
    assert!(stdout.contains("a.ml ==="), "{stdout}");
    assert!(stdout.contains("b.sexp ==="), "{stdout}");
    assert!(
        stdout.contains("(+ x 0)") && stdout.contains("(+ y 0)"),
        "{stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_count_over_a_directory_totals() {
    let dir = scratch_dir("qcount");
    std::fs::write(dir.join("a.ml"), "f(x + 0)\n").unwrap();
    std::fs::write(dir.join("b.ml"), "g(y + 0, z + 0)\n").unwrap();
    let (ok, stdout, _) = run(&["query", "(+ ,e 0)", dir.to_str().unwrap(), "--count"], "");
    assert!(ok);
    assert!(stdout.contains("total: 3"), "expected 3 total: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn query_json_is_a_flat_array_with_file_and_span() {
    let dir = scratch_dir("qjson");
    std::fs::write(dir.join("a.ml"), "f(x + 0)\n").unwrap();
    let (ok, stdout, _) = run(&["query", "(+ ,e 0)", dir.to_str().unwrap(), "--json"], "");
    assert!(ok);
    let s = stdout.trim();
    assert!(s.starts_with('[') && s.ends_with(']'), "array: {s}");
    assert!(s.contains("\"file\":"), "{s}");
    assert!(s.contains("\"span\":{\"start\":"), "{s}");
    assert!(s.contains("\"matched\":\"(+ x 0)\""), "{s}");
    assert!(s.contains("\"e\":\"x\""), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_write_mutates_the_file_in_place() {
    let dir = scratch_dir("rwrite");
    let f = dir.join("p.ml");
    std::fs::write(&f, "f(a + 0, b + 0)\n").unwrap();
    let (ok, _, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", f.to_str().unwrap(), "--write"],
        "",
    );
    assert!(ok, "stderr: {stderr}");
    assert!(stderr.contains("rewrote 2 site(s)"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&f).unwrap().trim(), "f(a, b)");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_write_leaves_unmatched_file_untouched() {
    let dir = scratch_dir("rnomatch");
    let f = dir.join("p.ml");
    std::fs::write(&f, "z - 1\n").unwrap();
    let (ok, _, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", f.to_str().unwrap(), "--write"],
        "",
    );
    assert!(ok);
    assert!(stderr.contains("no change"), "{stderr}");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "z - 1\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_write_rejects_stdin() {
    let (ok, _, stderr) = run(
        &["rewrite", "(+ ,x 0)", ",x", "--from", "ml", "--write"],
        "a + 0",
    );
    assert!(!ok);
    assert!(stderr.contains("FILE"), "reason: {stderr}");
}

#[test]
fn rewrite_diff_shows_a_unified_hunk_and_does_not_write() {
    let dir = scratch_dir("rdiff");
    let f = dir.join("p.ml");
    std::fs::write(&f, "f(a + 0)\n").unwrap();
    let (ok, stdout, _) = run(
        &["rewrite", "(+ ,x 0)", ",x", f.to_str().unwrap(), "--diff"],
        "",
    );
    assert!(ok);
    assert!(stdout.contains("@@ -1,1 +1,1 @@"), "hunk: {stdout}");
    assert!(
        stdout.contains("-f(a + 0)") && stdout.contains("+f(a)"),
        "{stdout}"
    );
    // the file itself is untouched (diff is preview only).
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "f(a + 0)\n");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- ask-88: multi-splice clause deletion; ask-89: formatting-preserving edit ----

#[test]
fn rewrite_deletes_a_clause_at_an_arbitrary_position_via_two_splices() {
    // ask-88: `(F ,@before TARGET ,@after) → (F ,@before ,@after)` deletes the clause wherever it
    // sits — the natural variadic-form edit that the one-splice limit used to forbid.
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(case ,@a (needs ,_) ,@b)",
            "(case ,@a ,@b)",
            "--from",
            "sexpr",
        ],
        "(case foo (doc \"d\") (needs bar) (result 1))",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "(case foo (doc \"d\") (result 1))");
}

#[test]
fn rewrite_write_preserves_the_hand_formatted_layout() {
    // ask-89: `--write` on a multi-line `.sexp` edits ONLY the changed subtree at its span; every
    // other byte — indentation, newlines, sibling forms — is kept verbatim. (The old whole-tree
    // reprint collapsed the file onto one line.)
    let dir = scratch_dir("preserve");
    let f = dir.join("c.sexp");
    let before = "(case foo\n  (doc \"a doc\")\n  (needs bar)\n  (result 1))\n";
    std::fs::write(&f, before).unwrap();
    let (ok, _, stderr) = run(
        &[
            "rewrite",
            "(case ,@a (needs ,_) ,@b)",
            "(case ,@a ,@b)",
            f.to_str().unwrap(),
            "--write",
        ],
        "",
    );
    assert!(ok, "stderr: {stderr}");
    assert!(stderr.contains("rewrote 1 site(s)"), "{stderr}");
    // ONLY the `(needs bar)` line is gone; the rest is byte-for-byte the original layout.
    let after = std::fs::read_to_string(&f).unwrap();
    assert_eq!(
        after, "(case foo\n  (doc \"a doc\")\n  (result 1))\n",
        "layout preserved, only the clause line removed"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_preserving_diff_is_minimal() {
    // ask-89: the preserving `--diff` shows only the removed clause line, not a whole-file reflow.
    let dir = scratch_dir("prediff");
    let f = dir.join("c.sexp");
    std::fs::write(
        &f,
        "(case foo\n  (doc \"a doc\")\n  (needs bar)\n  (result 1))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(case ,@a (needs ,_) ,@b)",
            "(case ,@a ,@b)",
            f.to_str().unwrap(),
            "--diff",
        ],
        "",
    );
    assert!(ok);
    assert!(
        stdout.contains("-  (needs bar)"),
        "clause removed: {stdout}"
    );
    // The doc line and result line are CONTEXT (space-prefixed), not changed.
    assert!(
        stdout.contains(" (case foo"),
        "unchanged context kept: {stdout}"
    );
    assert!(
        !stdout.contains("-  (doc") && !stdout.contains("-  (result"),
        "only the clause line changes: {stdout}"
    );
    // The file is untouched (diff is preview only).
    assert!(std::fs::read_to_string(&f).unwrap().contains("(needs bar)"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_preserving_inserts_a_child_in_place() {
    // ask-89: growing a form's arity splices the new child after its sibling, keeping the layout of
    // the surrounding multi-line file (rather than reflowing the whole thing).
    let dir = scratch_dir("preins");
    let f = dir.join("c.sexp");
    std::fs::write(&f, "(do\n  (f a)\n  (g))\n").unwrap();
    let (ok, _, stderr) = run(
        &[
            "rewrite",
            "(f ,x)",
            "(f ,x extra)",
            f.to_str().unwrap(),
            "--write",
        ],
        "",
    );
    assert!(ok, "stderr: {stderr}");
    let after = std::fs::read_to_string(&f).unwrap();
    assert_eq!(
        after, "(do\n  (f a extra)\n  (g))\n",
        "in-place insert: {after:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_reprint_flag_forces_canonical_layout() {
    // `--reprint` opts out of preserving mode: the whole file goes through the printer (one line for
    // this s-expr), the pre-ask-89 behavior kept for deliberate normalization.
    let dir = scratch_dir("reprint");
    let f = dir.join("c.sexp");
    std::fs::write(&f, "(case foo\n  (needs bar)\n  (result 1))\n").unwrap();
    let (ok, _, _) = run(
        &[
            "rewrite",
            "(case ,@a (needs ,_) ,@b)",
            "(case ,@a ,@b)",
            f.to_str().unwrap(),
            "--write",
            "--reprint",
        ],
        "",
    );
    assert!(ok);
    let after = std::fs::read_to_string(&f).unwrap();
    // Reprinted onto a single line (no interior newlines beyond the trailing one).
    assert_eq!(after.trim(), "(case foo (result 1))");
    assert_eq!(
        after.matches('\n').count(),
        1,
        "reflowed to one line: {after:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_write_and_diff_are_mutually_exclusive() {
    let dir = scratch_dir("rexcl");
    let f = dir.join("p.ml");
    std::fs::write(&f, "f(a + 0)\n").unwrap();
    let (ok, _, stderr) = run(
        &[
            "rewrite",
            "(+ ,x 0)",
            ",x",
            f.to_str().unwrap(),
            "--write",
            "--diff",
        ],
        "",
    );
    assert!(!ok);
    assert!(stderr.contains("mutually exclusive"), "reason: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rewrite_json_reports_file_count_and_result() {
    let dir = scratch_dir("rjson");
    let f = dir.join("p.sexp");
    std::fs::write(&f, "(f (+ a 0) (+ b 0))\n").unwrap();
    let (ok, stdout, _) = run(
        &["rewrite", "(+ ,x 0)", ",x", f.to_str().unwrap(), "--json"],
        "",
    );
    assert!(ok);
    let s = stdout.trim();
    assert!(s.starts_with('[') && s.ends_with(']'), "array: {s}");
    assert!(s.contains("\"count\":2"), "{s}");
    // Default formatting-preserving edit: only the two `(+ … 0)` sites change, and the source's
    // trailing newline is kept verbatim (fidelity), so `rewritten` is the faithful edited file.
    assert!(s.contains("\"rewritten\":\"(f a b)\\n\""), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- structural tree-diff (the `diff` subcommand) ----

#[test]
fn diff_reports_a_changed_subtree_at_its_path() {
    let dir = scratch_dir("diffa");
    std::fs::write(dir.join("a.ml"), "f(a + 0, b + 0)\n").unwrap();
    std::fs::write(dir.join("b.ml"), "f(a, b + 0)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "diff",
            dir.join("a.ml").to_str().unwrap(),
            dir.join("b.ml").to_str().unwrap(),
        ],
        "",
    );
    assert!(ok);
    // one change: child 1 replaced; the unchanged second operand is not reported.
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.contains("1: replace (+ a 0) => a"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diff_json_is_wellformed() {
    let dir = scratch_dir("diffj");
    std::fs::write(dir.join("a.sexp"), "(+ a b)\n").unwrap();
    std::fs::write(dir.join("b.sexp"), "(+ a c)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "diff",
            dir.join("a.sexp").to_str().unwrap(),
            dir.join("b.sexp").to_str().unwrap(),
            "--json",
        ],
        "",
    );
    assert!(ok);
    let s = stdout.trim();
    assert!(s.contains("\"path\":[2]"), "{s}");
    assert!(s.contains("\"kind\":\"replace\""), "{s}");
    assert!(
        s.contains("\"old\":\"b\"") && s.contains("\"new\":\"c\""),
        "{s}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn diff_identical_reports_no_change() {
    let dir = scratch_dir("diffid");
    std::fs::write(dir.join("a.ml"), "f(x)\n").unwrap();
    let (ok, stdout, stderr) = run(
        &[
            "diff",
            dir.join("a.ml").to_str().unwrap(),
            dir.join("a.ml").to_str().unwrap(),
        ],
        "",
    );
    assert!(ok);
    assert!(stdout.trim().is_empty(), "no stdout: {stdout:?}");
    assert!(stderr.contains("no structural changes"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- lint mode (the `lint` subcommand) ----

#[test]
fn lint_flags_a_matching_pattern_with_location_and_severity() {
    let dir = scratch_dir("lint1");
    std::fs::write(dir.join("code.ml"), "g(x)\nf(deprecated())\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "lint",
            dir.join("code.ml").to_str().unwrap(),
            "--rule",
            "(lint (deprecated ,@_) \"do not use\" error)",
        ],
        "",
    );
    assert!(!ok, "an error-severity diagnostic exits non-zero");
    assert!(
        stdout.contains("code.ml:2:"),
        "location on line 2: {stdout}"
    );
    assert!(stdout.contains("error: do not use"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_warning_only_exits_zero() {
    let dir = scratch_dir("lint2");
    std::fs::write(dir.join("code.sexp"), "(deprecated x)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "lint",
            dir.join("code.sexp").to_str().unwrap(),
            "--rule",
            "(lint (deprecated ,@_) \"avoid\" warning)",
        ],
        "",
    );
    assert!(ok, "a warning does not fail the run");
    assert!(stdout.contains("warning: avoid"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_clean_file_exits_zero_with_no_output() {
    let dir = scratch_dir("lint3");
    std::fs::write(dir.join("code.sexp"), "(fine x)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "lint",
            dir.join("code.sexp").to_str().unwrap(),
            "--rule",
            "(lint (deprecated ,@_) \"avoid\" error)",
        ],
        "",
    );
    assert!(ok);
    assert!(stdout.trim().is_empty(), "no diagnostics: {stdout:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_json_is_wellformed() {
    let dir = scratch_dir("lint4");
    std::fs::write(dir.join("code.ml"), "f(deprecated())\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "lint",
            dir.join("code.ml").to_str().unwrap(),
            "--rule",
            "(lint (deprecated ,@_) \"do not use\" error)",
            "--json",
        ],
        "",
    );
    assert!(!ok);
    let s = stdout.trim();
    assert!(s.starts_with('[') && s.ends_with(']'), "array: {s}");
    assert!(s.contains("\"severity\":\"error\""), "{s}");
    assert!(s.contains("\"message\":\"do not use\""), "{s}");
    assert!(s.contains("\"line\":1"), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_from_a_rules_file_over_a_directory() {
    let dir = scratch_dir("lint5");
    std::fs::write(dir.join("a.ml"), "f(deprecated())\n").unwrap();
    std::fs::write(dir.join("b.ml"), "g(ok())\n").unwrap();
    let rules = dir.join("r.lint");
    std::fs::write(&rules, "(lint (deprecated ,@_) \"no\" error)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "lint",
            dir.to_str().unwrap(),
            "--rules",
            rules.to_str().unwrap(),
        ],
        "",
    );
    assert!(!ok, "error found in a.ml");
    assert!(stdout.contains("a.ml:"), "flags a.ml: {stdout}");
    assert!(!stdout.contains("b.ml:"), "b.ml is clean: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_with_no_rules_runs_the_builtin_catalog() {
    // `cdz lint` with no explicit `--rule`/`--rules` runs the built-in `idiomatic` catalog (Tier-A
    // pack) — useful out of the box, not a usage error (DESIGN-cadenza-lint §2). `(f a)` has no
    // idiomatic issue, so it exits 0 cleanly.
    let (ok, _out, _stderr) = run(&["lint", "--from", "sexpr"], "(f a)");
    assert!(
        ok,
        "no explicit rules runs the built-in catalog and a clean program passes"
    );
    // A program matching a built-in lint (`if b true false` → idiomatic/if-bool) is flagged.
    let (ok2, out2, _e2) = run(&["lint", "--from", "sexpr"], "(if b true false)");
    assert!(ok2, "a warning-level built-in lint does not fail the run");
    assert!(
        out2.contains("idiomatic") || out2.contains("if"),
        "the built-in if-bool lint fires by default: {out2}"
    );
}

// ---- clone detection (the `clones` subcommand) ----

#[test]
fn clones_finds_a_duplicated_subtree_across_files() {
    let dir = scratch_dir("clone1");
    std::fs::write(dir.join("a.ml"), "f(validate(config, strict))\n").unwrap();
    std::fs::write(dir.join("b.ml"), "g(validate(config, strict))\n").unwrap();
    let (ok, stdout, _) = run(&["clones", dir.to_str().unwrap(), "--min-size", "3"], "");
    assert!(ok);
    assert!(stdout.contains("(validate config strict)"), "{stdout}");
    assert!(stdout.contains("2 occurrences"), "{stdout}");
    assert!(
        stdout.contains("a.ml:") && stdout.contains("b.ml:"),
        "cross-file: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clones_min_size_filters_trivial() {
    let dir = scratch_dir("clone2");
    // `x` recurs but is tiny; a big min-size finds nothing.
    std::fs::write(dir.join("a.sexp"), "(f x x x)\n").unwrap();
    let (ok, stderr) = {
        let (ok, _out, err) = run(
            &[
                "clones",
                dir.join("a.sexp").to_str().unwrap(),
                "--min-size",
                "5",
            ],
            "",
        );
        (ok, err)
    };
    assert!(ok);
    assert!(stderr.contains("no clones"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clones_json_is_wellformed() {
    let dir = scratch_dir("clone3");
    std::fs::write(dir.join("a.sexp"), "(do (k a b) (k a b))\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "clones",
            dir.join("a.sexp").to_str().unwrap(),
            "--min-size",
            "3",
            "--json",
        ],
        "",
    );
    assert!(ok);
    let s = stdout.trim();
    assert!(s.starts_with('[') && s.ends_with(']'), "array: {s}");
    assert!(s.contains("\"exemplar\":\"(k a b)\""), "{s}");
    assert!(s.contains("\"size\":4"), "{s}");
    assert!(s.contains("\"sites\":["), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clones_reports_nothing_when_all_distinct() {
    let dir = scratch_dir("clone4");
    std::fs::write(dir.join("a.sexp"), "(do (f a) (g b) (h c))\n").unwrap();
    let (ok, _out, stderr) = run(
        &[
            "clones",
            dir.join("a.sexp").to_str().unwrap(),
            "--min-size",
            "2",
        ],
        "",
    );
    assert!(ok);
    assert!(stderr.contains("no clones"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- near-clone detection (`clones --near`) ----

#[test]
fn near_clones_infers_a_pattern() {
    let dir = scratch_dir("near1");
    std::fs::write(dir.join("a.ml"), "f(scale(x, 2))\ng(scale(x, 3))\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "clones",
            dir.join("a.ml").to_str().unwrap(),
            "--near",
            "--min-size",
            "3",
        ],
        "",
    );
    assert!(ok);
    assert!(
        stdout.contains("(scale x ,m0)"),
        "inferred pattern: {stdout}"
    );
    assert!(stdout.contains("2 occurrences"), "{stdout}");
    assert!(stdout.contains("1 hole"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn near_clones_json_reports_pattern_and_holes() {
    let dir = scratch_dir("near2");
    std::fs::write(dir.join("a.sexp"), "(do (k a 1) (k b 2))\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "clones",
            dir.join("a.sexp").to_str().unwrap(),
            "--near",
            "--min-size",
            "3",
            "--json",
        ],
        "",
    );
    assert!(ok);
    let s = stdout.trim();
    assert!(s.contains("\"pattern\":\"(k ,m0 ,m1)\""), "{s}");
    assert!(s.contains("\"holes\":2"), "{s}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn near_clones_none_when_shapes_differ() {
    let dir = scratch_dir("near3");
    std::fs::write(dir.join("a.sexp"), "(do (f a b) (g c))\n").unwrap();
    let (ok, _out, stderr) = run(
        &[
            "clones",
            dir.join("a.sexp").to_str().unwrap(),
            "--near",
            "--min-size",
            "3",
        ],
        "",
    );
    assert!(ok);
    assert!(stderr.contains("no near-clones"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn near_clone_pattern_feeds_back_into_rewrite() {
    // The closing loop: the inferred pattern re-matches (and rewrites) the very sites it came from.
    let (ok, stdout, _) = run(
        &[
            "rewrite",
            "(scale x ,m0)",
            "(scaled x ,m0)",
            "--from",
            "ml",
            "--to",
            "ml",
        ],
        "g(scale(x, 2), scale(x, 7))",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "g(scaled(x, 2), scaled(x, 7))");
}

// ---- multi-file UX: extension filtering, resilience, empty feedback ----

#[test]
fn directory_walk_skips_non_source_files_even_with_from() {
    // The original trap: `--from sexpr` over a dir must NOT try to parse a README.
    let dir = scratch_dir("uxfilter");
    std::fs::write(dir.join("a.sexp"), "(f a)\n").unwrap();
    std::fs::write(dir.join("README.md"), "# not source )(][\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "target\n").unwrap();
    let (ok, stdout, stderr) = run(
        &[
            "query",
            "(f ,@_)",
            dir.to_str().unwrap(),
            "--from",
            "sexpr",
            "--count",
        ],
        "",
    );
    assert!(ok, "no crash on the README: {stderr}");
    assert_eq!(stdout.trim(), "1", "only the .sexp counted: {stdout}");
    assert!(
        !stderr.contains("README"),
        "README silently skipped: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn directory_with_no_source_files_warns() {
    let dir = scratch_dir("uxempty");
    std::fs::write(dir.join("notes.txt"), "hi\n").unwrap();
    let (ok, _stdout, stderr) = run(&["query", "(f ,@_)", dir.to_str().unwrap(), "--count"], "");
    assert!(ok);
    assert!(
        stderr.contains("no source files"),
        "warns on empty dir: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_broken_file_in_a_dir_is_skipped_not_fatal() {
    let dir = scratch_dir("uxresil");
    std::fs::write(dir.join("good.sexp"), "(f a)\n").unwrap();
    std::fs::write(dir.join("broken.sexp"), "(f a\n").unwrap(); // unterminated
    std::fs::write(dir.join("good2.sexp"), "(f b)\n").unwrap();
    let (ok, stdout, stderr) = run(&["query", "(f ,@_)", dir.to_str().unwrap(), "--count"], "");
    assert!(ok, "the sweep survives one broken file");
    assert!(
        stderr.contains("skipping") && stderr.contains("broken.sexp"),
        "{stderr}"
    );
    assert!(
        stdout.contains("total: 2"),
        "both good files counted: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_single_broken_file_is_still_a_hard_error() {
    let dir = scratch_dir("uxhard");
    std::fs::write(dir.join("broken.sexp"), "(f a\n").unwrap();
    let (ok, _stdout, stderr) = run(
        &[
            "query",
            "(f ,@_)",
            dir.join("broken.sexp").to_str().unwrap(),
            "--count",
        ],
        "",
    );
    assert!(!ok, "single broken target fails");
    assert!(
        !stderr.contains("skipping"),
        "not a skip — a hard error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn named_file_still_honors_from() {
    // An explicitly-named file with a non-matching extension is still read via --from.
    let dir = scratch_dir("uxnamed");
    let f = dir.join("prog.txt"); // .txt has no inferred format
    std::fs::write(&f, "(f a b)\n").unwrap();
    let (ok, stdout, _) = run(
        &[
            "query",
            "(f ,@_)",
            f.to_str().unwrap(),
            "--from",
            "sexpr",
            "--count",
        ],
        "",
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "1", "named .txt read as sexpr: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- combined structural + semantic query: `cdz query … --where 'type-of(x) = T'` ----
// The compiler-backed filter, unique to the unified `cdz` binary (it holds both libraries). A
// structural `(foo ,x)` match survives only if the binding `x`'s inferred type relates to the type.

#[test]
fn where_keeps_only_matches_whose_binding_has_the_asked_type() {
    let dir = scratch_dir("where_eq");
    let f = dir.join("prog.sexp");
    // `foo` is applied to an Int64 arg and a Bool arg; --where Int64 keeps only the Int64 call.
    std::fs::write(
        &f,
        "(module m (def (foo x) x) \
           (def (main) (if (foo true) (foo (: 42 Int64)) 0)) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(
        &[
            "query",
            "(foo ,x)",
            f.to_str().unwrap(),
            "--where",
            "type-of(x) = Int64",
            "--count",
        ],
        "",
    );
    assert!(ok);
    // Two structural (foo ,x) sites bind an Int64: the `(foo (: 42 Int64))` call — plus the def-site
    // `(foo x)` whose param `x` is NOT Int64 (it's an unconstrained param), and `(foo true)` is Bool.
    // So exactly ONE match has an Int64 binding.
    assert_eq!(
        stdout.trim(),
        "1",
        "only the Int64 call survives --where: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_neq_and_bool_variants_filter_as_expected() {
    let dir = scratch_dir("where_variants");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (foo x) x) \
           (def (main) (if (foo true) (foo (: 42 Int64)) 0)) (export main))\n",
    )
    .unwrap();
    let path = f.to_str().unwrap();

    // = Bool keeps the single Bool call.
    let (ok, out_bool, _) = run(
        &[
            "query",
            "(foo ,x)",
            path,
            "--where",
            "type-of(x) = Bool",
            "--count",
        ],
        "",
    );
    assert!(ok);
    assert_eq!(out_bool.trim(), "1", "one Bool call: {out_bool}");

    // The rendered (non-count) output carries a file:line:col location.
    let (ok, out_loc, _) = run(
        &["query", "(foo ,x)", path, "--where", "type-of(x) = Bool"],
        "",
    );
    assert!(ok);
    assert!(
        out_loc.contains("prog.sexp:"),
        "location is file:line:col: {out_loc}"
    );
    assert!(
        out_loc.contains("(foo true)"),
        "the Bool match is shown: {out_loc}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn where_malformed_predicate_is_an_error() {
    let dir = scratch_dir("where_bad");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) 42) (export main))\n").unwrap();
    let (ok, _, stderr) = run(
        &[
            "query",
            "(foo ,x)",
            f.to_str().unwrap(),
            "--where",
            "x is Int64",
        ],
        "",
    );
    assert!(!ok, "a malformed --where predicate fails");
    assert!(
        stderr.contains("unsupported --where predicate"),
        "clear error: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz check FILE` — diagnostics as you type (Query::Diagnostics) ----
// Reports every well-formedness fault without requiring an export/run; exits non-zero on any error.

#[test]
fn check_reports_a_fault_without_an_export_and_exits_nonzero() {
    let dir = scratch_dir("check_bad");
    let f = dir.join("prog.sexp");
    // Ill-typed (`if 5 …` — non-Bool condition) and, crucially, NO `(export …)`.
    std::fs::write(&f, "(module m (def (main) (if 5 1 2)))\n").unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(!ok, "an error-severity fault exits non-zero");
    assert!(
        stdout.contains("CDZ0203"),
        "reports the type fault: {stdout}"
    );
    assert!(
        stdout.contains("prog.sexp:"),
        "with a file:line:col location: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_prints_a_did_you_mean_help_line_for_a_misspelled_name() {
    // The rustc-gold-standard suggestion, surfaced end-to-end: an unbound name that is a near-miss for
    // an in-scope name (`compute` → `computee`) reports the fault AND a `help:` line an agent applies
    // directly — the structural "route to a fix" (`spec/capabilities/diagnostics.md`).
    let dir = scratch_dir("check_dym");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(!ok, "the unbound name is an error (exit non-zero)");
    assert!(
        stdout.contains("CDZ0101") && stdout.contains("did you mean `compute`?"),
        "the fault names the candidate: {stdout}"
    );
    assert!(
        stdout.contains("help (heuristic): replace with `compute`"),
        "a heuristic help line carries the structural fix: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_json_emits_a_machine_readable_diagnostic_with_a_structural_patch() {
    // `--json` gives an agent the fix as DATA — code, message, and a nested `fix` object carrying a
    // STRUCTURAL PATCH (`edits: [{from,to,text}]`, computed by the structural rewriter) it applies
    // literally (`source[from..to] := text`), not the human `help:` text. A did-you-mean is one edit
    // replacing the faulting call `(computee 1)` with `(compute 1)`.
    let dir = scratch_dir("check_json");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n";
    std::fs::write(&f, original).unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the unbound name is still an error (exit non-zero)");
    let line = stdout.lines().next().unwrap_or("");
    assert!(
        line.starts_with('{') && line.ends_with('}'),
        "a JSON object: {stdout}"
    );
    assert!(line.contains("\"code\":\"CDZ0101\""), "the code: {stdout}");
    assert!(
        line.contains("\"fix\":{") && line.contains("\"kind\":\"replace\""),
        "a structured fix with a kind: {stdout}"
    );
    assert!(
        line.contains("\"verified\":false"),
        "the applicability marker as a JSON bool: {stdout}"
    );
    assert!(
        line.contains("\"edits\":[{") && line.contains("\"text\":"),
        "an edits array with byte-range replacements: {stdout}"
    );
    // Applying the patch (each `source[from..to] := text`) fixes the program: `computee` → `compute`.
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("(compute 1)") && !patched.contains("computee"),
        "applying the structural patch repairs the source: {patched}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Apply a diagnostic JSON line's `fix.edits` to `src` (each `src[from..to] := text`, descending so
/// offsets stay valid) — the exact thing an agent does with the machine channel. A minimal JSON reader
/// (the edits are flat `{from,to,text}` objects); good enough for the test's known shape.
fn apply_json_edits(src: &str, json_line: &str) -> String {
    // Extract the `"edits":[ ... ]` array substring, then each `{from,to,text}` triple.
    let arr = match json_line.split_once("\"edits\":[") {
        Some((_, rest)) => rest.split_once("]}").map(|(a, _)| a).unwrap_or(rest),
        None => return src.to_string(),
    };
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for obj in arr.split("},{") {
        let num = |key: &str| -> usize {
            let pat = format!("\"{key}\":");
            let start = obj.find(&pat).map(|i| i + pat.len()).unwrap_or(0);
            obj[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        };
        // `"text":"…"` — the value between the quotes after the key (no escapes in these fixtures).
        let text = obj
            .split_once("\"text\":\"")
            .and_then(|(_, r)| r.split_once('"'))
            .map(|(t, _)| t.to_string())
            .unwrap_or_default();
        edits.push((num("from"), num("to"), text));
    }
    edits.sort_by_key(|(from, _, _)| std::cmp::Reverse(*from));
    let mut out = src.to_string();
    for (from, to, text) in edits {
        out.replace_range(from..to, &text);
    }
    out
}

#[test]
fn check_json_wrap_patch_is_two_inserts_that_preserve_the_wrapped_bytes() {
    // A `wrap` fix's machine-channel patch must NOT leak the internal `…` (U+2026) HOLE sentinel, and must
    // PRESERVE the wrapped node's bytes. The structural rewriter emits a wrap as TWO insert edits (an empty
    // range each) around the node — `(host (E) ` before, `)` after — so applying them wraps the untouched
    // body verbatim. No sentinel, no whole-node reprint. (CDZ0401 is a wrap: `(host (E) <body>)`.)
    let dir = scratch_dir("check_json_wrap");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(do (effect E (op get (-> Unit Int64))) (def (main) (+ 1 (E.get unit))) (export main))\n",
    )
    .unwrap();
    let (_, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    let line = stdout.lines().find(|l| l.contains("CDZ0401")).unwrap_or("");
    assert!(line.contains("\"kind\":\"wrap\""), "a wrap fix: {line}");
    assert!(
        !line.contains('…') && !line.contains("\"replacement\""),
        "no HOLE sentinel, no ambiguous single `replacement`: {line}"
    );
    assert!(
        line.contains("\"text\":\"(host (E) \"") && line.contains("\"text\":\")\""),
        "two insert edits — the wrapper prefix and suffix: {line}"
    );
    // Applying the patch preserves the inner `E.get` bytes (a whole-node reprint would canonicalize it).
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("(host (E) (+ 1 (E.get unit)))"),
        "the wrap applies cleanly, wrapped bytes preserved: {patched}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_json_on_a_clean_program_emits_nothing_and_exits_zero() {
    let dir = scratch_dir("check_json_clean");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) (: 42 Int64)) (export main))\n").unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(ok, "a clean program exits 0");
    assert_eq!(stdout.trim(), "", "and emits no JSON: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_verify_fixes_upgrades_a_confirmed_heuristic_fix_to_verified() {
    // `--verify-fixes` applies each proposed fix + re-checks: a did-you-mean whose candidate actually
    // clears the fault is UPGRADED heuristic → verified (`spec/capabilities/diagnostics.md` §A Confirmed
    // Fix Is Marked Verified). Without the flag it prints `help (heuristic):`; with it, plain `help:`.
    let dir = scratch_dir("check_verify");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n",
    )
    .unwrap();
    // Baseline: heuristic.
    let (_, base, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        base.contains("help (heuristic): replace with `compute`"),
        "heuristic by default: {base}"
    );
    // Verified: the `computee`→`compute` edit recompiles clean, so the marker drops.
    let (ok, stdout, _) = run(&["check", "--verify-fixes", f.to_str().unwrap()], "");
    assert!(!ok, "the fault is still an error until actually applied");
    assert!(
        stdout.contains("help: replace with `compute`"),
        "a confirmed fix loses the heuristic marker: {stdout}"
    );
    assert!(
        !stdout.contains("help (heuristic)"),
        "no heuristic marker on a verified fix: {stdout}"
    );
    // And the JSON flag flips to verified:true.
    let (_, js, _) = run(
        &["check", "--verify-fixes", "--json", f.to_str().unwrap()],
        "",
    );
    assert!(
        js.contains("\"verified\":true"),
        "JSON reports the upgrade: {js}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_applies_a_verified_did_you_mean_and_the_result_compiles_clean() {
    // `cdz fix --all` turns "here's the fix" into "fixed it": the `computee`→`compute` edit verifies
    // (recompiles clean), so it is applied to the file — and re-checking the repaired file is clean.
    let dir = scratch_dir("fix_apply");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    assert!(
        stderr.contains("applied 1 fix"),
        "reports the count: {stderr}"
    );
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(compute 1)") && !repaired.contains("computee"),
        "the file was repaired: {repaired}"
    );
    // The repaired file re-checks clean (the CDZ0101 is gone) — exit 0.
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_misspelled_export_carries_an_applicable_replace_fix_end_to_end() {
    // The export-position did-you-mean, surfaced end-to-end: `(export computee)` for a defined `compute`
    // reports CDZ0101 AND now carries a structural replace fix (previously it named the candidate in text
    // but carried no applicable patch). `--json` emits the `edits` array; `fix --all` applies it and the
    // repaired file recompiles clean. This is the export analogue of the unbound-name did-you-mean fix.
    let dir = scratch_dir("export_dym");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (compute) 1) (export computee))\n").unwrap();
    // The JSON channel carries the fix as an edits array an agent applies literally.
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the misspelled export is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0101\""))
        .expect("the export fault is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"replace\"") && line.contains("\"edits\":[{"),
        "CDZ0101 carries a structural replace patch, not just a message: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("(export compute)") && !patched.contains("computee"),
        "applying the patch renames the export to the real definition: {patched}"
    );
    // And `fix --all` applies it in place — the repaired file recompiles clean.
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(export compute)") && !repaired.contains("computee"),
        "the file was repaired: {repaired}"
    );
    let (ok3, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok3, "the repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_applies_a_mistyped_top_level_keyword_swap_end_to_end() {
    // A top-level declaration-keyword typo (`(exprot f)` for `export`) reports "unbound name … did you
    // mean `export`?" AND carries a replace fix on the head — a code-less DECLINE that nonetheless carries
    // a targeted, verifiable fix. `cdz fix --all` now applies it (the candidacy gate is on the fix's target
    // node, not the code, and `--verify-fixes` still proves the edit clears the fault), so the file is
    // repaired to `(export f)` and re-checks clean.
    let dir = scratch_dir("kw_typo");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (f) 1) (exprot f))\n").unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    assert!(
        stderr.contains("applied 1 fix"),
        "reports the count: {stderr}"
    );
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(export f)") && !repaired.contains("exprot"),
        "the keyword was swapped: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "the repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_redundant_match_arm_carries_a_delete_fix_that_fix_all_applies() {
    // A WARNING that carries a machine-applicable fix, surfaced end-to-end: an unreachable match arm
    // (CDZ0213) now carries a `delete` fix. `--json` emits the deletion as an `edits` array; `fix --all`
    // applies it (the FIRST warning-severity fix `--all` verifies — the verify machinery clears the
    // warning, not just an error) and the repaired file rechecks clean. rustc offers exactly this for an
    // unreachable pattern.
    let dir = scratch_dir("redundant_arm");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (f (: n Int64)) (match n (0 10) (0 20) (_ 30))) (export f))\n";
    std::fs::write(&f, original).unwrap();
    // The JSON channel carries the delete as an edits array (removing the shadowed `(0 20)` arm).
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(ok, "a redundant arm is a WARNING — check still exits 0");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0213\""))
        .expect("the redundant-arm warning is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"delete\"") && line.contains("\"edits\":[{"),
        "CDZ0213 carries a structural delete patch: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("(0 10)") && patched.contains("(_ 30)") && !patched.contains("(0 20)"),
        "applying the patch removes the shadowed arm, keeps the rest: {patched}"
    );
    // `fix --all` applies the warning fix in place (the verify machinery now clears warnings, not only
    // errors) — the repaired file rechecks clean.
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    assert!(
        stderr.contains("applied 1 fix"),
        "reports the applied fix: {stderr}"
    );
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(0 10)") && repaired.contains("(_ 30)") && !repaired.contains("(0 20)"),
        "the file was repaired: {repaired}"
    );
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file is clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_duplicate_export_carries_a_delete_fix_that_fix_all_applies() {
    // A duplicate-name fault (CDZ0201), surfaced end-to-end: `(export a) (export a)` reports the fault AND
    // carries a `delete` fix on the redundant clause. `--json` emits the deletion; `fix --all` removes it
    // and the repaired file recompiles clean.
    let dir = scratch_dir("dup_export");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (a) 1) (export a) (export a))\n").unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the duplicate export is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0201\""))
        .expect("the duplicate is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"delete\"") && line.contains("\"edits\":[{"),
        "CDZ0201 carries a structural delete patch: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert_eq!(
        patched.matches("(export a)").count(),
        1,
        "applying the patch leaves exactly one export: {patched}"
    );
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file is clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_mistyped_sum_argument_carries_a_wrap_fix_that_fix_all_applies() {
    // The rustc-flagship "try wrapping in `Some`" repair, surfaced end-to-end at a CALL SITE: passing `5`
    // to a `(: o (Option Int64))` parameter reports CDZ0203 AND carries a `wrap` fix → `(Some 5)`. `--json`
    // emits the wrap as two inserts (before/after the arg); `fix --all` applies it and the file recompiles
    // clean. General over any sum (reads the expected sum's own variants), forced-choice only.
    let dir = scratch_dir("wrap_arg");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (f (: o (Option Int64))) o) (def (main) (f 5)) (export main))\n";
    std::fs::write(&f, original).unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the mistyped argument is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0203\""))
        .expect("the type mismatch is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"wrap\"") && line.contains("\"edits\":[{"),
        "CDZ0203 carries a structural wrap patch: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("(f (Some 5))"),
        "applying the patch wraps the argument in `Some`: {patched}"
    );
    // `fix --all` applies it in place and the repaired file recompiles clean.
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(f (Some 5))"),
        "the file was repaired: {repaired}"
    );
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file is clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_int_annotation_mismatch_carries_an_of_conversion_fix_that_fix_all_applies() {
    // The annotation-position numeric coercion, end-to-end: `(: n Int64)` for an `Int8` value reports
    // CDZ0203 AND carries a `wrap` fix → `(Int64.of n)`. `fix --all` applies it and the file recompiles.
    let dir = scratch_dir("annot_of");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (f (: n Int8)) (: n Int64)) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the annotation mismatch is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0203\""))
        .expect("the mismatch is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"wrap\"") && line.contains("Int64") && line.contains("of"),
        "CDZ0203 carries an Int64.of wrap patch: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("of") && patched.contains("Int64"),
        "applying the patch wraps the value in the conversion: {patched}"
    );
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file is clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_coercion_wrap_over_a_compound_value_produces_a_clean_two_insert_edit() {
    // REGRESSION: a coercion wrap whose ctor is a DOTTED member-access (`(Int64.of …)` → head `(. Int64
    // of)`) over a COMPOUND value (`(+ n 1)`, a list) used to fragment — the `same_head` diff treated the
    // named-head `(+ …)` and the list-head `((. Int64 of) …)` as alignable and LCS-aligned them into
    // leading-space inserts + empty deletes, yielding `( (. Int64 of) (+ n 1))` (a STRAY SPACE after `(`).
    // Now `same_head` returns false for a name-vs-list head, so the wrap-preserve path fires: two clean
    // inserts around the preserved compound. The result must have NO stray `( ` sequence.
    let dir = scratch_dir("wrap_compound");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (f (: n Int8)) (: (+ n 1) Int64)) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the mismatch is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0203\""))
        .expect("the mismatch is emitted as JSON");
    // The wrap is exactly TWO inserts (prefix + suffix), not a fragmented multi-edit LCS align.
    let n_edits = line.matches("\"text\":").count();
    assert_eq!(
        n_edits, 2,
        "a clean wrap is two inserts, got {n_edits}: {line}"
    );
    let (_, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(stderr.contains("applied"), "fix applies: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("((. Int64 of) (+ n 1))") && !repaired.contains("( (. Int64 of)"),
        "the wrap is clean (no stray space after `(`): {repaired}"
    );
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file recompiles clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_string_where_bytes_expected_carries_a_to_bytes_fix_that_fix_all_applies() {
    // A total-conversion coercion, surfaced end-to-end: `(f "hi")` for a `(: b Bytes)` parameter reports
    // CDZ0203 AND carries a `wrap` fix → `(String.to-bytes "hi")`. `--json` emits the wrap as two inserts;
    // `fix --all` applies it and the repaired file recompiles clean.
    let dir = scratch_dir("to_bytes");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (f (: b Bytes)) b) (def (main) (f \"hi\")) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the String/Bytes mismatch is an error");
    let line = stdout
        .lines()
        .find(|l| l.contains("\"code\":\"CDZ0203\""))
        .expect("the mismatch is emitted as JSON");
    assert!(
        line.contains("\"kind\":\"wrap\"") && line.contains("String") && line.contains("to-bytes"),
        "CDZ0203 carries a String.to-bytes wrap patch: {line}"
    );
    let patched = apply_json_edits(&std::fs::read_to_string(&f).unwrap(), line);
    assert!(
        patched.contains("to-bytes") && patched.contains("\"hi\""),
        "applying the patch wraps the string in the encode: {patched}"
    );
    let (ok2, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok2, "fix succeeds: {stderr}");
    let (ok3, out3, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok3 && out3.trim().is_empty(),
        "the repaired file is clean: {out3}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_json_reports_each_applied_fix() {
    // `cdz fix --json` tells an agent WHICH faults were repaired — a JSON array of `{code, kind, message}`
    // — not just the human "applied N" count. Two independent faults (a did-you-mean + an out-of-range
    // widen) → two report objects, and the file is still written.
    let dir = scratch_dir("fix_json");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (compute x) x) (def (main) (+ (computee 1) (: 999 Int8))) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["fix", "--all", "--json", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds");
    assert!(
        stdout.trim_start().starts_with('[') && stdout.trim_end().ends_with(']'),
        "a JSON array report: {stdout}"
    );
    assert!(
        stdout.contains("\"code\":\"CDZ0101\"") && stdout.contains("\"code\":\"CDZ0302\""),
        "both repaired faults are reported by code: {stdout}"
    );
    assert!(
        stdout.contains("\"kind\":\"replace\""),
        "each fix names its kind: {stdout}"
    );
    // The file was actually repaired (JSON report does not imply dry-run).
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(compute 1)") && repaired.contains("Int16"),
        "the file was written: {repaired}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_json_dry_run_reports_without_writing_and_empty_when_nothing_applies() {
    // `--json --dry-run` emits the report but leaves the file untouched; a clean-of-fixes file reports an
    // empty array `[]` (the honest "nothing applied" for a machine consumer).
    let dir = scratch_dir("fix_json_dry");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n";
    std::fs::write(&f, original).unwrap();
    let (ok, stdout, _) = run(
        &["fix", "--all", "--json", "--dry-run", f.to_str().unwrap()],
        "",
    );
    assert!(ok);
    assert!(
        stdout.contains("\"code\":\"CDZ0101\""),
        "reports the fix: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "--dry-run leaves the file untouched"
    );
    // A file with no applicable fix → empty array.
    let g = dir.join("clean.sexp");
    std::fs::write(&g, "(module m (def (main) 0) (export main))\n").unwrap();
    let (ok2, stdout2, _) = run(&["fix", "--all", "--json", g.to_str().unwrap()], "");
    assert!(ok2);
    assert_eq!(
        stdout2.trim(),
        "[]",
        "nothing applied → empty report: {stdout2}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_dry_run_previews_without_writing() {
    // `--dry-run` prints the repaired program but leaves the file untouched.
    let dir = scratch_dir("fix_dry");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n";
    std::fs::write(&f, original).unwrap();
    let (ok, stdout, _) = run(&["fix", "--all", "--dry-run", f.to_str().unwrap()], "");
    assert!(ok);
    assert!(
        stdout.contains("(compute 1)"),
        "previews the repair: {stdout}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "the file is NOT modified by --dry-run"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_drops_a_fractional_form_from_an_integer_valued_float_in_an_int_context() {
    // The MIRROR of the `of-int` coercion fix: an INTEGER position is given a FLOAT literal (`(+ 2
    // 2.0)`), CDZ0301. There is no float→int prelude op to wrap with, so the repair for an
    // integer-VALUED literal is to drop the fractional form (`2.0` → `2`). `--all` verifies it
    // (recompiles clean) and applies it; the repaired file re-checks clean.
    let dir = scratch_dir("fix_floatlit");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) (+ 2 2.0)) (export main))\n").unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(+ 2 2)") && !repaired.contains("2.0"),
        "the float literal became the integer: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_retypes_the_int_operand_of_an_int_float_mix() {
    // An int/float mix (`(+ 2 2.5)`) IS cleanly repairable — NOT by touching the fractional `2.5`
    // (rounding/truncating would be a semantic choice the compiler must not make), but by widening the
    // INT operand `2` -> `2.0`, giving the all-Float64 `(+ 2.0 2.5)`. The retype fix conforms an operand
    // to the other's type regardless of order (`rcdzc@484876e6`), so it targets the literal-int side; the
    // repaired program re-checks clean. (The earlier honesty-boundary test asserted NO fix here, from
    // when the coercion only ever targeted the second operand — the fractional `2.5` — and thus declined.)
    let dir = scratch_dir("frac_retype");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) (+ 2 2.5)) (export main))\n").unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(!ok, "still an error");
    assert!(
        stdout.contains("CDZ0301"),
        "the mismatch is reported: {stdout}"
    );
    assert!(
        stdout.contains("replace with `2.0`"),
        "the fix widens the INT operand to float, not the fractional literal: {stdout}"
    );
    // `fix --all` applies it — the int became `2.0`, the `2.5` is untouched, and it re-checks clean.
    let (fixed_ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(fixed_ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(+ 2.0 2.5)"),
        "the int operand widened to float: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_without_all_leaves_a_heuristic_only_file_untouched() {
    // Default `fix` applies only COMPILER-verified (rule) fixes; a file whose only fix is heuristic
    // (a did-you-mean) is left untouched and reports no applicable fixes.
    let dir = scratch_dir("fix_noall");
    let f = dir.join("prog.sexp");
    let original = "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n";
    std::fs::write(&f, original).unwrap();
    let (ok, _, stderr) = run(&["fix", f.to_str().unwrap()], "");
    assert!(ok);
    assert!(
        stderr.contains("no applicable fixes"),
        "nothing applied without --all: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        original,
        "the file is untouched"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_surfaces_a_non_exhaustive_match_on_a_param_with_the_add_arm_fix() {
    // A non-exhaustive match on a function PARAMETER used to be invisible to `cdz check` (the lowering
    // walk ran only on nullary exported bodies). It is now surfaced with its structured "add the missing
    // arm" insert fix — an agent reads the CDZ0210 and the arm to add.
    let dir = scratch_dir("check_nx");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (type C (A) (B) (D)) (def (f (: c C)) (match c ((A) 1) ((B) 2))) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(!ok, "non-exhaustive is an error");
    assert!(
        stdout.contains("CDZ0210") && stdout.contains("`D` not covered"),
        "surfaces the missing variant: {stdout}"
    );
    assert!(
        stdout.contains("add `(D (trap \"TODO: D\"))`"),
        "offers the add-arm fix with a diverging placeholder body: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_applies_an_add_arm_fix_whose_trap_placeholder_type_checks() {
    // The add-arm insert bodies its placeholder with a diverging `(trap "TODO: …")`, NOT `unit`: a trap
    // has type `∀a. a` (it never returns a value), so it unifies with whatever type the other arms have
    // and the completed match type-checks in ONE shot. So `--verify-fixes` UPGRADES the fix to verified
    // and `fix --all` APPLIES it, writing the covering arm — the author then replaces the `trap` with the
    // real body. (The earlier `unit` placeholder mismatched an `Int64` match and stayed declined; the
    // trap body is the one-shot-correct choice.)
    let dir = scratch_dir("fix_nx_apply");
    let f = dir.join("prog.sexp");
    let original =
        "(module m (type C (A) (B) (D)) (def (f (: c C)) (match c ((A) 1) ((B) 2))) (export f))\n";
    std::fs::write(&f, original).unwrap();
    // `--verify-fixes` UPGRADES it: the help line drops the `(heuristic)` marker (the trap body clears
    // the CDZ0210 and introduces no new error).
    let (_, stdout, _) = run(&["check", "--verify-fixes", f.to_str().unwrap()], "");
    assert!(
        stdout.contains("help: add `(D (trap \"TODO: D\"))`")
            && !stdout.contains("help (heuristic): add `(D"),
        "the trap-bodied insert verifies (no heuristic marker): {stdout}"
    );
    // `fix --all` applies it — the repaired file has the covering arm and re-checks clean.
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(D (trap \"TODO: D\"))"),
        "the covering arm was inserted: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "the repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_applies_a_rule_verified_fix_without_all() {
    // The compiler's OWN verified fix (CDZ0306 `_`-prefix silence) applies WITHOUT `--all` — it is
    // proven by a rule, not a recompile. `q` unused → `_q`, leaving `p` untouched.
    let dir = scratch_dir("fix_rule");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (f p q) (+ p 1)) (export f))\n").unwrap();
    let (ok, _, stderr) = run(&["fix", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(f p _q)"),
        "the unused param is prefixed: {repaired}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_deletes_a_latent_authority_effect_and_recompiles_clean() {
    // The `Edit::Delete` path end-to-end: a `host` delegates `log` but never performs it (CDZ0404); the
    // delete fix removes `log` from the manifest (`(host (log) 42)` → `(host () 42)`) — cleanly (no
    // stray space), and the repaired file re-checks clean.
    let dir = scratch_dir("fix_delete");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(do (effect log (op emit (-> String Unit))) (def (main) (host (log) 42)) (export main))\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(host () 42)"),
        "the unreached effect is deleted cleanly: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_shows_one_error_for_a_misspelled_handler_op_not_a_cascade() {
    // A misspelled handler op surfaces as ONE actionable CDZ0403 (with a `did you mean` fix), NOT that
    // error PLUS the emit path's "not yet reducible by the tail-resumptive fold" decline — the malformed
    // handler can't fold, so the decline is a consequence of the misspelling. An agent that applies the
    // `replace with get` fix should not then see a confusing second error.
    let dir = scratch_dir("check_handler_cascade");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(do (effect E (op get (-> Unit Int64))) \
         (def (main) (handle E 0 ((gett (u) s (resume s s))) 42)) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(!ok, "still an error");
    assert!(
        stdout.contains("CDZ0403") && stdout.contains("replace with `get`"),
        "the coded reject + its fix: {stdout}"
    );
    assert!(
        !stdout.contains("not yet reducible"),
        "the reducibility decline must not shadow the coded reject: {stdout}"
    );
    // Exactly one `error` line (the coded reject); the `help` line is not an error.
    let error_lines = stdout.lines().filter(|l| l.contains("error")).count();
    assert_eq!(error_lines, 1, "one primary error, not a cascade: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_renames_a_duplicate_parameter_and_clears_the_hard_error() {
    // CDZ0102's rename fix end-to-end: `(def (f (: x Int64) (: x Int64)) …)` → the second `x` becomes
    // `x2`, and the repaired file no longer has the CDZ0102 error (the fresh binder is unused — a CDZ0306
    // WARNING — but that is not an error, so the no-regression verify passes and `fix --all` applies it).
    let dir = scratch_dir("fix_dupparam");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (f (: x Int64) (: x Int64)) (+ x 1)) (export f))\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    // The duplicate `x` is renamed to a fresh `x2`; then, running to a FIXPOINT, the now-unused `x2` gets
    // the compiler-verified CDZ0306 `_`-prefix silence — so the end state is `_x2` (intentionally unused,
    // which is exactly right for a renamed duplicate the author has not yet wired up).
    assert!(
        repaired.contains("(: x Int64) (: _x2 Int64)"),
        "the duplicate parameter is renamed (and the unused rename `_`-silenced): {repaired}"
    );
    // The CDZ0102 hard error is gone AND the program re-checks fully clean (the `_`-prefix cleared the
    // unused-param warning too).
    let (ok2, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "no error remains: {stdout}");
    assert!(
        !stdout.contains("CDZ0102"),
        "the non-linear error is cleared: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_wraps_a_no_home_effect_in_a_host_delegation_and_recompiles_clean() {
    // CDZ0401's `Edit::Wrap` end-to-end: an ungranted effect gets `(host (E) <body>)` wrapped around the
    // entrypoint body, and the repaired file re-checks clean. The wrap verifies (recompile, no regression),
    // so `fix --all` applies it.
    let dir = scratch_dir("fix_nohome");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(do (effect E (op get (-> Unit Int64))) (def (main) (+ 1 (E.get unit))) (export main))\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(host (E) (+ 1 (E.get unit)))"),
        "the entrypoint body is wrapped in a host delegation: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_widens_an_out_of_range_annotation_and_recompiles_clean() {
    // CDZ0302's widen fix end-to-end: `(: 999 Int8)` → replace the annotation with `Int16` (the smallest
    // fitting width), and the repaired file re-checks clean. The widen verifies (recompile, no
    // regression), so `fix --all` applies it.
    let dir = scratch_dir("fix_widen");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) (: 999 Int8)) (export main))\n").unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(: 999 Int16)"),
        "the annotation is widened to the fitting type: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok2, "repaired file has no errors: {repaired}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fix_all_applies_one_wrap_when_a_second_independent_fault_survives() {
    // TWO ungranted effects. The structural apply runs to a FIXPOINT (re-parse + re-diagnose between
    // fixes), so it delegates BOTH — `(host (A) …)` then, on the next pass over the edited program,
    // `(host (B) …)` — and the file re-checks clean. (The message-keyed no-regression baseline still
    // matters per pass: applying A's wrap leaves B's CDZ0401, a PRE-EXISTING fault keyed by (code,
    // message), so the wrap is not wrongly declined as introducing a "new" error under node renumbering.)
    let dir = scratch_dir("fix_two_nohome");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(do (effect A (op ga (-> Unit Int64))) (effect B (op gb (-> Unit Int64))) \
         (def (main) (+ (A.ga unit) (B.gb unit))) (export main))\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("(host (A)") && repaired.contains("(host (B)"),
        "both effects delegated to a fixpoint: {repaired}"
    );
    let (ok2, _, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        ok2,
        "the fully-delegated program re-checks clean: {repaired}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_type_at_resolves_a_nested_node_to_the_right_type() {
    // Regression for the ML span↔node mismatch: `read_ml` builds nodes non-canonically, so without the
    // load-time canonicalize+remap a `type-at` on a nested node returned the WRONG node's type (the body
    // reference reported the enclosing def's arrow). `def add(a: Int64, b: Int64) = a + b` — hovering the
    // body `a` must be Int64 (its own type), not the arrow. (An ANNOTATED param so the type is definite;
    // a bare param is `unknown` until a call site, which would not exercise the id-mismatch.)
    let dir = scratch_dir("ml_typeat");
    let f = dir.join("prog.cdz");
    let src = "def add(a: Int64, b: Int64) = a + b\n";
    std::fs::write(&f, src).unwrap();
    let body_a = src.rfind("a + b").unwrap(); // the body reference `a`
    let (ok, stdout, _) = run(&["type-at", f.to_str().unwrap(), &body_a.to_string()], "");
    assert!(ok, "type-at succeeds: {stdout}");
    assert!(
        stdout.starts_with("Int64"),
        "the body `a` is Int64, not add's arrow (the span-mismatch bug): {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_fix_targets_the_value_node_not_the_annotation_type() {
    // The ML span fix, at the fix layer: an annotation-mismatch wrap on a `.cdz` file must target the
    // VALUE being wrapped (byte range of `n`), not the annotation type — the shifted-node bug would have
    // pointed the fix at `Option`. Checked via the JSON fix range.
    let dir = scratch_dir("ml_fixrange");
    let f = dir.join("prog.cdz");
    std::fs::write(
        &f,
        "def f(n: Int64) = (n : Option)\ntype Option = Some(Int64) | None\n",
    )
    .unwrap();
    let (_, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    // The CDZ0203 line's fix must span the body `n` at byte 19 (`(n : Option)` — the `n`), width 1.
    let line = stdout.lines().find(|l| l.contains("CDZ0203")).unwrap_or("");
    assert!(
        line.contains("\"kind\":\"wrap\"")
            && line.contains("\"from\":19")
            && line.contains("\"to\":20"),
        "the wrap targets the value `n` (byte 19-20), not the annotation type: {line}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_fix_renders_a_wrap_in_ml_syntax_and_applies_it() {
    // `cdz fix` on an ML file must render a wrap in ML surface syntax — `Some(n)`, not the s-expr
    // `(Some n)` (which is a parse error in ML). Before surface-aware rendering, `cdz fix` silently
    // DECLINED every wrap on `.cdz`/`.ml` (the verify re-parse failed). Now it applies + recompiles clean.
    let dir = scratch_dir("ml_wrapfix");
    let f = dir.join("prog.cdz");
    std::fs::write(
        &f,
        "type Option = Some(Int64) | None\ndef f(n: Int64) = (n : Option)\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        repaired.contains("Some(n)") && !repaired.contains("(Some n)"),
        "the wrap is rendered in ML syntax `Some(n)`, not s-expr `(Some n)`: {repaired}"
    );
    // The repaired ML file re-checks clean (the CDZ0203 is gone).
    let (_, check, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        !check.contains("CDZ0203"),
        "the repaired file no longer has the annotation mismatch: {check}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_fix_deletes_a_discarded_do_statement_and_recompiles_clean() {
    // `cdz fix` on an ML file must DELETE a discarded non-final sequencing statement (CDZ0307) cleanly —
    // absorbing the `;` separator that follows it, so the surviving statements stay well-formed. Before
    // the ML separator-aware widening, the byte-delete left an orphaned `;` (`inc(8); n*2` → ` ; n*2`, a
    // parse error), so `cdz fix` silently declined every discarded-value delete on `.cdz`/`.ml`. Now it
    // applies + recompiles clean.
    let dir = scratch_dir("ml_discardfix");
    let f = dir.join("prog.cdz");
    std::fs::write(
        &f,
        "def inc(n) = n + 1\n\ndef dbl(n) =\n  inc(8);\n  n * 2\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        !repaired.contains("inc(8)") && repaired.contains("n * 2"),
        "the discarded `inc(8);` statement is removed, the value `n * 2` kept: {repaired}"
    );
    assert!(
        !repaired.contains(';'),
        "no orphaned `;` separator remains: {repaired}"
    );
    // The repaired ML file re-checks clean (the CDZ0307 is gone, and no new parse/type error).
    let (check_ok, check, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        check_ok && !check.contains("CDZ0307"),
        "the repaired file no longer discards a value and has no error: {check}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_fix_deletes_the_first_of_two_discarded_do_statements_leaving_the_second() {
    // Two discarded inline statements — `inc(1); inc(2); n * 2`. `cdz fix --all` deletes BOTH across its
    // fixpoint passes (each pass absorbs one `;` separator), leaving only the block's value `n * 2`. Pins
    // that the ML separator widening composes over a longer sequence, not just a single deletion.
    let dir = scratch_dir("ml_discard2");
    let f = dir.join("prog.cdz");
    std::fs::write(
        &f,
        "def inc(n) = n + 1\n\ndef f(n) =\n  inc(1); inc(2); n * 2\n",
    )
    .unwrap();
    let (ok, _, stderr) = run(&["fix", "--all", f.to_str().unwrap()], "");
    assert!(ok, "fix succeeds: {stderr}");
    let repaired = std::fs::read_to_string(&f).unwrap();
    assert!(
        !repaired.contains("inc(1)") && !repaired.contains("inc(2)") && repaired.contains("n * 2"),
        "both discarded statements removed, the value kept: {repaired}"
    );
    let (check_ok, check, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(
        check_ok && !check.contains("CDZ0307"),
        "the repaired file re-checks clean: {check}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ml_drops_a_non_renderable_insert_fix_but_keeps_the_message() {
    // An `insert` fix (a handle/match arm) renders s-expr arm syntax that can't be byte-spliced into an
    // ML file (arm syntax only exists in-context). On ML the structured fix is DROPPED — but the message
    // still names the arm to add (guidance). A CDZ0405 non-exhaustive handler exercises this.
    let dir = scratch_dir("ml_insert");
    let f = dir.join("prog.cdz");
    std::fs::write(
        &f,
        "effect Diag =\n  | emit : Int64 -> Unit\n  | collect : Unit -> List(Int64)\n\n\
         def main() =\n  handle Diag([]) with\n    | emit(code, s) => resume(unit, List.push(s, code))\n  in\n  Diag.emit(1)\n",
    )
    .unwrap();
    let (_, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    let line = stdout.lines().find(|l| l.contains("CDZ0405")).unwrap_or("");
    assert!(!line.is_empty(), "CDZ0405 is reported: {stdout}");
    // No structured fix on ML (the s-expr arm can't be spliced), but the message names the arm.
    assert!(
        !line.contains("\"fix\":"),
        "no structured insert fix on ML: {line}"
    );
    assert!(
        line.contains("collect"),
        "the message still names the missing op / arm: {line}"
    );
    // s-expr (a `.sexp` file) DOES keep the structured insert fix — the surface it renders for.
    let sf = dir.join("prog.sexp");
    std::fs::write(
        &sf,
        "(do (effect Diag (op emit (-> Int64 Unit)) (op collect (-> Unit (List Int64)))) \
         (def (main) (handle Diag (list) ((emit (code) s (resume unit (List.push s code)))) \
         (do (Diag.emit 1) 0))) (export main))\n",
    )
    .unwrap();
    let (_, sjson, _) = run(&["check", "--json", sf.to_str().unwrap()], "");
    let sline = sjson.lines().find(|l| l.contains("CDZ0405")).unwrap_or("");
    assert!(
        sline.contains("\"kind\":\"insert\""),
        "s-expr keeps the structured insert fix: {sline}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_on_a_clean_program_prints_nothing_and_exits_zero() {
    let dir = scratch_dir("check_ok");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) (: 42 Int64)) (export main))\n").unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok, "a clean program exits 0");
    assert_eq!(stdout.trim(), "", "and prints no diagnostics: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz def FILE OFFSET` — go-to-definition (Query::ResolveOf) ----

#[test]
fn def_jumps_from_a_reference_to_its_definition() {
    let dir = scratch_dir("def_go");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (helper) 1) (def (main) helper) (export main))\n";
    std::fs::write(&f, src).unwrap();
    // The reference is the last `helper` (in `(def (main) helper)`).
    let ref_off = src.rfind("helper").unwrap();
    let (ok, stdout, _) = run(&["def", f.to_str().unwrap(), &ref_off.to_string()], "");
    assert!(ok, "go-to-def succeeds on a reference");
    // Jumps to helper's def body — line 1, the column of `1` (helper's body).
    assert!(
        stdout.contains("prog.sexp:1:"),
        "points at a file:line:col: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_json_emits_a_structured_location_object() {
    // `--json` emits the go-to-definition target as a single machine-readable {file,line,col} object — the
    // shape an editor consumes to jump without re-parsing the `file:line:col` text. PARSE it (serde_json is
    // in-crate) so a malformed object is caught, not substring-matched.
    let dir = scratch_dir("def_json");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (helper) 1) (def (main) helper) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let ref_off = src.rfind("helper").unwrap();
    let (ok, stdout, err) = run(
        &["def", f.to_str().unwrap(), &ref_off.to_string(), "--json"],
        "",
    );
    assert!(ok, "go-to-def --json succeeds on a reference: {err}");
    let rows: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 1, "one location object: {stdout}");
    let v: serde_json::Value = serde_json::from_str(rows[0])
        .unwrap_or_else(|e| panic!("row is valid JSON ({e}): {}", rows[0]));
    assert!(v["file"].is_string(), "`file` is a string: {stdout}");
    assert!(
        v["line"].is_number() && v["col"].is_number(),
        "`line`/`col` are numbers: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_on_a_non_reference_reports_no_definition() {
    let dir = scratch_dir("def_lit");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (main) 42) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let lit_off = src.find("42").unwrap();
    let (ok, _, stderr) = run(&["def", f.to_str().unwrap(), &lit_off.to_string()], "");
    assert!(!ok, "a literal has no definition to jump to");
    assert!(stderr.contains("no definition"), "clear message: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn def_error_paths_are_clear_not_panics() {
    // ERROR PATHS (`cdz def` must share the discipline its `type-at`/`doc-at` siblings already pin): a
    // bad offset / missing file / missing arg is a clear error naming the tool, NOT a panic or a silent
    // success. `def` had happy-path + json + non-reference coverage but its IO/arg error paths were
    // unpinned — this closes that consistency gap so a future regression (e.g. an unwrap on a past-EOF
    // offset) is caught at the gate, matching the type-at/doc-at error-path pins.
    let dir = scratch_dir("def_err");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) 1) (export main))\n").unwrap();
    let path = f.to_str().unwrap();
    // An offset past EOF is a clear "no node at byte offset" error naming the tool (shared with type-at).
    let (ok, _o, err) = run(&["def", path, "999999"], "");
    assert!(!ok, "an offset past EOF fails");
    assert!(
        err.contains("cdz:") && err.contains("no node at byte offset"),
        "clear no-node error naming the tool: {err}"
    );
    // A missing FILE is an I/O error naming the tool (via load_program_spanned).
    let (ok, _o, err) = run(&["def", "/no/such/file.sexp", "0"], "");
    assert!(!ok, "a missing file fails");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
    // A missing OFFSET argument is a clap usage error (offset is a required positional).
    let (ok, _o, err) = run(&["def", path], "");
    assert!(!ok, "a missing offset is a usage error");
    assert!(
        err.contains("error") || err.contains("Usage") || err.contains("required"),
        "clap usage error on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz scope FILE OFFSET` — variable scope tracking (Query::ScopeAt) ----

#[test]
fn scope_lists_the_visible_bindings_with_types() {
    let dir = scratch_dir("scope_vis");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (f (: p Int64)) (let ((q (: 5 Int64))) (+ p q))) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let off = src.find("(+ p q)").unwrap();
    let (ok, stdout, _) = run(&["scope", f.to_str().unwrap(), &off.to_string()], "");
    assert!(ok);
    // Both the param `p` and the let-binding `q` are in scope, both Int64, at a file:line:col.
    assert!(stdout.contains("p : Int64"), "param p in scope: {stdout}");
    assert!(
        stdout.contains("q : Int64"),
        "let-binding q in scope: {stdout}"
    );
    assert!(stdout.contains("prog.sexp:"), "with locations: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scope_json_emits_one_structured_object_per_binding() {
    // `--json` emits one machine-readable object per visible binding — {file,line,col,name,type} — the
    // shape an editor consumes for a scope/completion view without re-parsing the `file:line:col: name :
    // type` text. Both output shapes are computed from the SAME resolved bindings, so they keep row parity.
    let dir = scratch_dir("scope_json");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (f (: p Int64)) (let ((q (: 5 Int64))) (+ p q))) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let off = src.find("(+ p q)").unwrap();
    let (ok, stdout, err) = run(
        &["scope", f.to_str().unwrap(), &off.to_string(), "--json"],
        "",
    );
    assert!(ok, "cdz scope --json should succeed: {err}");
    let rows: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        rows.len(),
        2,
        "one JSON object per visible binding (p, q): {stdout}"
    );
    // PARSE each row (serde_json is in-crate) — a substring check would pass for MALFORMED JSON; parsing
    // rejects it. Collect the binding names off the parsed values.
    let mut names = std::collections::BTreeSet::new();
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(
            v["line"].is_number() && v["col"].is_number(),
            "`line`/`col` are numbers: {row}"
        );
        assert!(v["type"].is_string(), "`type` is a string: {row}");
        names.insert(v["name"].as_str().expect("`name` is a string").to_string());
    }
    // Both bindings ride through as structured `name` fields (parsed, not substring).
    assert!(
        names.contains("p") && names.contains("q"),
        "both bindings p + q are emitted as structured objects: {names:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scope_at_top_level_is_empty() {
    let dir = scratch_dir("scope_top");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (main) 42) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let off = src.find("42").unwrap();
    let (ok, stdout, _) = run(&["scope", f.to_str().unwrap(), &off.to_string()], "");
    assert!(ok, "no local scope is not an error");
    assert_eq!(
        stdout.trim(),
        "",
        "no local bindings at the top level: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scope_error_paths_are_clear_not_panics() {
    // ERROR PATHS (`cdz scope` is an OFFSET-based query like `def`/`type-at`/`doc-at`, and must share
    // their error-path discipline): a bad offset / missing file / missing arg is a clear error naming
    // the tool, NOT a panic or silent success. `scope` had happy + json + top-level-empty coverage but
    // its IO/arg error paths were unpinned — this closes that consistency gap so a future regression
    // (e.g. an unwrap on a past-EOF offset in run_scope) is caught at the gate, matching the def pins.
    // NOTE: an in-bounds offset with no local bindings is SUCCESS ("no bindings in scope"), pinned by
    // `scope_at_top_level_is_empty` above — that is NOT an error path; only past-EOF/missing-file/arg are.
    let dir = scratch_dir("scope_err");
    let f = dir.join("prog.sexp");
    std::fs::write(&f, "(module m (def (main) 1) (export main))\n").unwrap();
    let path = f.to_str().unwrap();
    // An offset past EOF is a clear "no node at byte offset" error naming the tool (shared with def).
    let (ok, _o, err) = run(&["scope", path, "999999"], "");
    assert!(!ok, "an offset past EOF fails");
    assert!(
        err.contains("cdz:") && err.contains("no node at byte offset"),
        "clear no-node error naming the tool: {err}"
    );
    // A missing FILE is an I/O error naming the tool (via load_program_spanned).
    let (ok, _o, err) = run(&["scope", "/no/such/file.sexp", "0"], "");
    assert!(!ok, "a missing file fails");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
    // A missing OFFSET argument is a clap usage error (offset is a required positional).
    let (ok, _o, err) = run(&["scope", path], "");
    assert!(!ok, "a missing offset is a usage error");
    assert!(
        err.contains("error") || err.contains("Usage") || err.contains("required"),
        "clap usage error on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz exports FILE` — the module interface (Query::Exports) ----

#[test]
fn exports_lists_each_export_with_its_type() {
    let dir = scratch_dir("exports");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (v) (: 5 Int64)) \
               (export inc) (export v))\n";
    std::fs::write(&f, src).unwrap();
    let (ok, stdout, _) = run(&["exports", f.to_str().unwrap()], "");
    assert!(ok);
    assert!(
        stdout.contains("inc : (-> Int64 Int64)"),
        "fn export: {stdout}"
    );
    assert!(stdout.contains("v : Int64"), "value export: {stdout}");
    assert!(stdout.contains("prog.sexp:"), "with locations: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn exports_json_emits_one_structured_object_per_export() {
    // `--json` emits one machine-readable object per export — {file,line,col,name,type} — the shape a
    // tool consumes to read a module's public interface without re-parsing the `file:line:col: name :
    // type` text. Both output shapes are computed from the SAME resolved export, so they keep row parity.
    let dir = scratch_dir("exports_json");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (inc (: n Int64)) (+ n 1)) (def (v) (: 5 Int64)) \
               (export inc) (export v))\n";
    std::fs::write(&f, src).unwrap();
    let (ok, stdout, err) = run(&["exports", f.to_str().unwrap(), "--json"], "");
    assert!(ok, "cdz exports --json should succeed: {err}");
    let rows: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 2, "one JSON object per export: {stdout}");
    // PARSE each row (serde_json is in-crate) — a substring check would pass for MALFORMED JSON; parsing
    // rejects it. Collect the (name → type) map off the parsed values.
    let mut exports = std::collections::BTreeMap::new();
    for row in &rows {
        let v: serde_json::Value =
            serde_json::from_str(row).unwrap_or_else(|e| panic!("row is valid JSON ({e}): {row}"));
        assert!(v["file"].is_string(), "`file` is a string: {row}");
        assert!(
            v["line"].is_number() && v["col"].is_number(),
            "`line`/`col` are numbers: {row}"
        );
        let name = v["name"].as_str().expect("`name` is a string").to_string();
        let ty = v["type"].as_str().expect("`type` is a string").to_string();
        exports.insert(name, ty);
    }
    // The export names + their types ride through as structured fields (parsed, not substring).
    assert_eq!(
        exports.get("inc").map(String::as_str),
        Some("(-> Int64 Int64)"),
        "the function export inc carries its arrow type: {exports:?}"
    );
    assert_eq!(
        exports.get("v").map(String::as_str),
        Some("Int64"),
        "the value export v carries its type: {exports:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz type-at` hover polish: keywords + def signatures, not `Any` ----

#[test]
fn type_at_on_a_keyword_names_it_not_any() {
    let dir = scratch_dir("hover_kw");
    let f = dir.join("prog.sexp");
    let src = "(module m (def (main) (: 42 Int64)) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let def_off = src.find("def").unwrap();
    let (ok, stdout, _) = run(&["type-at", f.to_str().unwrap(), &def_off.to_string()], "");
    assert!(ok);
    assert!(
        stdout.contains("keyword def"),
        "def is a keyword, not Any: {stdout}"
    );
    assert!(!stdout.contains("Any"), "no misleading Any: {stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- `cdz check` surfaces unused-binding warnings (CDZ0306), with `_`-prefix opt-out ----

#[test]
fn check_warns_on_an_unused_binding_and_underscore_silences_it() {
    let dir = scratch_dir("check_unused");
    let f = dir.join("prog.sexp");
    // `q` (param) and `b` (let) are unused; `p` and `a` are used. (Params ANNOTATED — an exported def's
    // unannotated param is itself a CDZ0201 ambiguous-param error, off-topic for this unused-binding test.)
    std::fs::write(
        &f,
        "(module m (def (f (: p Int64) (: q Int64)) (let ((a (: 1 Int64)) (b (: 2 Int64))) (+ a p))) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok, "warnings do not fail the build (exit 0)");
    assert!(stdout.contains("CDZ0306"), "an unused warning: {stdout}");
    assert!(
        stdout.contains("`q`") && stdout.contains("`b`"),
        "both unused named: {stdout}"
    );

    // `_`-prefix silences both.
    std::fs::write(
        &f,
        "(module m (def (f (: p Int64) (: _q Int64)) (let ((a (: 1 Int64)) (_b (: 2 Int64))) (+ a p))) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok);
    assert!(
        !stdout.contains("CDZ0306"),
        "`_`-prefixed are silenced: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_prints_a_verified_help_line_for_an_unused_binding() {
    // The unused-binding warning now carries a VERIFIED (machine-applicable) fix — the `help:` line is
    // printed WITHOUT the `(heuristic)` marker a guessed fix gets, so an agent branches on applicability.
    let dir = scratch_dir("check_verified_fix");
    let f = dir.join("prog.sexp");
    // Params annotated (an unannotated exported param is a CDZ0201, off-topic here); `q` is the unused one.
    std::fs::write(
        &f,
        "(module m (def (f (: p Int64) (: q Int64)) (+ p 1)) (export f))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", f.to_str().unwrap()], "");
    assert!(ok, "a warning does not fail the build");
    assert!(
        stdout.contains("help: replace with `_q`"),
        "a verified fix prints a plain help line: {stdout}"
    );
    assert!(
        !stdout.contains("help (heuristic)"),
        "a verified fix is NOT marked heuristic: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn compile_locates_a_source_error_instead_of_leaking_a_node_id() {
    // `cdz compile foo.sexp` on a program with a semantic error used to report `error [CDZ0101] (node
    // 6): unbound name ...` — a raw internal node id, no source position — while `cdz check` on the same
    // file gave `foo.sexp:1:22: error [CDZ0101]: ...`. A source-file compile now attaches spans and the
    // CLI reporter maps the diagnostic to `path:line:col`, so `compile` locates errors like `check`.
    let dir = scratch_dir("compile_loc");
    let file = dir.join("bad.sexp");
    std::fs::write(&file, "(do (def (main) (+ 1 nope)) (export main))\n").unwrap();
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args([
            "compile",
            file.to_str().unwrap(),
            "-o",
            dir.join("out.wasm").to_str().unwrap(),
        ])
        .output()
        .expect("run cdz compile");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(!out.status.success(), "an unbound name fails the compile");
    assert!(
        stderr.contains("bad.sexp:1:") && stderr.contains("CDZ0101"),
        "compile locates the error as path:line:col with its code: {stderr}"
    );
    assert!(
        !stderr.contains("(node "),
        "compile must NOT leak a raw internal node id: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_by_name_reads_a_user_doc_a_builtin_and_a_keyword() {
    // `cdz doc NAME FILE` surfaces a definition's `(doc "…")`, and falls back to a built-in module's
    // `(meta doc)` channel / a grammar keyword's help. All in ONE binary (front-end + compiler).
    let dir = scratch_dir("docname");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def answer (doc \"the ultimate answer\") 42) (def (main) answer) (export main))\n",
    )
    .unwrap();
    let path = f.to_str().unwrap();
    // A user def's docstring.
    let (ok, stdout, _) = run(&["doc", "answer", path], "");
    assert!(ok, "doc succeeds: {stdout}");
    assert_eq!(stdout.trim(), "the ultimate answer");
    // A built-in module — read off its record's `(meta doc)` channel, not a name match.
    let (ok, stdout, _) = run(&["doc", "List", path], "");
    assert!(ok);
    assert!(
        stdout.contains("persistent") && stdout.contains("sequence"),
        "List's built-in doc: {stdout}"
    );
    // A grammar keyword — from the keyword doc table.
    let (ok, stdout, _) = run(&["doc", "if", path], "");
    assert!(ok);
    assert!(
        stdout.starts_with("Conditional"),
        "if's keyword doc: {stdout}"
    );
    // Total, with DISTINCT verdicts: a real-but-undocumented name says "no documentation for", while a
    // name that refers to NOTHING (a typo) says "no such definition" — so a user can tell the two apart.
    // `main` is a real def with no docstring:
    let (ok, stdout, _) = run(&["doc", "main", path], "");
    assert!(
        ok,
        "an undocumented real def still exits 0 (total): {stdout}"
    );
    assert_eq!(stdout.trim(), "no documentation for `main`");
    // `ghost` names nothing → the distinct "no such definition" line. The sidecar query stays TOTAL (it
    // returns a defined line), but the cdz-command layer maps this "no such definition" variant to a
    // NON-ZERO exit — so a user/script can tell an unresolvable name (a typo) from a real-but-undocumented
    // one. (v-cdz-tooling's half of the split; the "no documentation for" case above stays exit 0.)
    let (ok, stdout, _) = run(&["doc", "ghost", path], "");
    assert!(
        !ok,
        "an unresolvable name now exits non-zero (distinct from undocumented): {stdout}"
    );
    assert_eq!(stdout.trim(), "no such definition `ghost`");
    // A typo that is a NEAR-MISS of a real def gets a "did you mean?" — `answr` → `answer` — and also fails.
    let (ok, stdout, _) = run(&["doc", "answr", path], "");
    assert!(
        !ok,
        "an unresolvable near-miss also exits non-zero: {stdout}"
    );
    assert_eq!(
        stdout.trim(),
        "no such definition `answr` — did you mean `answer`?"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn doc_at_offset_reads_the_doc_at_a_use_and_at_the_definition() {
    // `cdz doc-at FILE OFFSET` resolves the cursor to a node, then to the definition it is or references,
    // and prints that definition's docstring — the "documentation at cursor" hover.
    let dir = scratch_dir("docat");
    let f = dir.join("prog.sexp");
    let src =
        "(module m (def helper (doc \"a helper value\") 7) (def (main) helper) (export main))\n";
    std::fs::write(&f, src).unwrap();
    let path = f.to_str().unwrap();
    // Hover the USE of `helper` in main (the last occurrence).
    let use_off = src.rfind("helper").unwrap();
    let (ok, stdout, _) = run(&["doc-at", path, &use_off.to_string()], "");
    assert!(ok, "doc-at succeeds: {stdout}");
    assert_eq!(stdout.trim(), "a helper value");
    // Hover the DEFINITION's own name occurrence (`helper` in the def signature).
    let def_off = src.find("def helper").unwrap() + 4;
    let (ok, stdout, _) = run(&["doc-at", path, &def_off.to_string()], "");
    assert!(ok);
    assert_eq!(stdout.trim(), "a helper value");
    // A node that documents nothing (the `7` literal) — a defined "no documentation" line, exit 0.
    let lit_off = src.find(") 7)").unwrap() + 2;
    let (ok, stdout, _) = run(&["doc-at", path, &lit_off.to_string()], "");
    assert!(ok, "a node with no doc still exits 0 (total): {stdout}");
    assert!(
        stdout.contains("no documentation"),
        "empty hover message: {stdout}"
    );
    // ERROR PATHS (the doc companion of `cdz type-at` must share its discipline): an offset past EOF is a
    // clear error naming the tool + "no node at byte offset", NOT a panic or a silent empty success.
    let (ok, _o, err) = run(&["doc-at", path, "999999"], "");
    assert!(!ok, "an offset past EOF fails");
    assert!(
        err.contains("cdz:") && err.contains("no node at byte offset"),
        "clear no-node error naming the tool: {err}"
    );
    // A missing FILE is an I/O error naming the tool.
    let (ok, _o, err) = run(&["doc-at", "/no/such/file.sexp", "0"], "");
    assert!(!ok, "a missing file fails");
    assert!(err.contains("cdz:"), "the error names the tool: {err}");
    // A missing OFFSET argument is a clap usage error.
    let (ok, _o, err) = run(&["doc-at", path], "");
    assert!(!ok, "a missing offset is a usage error");
    assert!(
        err.contains("error") || err.contains("Usage") || err.contains("required"),
        "clap usage error on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
