//! End-to-end tests for `cdz type-at FILE OFFSET` — "type at cursor" (the LSP hover analogue).
//!
//! `type-at` reports the inferred type of the node at a source BYTE OFFSET, rendered `<type> @
//! file:line:col-line:col` (the type + the node's source span). It's an in-process semantic query only
//! the unified `cdz` binary can answer (compiler inference + the front-end span table in one process).
//! The sidecar's TypeAt logic is unit-tested in rcdzc; what is pinned HERE is the CLI-RENDERING layer —
//! resolving the byte offset to a node, rendering its type + span, and the no-node / missing-file
//! contracts. Drives the built binary over a temp `.sexp` file.

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

/// Write `src` to a unique temp `.sexp` file; returns (dir, path, source).
fn temp_src(tag: &str, src: &str) -> (std::path::PathBuf, String, String) {
    let dir = std::env::temp_dir().join(format!("cdz-typeat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("m.sexp");
    std::fs::write(&path, src).unwrap();
    (dir, path.to_str().unwrap().to_string(), src.to_string())
}

const PROG: &str =
    "(module m (def (inc (: n Int64)) (+ n 1)) (def (main (: a Int64)) (inc a)) (export main))";

/// The byte offset of the first occurrence of `needle` in `src` (+ `plus`), as a decimal string for the
/// CLI arg — so a test names a cursor position by the source text it sits on, not a magic number.
fn offset_of(src: &str, needle: &str, plus: usize) -> String {
    (src.find(needle).expect("needle in source") + plus).to_string()
}

#[test]
fn type_at_a_call_reports_the_function_type_with_a_span() {
    let (dir, file, src) = temp_src("call", PROG);
    // Cursor on the `inc` call in `(inc a)` — its type is the function type.
    let off = offset_of(&src, "(inc a)", 1);
    let (ok, out, err) = run(&["type-at", &file, &off]);
    assert!(ok, "cdz type-at should succeed: {err}");
    assert!(
        out.contains("-> Int64 Int64") && out.contains(&format!("@ {file}:")),
        "a call renders `<fn type> @ file:line:col-…`: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_at_a_literal_reports_its_scalar_type() {
    let (dir, file, src) = temp_src("lit", PROG);
    // Cursor on the `1` literal in `(+ n 1)`.
    let off = offset_of(&src, "n 1)", 2);
    let (ok, out, err) = run(&["type-at", &file, &off]);
    assert!(ok, "type-at on a literal failed: {err}");
    assert!(
        out.trim_start().starts_with("Int64"),
        "the `1` literal is `Int64`: {out}"
    );
    assert!(
        out.contains(&format!("@ {file}:")),
        "carries a source span: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_at_an_offset_with_no_node_reports_no_node_and_fails() {
    // An out-of-range offset resolves to NO node — `type-at` needs a node to type, so this is a FAILURE
    // (unlike `uses`/`symbols`, whose empty result is a success): a `no node at byte offset` note on
    // stderr, non-zero exit. Pins that contract (a "hover" on empty space is an error, not an empty OK).
    let (dir, file, _src) = temp_src("nonode", PROG);
    let (ok, _out, err) = run(&["type-at", &file, "99999"]);
    assert!(!ok, "an offset with no node fails (nothing to type there)");
    assert!(
        err.contains("no node at byte offset"),
        "reports there is no node at the offset: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_at_an_offset_inside_a_multibyte_char_does_not_panic() {
    // An editor cursor byte offset can land INSIDE a multibyte UTF-8 character (a naive `&src[off..]` slice
    // would panic at a non-char boundary — a hard crash for an editor/AI caller passing a live cursor). The
    // offset resolver must be byte-boundary safe: resolve to the enclosing node (a clean result) or "no
    // node" — never panic. Pin it with a name holding a 2-byte `é`: put the cursor on the SECOND byte of the
    // `é` (its interior), which is not a char boundary.
    let src = "(module m (def (gr\u{e9}et (: n Int64)) (+ n 1)) (export gr\u{e9}et))";
    let (dir, file, _src) = temp_src("multibyte", src);
    // Byte offset of the `é` in the first `gréet`, +1 → the interior (second) byte of the 2-byte char.
    let e_byte = src.find('\u{e9}').expect("é in source");
    let off = (e_byte + 1).to_string();
    let (ok, out, err) = run(&["type-at", &file, &off]);
    // Assert the POSITIVE contract, not just the absence of a panic marker: `!err.contains("panicked")` is
    // ALSO satisfied by an EMPTY stderr, so a SILENT crash (SIGABRT / segfault — no "panicked" text) would
    // slip through. Require one of the two legitimate outcomes: (a) SUCCESS with a resolved node (stdout
    // carries the `@ <file>:` span the renderer always prints), or (b) a CLEAN non-zero "no node at byte
    // offset". A silent crash is NEITHER (a byte-boundary panic aborts before any output), so it now fails.
    let resolved_a_node = ok && out.contains(&format!("@ {file}:"));
    let clean_no_node = !ok && err.contains("no node at byte offset");
    assert!(
        resolved_a_node || clean_no_node,
        "a mid-multibyte offset must resolve cleanly (node or `no node`), never silently crash: \
         exit_ok={ok}\nstdout:\n{out}\nstderr:\n{err}"
    );
    // Belt-and-suspenders: also assert no explicit Rust panic marker (a caught panic that still exits).
    assert!(
        !err.contains("panicked") && !err.contains("RUST_BACKTRACE"),
        "no Rust panic marker on a mid-multibyte offset: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn type_at_on_a_missing_file_errors_with_the_cdz_prog_name() {
    let (ok, _out, err) = run(&["type-at", "/no/such/file.sexp", "5"]);
    assert!(!ok, "a missing file should fail");
    assert!(err.contains("cdz:"), "error names the tool: {err}");
}

#[test]
fn type_at_a_non_numeric_offset_names_what_a_byte_offset_is() {
    // A non-numeric OFFSET (a stale/mis-typed editor arg) must get an ACTIONABLE message naming what the
    // argument is — a 0-based byte offset — not clap's bare `invalid digit found in string`. These are
    // editor/script-facing queries; a caller that passes garbage should be told the expected shape. The
    // custom `value_parser` is shared by def/scope/type-at/doc-at (all take a byte offset).
    let (dir, file, _src) = temp_src("badoffset", PROG);
    let (ok, _out, err) = run(&["type-at", &file, "not-a-number"]);
    assert!(!ok, "a non-numeric offset is a usage error");
    assert!(
        err.contains("not a byte offset") && err.contains("0-based"),
        "the message names what a byte offset is, not a bare digit-parse error: {err}"
    );
    assert!(
        !err.contains("invalid digit found in string"),
        "must NOT leak clap's generic digit-parse blur: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
