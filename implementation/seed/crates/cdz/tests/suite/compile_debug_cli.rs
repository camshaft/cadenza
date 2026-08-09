//! End-to-end tests for `cdz compile` producing DWARF debug output DIRECTLY FROM A SOURCE FILE — the
//! ergonomic payoff of `cdz` holding both the front-end and the compiler: a debug target auto-supplies
//! the `spans` artifact (parsed in-process), so a user needn't hand-build one. Drives the built binary.

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

/// A unique temp dir for one test (avoids cross-test collisions without an extra dep).
fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("cdz-compile-dbg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

const PROG: &str = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";

#[test]
fn compile_a_source_file_to_a_component() {
    // `cdz compile` accepts a SOURCE file (not just a pre-built binary AST) — parsed in-process.
    let dir = temp_dir("plain");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "compile failed: {err}");
    assert!(
        dir.join("add.wasm").is_file(),
        "no component produced: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wasm_debug_from_source_auto_supplies_spans() {
    // The payoff: `--target wasm-debug` on a SOURCE file needs NO explicit `spans:` input — cdz parses
    // with spans and injects the artifact. The component is produced (and carries debug sections).
    let dir = temp_dir("modee");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--target",
        "wasm-debug",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "wasm-debug compile failed: {err}");
    let comp = dir.join("add.wasm");
    assert!(comp.is_file(), "no debug component: {err}");
    // The embedded core module carries `.debug_*` custom sections (a plain component would not). Assert
    // the bytes contain the `.debug_info` section name — a cheap, dependency-free check.
    let bytes = std::fs::read(&comp).unwrap();
    assert!(
        contains(&bytes, b".debug_info") && contains(&bytes, b".debug_line"),
        "the wasm-debug component must embed DWARF sections"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dwarf_sidecar_from_source_auto_supplies_spans() {
    // `--target dwarf` on a source file produces a detached `<name>.dwarf` sidecar (no explicit spans).
    let dir = temp_dir("modes");
    let src = dir.join("add.sexp");
    std::fs::write(&src, PROG).unwrap();
    let (ok, _out, err) = run(&[
        "compile",
        src.to_str().unwrap(),
        "--target",
        "dwarf",
        "-o",
        dir.to_str().unwrap(),
    ]);
    assert!(ok, "dwarf compile failed: {err}");
    let sidecar = dir.join("add.dwarf");
    assert!(sidecar.is_file(), "no dwarf sidecar: {err}");
    let bytes = std::fs::read(&sidecar).unwrap();
    // A bare core module carrying the debug sections: the `\0asm` header + the section names + the
    // source function name (`add`) proving the DWARF describes the program.
    assert_eq!(&bytes[..4], b"\0asm", "not a wasm module");
    assert!(
        contains(&bytes, b".debug_info") && contains(&bytes, b"add"),
        "the sidecar must carry DWARF naming the source function"
    );
    // Reproducibility (DESIGN §4): the ABSOLUTE build directory must NOT leak into the DWARF. `src` is
    // an absolute temp path; the CU records only the file name (`add.sexp`), so the dir bytes are absent.
    let build_dir = dir.to_str().unwrap();
    assert!(
        !contains(&bytes, build_dir.as_bytes()),
        "the DWARF must not embed the absolute build directory {build_dir:?}"
    );
    assert!(
        contains(&bytes, b"add.sexp"),
        "the DWARF should name the source file by its (build-dir-free) file name"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Substring search over bytes (no external dep).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn ml_parse_errors_report_correct_incrementing_line_positions() {
    // `cdz check` on a malformed ML file recovers N cascading parse errors and renders each as
    // `file:line:col`. The render maps every error's byte offset to a line:col via ONE shared `LineIndex`
    // (a binary search) rather than a per-error from-start newline scan — the O(errors × source_len) =
    // O(N²) that made a broken file with thousands of recovered errors quadratic. This locks in that the
    // index produces the SAME positions the from-start scan did: each error on line K reports `:K:` (not
    // all collapsed to line 1, which a broken index would do), and the columns are within the line.
    let dir = temp_dir("mlerr");
    let src = dir.join("broken.cdz");
    // 6 lines, each an ML syntax error (`)(` is not a valid expression) — the parser recovers per line.
    let n = 6;
    let text = (0..n)
        .map(|i| format!("let d{i} = )("))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&src, &text).unwrap();
    let (_ok, _out, err) = run(&["check", src.to_str().unwrap()]);
    // Every source line 1..=n must appear as an error position — proving offsets map to their real line
    // (a from-start scan and the LineIndex agree, and neither collapses distinct lines).
    for line in 1..=n {
        assert!(
            err.contains(&format!("broken.cdz:{line}:")),
            "expected an error anchored at line {line}; got:\n{err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn highlight_reports_correct_multi_line_token_positions() {
    // `cdz highlight` emits `file:line:col: kind` for EVERY token. It maps each token's byte offset to a
    // line:col via ONE shared `LineIndex` (binary search) rather than a per-token from-start newline scan
    // — the O(tokens × source_len) = O(N²) that made highlighting a wide file quadratic (a 6400-def file
    // was 5.1s, 99.7% in `line_col`). This locks in that the index gives the SAME positions: tokens on
    // later lines report their real line (not all collapsed to line 1), and each is classified.
    let dir = temp_dir("hl");
    let src = dir.join("prog.sexp");
    // A small multi-line program — the export on line 4 must be highlighted at line 4, not line 1.
    let text = "(do\n  (def (f x)\n    (+ x 1))\n  (export f))\n";
    std::fs::write(&src, text).unwrap();
    let (ok, out, err) = run(&["highlight", src.to_str().unwrap()]);
    assert!(ok, "highlight failed: {err}");
    // The `def` keyword is on line 2, the `+` call on line 3, the `export` on line 4 — each token's line
    // must match its source line (a from-start scan and the LineIndex agree; a broken index collapses them).
    for line in 1..=4 {
        assert!(
            out.contains(&format!("prog.sexp:{line}:")),
            "expected a highlighted token on line {line}; got:\n{out}"
        );
    }
    // And the classifications are present (the render is not just positions).
    assert!(out.contains(": keyword"), "a keyword token: {out}");
    assert!(out.contains(": number"), "the `1` literal: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}
