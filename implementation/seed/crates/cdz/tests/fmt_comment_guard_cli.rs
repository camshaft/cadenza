//! End-to-end tests for the COMMENT-SAFETY GUARD in `cdz fmt` / `cdz normalize`'s in-place write path:
//! the tool REFUSES to overwrite a file when the reprint would DROP a `///` doc or `//` comment (the
//! signature of a reader comment/doc-attachment gap), turning silent comment-loss into a visible
//! fail-safe no-op. A trailing inline `//` the reader currently loses is the motivating case — it never
//! becomes an arena node, so only a raw-text count (what the guard uses) can catch it.

use std::process::Command;

/// Run `cdz <args…>`, returning (exit_ok, stdout, stderr).
fn run(args: &[&str]) -> (bool, String, String) {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe).args(args).output().expect("spawn cdz");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Write `src` to a unique temp `.cdz`; return (dir, path).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("cdz-guard-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.cdz");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string())
}

#[test]
fn fmt_refuses_to_write_when_it_would_drop_a_trailing_comment() {
    // A trailing inline `//` the reader drops (a known attachment gap): fmt must NOT overwrite the file
    // — it refuses, leaves the file byte-identical, and exits non-zero.
    //
    // The motivating input is a SAME-LINE trailing `//` on the LAST CALL ARGUMENT (`fuse(1, 2 // the
    // bar)`): the reader has no slot for a comment that trails the last arg before the `)` (it would sit
    // in the `)` leading slot, which `arg_exprs` does not drain), so a reprint LOSES it. (The earlier
    // input `fuse(1, // the bar\n 2)` — a comment on arg 2's OWN line — is now PRESERVED by `arg_exprs`'s
    // leading-comment capture, so it no longer exercises the drop path; this last-arg trailing case still
    // genuinely drops until the call printer grows same-line trailing-comment support.)
    let src = "def f() -> Int64 =\n  fuse(1, 2 // the bar\n  )\n";
    let (dir, path) = temp_src("fmt-refuse", src);
    let (ok, _out, err) = run(&["fmt", &path]);
    assert!(!ok, "must exit non-zero when refusing; stderr={err}");
    assert!(
        err.contains("refusing to format") && err.contains("comment"),
        "clear refusal message; got: {err}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        after, src,
        "the file must be left UNCHANGED (not clobbered)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_still_formats_a_file_that_preserves_all_comments() {
    // The guard must NOT false-trip: a file whose reprint keeps every `///`/`//` formats normally.
    let src = "/// doc\ndef f() -> Int64 =\n  // body note\n  1\n";
    let (dir, path) = temp_src("fmt-ok", src);
    let (ok, _out, err) = run(&["fmt", &path]);
    assert!(ok, "a comment-preserving fmt must succeed; stderr={err}");
    let after = std::fs::read_to_string(&path).unwrap();
    // Both comment markers survive.
    assert!(after.contains("///"), "doc preserved: {after}");
    assert!(
        after
            .lines()
            .any(|l| l.trim_start().starts_with("//") && !l.trim_start().starts_with("///")),
        "body comment preserved: {after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_does_not_count_a_slash_slash_inside_a_string() {
    // A `//` inside a string literal (a URL) is NOT a comment — the guard must not miscount it and
    // false-refuse a legitimate format.
    let src = "def url() -> String = \"http://example.com\"\n";
    let (dir, path) = temp_src("fmt-str", src);
    let (ok, _out, err) = run(&["fmt", &path]);
    assert!(
        ok,
        "a `//` inside a string must not trip the guard; stderr={err}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("http://example.com"),
        "the URL survives: {after}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn normalize_preserves_a_trailing_comment_and_writes() {
    // `normalize --match-to-let` on a file with a trailing comment: the comment reattaches (count
    // unchanged), so the guard allows the write and the match lowers.
    let src = "def f(p) = match p with | (a, b) => a + b // note\n";
    let (dir, path) = temp_src("norm-ok", src);
    let (ok, _out, err) = run(&["normalize", "--match-to-let", &path]);
    assert!(
        ok,
        "normalize must succeed when no comment is dropped; stderr={err}"
    );
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("let (a, b) = p in"),
        "the match lowered: {after}"
    );
    assert!(after.contains("// note"), "the comment survived: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fmt_refuses_to_write_when_it_would_drop_a_sexpr_semicolon_comment() {
    // The s-expr surface uses `;` line comments; the reader currently drops them (they never become a
    // node) so a reprint LOSES them. The guard must count `;` on a `.sexp` file (not the ML `//`) and
    // REFUSE the lossy write — fail-safe, file left byte-identical. (v-lsp/v-cdz-tooling report.)
    let dir = std::env::temp_dir().join(format!("cdz-guard-sexpr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("m.sexp");
    let src = "(module m\n  ; keep me\n  (def (add a b) (+ a b)) (export add))\n";
    std::fs::write(&path, src).unwrap();
    let p = path.to_str().unwrap();

    let exe = env!("CARGO_BIN_EXE_cdz");
    let out = Command::new(exe)
        .args(["fmt", p])
        .output()
        .expect("spawn cdz");
    assert!(
        !out.status.success(),
        "must refuse (exit non-zero) on a dropped `;` comment"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("refusing to format") && err.contains("`;`"),
        "clear s-expr refusal naming `;`; got: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        src,
        "the .sexp file must be left UNCHANGED (not clobbered)"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
