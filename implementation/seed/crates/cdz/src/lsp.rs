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
//! Capabilities implemented so far: the `initialize`/`shutdown` handshake, full-document sync
//! (`didOpen`/`didChange`/`didClose`), `textDocument/publishDiagnostics` (← the `Diagnostics` query),
//! `textDocument/hover` (← the `TypeAt` query), and `textDocument/semanticTokens/full` (← the
//! `Highlight` query). Go-to-definition (← `ResolveOf`), references (← `UsesOf`), completion (←
//! `ScopeAt`), and code actions (← the `Diagnostics` fix columns) are the next increments — each a
//! read of a column the query engine already exposes, wired to its LSP request.

use std::collections::HashMap;

use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    GotoDefinition, HoverRequest, References, Request as _, SemanticTokensFullRequest, Shutdown,
};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, Location,
    MarkedString, Position, PublishDiagnosticsParams, Range, ReferenceParams, SemanticToken,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri, WorkDoneProgressOptions,
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

    // The initialize handshake, done in two steps so the response carries the FULL `InitializeResult` —
    // NOT just the capabilities. The convenience `Connection::initialize(caps)` replies with only the
    // capabilities value, so a client never learns the server's name/version; `initialize_start` +
    // `initialize_finish` let us send `serverInfo` alongside the capabilities (some clients surface the
    // server name/version in their UI and logs, which is how an editor confirms it is talking to `cdz`).
    let (init_id, init_params) = connection.initialize_start()?;
    let _init: InitializeParams = serde_json::from_value(init_params)?;
    connection.initialize_finish(init_id, serde_json::to_value(initialize_result())?)?;

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

/// The capabilities this server advertises: FULL text-document sync (the client resends the whole
/// document on each change — simplest correct model; incremental sync is a later refinement),
/// diagnostics via `publishDiagnostics` (a push the server sends on open/change, so no explicit
/// capability flag beyond sync is required for the classic push model), `hover` (the "type at
/// cursor" read, backed by the `TypeAt` query), `definition` (go-to, backed by `ResolveOf`),
/// `references` (find-all-uses, backed by `UsesOf`), and `semanticTokens/full` (colour-by-meaning,
/// backed by the `Highlight` query — the token legend is `semantic_token_legend`).
fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions::default(),
                legend: semantic_token_legend(),
                // No range request (a `full` document tokenization is cheap for this query); `full`
                // without delta (we recompute the whole token list per request — simplest correct model).
                range: Some(false),
                full: Some(SemanticTokensFullOptions::Bool(true)),
            },
        )),
        ..Default::default()
    }
}

/// The full `InitializeResult` the server replies to `initialize` with — its `capabilities` PLUS its
/// `serverInfo` (name/version). Sent via `initialize_finish` (NOT the convenience `Connection::
/// initialize`, which would drop everything but the capabilities), so a client learns which server and
/// version it is talking to.
fn initialize_result() -> InitializeResult {
    InitializeResult {
        capabilities: capabilities(),
        server_info: Some(ServerInfo {
            name: "cdz-lsp".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    }
}

/// The ORDERED semantic-token-type legend the server publishes; a token's `token_type` field is an
/// INDEX into this list. It must stay in sync with [`highlight_kind_to_token_index`] (which maps each
/// Cadenza `HighlightKind` wire spelling to its index here). LSP defines a standard token-type
/// vocabulary; we use its members so a client's default theme colours our tokens without custom config.
fn semantic_token_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: SEMANTIC_TOKEN_TYPES.to_vec(),
        token_modifiers: Vec::new(),
    }
}

/// The token types, in legend order. Indexed by [`highlight_kind_to_token_index`]. Standard LSP token
/// types (a client theme knows them); Cadenza's `constructor` maps to `enumMember`, `label` to
/// `property`, `effect` to `event`, and `unbound`/`char`/`bytes`/`symbol`/`literal` fold to the nearest
/// standard type (a lexical fallback still applies for anything unmapped).
const SEMANTIC_TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::KEYWORD,     // 0
    SemanticTokenType::TYPE,        // 1
    SemanticTokenType::ENUM_MEMBER, // 2  (a variant constructor)
    SemanticTokenType::FUNCTION,    // 3
    SemanticTokenType::PARAMETER,   // 4
    SemanticTokenType::VARIABLE,    // 5
    SemanticTokenType::EVENT,       // 6  (an effect)
    SemanticTokenType::PROPERTY,    // 7  (a label — record field / member key)
    SemanticTokenType::NUMBER,      // 8
    SemanticTokenType::STRING,      // 9
    SemanticTokenType::OPERATOR,    // 10 (fallback for unbound / symbol / misc)
];

/// Map a `HighlightKind` wire spelling (the `Highlight` query's per-token second column) to its index
/// in [`SEMANTIC_TOKEN_TYPES`], or `None` to leave the token unclassified (the editor's lexical
/// fallback paints it). Kept exhaustive against the query's closed vocabulary so a new highlight kind
/// forces a decision here rather than silently dropping.
fn highlight_kind_to_token_index(kind: &str) -> Option<u32> {
    Some(match kind {
        "keyword" => 0,
        "type" => 1,
        "constructor" => 2,
        "function" => 3,
        "param" => 4,
        "variable" => 5,
        "effect" => 6,
        "label" => 7,
        "number" => 8,
        "string" | "char" | "bytes" => 9,
        // `symbol`/`literal`/`unbound` have no closer standard type; map to OPERATOR so they still get a
        // consistent colour (an editor can theme it, and a lexical fallback covers the rest).
        "symbol" | "literal" | "unbound" => 10,
        _ => return None,
    })
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

    /// Dispatch a client REQUEST. `hover` is answered from the `TypeAt` query; other feature requests
    /// (definition/references/completion) are the next increments and get a `MethodNotFound` error so the
    /// client is not left waiting. `shutdown` is handled in `serve` via `handle_shutdown`.
    fn handle_request(
        &mut self,
        req: Request,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        // Reserve the `Shutdown` method name so a future reorganization keeps it distinct from features.
        if req.method == Shutdown::METHOD {
            return Ok(());
        }
        match req.method.as_str() {
            HoverRequest::METHOD => {
                let (id, params) = cast_request::<HoverRequest>(req)?;
                let result = self.hover(&params);
                self.send_response(Response::new_ok(id, result))
            }
            SemanticTokensFullRequest::METHOD => {
                let (id, params) = cast_request::<SemanticTokensFullRequest>(req)?;
                let result = self.semantic_tokens(&params);
                self.send_response(Response::new_ok(id, result))
            }
            GotoDefinition::METHOD => {
                let (id, params) = cast_request::<GotoDefinition>(req)?;
                let result = self.goto_definition(&params);
                self.send_response(Response::new_ok(id, result))
            }
            References::METHOD => {
                let (id, params) = cast_request::<References>(req)?;
                let result = self.references(&params);
                self.send_response(Response::new_ok(id, result))
            }
            _ => {
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
        }
    }

    /// Answer a `textDocument/hover`: resolve the cursor position to the innermost node and report its
    /// type (the "type at cursor" read, backed by the `TypeAt` query — a grammar keyword names itself, a
    /// definition shows its signature). `None` (JSON `null`) when the document is not open, the position
    /// maps to no node, or the node has no meaningful hover — total, never an error.
    fn hover(&self, params: &HoverParams) -> Option<Hover> {
        let pos = &params.text_document_position_params;
        let doc = self.docs.get(&pos.text_document.uri)?;
        hover_at(&doc.text, doc.is_ml, pos.position)
    }

    /// Answer a `textDocument/semanticTokens/full`: classify every token in the document by the role it
    /// plays (type vs constructor vs local vs unbound), backed by the `Highlight` query. `None` when the
    /// document is not open; an empty token set otherwise (total). The tokens are LSP-delta-encoded
    /// against the published legend (`semantic_token_legend`).
    fn semantic_tokens(&self, params: &SemanticTokensParams) -> Option<SemanticTokensResult> {
        let doc = self.docs.get(&params.text_document.uri)?;
        let tokens = semantic_tokens_for(&doc.text, doc.is_ml);
        Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        }))
    }

    /// Answer a `textDocument/definition`: resolve the cursor's reference to its DEFINING occurrence
    /// (backed by the `ResolveOf` query — a def name, a `let` initializer, a parameter binder), returned
    /// as a `Location` in the same document. `None` when the document is not open, the position is not a
    /// navigable reference, or the target has no source span (a prelude/built-in binding) — total.
    fn goto_definition(&self, params: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let pos = &params.text_document_position_params;
        let doc = self.docs.get(&pos.text_document.uri)?;
        let loc = definition_at(&doc.text, doc.is_ml, pos.position, &pos.text_document.uri)?;
        Some(GotoDefinitionResponse::Scalar(loc))
    }

    /// Answer a `textDocument/references`: every occurrence that references the SAME definition as the
    /// name under the cursor (backed by the `UsesOf` query, keyed by the cursor's name). Honors the
    /// client's `include_declaration` flag — when set, the definition's own name occurrence is added.
    /// `None`/empty when the cursor is not on a name or the name has no references — total.
    fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let pos = &params.text_document_position;
        let doc = self.docs.get(&pos.text_document.uri)?;
        let include_decl = params.context.include_declaration;
        Some(references_at(
            &doc.text,
            doc.is_ml,
            pos.position,
            &pos.text_document.uri,
            include_decl,
        ))
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

/// Extract a typed request's `(id, params)`, mapping an extraction failure into the boxed error type.
/// The caller has already matched the method by name, so a `MethodMismatch` cannot occur; a
/// `JsonError` is a malformed params payload.
fn cast_request<R: lsp_types::request::Request>(
    req: Request,
) -> Result<(RequestId, R::Params), Box<dyn std::error::Error + Sync + Send>> {
    req.extract(R::METHOD).map_err(|e| match e {
        ExtractError::JsonError { error, .. } => Box::new(error) as Box<_>,
        ExtractError::MethodMismatch(r) => {
            format!("cdz lsp: internal method mismatch on `{}`", r.method).into()
        }
    })
}

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
        // CANONICALIZE the ML arena and remap the spans — the SAME normalization `load_program_spanned`
        // (the one-shot subcommands' loader) applies before handing the program to the compiler. This is
        // load-bearing for every NODE-ID-keyed query (definition/references/hover/tokens): the compiler
        // resolves + types over the CANONICAL arena, so a raw `read_ml` arena's node ids would not line up
        // with the ids the queries answer in — a go-to-definition would then jump to the wrong node. The
        // remapped span table keeps `node_at_offset` and each answer id pointing at the right source range.
        let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&parsed.arenas);
        let spans = parsed.spans.remap(&id_map, arenas.structure.len());
        Ok((arenas, spans, parsed.errors))
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

/// The UTF-8 byte offset of an LSP [`Position`] in `text` — the inverse of [`byte_to_position`]. LSP
/// `character` counts UTF-16 code units, so a column past a multibyte character maps to a byte offset
/// further along than the column number. A position past the end of its line clamps to the line end; a
/// line past the end clamps to the text end (a query is total — an out-of-range cursor never panics).
fn position_to_byte(text: &str, pos: Position) -> usize {
    let mut line: u32 = 0;
    let mut utf16_col: u32 = 0;
    for (i, ch) in text.char_indices() {
        if line == pos.line && utf16_col >= pos.character {
            return i;
        }
        if ch == '\n' {
            // Reached the end of the target line before its target column — clamp to the newline.
            if line == pos.line {
                return i;
            }
            line += 1;
            utf16_col = 0;
        } else if line == pos.line {
            utf16_col += ch.len_utf16() as u32;
        }
    }
    text.len()
}

// ── the analysis: source text + cursor → LSP hover, via the `TypeAt` query ──────────────────────────

/// Compute the LSP hover for the cursor `pos` in `text` — the "type at cursor" read. Parses the buffer
/// in-memory, resolves the position to the INNERMOST node covering it (`node_at_offset`), then drives
/// the `TypeAt` query, whose answer is a hover-ready string (a grammar keyword names itself, a
/// definition shows its `name : type` signature, an untypeable node says "unknown"). `None` when the
/// buffer does not parse to a span table, the position maps to no node, or the answer is empty — total,
/// never a panic.
fn hover_at(text: &str, is_ml: bool, pos: Position) -> Option<Hover> {
    let (arenas, spans, _errors) = parse_surface(text, is_ml).ok()?;
    let byte = position_to_byte(text, pos);
    let node = spans.node_at_offset(byte)?;

    let ast_bytes = cadenza_syntax::codec::encode(&arenas);
    let sidecar_bytes =
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::TypeAt {
            node: node.0,
        })]);
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast_bytes),
        rcdzc::Artifact::new(rcdzc::sidecar::KIND_SIDECAR, "drive", sidecar_bytes),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let bytes = compiled.artifact(rcdzc::sidecar::KIND_TYPE_AT)?;
    let ty = String::from_utf8_lossy(bytes);
    let ty = ty.trim();
    // A total-but-uninformative answer ("unknown", or empty) is not worth a hover popup — return None so
    // the editor shows nothing rather than a meaningless box.
    if ty.is_empty() || ty == "unknown" {
        return None;
    }
    // The hovered node's source range, so the editor underlines exactly the sub-expression it typed.
    let range = spans
        .get(node)
        .map(|s| byte_range_to_range(text, s.start, s.end));
    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(ty.to_string())),
        range,
    })
}

// ── the analysis: cursor → definition / references, via the `ResolveOf` / `UsesOf` queries ───────────

/// Run a single sidecar `query` over `arenas` and return its answer artifact of `kind` as text, or
/// `None` if the compile produced no such artifact. Shared by the query-backed analyses.
fn run_query_text(
    arenas: &cadenza_syntax::Arenas,
    query: rcdzc::sidecar::Query,
    kind: &str,
) -> Option<String> {
    let ast_bytes = cadenza_syntax::codec::encode(arenas);
    let sidecar_bytes = rcdzc::sidecar::encode(&[rcdzc::Request::Query(query)]);
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast_bytes),
        rcdzc::Artifact::new(rcdzc::sidecar::KIND_SIDECAR, "drive", sidecar_bytes),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    compiled
        .artifact(kind)
        .map(|b| String::from_utf8_lossy(b).into_owned())
}

/// The `Location` (in `uri`) of a node id in `spans`, or `None` if the node has no recorded span (a
/// prelude/built-in/synthesized node — nothing to navigate to).
fn node_location(
    text: &str,
    spans: &cadenza_syntax::spans::SpanTable,
    uri: &Uri,
    node: u32,
) -> Option<Location> {
    let span = spans.get(cadenza_syntax::StructId(node))?;
    Some(Location {
        uri: uri.clone(),
        range: byte_range_to_range(text, span.start, span.end),
    })
}

/// Go-to-definition: resolve the reference under `pos` to its defining occurrence (the `ResolveOf`
/// query) and return its `Location`. `None` when the buffer does not parse, the position maps to no
/// node, the node is not a navigable reference, or the target has no source span — total.
fn definition_at(text: &str, is_ml: bool, pos: Position, uri: &Uri) -> Option<Location> {
    let (arenas, spans, _errors) = parse_surface(text, is_ml).ok()?;
    let byte = position_to_byte(text, pos);
    let node = spans.node_at_offset(byte)?;
    let answer = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::ResolveOf { node: node.0 },
        rcdzc::sidecar::KIND_RESOLVE,
    )?;
    // The `ResolveOf` answer is the defining occurrence's node id (empty = not a navigable reference).
    let target: u32 = answer.trim().parse().ok()?;
    node_location(text, &spans, uri, target)
}

/// Find-references: every occurrence that references the SAME definition as the NAME under `pos`
/// (the `UsesOf` query, keyed by that name). When `include_declaration`, the definition the name
/// resolves to (via `ResolveOf`) is added so its own site appears in the list. Returns an empty vec
/// when the cursor is not on a name or the name has no references — total, never a panic.
fn references_at(
    text: &str,
    is_ml: bool,
    pos: Position,
    uri: &Uri,
    include_declaration: bool,
) -> Vec<Location> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let byte = position_to_byte(text, pos);
    let Some(node) = spans.node_at_offset(byte) else {
        return Vec::new();
    };
    // The name under the cursor: `UsesOf` is by NAME, so the cursor node must be a name atom. A cursor
    // on a non-name (a literal, a list form) has no name to find references of → empty.
    let Some(name) = arenas.as_name(node).map(str::to_string) else {
        return Vec::new();
    };

    let mut locations: Vec<Location> = Vec::new();
    if let Some(answer) = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::UsesOf { name: name.clone() },
        rcdzc::sidecar::KIND_USES,
    ) {
        for line in answer.lines() {
            if let Ok(id) = line.trim().parse::<u32>()
                && let Some(loc) = node_location(text, &spans, uri, id)
            {
                locations.push(loc);
            }
        }
    }

    // Optionally include the DECLARATION site (the def's name occurrence the cursor's name resolves to).
    // `UsesOf` excludes the defining occurrence by design, so `include_declaration` adds it back.
    if include_declaration
        && let Some(answer) = run_query_text(
            &arenas,
            rcdzc::sidecar::Query::ResolveOf { node: node.0 },
            rcdzc::sidecar::KIND_RESOLVE,
        )
        && let Ok(target) = answer.trim().parse::<u32>()
        && let Some(loc) = node_location(text, &spans, uri, target)
        && !locations.contains(&loc)
    {
        locations.push(loc);
    }

    locations
}

// ── the analysis: source text → LSP semantic tokens, via the `Highlight` query ──────────────────────

/// Compute the LSP semantic tokens for `text` — colour-by-meaning, backed by the `Highlight` query
/// (which classifies every user LEAF by the role it plays: type/constructor/function/param/local/…).
/// Each classified leaf's node id is mapped to its source span, then to an LSP (line, start-char,
/// length) triple, sorted by position, and DELTA-encoded per the LSP wire format (each token's line/
/// start are relative to the previous token). TOTAL: an un-analyzable buffer yields no tokens.
///
/// LSP requires a token to lie on a SINGLE line and to be non-overlapping; a leaf that spans multiple
/// lines (a multi-line string) or whose kind has no legend mapping is skipped (the editor's lexical
/// fallback paints it) — a defined omission, not a crash.
fn semantic_tokens_for(text: &str, is_ml: bool) -> Vec<SemanticToken> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let ast_bytes = cadenza_syntax::codec::encode(&arenas);
    let sidecar_bytes =
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::Highlight)]);
    let inputs = vec![
        rcdzc::Artifact::new(rcdzc::Artifact::KIND_AST, "main", ast_bytes),
        rcdzc::Artifact::new(rcdzc::sidecar::KIND_SIDECAR, "drive", sidecar_bytes),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let Some(bytes) = compiled.artifact(rcdzc::sidecar::KIND_HIGHLIGHT) else {
        return Vec::new();
    };
    let hl_text = String::from_utf8_lossy(bytes);

    // Gather each classified leaf as an absolute (line, start-char, length, token-type) tuple. The
    // Highlight query already emits leaves in ascending node-id order, but node id is not source order,
    // so sort by (line, start) before delta-encoding (LSP requires ascending position).
    let mut abs: Vec<(u32, u32, u32, u32)> = Vec::new();
    for line in hl_text.lines() {
        let mut cols = line.splitn(2, '\t');
        let (Some(node), Some(kind)) = (cols.next(), cols.next()) else {
            continue;
        };
        let Some(token_type) = highlight_kind_to_token_index(kind) else {
            continue;
        };
        let Some(id) = node.parse::<u32>().ok() else {
            continue;
        };
        let Some(span) = spans.get(cadenza_syntax::StructId(id)) else {
            continue;
        };
        let start = byte_to_position(text, span.start);
        let end = byte_to_position(text, span.end);
        // LSP tokens are single-line; skip a leaf that crosses a line boundary (rare — a multi-line
        // string literal), leaving it to the editor's lexical fallback.
        if start.line != end.line || end.character < start.character {
            continue;
        }
        let length = end.character - start.character;
        if length == 0 {
            continue;
        }
        abs.push((start.line, start.character, length, token_type));
    }
    abs.sort_by_key(|&(line, ch, _, _)| (line, ch));

    // Delta-encode: each token's line is relative to the previous token's line; its start char is
    // relative to the previous token's start when on the SAME line, else absolute.
    let mut out = Vec::with_capacity(abs.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for (line, start, length, token_type) in abs {
        let delta_line = line - prev_line;
        let delta_start = if delta_line == 0 {
            start - prev_start
        } else {
            start
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_start = start;
    }
    out
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
    fn position_to_byte_is_the_inverse_of_byte_to_position() {
        // Round-trip on a multi-line, multibyte source: every byte offset that starts a char maps to a
        // position and back to itself.
        let text = "ab\n€x\n𝟙y";
        for (i, _) in text.char_indices() {
            let pos = byte_to_position(text, i);
            assert_eq!(
                position_to_byte(text, pos),
                i,
                "round-trip failed at byte {i}"
            );
        }
    }

    #[test]
    fn position_to_byte_clamps_out_of_range() {
        let text = "ab\ncd";
        // A column past the line end clamps to the newline (byte 2), not into the next line.
        assert_eq!(position_to_byte(text, Position::new(0, 99)), 2);
        // A line past the end clamps to the text end.
        assert_eq!(position_to_byte(text, Position::new(9, 0)), text.len());
    }

    #[test]
    fn hover_on_a_definition_reports_its_type() {
        // Hovering a definition's name shows its signature — the "type at cursor" read. The cursor sits on
        // the `answer` name at column 4 of `def answer = 42`.
        let text = "def answer = 42";
        let h = hover_at(text, true, Position::new(0, 4)).expect("a hover");
        let rendered = match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        // The answer mentions the inferred type (an Int-family type for `42`).
        assert!(
            rendered.contains("Int"),
            "hover should report the type, got: {rendered}"
        );
        assert!(
            h.range.is_some(),
            "hover should carry the node's source range"
        );
    }

    #[test]
    fn hover_off_any_node_is_none_not_a_panic() {
        // A position past the end of the buffer maps to no meaningful node → no hover, never a panic.
        let text = "def answer = 42";
        // Far past the end.
        let _ = hover_at(text, true, Position::new(50, 50));
        // On leading whitespace of an empty-ish buffer.
        assert!(hover_at("   ", true, Position::new(0, 1)).is_none());
    }

    #[test]
    fn highlight_kind_map_covers_the_whole_query_vocabulary() {
        // Every `HighlightKind` wire spelling the query can emit must map to a legend index (or be a
        // deliberate fallback) — a new kind must force a decision here, not silently drop. All indices
        // must be in range of the published legend.
        for kind in [
            "keyword",
            "type",
            "constructor",
            "function",
            "param",
            "variable",
            "effect",
            "label",
            "number",
            "string",
            "char",
            "bytes",
            "symbol",
            "literal",
            "unbound",
        ] {
            let idx = highlight_kind_to_token_index(kind)
                .unwrap_or_else(|| panic!("highlight kind `{kind}` has no token index"));
            assert!(
                (idx as usize) < SEMANTIC_TOKEN_TYPES.len(),
                "index {idx} for `{kind}` is out of legend range"
            );
        }
    }

    #[test]
    fn semantic_tokens_classify_a_definition_and_delta_encode() {
        // A function definition yields tokens for its parts. The FIRST token's delta_line/delta_start are
        // absolute (no previous token); every token references a legend index and has a non-zero length.
        let toks = semantic_tokens_for("def double(x: Int64) -> Int64 = x + x", true);
        assert!(!toks.is_empty(), "expected some semantic tokens");
        for t in &toks {
            assert!(t.length > 0, "a token must have positive length");
            assert!(
                (t.token_type as usize) < SEMANTIC_TOKEN_TYPES.len(),
                "token_type {} out of legend range",
                t.token_type
            );
        }
        // Reconstruct absolute (line, start) from the deltas and assert strictly-ascending position —
        // the LSP wire-format invariant a client relies on.
        let mut line = 0u32;
        let mut start = 0u32;
        let mut prev: Option<(u32, u32)> = None;
        for t in &toks {
            line += t.delta_line;
            start = if t.delta_line == 0 {
                start + t.delta_start
            } else {
                t.delta_start
            };
            if let Some(p) = prev {
                assert!((line, start) >= p, "tokens must be in ascending position");
            }
            prev = Some((line, start));
        }
    }

    #[test]
    fn semantic_tokens_on_malformed_source_is_total() {
        // An un-analyzable buffer yields a defined (possibly empty) token set, never a panic.
        let _ = semantic_tokens_for("def (f x = (", true);
        let _ = semantic_tokens_for("", true);
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

    #[test]
    fn initialize_result_carries_server_info_and_capabilities() {
        // The initialize response must include BOTH the capabilities AND serverInfo (name/version) — a
        // regression guard for the bug where the InitializeResult was built then discarded so only the
        // capabilities reached the client (PR #391). We assert on the SERIALIZED JSON, which is exactly
        // what `initialize_finish` sends over the wire.
        let value = serde_json::to_value(initialize_result()).expect("serializes");
        let info = value
            .get("serverInfo")
            .expect("the initialize result must carry serverInfo");
        assert_eq!(info.get("name").and_then(|n| n.as_str()), Some("cdz-lsp"));
        assert!(
            info.get("version").and_then(|v| v.as_str()).is_some(),
            "serverInfo must carry a version"
        );
        assert!(
            value.get("capabilities").is_some(),
            "the initialize result must still carry capabilities"
        );
    }

    /// A throwaway document URI for the position-based analyses.
    fn test_uri() -> Uri {
        use std::str::FromStr;
        Uri::from_str("file:///t.cdz").unwrap()
    }

    /// The line/character of a `Position`, as a tuple, for terse assertions.
    fn lc(p: Position) -> (u32, u32) {
        (p.line, p.character)
    }

    #[test]
    fn definition_jumps_from_a_use_to_the_defining_name() {
        // `helper` is defined on line 0 and used on line 1; go-to-definition from the USE lands on the
        // DEFINITION's name occurrence (line 0), not the use.
        let text = "def helper(x: Int64) -> Int64 = x\ndef main = helper(1)";
        // Cursor on the `helper` call in `main` (line 1). "def main = " is 11 chars, so `helper` at col 11.
        let loc =
            definition_at(text, true, Position::new(1, 11), &test_uri()).expect("a definition");
        assert_eq!(
            loc.range.start.line, 0,
            "definition is on line 0, got {loc:?}"
        );
        // It points at the `helper` name (col 4 of `def helper…`), not its body.
        assert_eq!(lc(loc.range.start), (0, 4), "should land on the def name");
    }

    #[test]
    fn definition_off_a_reference_is_none() {
        // A cursor on a literal (not a navigable reference) yields no definition — total, never a panic.
        let text = "def answer = 42";
        // Cursor on the `42` literal.
        assert!(definition_at(text, true, Position::new(0, 13), &test_uri()).is_none());
        // Cursor past the end.
        assert!(definition_at(text, true, Position::new(9, 9), &test_uri()).is_none());
    }

    #[test]
    fn references_finds_every_use_of_the_name_under_the_cursor() {
        // `helper` is used twice (lines 1 and 2). References from any occurrence finds both uses;
        // excluding the declaration by default (UsesOf excludes the defining occurrence).
        let text = "def helper(x: Int64) -> Int64 = x\n\
                    def a = helper(1)\n\
                    def b = helper(2)";
        // Cursor on the `helper` use in `a` (line 1, col 8: "def a = " = 8 chars).
        let refs = references_at(text, true, Position::new(1, 8), &test_uri(), false);
        assert_eq!(refs.len(), 2, "two uses expected, got {refs:?}");
        let lines: Vec<u32> = refs.iter().map(|l| l.range.start.line).collect();
        assert!(
            lines.contains(&1) && lines.contains(&2),
            "uses on lines 1 and 2: {lines:?}"
        );
    }

    #[test]
    fn references_include_declaration_adds_the_definition_site() {
        let text = "def helper(x: Int64) -> Int64 = x\n\
                    def a = helper(1)";
        // With include_declaration, the def's own name occurrence (line 0) is added to the one use (line 1).
        let refs = references_at(text, true, Position::new(1, 8), &test_uri(), true);
        let lines: Vec<u32> = refs.iter().map(|l| l.range.start.line).collect();
        assert!(
            lines.contains(&0),
            "declaration (line 0) should be included: {lines:?}"
        );
        assert!(
            lines.contains(&1),
            "the use (line 1) should be present: {lines:?}"
        );
    }

    #[test]
    fn references_off_a_name_is_empty() {
        // A cursor not on a name (a literal) yields no references — total.
        let text = "def answer = 42";
        let refs = references_at(text, true, Position::new(0, 13), &test_uri(), false);
        assert!(refs.is_empty(), "expected no references, got {refs:?}");
    }
}
