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
