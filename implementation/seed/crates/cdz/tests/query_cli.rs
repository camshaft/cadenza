//! End-to-end tests for the `cdz query` / `rewrite` codemod subcommands (the front-end surface,
//! now served by the unified `cdz` binary — same code as the retired `cdz-syntax`, via `cadenza_syntax::cli`).
//!
//! These drive the actual built binary over stdin/stdout — the integration counterpart to the
//! `query` module's unit tests, proving the CLI wiring (arg parsing, format resolution, the
//! validated-transaction rewrite, exit codes) end-to-end.

use std::io::Write;
use std::process::{Command, Stdio};

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
    // Write stdin, tolerating a broken pipe: a command that rejects its args (e.g. a bad pattern)
    // exits and closes its stdin BEFORE we finish writing, so `write_all` races against that exit.
    // On a slower runner that surfaces as `BrokenPipe` — benign here, since the assertions check the
    // exit status and stderr, not that every byte was consumed.
    if let Err(e) = child.stdin.take().unwrap().write_all(stdin.as_bytes())
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        panic!("write stdin to cdz: {e}");
    }
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
fn lint_with_no_rules_is_an_error() {
    let (ok, _, stderr) = run(&["lint", "--from", "sexpr"], "(f a)");
    assert!(!ok);
    assert!(stderr.contains("no lint rules"), "reason: {stderr}");
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
fn check_json_emits_a_machine_readable_diagnostic_with_a_structured_fix() {
    // `--json` gives an agent the fix as DATA — code, message, byte range, and a nested `fix` object
    // (kind + replacement + verified + byte range) it applies directly, not the human `help:` text.
    let dir = scratch_dir("check_json");
    let f = dir.join("prog.sexp");
    std::fs::write(
        &f,
        "(module m (def (compute x) x) (def (main) (computee 1)) (export main))\n",
    )
    .unwrap();
    let (ok, stdout, _) = run(&["check", "--json", f.to_str().unwrap()], "");
    assert!(!ok, "the unbound name is still an error (exit non-zero)");
    // One JSON object, carrying the code, a byte range, and the structured replace fix.
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
        line.contains("\"replacement\":\"compute\""),
        "the replacement text: {stdout}"
    );
    assert!(
        line.contains("\"verified\":false"),
        "the applicability marker as a JSON bool: {stdout}"
    );
    assert!(
        line.contains("\"from\":") && line.contains("\"to\":"),
        "a byte range to apply the edit over: {stdout}"
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
    // `q` (param) and `b` (let) are unused; `p` and `a` are used.
    std::fs::write(
        &f,
        "(module m (def (f p q) (let ((a (: 1 Int64)) (b (: 2 Int64))) (+ a p))) (export f))\n",
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
        "(module m (def (f p _q) (let ((a (: 1 Int64)) (_b (: 2 Int64))) (+ a p))) (export f))\n",
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
    std::fs::write(&f, "(module m (def (f p q) (+ p 1)) (export f))\n").unwrap();
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
