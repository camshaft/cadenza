//! End-to-end test for `cdz lsp` — the stdio Language Server FOLDED into the unified `cdz` binary.
//!
//! Unit tests in `src/lsp.rs` cover the analysis functions (hover/completion/… over a text buffer);
//! THIS test drives the whole SERVER: it spawns the real `cdz lsp` process, speaks framed JSON-RPC
//! over its stdin/stdout, and asserts the initialize handshake, a diagnostics push, each request
//! capability, and a clean shutdown. It is the gate that a protocol-level regression (framing, message
//! dispatch, the shutdown/exit sequence, a capability that stops being advertised) turns red — the
//! layer the pure-function unit tests can't reach.
//!
//! One session drives everything (spawning the process is the expensive part) and every capability is
//! asserted within it.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Frame one JSON-RPC message with the LSP `Content-Length` header.
fn frame(v: &serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).expect("serialize");
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    out
}

/// Parse every framed JSON-RPC message out of a raw stdout buffer (headers + bodies concatenated).
fn parse_frames(mut data: &[u8]) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    while let Some(hdr_end) = find(data, b"\r\n\r\n") {
        let header = std::str::from_utf8(&data[..hdr_end]).unwrap_or("");
        let len: usize = header
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .and_then(|n| n.trim().parse().ok())
            .expect("a Content-Length header");
        let body_start = hdr_end + 4;
        let body_end = body_start + len;
        if body_end > data.len() {
            break; // truncated (shouldn't happen once the process has exited)
        }
        let body = &data[body_start..body_end];
        // A body that was FRAMED (its Content-Length is present and the bytes are all here) but does not
        // parse as JSON is a PROTOCOL VIOLATION — fail hard rather than skip it. Silently dropping an
        // unparseable-but-framed message would let a real regression (the server emitting invalid JSON
        // for some message) pass this end-to-end test green; the whole point of the gate is to catch it.
        let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_else(|e| {
            panic!(
                "server emitted a framed message whose body is not valid JSON ({e}): {:?}",
                String::from_utf8_lossy(body)
            )
        });
        out.push(v);
        data = &data[body_end..];
    }
    out
}

/// The index of `needle` in `hay`, if present.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Drive a full `cdz lsp` session over the given program text and return every message the server
/// emitted. Sends initialize → initialized → didOpen → the six feature requests below (hover,
/// definition, completion, documentSymbol, references, semanticTokens) → shutdown → exit, then closes
/// stdin and reads stdout to EOF (a clean server exit).
fn drive_session(program: &str) -> Vec<serde_json::Value> {
    let uri = "file:///t.cdz";
    let msgs = vec![
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":program}}}),
        // hover on the `helper` def name (line 0, char 4).
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":uri},"position":{"line":0,"character":4}}}),
        // definition on the `helper` call in main (line 1, char 11 — "def main = " is 11 chars).
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"textDocument/definition","params":{"textDocument":{"uri":uri},"position":{"line":1,"character":11}}}),
        // completion in main's body (line 1, char 11).
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"textDocument/completion","params":{"textDocument":{"uri":uri},"position":{"line":1,"character":11}}}),
        // the document outline.
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":uri}}}),
        // find-references on the `helper` call in main (line 1, char 11).
        serde_json::json!({"jsonrpc":"2.0","id":6,"method":"textDocument/references","params":{"textDocument":{"uri":uri},"position":{"line":1,"character":11},"context":{"includeDeclaration":false}}}),
        // semantic tokens for the whole document.
        serde_json::json!({"jsonrpc":"2.0","id":7,"method":"textDocument/semanticTokens/full","params":{"textDocument":{"uri":uri}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];
    drive_messages(&msgs)
}

/// Spawn `cdz lsp`, send each framed message on stdin, then close stdin (EOF) so the server's reader
/// thread ends and the process exits cleanly; return every framed message the server emitted. The
/// generic driver `drive_session` and the lifecycle tests build on — the caller supplies the exact
/// message sequence (which MUST end with shutdown+exit for a clean exit).
fn drive_messages(msgs: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cdz lsp");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for m in msgs {
            stdin.write_all(&frame(m)).expect("write");
        }
        stdin.flush().expect("flush");
    }
    // Drop stdin (EOF) so the reader thread inside the server ends and the process exits cleanly.
    drop(child.stdin.take());

    let mut buf = Vec::new();
    child
        .stdout
        .as_mut()
        .expect("stdout")
        .read_to_end(&mut buf)
        .expect("read stdout");
    let status = child.wait().expect("wait");
    assert!(
        status.success(),
        "cdz lsp should exit cleanly, got {status:?}"
    );

    parse_frames(&buf)
}

/// The response object with the given request id, if any.
fn response(msgs: &[serde_json::Value], id: i64) -> Option<&serde_json::Value> {
    msgs.iter()
        .find(|m| m.get("id").and_then(|v| v.as_i64()) == Some(id))
}

#[test]
fn lsp_session_handshake_diagnostics_and_every_capability() {
    // `helper` is a top-level function used by `main`; a program that is well-formed so the queries
    // have something to answer.
    let program = "def helper(x: Int64) -> Int64 = x + x\ndef main = helper(1)";
    let msgs = drive_session(program);

    // 1. INITIALIZE — capabilities advertised + serverInfo carried (the PR#391 regression guard).
    let init = response(&msgs, 1).expect("an initialize response");
    let caps = init
        .pointer("/result/capabilities")
        .expect("capabilities in the initialize result");
    for cap in [
        "hoverProvider",
        "definitionProvider",
        "referencesProvider",
        "completionProvider",
        "documentSymbolProvider",
        "semanticTokensProvider",
        "codeActionProvider",
        "codeLensProvider",
    ] {
        assert!(
            caps.get(cap).is_some(),
            "capability `{cap}` must be advertised: {caps}"
        );
    }
    assert_eq!(
        init.pointer("/result/serverInfo/name")
            .and_then(|v| v.as_str()),
        Some("cdz-lsp"),
        "the initialize result must carry serverInfo.name"
    );

    // 2. DIAGNOSTICS — a well-formed program publishes an (empty) diagnostics notification for its URI.
    let diag_note = msgs.iter().find(|m| {
        m.get("method").and_then(|v| v.as_str()) == Some("textDocument/publishDiagnostics")
    });
    let diag_note = diag_note.expect("a publishDiagnostics notification");
    assert_eq!(
        diag_note.pointer("/params/uri").and_then(|v| v.as_str()),
        Some("file:///t.cdz")
    );

    // 3. HOVER — at (0,4) the cursor is on the `helper` def name; hover shows its signature.
    let hover = response(&msgs, 2).expect("a hover response");
    let contents = hover
        .pointer("/result/contents")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        contents.contains("helper") && contents.contains("->"),
        "hover should show the function's signature, got {contents:?}"
    );

    // 4. DEFINITION — go-to from the `helper` use in main lands on line 0 (the definition).
    let def = response(&msgs, 3).expect("a definition response");
    assert_eq!(
        def.pointer("/result/range/start/line")
            .and_then(|v| v.as_i64()),
        Some(0),
        "definition should point at the top-level `helper` on line 0: {def}"
    );

    // 5. COMPLETION — the candidate list includes the top-level `helper` and `main`.
    let comp = response(&msgs, 4).expect("a completion response");
    let labels: Vec<&str> = comp
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|it| it.get("label").and_then(|l| l.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        labels.contains(&"helper"),
        "completion should offer `helper`: {labels:?}"
    );
    assert!(
        labels.contains(&"main"),
        "completion should offer `main`: {labels:?}"
    );

    // 6. DOCUMENT SYMBOL — the outline lists both top-level defs.
    let syms = response(&msgs, 5).expect("a documentSymbol response");
    let names: Vec<&str> = syms
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|it| it.get("name").and_then(|n| n.as_str()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        names.contains(&"helper"),
        "outline should list `helper`: {names:?}"
    );
    assert!(
        names.contains(&"main"),
        "outline should list `main`: {names:?}"
    );

    // 7. REFERENCES — find-references on the `helper` call in main returns at least that use (line 1).
    let refs = response(&msgs, 6).expect("a references response");
    let ref_lines: Vec<i64> = refs
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|loc| loc.pointer("/range/start/line").and_then(|l| l.as_i64()))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        ref_lines.contains(&1),
        "references should include the `helper` use on line 1: {ref_lines:?}"
    );

    // 8. SEMANTIC TOKENS — a non-empty delta-encoded token stream (length a multiple of 5, the LSP
    //    5-tuple per token). Any regression that stops emitting tokens or breaks the encoding fails here.
    let toks = response(&msgs, 7).expect("a semanticTokens response");
    let data = toks
        .pointer("/result/data")
        .and_then(|v| v.as_array())
        .expect("a semanticTokens data array");
    assert!(!data.is_empty(), "semantic tokens should be non-empty");
    assert_eq!(
        data.len() % 5,
        0,
        "semantic-token data must be a multiple of 5 (the LSP per-token 5-tuple): {}",
        data.len()
    );
}

#[test]
fn lsp_session_publishes_diagnostics_for_a_broken_program() {
    // A program with an unbound name must publish a non-empty diagnostics set with the offending range.
    let program = "def double(x: Int64) -> Int64 = x + mystery";
    let msgs = drive_session(program);
    let diag_note = msgs
        .iter()
        .find(|m| {
            m.get("method").and_then(|v| v.as_str()) == Some("textDocument/publishDiagnostics")
        })
        .expect("a publishDiagnostics notification");
    let diags = diag_note
        .pointer("/params/diagnostics")
        .and_then(|v| v.as_array())
        .expect("a diagnostics array");
    assert!(
        diags.iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("unbound name") && m.contains("mystery"))),
        "expected an unbound-name diagnostic, got {diags:?}"
    );
}

/// Every `publishDiagnostics` notification's diagnostics array, in emission order.
fn diagnostic_pushes(msgs: &[serde_json::Value]) -> Vec<&Vec<serde_json::Value>> {
    msgs.iter()
        .filter(|m| {
            m.get("method").and_then(|v| v.as_str()) == Some("textDocument/publishDiagnostics")
        })
        .filter_map(|m| m.pointer("/params/diagnostics").and_then(|v| v.as_array()))
        .collect()
}

/// Every `publishDiagnostics` push as `(uri, diagnostics)` — for a multi-document session where the
/// pushes must be told apart by their target file.
fn diagnostic_pushes_by_uri(msgs: &[serde_json::Value]) -> Vec<(String, Vec<serde_json::Value>)> {
    msgs.iter()
        .filter(|m| {
            m.get("method").and_then(|v| v.as_str()) == Some("textDocument/publishDiagnostics")
        })
        .filter_map(|m| {
            let uri = m
                .pointer("/params/uri")
                .and_then(|v| v.as_str())?
                .to_string();
            let diags = m
                .pointer("/params/diagnostics")
                .and_then(|v| v.as_array())?
                .clone();
            Some((uri, diags))
        })
        .collect()
}

#[test]
fn lsp_didchange_re_lints_and_didclose_clears() {
    // The core "diagnostics as you type" loop: open a BROKEN buffer (unbound name) → a non-empty
    // diagnostics push; didChange to a CLEAN buffer → a fresh EMPTY push (the fix cleared the squiggle);
    // didClose → an empty push (stale diagnostics cleared for a closed file). Each edit must trigger its
    // own publish, in order — the protocol path the single-didOpen tests don't exercise.
    let uri = "file:///live.cdz";
    let broken = "def double(x: Int64) -> Int64 = x + mystery";
    let fixed = "def double(x: Int64) -> Int64 = x + x";
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":broken}}}),
        // Edit the buffer to the fixed text (FULL sync — the whole new text).
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":fixed}]}}),
        // Close the document.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    // Three pushes: open (broken → non-empty), change (fixed → empty), close (→ empty).
    assert_eq!(
        pushes.len(),
        3,
        "expected 3 diagnostics pushes (open/change/close), got {}",
        pushes.len()
    );
    assert!(
        pushes[0].iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("mystery"))),
        "the open push should carry the unbound-name diagnostic: {:?}",
        pushes[0]
    );
    assert!(
        pushes[1].is_empty(),
        "the didChange to fixed text should clear the diagnostic: {:?}",
        pushes[1]
    );
    assert!(
        pushes[2].is_empty(),
        "the didClose should clear diagnostics: {:?}",
        pushes[2]
    );
}

#[test]
fn lsp_follows_the_import_closure_for_a_multi_file_package() {
    // A multi-file package: `lib.sexp` exports `helper`, `main.sexp` imports + uses it. Opening the
    // IMPORTER must NOT report `helper` as an unknown import (CDZ0201) — the server follows the
    // `(import …)` closure (reading the sibling from disk) so the cross-file name resolves. Before this
    // increment the single-buffer analysis reported a false "unknown package file lib".
    let dir = std::env::temp_dir().join(format!("cdz-lsp-pkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");

    // The document URI is the on-disk main.sexp (so the server can find its import closure).
    let uri = format!("file://{}", main_path.display());
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the opened importer");
    assert!(
        opened.is_empty(),
        "the importer should have NO diagnostics — helper resolves across files (no false CDZ0201): {opened:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_untitled_buffer_with_imports_degrades_to_single_buffer_not_a_package_load() {
    // A non-`file://` URI (an UNTITLED, never-saved buffer) that DECLARES an import has no on-disk path
    // to resolve sibling files against, so the server must NOT attempt package analysis — it degrades to
    // the single-buffer path (where the import is simply unfollowed, so the imported name reads as
    // unbound). This must be TOTAL: a diagnostics push, no crash, no hang. Pins the `uri_to_path(uri)`
    // guard on all the package paths (a `None` path → single-buffer).
    let uri = "untitled:Untitled-1";
    let text = "import { helper } from \"lib\"\ndef main() -> Int64 = helper(1)";
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the opened untitled buffer (server must not crash/hang)");
    // Single-buffer analysis leaves the imported name unbound (the import is not followed for a pathless
    // buffer) — so there IS a diagnostic, and the server stayed total.
    assert!(
        opened.iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("helper") || m.contains("import"))),
        "an untitled buffer's unfollowed import should surface a single-buffer diagnostic: {opened:?}"
    );
}

#[test]
fn lsp_code_lens_lists_a_specialized_generics_instances() {
    // End-to-end: textDocument/codeLens over a program with a recursive generic (`loopn`, specialized at
    // Int64 and String) returns one lens whose title names both monomorphizations — the Instantiations
    // query surfaced as an editor CodeLens (a fact no other tool shows).
    let uri = "file:///t.sexp";
    let program = "(do (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
                   (def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\")))))";
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":program}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/codeLens","params":{"textDocument":{"uri":uri}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let resp = response(&msgs, 10).expect("a codeLens response");
    let lenses = resp
        .pointer("/result")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        lenses.len(),
        1,
        "one lens (on the specialized `loopn`): {lenses:?}"
    );
    let title = lenses[0]
        .pointer("/command/title")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        title.contains("x: Int64") && title.contains("x: String"),
        "the lens title should name both monomorphizations, got {title:?}"
    );
    // The command id must be non-empty (LSP requirement; some clients drop an empty-id lens).
    assert_eq!(
        lenses[0]
            .pointer("/command/command")
            .and_then(|v| v.as_str()),
        Some("cadenza.showInstantiations"),
        "the lens must carry a non-empty command id: {:?}",
        lenses[0].pointer("/command")
    );
}

#[test]
fn lsp_package_analysis_uses_the_open_buffer_overlay_not_the_stale_disk_lib() {
    // The open-buffer OVERLAY: an imported library that is ITSELF OPEN contributes its LIVE (unsaved)
    // text to the importer's cross-file analysis, not its stale on-disk version. On disk, lib does NOT
    // export `helper` (so a disk-only read would fault the importer with a CDZ0201 "does not export").
    // We open BOTH the lib buffer (WITH the export) and the importer; the importer must be CLEAN —
    // proving `open_resolver` fed the lib's live buffer text into the closure, overriding disk.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-overlay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // On DISK: helper is defined but NOT exported (a disk read → importer faults).
    let lib_path = dir.join("lib.sexp");
    std::fs::write(&lib_path, "(module lib (def (helper x) (+ x 1)))").expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let lib_uri = format!("file://{}", lib_path.display());
    let main_uri = format!("file://{}", main_path.display());
    // The OPEN lib buffer DOES export helper (the unsaved edit not yet on disk).
    let lib_open_text = "(module lib (def (helper x) (+ x 1)) (export helper))";

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        // Open the LIB buffer first (its live text has the export), then the importer.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":lib_uri,"languageId":"cadenza","version":1,"text":lib_open_text}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes_by_uri(&msgs);
    // The importer's push (main.sexp) must be empty — the live lib overlay resolved the import.
    let main_push = pushes
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("main.sexp"))
        .map(|(_, d)| d.clone())
        .expect("a diagnostics push for the opened importer");
    assert!(
        main_push.is_empty(),
        "the importer should be clean — the OPEN lib buffer (with the export) overrides the stale disk \
         lib (without it); got {main_push:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_editing_an_open_library_re_lints_its_open_importers() {
    // Reverse-dependency invalidation: lib exports `helper` (importer clean); a didChange to the OPEN lib
    // that DROPS the export must re-lint the importer LIVE — a fresh diagnostics push for main.sexp
    // showing helper is no longer available. Before this, didChange re-linted only the edited doc, so an
    // importer kept a stale-clean squiggle until it was itself touched.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-revdep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let lib_path = dir.join("lib.sexp");
    std::fs::write(
        &lib_path,
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let lib_uri = format!("file://{}", lib_path.display());
    let main_uri = format!("file://{}", main_path.display());
    let lib_no_export = "(module lib (def (helper x) (+ x 1)))"; // drops (export helper)

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":lib_uri,"languageId":"cadenza","version":1,"text":"(module lib (def (helper x) (+ x 1)) (export helper))"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        // Edit the OPEN lib to drop the export — the importer must be re-linted.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":lib_uri,"version":2},"contentChanges":[{"text":lib_no_export}]}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes_by_uri(&msgs);
    // The LAST push for main.sexp (after the lib edit) must be NON-empty — the importer saw the dropped
    // export. (The first main push, right after its didOpen, was clean.)
    let last_main = pushes
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("main.sexp"))
        .map(|(_, d)| d.clone())
        .expect("a diagnostics push for the importer");
    assert!(
        !last_main.is_empty(),
        "editing the open lib to drop the export must re-lint the importer (reverse-dep invalidation); \
         got {last_main:?}"
    );
    // And there must be MORE than one main.sexp push (open + the re-lint triggered by the lib edit).
    let main_pushes = pushes
        .iter()
        .filter(|(u, _)| u.ends_with("main.sexp"))
        .count();
    assert!(
        main_pushes >= 2,
        "the importer should be re-published after the lib edit (>=2 main pushes), got {main_pushes}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_editing_a_transitive_dependency_re_lints_the_indirect_importer() {
    // Reverse-dep invalidation is TRANSITIVE: main imports mid imports leaf. Editing the OPEN leaf to
    // drop its export must re-lint `main` (an INDIRECT importer, main→mid→leaf), not just mid — because
    // `republish_importers_of` checks each open doc's full transitive closure, not only direct imports.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-transdep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("leaf.sexp"),
        "(module leaf (def (base x) (+ x 1)) (export base))",
    )
    .expect("leaf");
    std::fs::write(
        dir.join("mid.sexp"),
        "(do (import \"leaf\" (base)) (def (mid y) (base y)) (export mid))",
    )
    .expect("mid");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"mid\" (mid)) (def (main) (mid 1)) (export main))";
    std::fs::write(&main_path, main_text).expect("main");
    let leaf_uri = format!("file://{}", dir.join("leaf.sexp").display());
    let mid_uri = format!("file://{}", dir.join("mid.sexp").display());
    let main_uri = format!("file://{}", main_path.display());
    let mid_text = "(do (import \"leaf\" (base)) (def (mid y) (base y)) (export mid))";

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":leaf_uri,"languageId":"cadenza","version":1,"text":"(module leaf (def (base x) (+ x 1)) (export base))"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":mid_uri,"languageId":"cadenza","version":1,"text":mid_text}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        // Edit the OPEN leaf to drop its export — main (transitive importer) must be re-linted.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":leaf_uri,"version":2},"contentChanges":[{"text":"(module leaf (def (base x) (+ x 1)))"}]}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes_by_uri(&msgs);
    let main_pushes = pushes
        .iter()
        .filter(|(u, _)| u.ends_with("main.sexp"))
        .count();
    assert!(
        main_pushes >= 2,
        "the INDIRECT importer `main` should be re-published after the transitive `leaf` edit \
         (>=2 main pushes), got {main_pushes}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_closing_an_open_library_reverts_its_importers_to_the_on_disk_version() {
    // Reverse-dep invalidation on CLOSE (the symmetric counterpart to the didChange revdep test): the
    // on-disk lib exports `helper` (importer clean). Open both, then didChange the OPEN lib to DROP the
    // export — the importer goes red against the live overlay. Now didClose the lib: the overlay is gone,
    // so the importer must revert to the ON-DISK lib (which still exports `helper`) and be re-linted CLEAN.
    // This pins the lsp.rs didClose→republish_importers_of path (closing a library re-lints its importers
    // against the on-disk version), which the buffer-drop/clear test does not exercise.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-closerevdep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let lib_path = dir.join("lib.sexp");
    // On disk: exports helper (so an importer is clean against the on-disk version).
    std::fs::write(
        &lib_path,
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let lib_uri = format!("file://{}", lib_path.display());
    let main_uri = format!("file://{}", main_path.display());
    let lib_no_export = "(module lib (def (helper x) (+ x 1)))"; // live overlay drops (export helper)

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":lib_uri,"languageId":"cadenza","version":1,"text":"(module lib (def (helper x) (+ x 1)) (export helper))"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        // Live-edit the OPEN lib to drop the export — the importer goes red against the overlay.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":lib_uri,"version":2},"contentChanges":[{"text":lib_no_export}]}}),
        // Close the lib — the overlay is gone; the importer must revert to the on-disk (exporting) version.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":lib_uri}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes_by_uri(&msgs);
    // The LAST push for main.sexp (after the lib CLOSE) must be EMPTY — the importer reverted to the
    // on-disk lib, which still exports `helper`, so the earlier dropped-export error is cleared.
    let last_main = pushes
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("main.sexp"))
        .map(|(_, d)| d.clone())
        .expect("a diagnostics push for the importer");
    assert!(
        last_main.is_empty(),
        "closing the open lib must revert the importer to the on-disk (exporting) version and re-lint it \
         clean; got {last_main:?}"
    );
    // And there must be MORE than one main.sexp push (open + at least the re-lint triggered by the close),
    // proving the close actually re-published the importer rather than leaving a stale red squiggle.
    let main_pushes = pushes
        .iter()
        .filter(|(u, _)| u.ends_with("main.sexp"))
        .count();
    assert!(
        main_pushes >= 2,
        "the importer should be re-published after the lib close (>=2 main pushes), got {main_pushes}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_opening_a_library_re_lints_an_already_open_importer() {
    // Third leg of the reverse-dep family (didChange done, didClose done): the didOpen path. The importer is
    // opened FIRST while the on-disk lib does NOT export `helper` — so it is RED (CDZ0201 does-not-export).
    // Then we open the LIB buffer WITH the export: opening a library must refresh the diagnostics of every
    // already-open importer (its live buffer now overlays the on-disk version), re-linting the importer
    // CLEAN. This pins the lsp.rs didOpen->republish_importers_of path; the overlay test opens the lib
    // FIRST (so the importer's own open reads the overlay) and does not exercise the open-order where the
    // importer was already red and must be re-published by the LIB's open.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-openrevdep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // On DISK: helper is defined but NOT exported — an importer opened against disk faults.
    let lib_path = dir.join("lib.sexp");
    std::fs::write(&lib_path, "(module lib (def (helper x) (+ x 1)))").expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let lib_uri = format!("file://{}", lib_path.display());
    let main_uri = format!("file://{}", main_path.display());
    // The OPEN lib buffer DOES export helper (the unsaved edit not yet on disk).
    let lib_open_text = "(module lib (def (helper x) (+ x 1)) (export helper))";

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        // Open the IMPORTER first — against the on-disk lib (no export) it is RED.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        // Now open the LIB buffer WITH the export — the already-open importer must be re-linted CLEAN.
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":lib_uri,"languageId":"cadenza","version":1,"text":lib_open_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes_by_uri(&msgs);
    // The LAST push for main.sexp (after the lib open) must be EMPTY — the live lib overlay resolved the
    // import; the importer's earlier red (from the disk-only read) is cleared.
    let last_main = pushes
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("main.sexp"))
        .map(|(_, d)| d.clone())
        .expect("a diagnostics push for the importer");
    assert!(
        last_main.is_empty(),
        "opening the lib (with the export) must re-lint the already-open importer clean; got {last_main:?}"
    );
    // And there must be ≥2 main pushes (its own open → red; the lib open → re-lint clean), proving the lib
    // open actually re-published the importer rather than leaving a stale red squiggle.
    let main_pushes = pushes
        .iter()
        .filter(|(u, _)| u.ends_with("main.sexp"))
        .count();
    assert!(
        main_pushes >= 2,
        "the importer should be re-published after the lib open (>=2 main pushes), got {main_pushes}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_goto_definition_jumps_across_files_to_an_imported_def() {
    // Cross-file go-to-definition: from the `helper` USE in main.sexp, jump to its DEFINITION in
    // lib.sexp — the target Location is in the OTHER file. Before this increment definition was
    // single-buffer and returned null for an imported name.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xdef-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    // The `helper` USE in main is the second occurrence (the first is inside the import clause).
    let use_char = main_text
        .match_indices("helper")
        .nth(1)
        .map(|(i, _)| i)
        .expect("a helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/definition","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":use_char}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let def = response(&msgs, 10).expect("a definition response");
    let target_uri = def
        .pointer("/result/uri")
        .and_then(|v| v.as_str())
        .expect("a definition Location with a uri");
    assert!(
        target_uri.ends_with("lib.sexp"),
        "cross-file definition should jump into lib.sexp, got {target_uri}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_hover_types_an_imported_name_across_files() {
    // Cross-file hover: hovering a use of an IMPORTED name shows its type (resolved from the other
    // file), not "unknown"/nothing. `helper` is defined in lib.sexp; hovering its use in main.sexp
    // yields its function arrow type.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xhover-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    let use_char = main_text
        .match_indices("helper")
        .nth(1)
        .map(|(i, _)| i)
        .expect("a helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/hover","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":use_char}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let hover = response(&msgs, 10).expect("a hover response");
    let contents = hover
        .pointer("/result/contents")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        contents.contains("->"),
        "hovering an imported function should show its arrow type, got {contents:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_hover_shows_the_docstring_of_an_imported_documented_def() {
    // Cross-file hover carries the DOCSTRING too: lib documents `helper` with a `///` doc; hovering its
    // use in main shows that doc (as Markdown, alongside the type) — the DocAt query over the closure.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xhoverdoc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.ml"),
        "/// Adds one to its argument.\ndef helper(x: Int64) -> Int64 = x + 1\nexport { helper }",
    )
    .expect("write lib");
    let main_path = dir.join("main.ml");
    let main_text = "import { helper } from \"lib\"\ndef main() -> Int64 = helper(41)";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    // Cursor on the `helper` USE in main's body (line 1) — the last occurrence.
    let (line, ch) = {
        let last = main_text.lines().last().unwrap();
        (1i64, last.find("helper").expect("a helper use") as i64)
    };

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/hover","params":{"textDocument":{"uri":main_uri},"position":{"line":line,"character":ch}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let hover = response(&msgs, 10).expect("a hover response");
    // A documented hover is Markup — read `/result/contents/value`.
    let value = hover
        .pointer("/result/contents/value")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        value.contains("Adds one to its argument."),
        "cross-file hover should carry the imported def's docstring, got {value:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_references_span_multiple_files_across_the_closure() {
    // Cross-file find-references: `helper` is defined + used in lib.sexp AND imported + used in
    // main.sexp. References on the use in main returns Locations in BOTH files (single-buffer would
    // only ever see main's).
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xref-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (def (twice y) (helper (helper y))) (export helper twice))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    let use_char = main_text
        .match_indices("helper")
        .nth(1)
        .map(|(i, _)| i)
        .expect("a helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/references","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":use_char},"context":{"includeDeclaration":false}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let refs = response(&msgs, 10).expect("a references response");
    let uris: Vec<String> = refs
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    l.pointer("/uri")
                        .and_then(|u| u.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        uris.iter().any(|u| u.ends_with("main.sexp")),
        "references should include a use in main.sexp: {uris:?}"
    );
    assert!(
        uris.iter().any(|u| u.ends_with("lib.sexp")),
        "references should span into lib.sexp (cross-file): {uris:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_completion_offers_an_imported_name_across_files() {
    // Cross-file completion: `helper` is exported by lib.sexp and imported into main.sexp. The
    // completion candidate set inside main must OFFER `helper` — an imported name appears in neither
    // single-buffer source (`Symbols` lists only main's own decls, `ScopeAt` walks only lexical binders),
    // so single-buffer completion would omit it. The item carries the library's kind + type.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xcompl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    // Cursor at the `helper` USE in main's body (the second occurrence).
    let use_char = main_text
        .match_indices("helper")
        .nth(1)
        .map(|(i, _)| i)
        .expect("a helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/completion","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":use_char}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let completion = response(&msgs, 10).expect("a completion response");
    let labels: Vec<String> = completion
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    i.pointer("/label")
                        .and_then(|l| l.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        labels.iter().any(|l| l == "helper"),
        "completion should offer the imported name `helper`: {labels:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_package_references_on_a_shadowing_local_does_not_leak_the_imported_symbols_uses() {
    // The PACKAGE-level shadowing guard: `helper` is imported from lib AND is the name of a PARAMETER of
    // `g` in main that shadows it. A references request on the LOCAL `helper` (the param use in g's body)
    // must NOT return the IMPORTED `helper`'s uses — `UsesOf` is name-keyed and can't distinguish them,
    // so the guard suppresses it (empty). Regression for the too-permissive `resolves_to.is_some()` guard
    // (a local binder also resolves — to itself — so the check must be "resolves to a Symbols node").
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xshadow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (use1) (helper 1)) (def (g helper) helper) (export use1))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    // The LOCAL `helper` use in g's body — the LAST occurrence of `helper` in the text.
    let local_use = main_text
        .match_indices("helper")
        .last()
        .map(|(i, _)| i)
        .expect("a local helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/references","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":local_use},"context":{"includeDeclaration":false}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let refs = response(&msgs, 10).expect("a references response");
    let locs = refs
        .pointer("/result")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        locs.is_empty(),
        "a shadowing LOCAL `helper` must not leak the imported symbol's refs, got {locs:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_package_references_include_declaration_points_at_the_imported_def() {
    // includeDeclaration cross-file: references on a use of an IMPORTED `helper` (in main), WITH
    // includeDeclaration, must add the declaration site — which lives in lib.sexp. Guards the refactor
    // that derives the declaration node from the PACKAGE `Symbols` answer (a GLOBAL id demuxed to lib),
    // not the entry-local `Symbols` (which would have no `helper` decl and thus miss it).
    let dir = std::env::temp_dir().join(format!("cdz-lsp-xrefdecl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("lib.sexp"),
        "(module lib (def (helper x) (+ x 1)) (export helper))",
    )
    .expect("write lib");
    let main_path = dir.join("main.sexp");
    let main_text = "(do (import \"lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&main_path, main_text).expect("write main");
    let main_uri = format!("file://{}", main_path.display());
    let use_char = main_text
        .match_indices("helper")
        .nth(1)
        .map(|(i, _)| i)
        .expect("a helper use") as i64;

    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":main_uri,"languageId":"cadenza","version":1,"text":main_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":10,"method":"textDocument/references","params":{"textDocument":{"uri":main_uri},"position":{"line":0,"character":use_char},"context":{"includeDeclaration":true}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let refs = response(&msgs, 10).expect("a references response");
    let uris: Vec<String> = refs
        .pointer("/result")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|l| l.pointer("/uri").and_then(|u| u.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        uris.iter().any(|u| u.ends_with("lib.sexp")),
        "includeDeclaration should add the imported def's site in lib.sexp: {uris:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_diagnostics_are_total_on_a_cyclic_import_pair() {
    // A user mid-refactor can create a temporary import CYCLE (a imports b, b imports a). The server's
    // package-diagnostics path drives `closure::load` on the opened file's closure — it must DETECT the
    // cycle and publish a clean diagnostic, NEVER hang or crash the editor (queries over incomplete/
    // malformed source stay TOTAL). Pins that the closure loader's cycle guard reaches the LSP surface.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-cyc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("b.sexp"),
        "(do (import \"a\" (aye)) (def (bee) 1) (export bee))",
    )
    .expect("write b");
    let a_path = dir.join("a.sexp");
    let a_text = "(do (import \"b\" (bee)) (def (aye) (bee)) (export aye))";
    std::fs::write(&a_path, a_text).expect("write a");

    let uri = format!("file://{}", a_path.display());
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":a_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    // The session must COMPLETE (no hang — the test harness would otherwise never return) and publish a
    // diagnostics push for the opened file. The cycle is a real fault, so the push is NON-empty and names
    // the cyclic-import error (CDZ0201), not a panic or silence.
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the opened file in the cycle");
    assert!(
        !opened.is_empty(),
        "a cyclic import is a fault — the server must report it, not silently pass: {opened:?}"
    );
    let has_cyclic = opened.iter().any(|d| {
        d.get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("cyclic"))
    });
    assert!(
        has_cyclic,
        "the diagnostic should name the cyclic import (CDZ0201 cyclic module imports): {opened:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_diagnostics_surface_a_self_import_as_a_cycle() {
    // A file that imports ITSELF is a degenerate 1-node cycle. Like the a↔b pair, `link()` rejects it
    // before the Diagnostics query runs (no KIND_DIAGNOSTICS artifact), so the server must surface the
    // link fault (`compiled.diagnostics`) rather than falling back to the misleading single-buffer pair.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-self-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("selfimp.sexp");
    let text = "(do (import \"selfimp\" (foo)) (def (foo) 1) (export foo))";
    std::fs::write(&path, text).expect("write");
    let uri = format!("file://{}", path.display());
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the self-importing file");
    assert!(
        opened.iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("cyclic"))),
        "a self-import should surface the cyclic-import diagnostic, not the single-buffer fallback: {opened:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_diagnostics_surface_a_colliding_import() {
    // Two libraries export the same name; importing both into one file collides. `link()` rejects it up
    // front (no Diagnostics artifact), so — like the cyclic case — the server must surface the link fault
    // (`dup` imported more than once), not the misleading single-buffer fallback.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-collide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("libx.sexp"),
        "(module libx (def (dup) 1) (export dup))",
    )
    .expect("write libx");
    std::fs::write(
        dir.join("liby.sexp"),
        "(module liby (def (dup) 2) (export dup))",
    )
    .expect("write liby");
    let path = dir.join("collide.sexp");
    let text =
        "(do (import \"libx\" (dup)) (import \"liby\" (dup)) (def (main) (dup)) (export main))";
    std::fs::write(&path, text).expect("write");
    let uri = format!("file://{}", path.display());
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the colliding-import file");
    assert!(
        opened.iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("imported more than once"))),
        "a colliding import should surface the collision diagnostic, not the single-buffer fallback: {opened:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lsp_diagnostics_surface_a_missing_imported_sibling() {
    // A buffer imports a sibling library that does NOT exist on disk — `link()` rejects it up front
    // (CDZ0201 "unknown package file"), before any Diagnostics artifact is produced, exactly like the
    // cyclic/self/colliding cases. The server must surface that link fault (matching `cdz check`), not
    // fall back to the misleading single-buffer diagnostics. Completes the up-front-link-rejection class:
    // cyclic + self + colliding + MISSING-sibling all reach the LSP surface faithfully.
    let dir = std::env::temp_dir().join(format!("cdz-lsp-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    // Only the importer exists — `nonexistent_lib.sexp` is deliberately never written.
    let path = dir.join("importer.sexp");
    let text = "(do (import \"nonexistent_lib\" (helper)) (def (main) (helper 41)) (export main))";
    std::fs::write(&path, text).expect("write");
    let uri = format!("file://{}", path.display());
    let msgs = drive_messages(&[
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"processId":null,"rootUri":null}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"cadenza","version":1,"text":text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]);
    let pushes = diagnostic_pushes(&msgs);
    let opened = pushes
        .last()
        .expect("a diagnostics push for the missing-sibling importer");
    assert!(
        opened.iter().any(|d| d
            .get("message")
            .and_then(|m| m.as_str())
            .is_some_and(|m| m.contains("unknown package file"))),
        "a missing imported sibling should surface the unknown-package-file link fault, not the single-buffer fallback: {opened:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
