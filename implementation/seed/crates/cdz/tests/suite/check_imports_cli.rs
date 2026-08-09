//! End-to-end tests for `cdz check` FOLLOWING a file's import closure — the diagnostics loop for a
//! multi-file package. `cdz check FILE` loads FILE plus the transitive closure of the files it
//! `(import …)`s (resolved as siblings in FILE's directory), links them into one program, and reports
//! diagnostics for the whole — a cross-file reference (an imported type/definition) resolves instead of
//! surfacing as "unbound name". A diagnostic that lands in an imported library is located at THAT
//! library's own `path:line:col` (via the package `link-map` demux). A file that imports nothing is
//! checked exactly as a standalone file (byte-identical to before).
//!
//! These drive the actual built binary over temp files — the integration counterpart to the in-crate
//! logic, proving the package-load + link + demux wiring end-to-end.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

/// A fresh temp directory unique to this test process + a discriminator, so parallel tests don't
/// collide. Cleaned by the OS temp reaper; each test writes its own files under it.
fn pkg_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-check-imports-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir temp pkg");
    dir
}

fn write(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write file");
    path.to_string_lossy().into_owned()
}

#[test]
fn a_check_follows_an_import_and_resolves_a_cross_file_name() {
    // `app` imports `kind-of-int` from `lib` and calls it; the cross-file reference resolves +
    // type-checks, so the check is CLEAN. Before this change `cdz check app.sexp` reported `import`
    // unmodeled + `kind-of-int` unbound. `lib` OWNS the `Ast` type and builds/matches it internally
    // (a module's own constructors are always usable); the entry only calls the exported function —
    // the realistic decode shape, and one that respects constructor visibility (CDZ0214).
    let dir = pkg_dir("clean");
    write(
        &dir,
        "lib.sexp",
        "(do (type Ast (Int Int64) (Name String)) \
           (def (kind-of-int (: v Int64)) \
             (match (Ast.Int v) (((. Ast Int) _) 1) (((. Ast Name) _) 2))) \
           (export kind-of-int))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (kind-of-int)) (def (main) (kind-of-int 5)) (export main))",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(
        ok,
        "clean cross-file check should succeed: {stdout}{stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "no diagnostics expected: {stdout}"
    );
}

#[test]
fn a_wildcard_exported_constructor_that_collides_with_a_prelude_type_name_imports_and_constructs() {
    // REGRESSION: a variant whose name collides with a prelude TYPE/MODULE name (`Int`/`Bool`/`List`) —
    // the natural spelling of an AST sum, `(type Ast (Int Int64) (Bool Bool) (List (List Ast)) …)` — is
    // omitted from the BARE constructor map (so bare `Int` keeps meaning the width type), and the CDZ0214
    // withheld check used to consult THAT bare map. So a wildcard-exported `Ast.Int` imported into a
    // sibling was falsely rejected "constructor `Int` is not exported" — it even flagged the DECLARING
    // file's own `Ast.Int`. A prelude-named ctor is unreachable bare but perfectly reachable QUALIFIED, so
    // the withheld check now consults the qualified surface (`file_scoped_variant_ctor_qualified`). This
    // pins that `lib` exports `Ast.*`, the entry `(import "lib" (Ast tag))` constructs `(. Ast Int …)`
    // AND `(. Ast List …)`, and the whole package checks CLEAN.
    let dir = pkg_dir("prelude-name-ctor");
    write(
        &dir,
        "lib.sexp",
        "(do (type Ast (Int Int64) (Bool Bool) (List (List Ast))) \
           (def (tag (: node Ast)) \
             (match node (((. Ast Int) _) 1) (((. Ast Bool) _) 2) (((. Ast List) _) 3))) \
           (export (. Ast *)) (export tag))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (Ast tag)) \
           (def (main) (tag ((. Ast List) (list ((. Ast Int) 1) ((. Ast Bool) true))))) \
           (export main))",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(
        ok,
        "a wildcard-exported prelude-named ctor must import + construct cleanly: {stdout}{stderr}"
    );
    assert!(
        stdout.trim().is_empty(),
        "no diagnostics expected (no false CDZ0214): {stdout}"
    );
}

#[test]
fn an_abstract_import_still_withholds_a_prelude_named_constructor() {
    // The dual of the regression above: when `lib` exports the type HANDLE alone (bare `(export Ast)`, no
    // `(. Ast *)`), a prelude-named ctor `Ast.Int` in the IMPORTER is STILL a withheld constructor
    // (CDZ0214) — the qualified-surface fix must not leak an abstract type's constructors. The declaring
    // file's own `Ast.Int` remains usable; only the importer is blocked.
    let dir = pkg_dir("prelude-name-abstract");
    write(
        &dir,
        "lib.sexp",
        "(do (type Ast (Int Int64) (Name String)) \
           (def (tag (: node Ast)) (match node (((. Ast Int) _) 1) (((. Ast Name) _) 2))) \
           (export Ast) (export tag))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (Ast tag)) (def (main) (tag ((. Ast Int) 5))) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app]);
    assert!(!ok, "an abstract import's ctor use must fail the check");
    assert!(
        stdout.contains("CDZ0214") && stdout.contains('`') && stdout.contains("Int"),
        "the importer's `Ast.Int` is a withheld constructor: {stdout}"
    );
}

#[test]
fn a_diagnostic_in_an_imported_library_is_located_in_that_library() {
    // The type error is in `lib`, not `app` — the check must point at `lib.sexp`, not the entry.
    let dir = pkg_dir("lib-error");
    let lib = write(
        &dir,
        "lib.sexp",
        "(do (def (helper (: x Int64)) (+ x true)) (export helper))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app]);
    assert!(!ok, "a type error must fail the check");
    assert!(
        stdout.contains(&lib) && stdout.contains("CDZ0203"),
        "the diagnostic should be located in lib.sexp with the type code: {stdout}"
    );
}

#[test]
fn a_transitive_import_error_is_reached_and_located() {
    // app → mid → base; the error is in `base`, two hops from the entry.
    let dir = pkg_dir("transitive");
    let base = write(
        &dir,
        "base.sexp",
        "(do (def (b (: x Int64)) (+ x false)) (export b))",
    );
    write(
        &dir,
        "mid.sexp",
        "(do (import \"base\" (b)) (def (m (: x Int64)) (b x)) (export m))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"mid\" (m)) (def (main) (m 3)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app]);
    assert!(!ok, "the transitive type error must fail the check");
    assert!(
        stdout.contains(&base),
        "the diagnostic should be located in base.sexp: {stdout}"
    );
}

#[test]
fn an_unresolved_import_reports_a_precise_diagnostic() {
    // An import naming no sibling file: the LINK path gives a precise "unknown package file" error,
    // not the generic "imports are not modeled here" a bare single-file compile falls back to.
    let dir = pkg_dir("unresolved");
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"nope\" (x)) (def (main) (x 1)) (export main))",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(!ok, "an unresolved import must fail the check");
    let out = format!("{stdout}{stderr}");
    assert!(
        out.contains("nope") && out.contains("unknown package file"),
        "an unresolved import should name the missing file: {out}"
    );
}

#[test]
fn a_package_json_diagnostic_names_its_file() {
    // In `--json`, a diagnostic that belongs to an imported file carries a `file` field so an agent
    // knows which file the `from`/`to` byte offsets index.
    let dir = pkg_dir("json");
    let lib = write(
        &dir,
        "lib.sexp",
        "(do (def (helper (: x Int64)) (+ x true)) (export helper))",
    );
    let app = write(
        &dir,
        "app.sexp",
        "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &app, "--json"]);
    assert!(!ok);
    assert!(
        stdout.contains("\"file\":") && stdout.contains(&lib),
        "the JSON diagnostic should carry the imported file's path: {stdout}"
    );
}

#[test]
fn a_comment_on_an_import_is_seen_through_by_the_link_scan() {
    // A `//`/`///` comment on an `(import …)` reifies (in the ML reader) to `(comment "…" (import …))`.
    // The link scan reads imports off the RAW arena before `Db::load`'s comment-strip, so an un-peeled
    // wrapper would leave the import unrecognized → spliced as an unmodeled top-level form → "`import`
    // … not modeled" + the imported names unbound. `link_inputs` now peels comments first, so a
    // documented import resolves exactly as a bare one. (ML surface — `.cdz` — is where the reader
    // produces the wrapper.)
    let dir = pkg_dir("comment-import");
    write(
        &dir,
        "lib.cdz",
        "type Ty =\n  | Var(Int64)\n  | Con(String)\n\
         def kind(t: Ty) -> Int64 = match t with\n  | Ty.Var(n) => n\n  | Ty.Con(_) => 0\n\
         export { Ty.*, kind }\n",
    );
    let app = write(
        &dir,
        "app.cdz",
        "/// bring in the type + its reader\nimport { Ty, kind } from \"lib\"\n\
         def main() -> Int64 = kind(Ty.Var(7))\nexport { main }\n",
    );
    let (ok, stdout, stderr) = run(&["check", &app]);
    assert!(
        ok && stdout.trim().is_empty(),
        "a documented import must resolve like a bare one: {stdout}{stderr}"
    );
}

#[test]
fn a_file_with_no_imports_checks_as_a_standalone_file() {
    // The single-file path is unchanged: a self-contained program with a type error fails, located in
    // itself, with no package machinery engaged.
    let dir = pkg_dir("standalone");
    let solo = write(
        &dir,
        "solo.sexp",
        "(do (def (main) (+ 1 true)) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &solo]);
    assert!(!ok);
    assert!(
        stdout.contains(&solo) && stdout.contains("CDZ0203"),
        "a standalone type error is located in the file itself: {stdout}"
    );
}

#[test]
fn an_ml_file_that_does_not_parse_fails_the_check_even_with_no_semantic_fault() {
    // A CLEAN TRUNCATION — an unclosed `(` — that the ML reader RECOVERS from by dropping the broken
    // subtree, leaving a well-formed but incomplete arena that carries NO downstream semantic fault. The
    // parse error is printed to stderr, but `check` used to exit 0 (its exit keyed only on the SEMANTIC
    // fault set), reporting a file that does not parse as clean — an editor/CI then treats a broken file
    // as passing. A parse error IS an error-severity fault; the check must FAIL.
    let dir = pkg_dir("ml-unclosed");
    let bad = write(&dir, "bad.cdz", "def main() = (1 + 2\n");
    let (ok, _stdout, stderr) = run(&["check", &bad]);
    assert!(!ok, "an unparseable ML file must fail the check");
    assert!(
        stderr.contains("expected `)`"),
        "the parse error is still surfaced: {stderr}"
    );
}

#[test]
fn a_recovered_error_placeholder_does_not_cascade_into_an_unbound_name() {
    // `if … then …` with no `else` leaves an `<error>` PLACEHOLDER node where the else-branch belongs.
    // In expression position that placeholder reduces to a bare name `<error>`, which the checker reported
    // as `unbound name `<error>`` (CDZ0101) — a spurious fault naming a token the user never wrote, piled
    // on top of the real "expected `else`" parse error. `<error>` is UNLEXABLE on the ML surface, so a
    // diagnostic naming it is ALWAYS the placeholder; it is suppressed (the parse error already says the
    // fix). The check still FAILS (the parse error), and the noise line is gone.
    let dir = pkg_dir("ml-if-no-else");
    let bad = write(&dir, "bad.cdz", "def main() = if true then 1\n");
    let (ok, stdout, stderr) = run(&["check", &bad]);
    assert!(!ok, "a missing `else` must fail the check");
    assert!(
        stderr.contains("expected `else`"),
        "the real parse error is surfaced: {stderr}"
    );
    assert!(
        !stdout.contains("`<error>`") && !stderr.contains("`<error>`"),
        "the `<error>`-placeholder cascade must be suppressed: {stdout}{stderr}"
    );
}

#[test]
fn a_recovered_error_placeholder_is_suppressed_in_any_downstream_code_not_just_unbound() {
    // A failed production leaves the `<error>` placeholder wherever it was parsing, so it surfaces in
    // DIFFERENT downstream checks by position — not only `unbound name` (CDZ0101). A garbled PARAMETER
    // list (`(d: <Int64 meter>)`, an unsupported quantity-type surface) recovers several `<error>` binders,
    // which the linearity check then reports as "parameter `<error>` is bound more than once" (CDZ0102) —
    // plus misleading `<error>2` / `_<error>` fixes on the synthetic token. The suppression keys on the
    // `<error>` reference in ANY code, so every placeholder-cascade line (and its bogus fix) is dropped,
    // leaving only the real parse errors. The check still FAILS (the parse error).
    let dir = pkg_dir("ml-garbled-params");
    let bad = write(
        &dir,
        "bad.cdz",
        "def f (d: <Int64 meter>) = d\ndef main() = f 1\n",
    );
    let (ok, stdout, stderr) = run(&["check", &bad]);
    assert!(!ok, "a garbled parameter list must fail the check");
    assert!(
        !stdout.contains("`<error>`"),
        "no placeholder-referencing diagnostic (CDZ0102 or otherwise) survives: {stdout}"
    );
    assert!(
        !stdout.contains("<error>2") && !stdout.contains("_<error>"),
        "the misleading fixes on the synthetic placeholder are gone too: {stdout}"
    );
    assert!(
        stderr.contains("expected"),
        "the real parse error is still surfaced: {stderr}"
    );
}

#[test]
fn a_non_exhaustive_match_advertises_its_insert_arms_fix_on_both_ml_surfaces_in_agreement() {
    // The text `help:` line and the JSON `fix` object must AGREE on whether a diagnostic has an
    // APPLICABLE fix — both gated on the SAME built structural patch. The concrete case: a non-exhaustive
    // `match` (CDZ0210) carries an INSERT-ARMS "add the missing arm" fix. This was HISTORICALLY DROPPED on
    // the ML surface (a stale insert+ml suppression, because the textedit render printed a spliced arm as
    // a standalone application, not `| pat => body`). Now that v-syntax's render_child renders the arm
    // correctly + the suppression is removed, the fix IS applyable on ML — so BOTH surfaces must advertise
    // it (agreement in the POSITIVE direction): a `help (heuristic): add …` line AND a JSON `fix`. (The
    // `fix_cli` suite pins that the JSON edit is a valid ML arm that applies to an exhaustive re-check.)
    let dir = pkg_dir("ml-insert-arms-agree");
    let bad = write(
        &dir,
        "bad.cdz",
        "type T = A | B\ndef f (t: T) = match t with A => 1\ndef main() = f A\n",
    );
    let (ok, stdout, _) = run(&["check", &bad]);
    assert!(!ok, "a non-exhaustive match fails the check");
    assert!(
        stdout.contains("non-exhaustive match"),
        "the CDZ0210 diagnostic is reported: {stdout}"
    );
    assert!(
        stdout.contains("help: add") || stdout.contains("help (heuristic): add"),
        "the add-arm help line IS shown now the ML insert fix builds: {stdout}"
    );
    let (_, json, _) = run(&["check", &bad, "--json"]);
    let exhaustive_line = json
        .lines()
        .find(|l| l.contains("non-exhaustive match"))
        .expect("the CDZ0210 diagnostic is present in json");
    assert!(
        exhaustive_line.contains("\"fix\""),
        "the json diagnostic carries the fix now the patch builds on ML (agrees with the help line): {exhaustive_line}"
    );
}

#[test]
fn a_match_arm_binder_used_deep_in_a_nested_let_if_chain_is_not_a_false_cdz0101() {
    // REGRESSION (the false-CDZ0101 that stalled the fleet ~82min — v-inference fix 0025900937): the
    // Diagnostics resolver surfaced a spurious `unbound name` CDZ0101 for a SYNTHESIZED node (an inference
    // β-copy artifact) when a `match`-arm binder was used DEEP inside a nested `let`/`if` chain — the copy
    // lost the binder's scope, and the false error even anchored onto a SIBLING call node (a span with no
    // occurrence of the name). `cdz check` then FALSE-RED green source that `cdz test` ran clean
    // (check≠test divergence). This pins the SHAPE: a match-arm binder (`tyname`) bound then used across a
    // multi-level `let`/`if` nest, all types consistent — so any error is spurious. `cdz check` must be
    // CLEAN (no CDZ0101, exit 0); a re-introduced synth-node false-positive reds this. (Check-path only —
    // no `cdz test`, so this is storeless-CI-safe; the front-end resolve is what regressed, not execution.)
    let dir = pkg_dir("match-binder-deep-nest");
    let solo = write(
        &dir,
        "ann.cdz",
        "def kind-of(n: String) = true\n\
         def width-of(n: String) = 64\n\
         def fits(k: Int64, w: Int64) = true\n\
         def use-it(k: Int64) = k\n\
         def scan(s: String, k: Int64) = (s, k + 1)\n\
         \n\
         def ann-with-value(s: String, k: Int64) =\n\
           (match scan(s, k) with | (tyname, a2) =>\n\
             (let a3 = width-of(tyname) in\n\
              (if (kind-of(tyname)) then\n\
                 (let w = width-of(tyname) in\n\
                  (if (fits(k, w)) then use-it(k)\n\
                   else use-it(a2)))\n\
               else use-it(a2))))\n",
    );
    let (ok, stdout, stderr) = run(&["check", &solo]);
    assert!(
        ok,
        "a match-arm binder used deep in a nested let/if chain checks CLEAN — exit 0: {stdout}{stderr}"
    );
    assert!(
        !stdout.contains("unbound name `tyname`") && !stderr.contains("unbound name `tyname`"),
        "the in-scope match-arm binder must NOT surface a false CDZ0101 (the synth-node β-copy bug): {stdout}{stderr}"
    );
    assert!(
        !stdout.contains("error [CDZ0101]") && !stderr.contains("error [CDZ0101]"),
        "no spurious unbound-name error anywhere in this well-formed program: {stdout}{stderr}"
    );
}

#[test]
fn a_legit_error_symbol_in_sexpr_is_not_suppressed_as_a_placeholder() {
    // The `<error>`-cascade suppression is GATED on the file having actually had a parse error. In s-expr
    // `<error>` is a LEGAL symbol (the reader hard-errors on a malformed program, so a well-formed one
    // that names `<error>` had zero parse errors) — so a real `unbound name `<error>`` there is NOT a
    // recovery placeholder and must still be reported.
    let dir = pkg_dir("sexpr-error-name");
    let solo = write(
        &dir,
        "solo.sexp",
        "(module m (def (main) <error>) (export main))",
    );
    let (ok, stdout, _) = run(&["check", &solo]);
    assert!(!ok, "a genuinely unbound `<error>` name still fails");
    assert!(
        stdout.contains("unbound name `<error>`"),
        "a real `<error>` name in a parse-clean file is reported, not suppressed: {stdout}"
    );
}
