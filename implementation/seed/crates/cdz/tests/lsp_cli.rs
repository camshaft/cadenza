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
        if let Ok(v) = serde_json::from_slice(&data[body_start..body_end]) {
            out.push(v);
        }
        data = &data[body_end..];
    }
    out
}

/// The index of `needle` in `hay`, if present.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Drive a full `cdz lsp` session over the given program text and return every message the server
/// emitted. Sends initialize → initialized → didOpen → the four requests below → shutdown → exit, then
/// closes stdin and reads stdout to EOF (a clean server exit).
fn drive_session(program: &str) -> Vec<serde_json::Value> {
    let exe = env!("CARGO_BIN_EXE_cdz");
    let mut child = Command::new(exe)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cdz lsp");

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
        serde_json::json!({"jsonrpc":"2.0","id":99,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for m in &msgs {
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
