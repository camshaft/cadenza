//! `cdz lsp` — a synchronous stdio Language Server for Cadenza.
//!
//! This is the persistent editor face of the SAME in-process query engine the one-shot subcommands
//! (`cdz check`/`type-at`/`def`/…) drive: an editor launches `cdz lsp`, and the server holds each open
//! document in memory and re-runs the compiler's fact-column reads on every edit, publishing the
//! results back over the Language Server Protocol. It is not a second implementation of the language —
//! it wraps the ONE compiler behind LSP, exactly as `spec/capabilities/tooling-and-lsp.md` requires
//! ("the tooling an editor drives … is not a second implementation of the language but a view onto the
//! one compiler").
//!
//! Transport is rust-analyzer's own [`lsp_server`] over stdio — a synchronous JSON-RPC loop, no async
//! runtime. Each message is dispatched by method name; a document's diagnostics are recomputed and
//! published whenever it opens or changes (the "diagnostics as you type" primitive).
//!
//! Increment 1 (this module's current scope): the `initialize`/`shutdown` handshake, full-document sync
//! (`didOpen`/`didChange`/`didClose`), and `textDocument/publishDiagnostics`. Hover (`type-at`),
//! semantic tokens (`highlight`), and go-to (`def`) are the next increments — each a read of a column
//! the query engine already exposes, wired to its LSP request.

use std::collections::HashMap;

use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Request as _, Shutdown};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializeResult, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};

/// Run the stdio LSP server to completion: perform the initialize handshake, then loop over incoming
/// messages until the client sends `shutdown`+`exit` (or the connection closes). Returns `Ok(())` on a
/// clean shutdown. Errors only on a transport-level failure (a malformed stream), never on a bad query
/// — a query is total, so an un-analyzable buffer yields empty diagnostics, never a crash (the spec's
/// "a tooling query MUST NOT crash the editor session on malformed source").
pub fn run() -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
    // The stdio transport: stdin/stdout carry framed JSON-RPC, so nothing else may write to stdout (a
    // stray `println!` would corrupt the stream). Diagnostic logging, if any, goes to stderr.
    let (connection, io_threads) = Connection::stdio();

    // The initialize handshake: advertise the capabilities this server supports, then serve until exit.
    let server_capabilities = serde_json::to_value(capabilities())?;
    let init_params = connection.initialize(server_capabilities)?;
    let _init: InitializeParams = serde_json::from_value(init_params)?;

    // Announce our name/version in the InitializeResult too — some clients surface it. `initialize`
    // already replied with the capabilities value; this is informational and folded into the same reply
    // by `lsp_server`, so we only need to have supplied the capabilities above. (Kept for clarity of
    // what the server reports; `InitializeResult` is constructed to document the shape.)
    let _ = InitializeResult {
        capabilities: capabilities(),
        server_info: Some(ServerInfo {
            name: "cdz-lsp".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };

    // Serve until the client shuts the connection down. `serve` takes the connection BY VALUE and drops
    // it on return — this is load-bearing: `io_threads.join()` waits for the writer thread, which only
    // exits once every clone of the connection's sender is dropped. If the server kept the connection
    // alive across the join, the writer thread would never see its channel disconnect and the join would
    // hang forever. So the connection must be gone before we join.
    let mut server = Server::new(connection);
    server.serve()?;
    drop(server);
    io_threads.join()?;
    Ok(())
}

/// The capabilities this server advertises. Increment 1: FULL text-document sync (the client resends
/// the whole document on each change — simplest correct model; incremental sync is a later refinement)
/// and diagnostics via `publishDiagnostics` (a push the server sends on open/change, so no explicit
/// capability flag beyond sync is required for the classic push model).
fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        ..Default::default()
    }
}

/// One open document's state: its full source text. The parsed program is recomputed on demand (a query
/// is cheap relative to an edit's think-time, and recomputing keeps the "incremental result equals a
/// full compilation" invariant trivially — there is no incremental cache to diverge from a batch).
struct Document {
    text: String,
    /// Whether this document is the ML surface (`.cdz`/`.ml`) vs s-expr (`.sexp`/`.sexpr`), inferred
    /// from the document URI's extension — the same split the one-shot subcommands make on the file path.
    is_ml: bool,
}

/// The server loop state: the transport plus the open-document set, keyed by URI.
struct Server {
    connection: Connection,
    docs: HashMap<Uri, Document>,
}

impl Server {
    fn new(connection: Connection) -> Server {
        Server {
            connection,
            docs: HashMap::new(),
        }
    }

    /// The main receive loop: dispatch each message until the client shuts the connection down.
    fn serve(&mut self) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        while let Ok(msg) = self.connection.receiver.recv() {
            match msg {
                Message::Request(req) => {
                    // `lsp_server` handles the shutdown REQUEST/exit NOTIFICATION protocol: once the
                    // client sends `shutdown`, `handle_shutdown` returns true and we stop after replying.
                    if self.connection.handle_shutdown(&req)? {
                        return Ok(());
                    }
                    self.handle_request(req)?;
                }
                Message::Notification(note) => self.handle_notification(note)?,
                // A response to a request WE sent — increment 1 sends none, so nothing to correlate.
                Message::Response(_) => {}
            }
        }
        Ok(())
    }

    /// Dispatch a client REQUEST. Increment 1 answers no feature requests yet (hover/definition are the
    /// next increments); an unknown method gets a `MethodNotFound` error response so the client is not
    /// left waiting. `shutdown` is handled in `serve` via `handle_shutdown`.
    fn handle_request(
        &mut self,
        req: Request,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        // Reserve the `Shutdown` method name so a future reorganization keeps it distinct from features.
        if req.method == Shutdown::METHOD {
            return Ok(());
        }
        let resp = Response::new_err(
            req.id.clone(),
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!(
                "cdz lsp: unsupported request `{}` (not yet implemented)",
                req.method
            ),
        );
        self.send_response(resp)
    }

    /// Dispatch a client NOTIFICATION — the document-sync lifecycle. Each open/change recomputes and
    /// publishes that document's diagnostics; a close clears them (an empty diagnostic list).
    fn handle_notification(
        &mut self,
        note: Notification,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        match note.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let params: DidOpenTextDocumentParams = extract_note(note)?;
                let uri = params.text_document.uri;
                let is_ml = uri_is_ml(&uri);
                self.docs.insert(
                    uri.clone(),
                    Document {
                        text: params.text_document.text,
                        is_ml,
                    },
                );
                self.publish(&uri)?;
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams = extract_note(note)?;
                let uri = params.text_document.uri;
                // FULL sync: the last content change carries the whole new document text.
                if let Some(change) = params.content_changes.into_iter().next_back() {
                    let is_ml = uri_is_ml(&uri);
                    self.docs.insert(
                        uri.clone(),
                        Document {
                            text: change.text,
                            is_ml,
                        },
                    );
                    self.publish(&uri)?;
                }
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams = extract_note(note)?;
                let uri = params.text_document.uri;
                self.docs.remove(&uri);
                // Clear the document's diagnostics on close (an empty list), so a client does not keep
                // showing stale errors for a file no longer open.
                self.send_diagnostics(&uri, Vec::new())?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Recompute `uri`'s diagnostics and publish them. A missing document (never opened) publishes an
    /// empty list — total, never an error.
    fn publish(&mut self, uri: &Uri) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        let diags = match self.docs.get(uri) {
            Some(doc) => diagnostics_for(&doc.text, doc.is_ml),
            None => Vec::new(),
        };
        self.send_diagnostics(uri, diags)
    }

    /// Send a `textDocument/publishDiagnostics` notification for `uri`.
    fn send_diagnostics(
        &self,
        uri: &Uri,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        let params = PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics,
            version: None,
        };
        let note = Notification::new(
            PublishDiagnostics::METHOD.to_string(),
            serde_json::to_value(params)?,
        );
        self.connection.sender.send(Message::Notification(note))?;
        Ok(())
    }

    fn send_response(
        &self,
        resp: Response,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        self.connection.sender.send(Message::Response(resp))?;
        Ok(())
    }
}

/// Extract a typed notification's params, mapping an extraction error into the boxed error type.
fn extract_note<P: serde::de::DeserializeOwned>(
    note: Notification,
) -> Result<P, Box<dyn std::error::Error + Sync + Send>> {
    // `Notification::extract` returns the params on a method match; here the caller already matched the
    // method, so a failure is a malformed params payload.
    serde_json::from_value(note.params).map_err(Into::into)
}

/// Whether a document URI names an ML-surface source (`.cdz`/`.ml`) vs s-expr (`.sexp`/`.sexpr`) — the
/// same extension split `is_ml_source` makes on a file path. An unrecognized extension defaults to ML
/// (the primary surface an editor edits).
fn uri_is_ml(uri: &Uri) -> bool {
    let s = uri.as_str();
    if s.ends_with(".sexp") || s.ends_with(".sexpr") {
        false
    } else {
        // `.cdz`/`.ml` and anything else → ML.
        true
    }
}

// The `_ = ExtractError` import guard: keep the type referenced so a future typed-request dispatch that
// uses `Request::extract` (which yields `ExtractError`) compiles without re-importing.
#[allow(dead_code)]
fn _extract_error_marker(_: ExtractError<Request>) {}

#[allow(dead_code)]
fn _request_id_marker(_: RequestId) {}

// ── the analysis: source text → LSP diagnostics, via the SAME query engine `cdz check` uses ─────────

/// Compute the LSP diagnostics for `text` on the given surface — the "diagnostics as you type" read.
/// Parses the buffer in-memory (the ML reader RECOVERS from a syntax error, so a mid-edit buffer still
/// yields a tree and its recovered parse errors), then drives the compiler's `Diagnostics` query over
/// the parsed program and maps each fault's node id back to a source `Range` via the span table. TOTAL:
/// an un-analyzable buffer yields whatever partial set the recovering parse + query produce, never a
/// panic — matching the one-shot `cdz check` path and the spec's totality obligation.
///
/// Single-buffer scope (increment 1): a document's own `(import …)` closure is NOT followed here (that
/// needs the workspace file set); a cross-file reference reads as unresolved, exactly as a lone-file
/// `cdz check` on an importing file would before the package path. Following the closure is a later
/// increment (it needs the server to hold the workspace, not just open buffers).
fn diagnostics_for(text: &str, is_ml: bool) -> Vec<Diagnostic> {
    // Parse in-memory to arenas + a span table. Both surfaces produce a span table (the spanned readers);
    // the ML reader recovers, the s-expr reader hard-errors on a malformed program (then we surface the
    // parse failure as a diagnostic at the reported position).
    let (arenas, spans, parse_errors) = match parse_surface(text, is_ml) {
        Ok(t) => t,
        // A hard parse failure (s-expr): report it as a single error diagnostic at its span, nothing more.
        Err((span, message)) => {
            return vec![Diagnostic {
                range: byte_range_to_range(text, span.0, span.1),
                severity: Some(DiagnosticSeverity::ERROR),
                message,
                source: Some("cdz".to_string()),
                ..Default::default()
            }];
        }
    };

    let mut out = Vec::new();

    // 1. Recovered PARSE errors (ML surface). Each is an error-severity fault at its own span — the same
    //    faults `cdz check` prints before the semantic set. A recovered `<error>` placeholder can cascade
    //    into a spurious `unbound name `<error>`` downstream; the semantic pass below drops those.
    for pe in &parse_errors {
        out.push(Diagnostic {
            range: byte_range_to_range(text, pe.span.start, pe.span.end),
            severity: Some(DiagnosticSeverity::ERROR),
            message: pe.message.clone(),
            source: Some("cdz".to_string()),
            ..Default::default()
        });
    }

    // 2. Semantic faults — the `Diagnostics` query over the parsed program (type mismatch, unbound name,
    //    duplicate def, non-linear binder, …), the SAME read `cdz check` performs. Node-id-keyed; map
    //    each to a `Range` through the span table.
    let ast_bytes = cadenza_syntax::codec::encode(&arenas);
    let sidecar_bytes =
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics)]);
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast_bytes),
        rcdzc::Artifact::new(rcdzc::sidecar::KIND_SIDECAR, "drive", sidecar_bytes),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    if let Some(bytes) = compiled.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS) {
        let diag_text = String::from_utf8_lossy(bytes);
        for line in diag_text.lines() {
            if let Some(d) = parse_diag_line(line, text, &spans) {
                out.push(d);
            }
        }
    }
    out
}

/// Parse `text` on its surface to `(arenas, spans, recovered-parse-errors)`, or `Err((byte-span, msg))`
/// for a surface whose reader hard-fails (s-expr). The ML reader always returns `Ok` (it recovers,
/// carrying the errors in the third tuple slot).
#[allow(clippy::type_complexity)]
fn parse_surface(
    text: &str,
    is_ml: bool,
) -> Result<
    (
        cadenza_syntax::Arenas,
        cadenza_syntax::spans::SpanTable,
        Vec<cadenza_syntax::parser::ParseError>,
    ),
    ((usize, usize), String),
> {
    if is_ml {
        let parsed = cadenza_syntax::parser::read_ml(text);
        Ok((parsed.arenas, parsed.spans, parsed.errors))
    } else {
        // The s-expr surface: a single top-level form stays bare (mirrors the ML root convention), else
        // wrap in `(do …)`. `read_spanned` succeeds iff there is exactly one form; fall back to the
        // multi-form spanned reader. A genuine malformed program hard-errors — surface it as a diagnostic.
        match cadenza_syntax::sexpr::read_spanned(text) {
            Ok((arenas, spans)) => Ok((arenas, spans, Vec::new())),
            Err(_) => match cadenza_syntax::sexpr::read_all_spanned(text) {
                Ok((arenas, spans)) => Ok((arenas, spans, Vec::new())),
                Err(e) => Err(((0, text.len().min(1)), format!("s-expr parse: {}", e.0))),
            },
        }
    }
}

/// Parse one TAB-separated `Diagnostics`-query line into an LSP [`Diagnostic`], or `None` to drop it.
/// The line shape is `severity  code  node-id  fix-kind  fix-node  fix-repl  fix-verified  message`
/// (eight columns, message last). Drops a cascade artifact — an `unbound name `<error>`` on a recovered
/// `<error>` placeholder (the parse error already said what to fix). An unanchored (`-`) node maps to the
/// document start so the diagnostic is still shown (never silently dropped).
fn parse_diag_line(
    line: &str,
    text: &str,
    spans: &cadenza_syntax::spans::SpanTable,
) -> Option<Diagnostic> {
    let mut cols = line.splitn(8, '\t');
    let severity = cols.next()?;
    let code = cols.next()?;
    let node = cols.next()?;
    let _fix_kind = cols.next()?;
    let _fix_node = cols.next()?;
    let _fix_repl = cols.next()?;
    let _fix_verified = cols.next()?;
    let message = cols.next().unwrap_or("");

    // Drop the `<error>`-placeholder cascade: a recovered parse placeholder reduces to a bare name
    // `<error>`, which the checker reports as an unbound-name fault referencing a token the user never
    // wrote. `<error>` is unlexable on either surface, so such a message is always the placeholder.
    if message.contains("`<error>`") {
        return None;
    }

    let severity = match severity {
        "error" => DiagnosticSeverity::ERROR,
        "warning" => DiagnosticSeverity::WARNING,
        _ => DiagnosticSeverity::INFORMATION,
    };

    // The node's source range via the span table; an unanchored/unmapped node → the document start.
    let range = node
        .parse::<u32>()
        .ok()
        .and_then(|id| spans.get(cadenza_syntax::StructId(id)))
        .map(|s| byte_range_to_range(text, s.start, s.end))
        .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0)));

    let code = if code == "-" {
        None
    } else {
        Some(lsp_types::NumberOrString::String(code.to_string()))
    };

    Some(Diagnostic {
        range,
        severity: Some(severity),
        code,
        source: Some("cdz".to_string()),
        message: message.to_string(),
        ..Default::default()
    })
}

// ── position mapping: UTF-8 byte offset → LSP UTF-16 (line, character) ──────────────────────────────

/// Convert a `[start, end)` UTF-8 byte range in `text` to an LSP [`Range`]. LSP positions are 0-based
/// `(line, character)` where CHARACTER counts UTF-16 code units (the protocol's default encoding), so a
/// non-ASCII byte offset must be re-counted in UTF-16 — a byte column would misplace the marker on any
/// line with a multibyte character.
fn byte_range_to_range(text: &str, start: usize, end: usize) -> Range {
    Range::new(byte_to_position(text, start), byte_to_position(text, end))
}

/// The 0-based UTF-16 [`Position`] of UTF-8 byte offset `byte` in `text`. Walks the source counting
/// newlines for the line, and UTF-16 code units since the last newline for the character. A byte past
/// the end clamps to the end (like the one-shot `line_col`).
fn byte_to_position(text: &str, byte: usize) -> Position {
    let byte = byte.min(text.len());
    let mut line: u32 = 0;
    let mut character: u32 = 0;
    for (i, ch) in text.char_indices() {
        if i >= byte {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }
    Position::new(line, character)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_to_position_counts_lines_and_utf16_columns() {
        let text = "ab\ncde";
        // Byte 0 = start.
        assert_eq!(byte_to_position(text, 0), Position::new(0, 0));
        // Byte 1 = second char of line 0.
        assert_eq!(byte_to_position(text, 1), Position::new(0, 1));
        // Byte 3 = right after the newline → start of line 1.
        assert_eq!(byte_to_position(text, 3), Position::new(1, 0));
        // Byte 5 = third char of line 1.
        assert_eq!(byte_to_position(text, 5), Position::new(1, 2));
        // Past the end clamps.
        assert_eq!(byte_to_position(text, 999), Position::new(1, 3));
    }

    #[test]
    fn byte_to_position_uses_utf16_units_not_bytes() {
        // `€` is 3 UTF-8 bytes but 1 UTF-16 code unit; a byte after it must map to character 1, not 3.
        let text = "€x";
        assert_eq!(byte_to_position(text, 0), Position::new(0, 0));
        assert_eq!(byte_to_position(text, 3), Position::new(0, 1)); // after the euro sign
        assert_eq!(byte_to_position(text, 4), Position::new(0, 2)); // after the `x`
    }

    #[test]
    fn byte_to_position_astral_char_is_two_utf16_units() {
        // A char outside the BMP (`𝟙`, U+1D7D9) is 4 UTF-8 bytes AND 2 UTF-16 code units (a surrogate
        // pair) — the char after it lands at character 2, the case a naive char-count would get wrong.
        let text = "𝟙y";
        assert_eq!(byte_to_position(text, 4), Position::new(0, 2)); // after the astral char
    }

    #[test]
    fn diagnostics_for_a_clean_program_is_empty() {
        // A well-formed ML program has no faults — the result is total and empty, never an error. `x + x`
        // uses the parameter (so no unused-param warning) and references only bound names.
        let diags = diagnostics_for("def double(x: Int64) -> Int64 = x + x", true);
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn diagnostics_for_an_unbound_name_reports_one_error_diagnostic() {
        // A clean-parsing program with a genuine semantic fault (an unbound name) yields exactly that
        // fault as an ERROR-severity diagnostic carrying its CDZ code — the "diagnostics as you type" read.
        // `x` is used (no unused-param warning), so the ONLY fault is the unbound `mystery`.
        let diags = diagnostics_for("def double(x: Int64) -> Int64 = x + mystery", true);
        assert_eq!(diags.len(), 1, "one diagnostic expected, got {diags:?}");
        let d = &diags[0];
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert!(
            d.message.contains("unbound name") && d.message.contains("mystery"),
            "message: {}",
            d.message
        );
        assert_eq!(
            d.code,
            Some(lsp_types::NumberOrString::String("CDZ0101".to_string()))
        );
        // The range points at the offending name, not the document start.
        assert!(
            d.range.start != Position::new(0, 0) || d.range.end != Position::new(0, 0),
            "the diagnostic should span the name, not collapse to the doc start"
        );
    }

    #[test]
    fn diagnostics_for_malformed_source_is_total_not_a_panic() {
        // A buffer that does not fully parse still returns a defined partial result — never a panic (the
        // spec's "a tooling query MUST NOT crash the editor session on malformed source"). We only assert
        // it returns; the exact recovered set is the parser's business.
        let _ = diagnostics_for("def (f x = (", true);
        let _ = diagnostics_for("(((", true);
        let _ = diagnostics_for("", true);
    }

    #[test]
    fn parse_diag_line_drops_the_error_placeholder_cascade() {
        // An `<error>`-placeholder cascade fault (a spurious unbound-name on a recovered parse placeholder)
        // is dropped — the parse error already said what to fix.
        let spans = cadenza_syntax::parser::read_ml("x").spans;
        let line = "error\tCDZ0101\t-\t-\t-\t-\t-\tunbound name `<error>`";
        assert!(parse_diag_line(line, "x", &spans).is_none());
    }

    #[test]
    fn parse_diag_line_maps_a_warning_with_no_code() {
        // A warning-severity, uncoded fault maps to a WARNING diagnostic with no code, at the doc start
        // when its node is unanchored (`-`).
        let spans = cadenza_syntax::parser::read_ml("x").spans;
        let line = "warning\t-\t-\t-\t-\t-\t-\tsomething is unused";
        let d = parse_diag_line(line, "x", &spans).expect("a diagnostic");
        assert_eq!(d.severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(d.code, None);
        assert_eq!(d.message, "something is unused");
    }
}
