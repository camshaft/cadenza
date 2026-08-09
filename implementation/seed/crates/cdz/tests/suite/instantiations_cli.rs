//! End-to-end tests for `cdz instantiations NAME FILE` — the CLI-RENDERING layer over the compiler's
//! `Instantiations` sidecar query. The sidecar's own logic (what disposition a def gets, which concrete
//! instances a generic monomorphizes into) is exercised by `rcdzc`'s unit tests; what is pinned HERE is
//! the thing only the `cdz` binary does: mapping each result node back to a source `file:line:col`
//! through the span table, glossing the disposition readably, and printing each specialization as
//! `NAME[arg, arg, …] → spec`. Drives the built binary over a real source file.

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
    let dir = std::env::temp_dir().join(format!("cdz-inst-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    path.to_str().unwrap().to_string()
}

/// A recursive generic threaded at TWO concrete element types → the compiler monomorphizes it into two
/// functions; `loopn` at Int64 (via `a`) and at String (via `"hi"`).
const GENERIC: &str = "(module m \
    (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
    (def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\")))) \
    (export main))";

#[test]
fn a_specialized_generic_reports_each_instantiation_with_a_location() {
    let file = temp_src("spec", GENERIC);
    let (ok, out, err) = run(&["instantiations", "loopn", &file]);
    assert!(ok, "instantiations should succeed: {err}");
    // The disposition line: the def name, its fate, and the human gloss. The location is `file:line:col`
    // mapped through the span table — the byte-offset→line:col step the sidecar CANNOT do (it has ids).
    assert!(
        out.contains("loopn — specialized"),
        "disposition + gloss: {out}"
    );
    assert!(
        out.contains("monomorphized"),
        "the `specialized` gloss is present: {out}"
    );
    // Each instantiation is rendered `NAME[arg, arg, …] → spec` — the `;`-joined sidecar arg list becomes
    // a comma-joined bracketed list. BOTH concrete instances appear (Int64 and String for `x`).
    assert!(
        out.contains("loopn[n: Int64, x: Int64] →"),
        "Int64 instantiation, comma-joined + arrow: {out}"
    );
    assert!(
        out.contains("loopn[n: Int64, x: String] →"),
        "String instantiation: {out}"
    );
    // Every printed line carries a `file:line:col` prefix (the span-map payoff). Assert the file name and
    // a `:LINE:COL:` shape appears on the disposition line.
    let disp_line = out
        .lines()
        .find(|l| l.contains("specialized"))
        .expect("a disposition line");
    assert!(
        disp_line.starts_with(&file) && disp_line[file.len()..].starts_with(':'),
        "disposition line is `file:line:col: …`: {disp_line}"
    );
    let _ = std::fs::remove_dir_all(std::path::Path::new(&file).parent().unwrap());
}

#[test]
fn an_emitted_definition_reports_its_disposition_only() {
    // A monomorphic exported function is EMITTED — one disposition line, no `inst` (instantiation) lines.
    let file = temp_src("emit", GENERIC);
    let (ok, out, err) = run(&["instantiations", "main", &file]);
    assert!(ok, "should succeed: {err}");
    assert!(out.contains("main — emitted"), "emitted disposition: {out}");
    assert!(
        out.contains("standalone function"),
        "the `emitted` gloss: {out}"
    );
    // No specialization arrow — an emitted monomorphic def has no `NAME[…] → spec` line.
    assert!(
        !out.contains(" → "),
        "no instantiation lines for `main`: {out}"
    );
    let _ = std::fs::remove_dir_all(std::path::Path::new(&file).parent().unwrap());
}

#[test]
fn an_inlined_definition_is_glossed_as_beta_reduced() {
    // A NON-recursive generic is β-reduced into each call site — reported `inlined`, no standalone function.
    let file = temp_src(
        "inl",
        "(module m (def (ident v) v) \
         (def (main (: x Int64)) (+ (ident x) (ident 1))) (export main))",
    );
    let (ok, out, err) = run(&["instantiations", "ident", &file]);
    assert!(ok, "should succeed: {err}");
    assert!(
        out.contains("ident — inlined"),
        "inlined disposition: {out}"
    );
    assert!(out.contains("β-reduced"), "the `inlined` gloss: {out}");
    let _ = std::fs::remove_dir_all(std::path::Path::new(&file).parent().unwrap());
}

#[test]
fn a_linear_recursion_is_reported_as_transformed_into_an_accumulator() {
    // A linear non-tail recursion is rewritten into an accumulator loop — reported `transformed→NAME$acc`
    // (a combination tag that carries its own words, so it gets the accumulator gloss).
    let file = temp_src(
        "acc",
        "(module m (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) \
         (def (main (: k Int64)) (sm k)) (export main))",
    );
    let (ok, out, err) = run(&["instantiations", "sm", &file]);
    assert!(ok, "should succeed: {err}");
    assert!(
        out.contains("sm — transformed→sm$acc"),
        "transformed disposition names the accumulator copy: {out}"
    );
    assert!(
        out.contains("accumulator loop"),
        "the transformed gloss: {out}"
    );
    let _ = std::fs::remove_dir_all(std::path::Path::new(&file).parent().unwrap());
}

#[test]
fn an_unknown_name_reports_no_such_definition_on_stderr_and_fails() {
    // An unknown name: the query runs (total) but the name resolves to NOTHING → a `no such definition`
    // note on stderr, empty stdout, and a NON-ZERO exit — consistent with `cdz type`/`cdz doc`, so a
    // script can tell a typo from a real result rather than reading a success exit on "no such definition".
    let file = temp_src("ghost", GENERIC);
    let (ok, out, err) = run(&["instantiations", "ghost", &file]);
    assert!(!ok, "an unknown name now exits NON-ZERO: {out}");
    assert!(out.trim().is_empty(), "stdout is empty: {out}");
    assert!(
        err.contains("no such definition `ghost`"),
        "the not-found note is on stderr: {err}"
    );
    let _ = std::fs::remove_dir_all(std::path::Path::new(&file).parent().unwrap());
}
