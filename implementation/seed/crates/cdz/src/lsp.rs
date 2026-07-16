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
//! `textDocument/hover` (← the `TypeAt` query), `textDocument/semanticTokens/full` (← the `Highlight`
//! query), `textDocument/definition` (← `ResolveOf`), `textDocument/references` (← `UsesOf`),
//! `textDocument/completion` (← `ScopeAt` + `Symbols`), `textDocument/documentSymbol` (the outline, ←
//! `Symbols`), and `textDocument/codeAction` (quick-fixes ← the `Diagnostics` fix columns, applied via
//! the shared `crate::fix::fix_edits` so they match `cdz fix`). Each capability is a read of a column
//! the query engine already exposes, wired to its LSP request.

use std::collections::HashMap;

use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
#[allow(deprecated)]
// `DocumentSymbol` has a deprecated `deprecated` field we must populate (via `..default`).
use lsp_types::DocumentSymbol;
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{
    CodeActionRequest, Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, References,
    Request as _, SemanticTokensFullRequest, Shutdown,
};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, CodeActionResponse,
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, Location, MarkedString, Position, PublishDiagnosticsParams,
    Range, ReferenceParams, SemanticToken, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo,
    SymbolKind, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Uri,
    WorkDoneProgressOptions, WorkspaceEdit,
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
/// `references` (find-all-uses, backed by `UsesOf`), `completion` (scope bindings + top-level symbols,
/// backed by `ScopeAt` + `Symbols`), `documentSymbol` (the outline, backed by `Symbols`), and
/// `semanticTokens/full` (colour-by-meaning, backed by the `Highlight` query — the token legend is
/// `semantic_token_legend`).
fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        // The document outline (Ctrl-Shift-O / the breadcrumb bar) — every top-level declaration, backed
        // by the `Symbols` query.
        document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
        // Completion with no trigger characters — the client invokes it on the usual identifier typing /
        // Ctrl-Space. We return the full candidate set (locals + top-level symbols) and let the client
        // filter by the typed prefix (the standard "server offers, client filters" model).
        completion_provider: Some(CompletionOptions::default()),
        // Quick-fixes from the diagnostic fix columns — all 4 kinds (replace/wrap/insert/delete) via the
        // shared `crate::fix::fix_edits`, so a `cdz lsp` quick-fix applies IDENTICALLY to `cdz fix`.
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
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
            Completion::METHOD => {
                let (id, params) = cast_request::<Completion>(req)?;
                let result = self.completion(&params);
                self.send_response(Response::new_ok(id, result))
            }
            DocumentSymbolRequest::METHOD => {
                let (id, params) = cast_request::<DocumentSymbolRequest>(req)?;
                let result = self.document_symbol(&params);
                self.send_response(Response::new_ok(id, result))
            }
            CodeActionRequest::METHOD => {
                let (id, params) = cast_request::<CodeActionRequest>(req)?;
                let result = self.code_action(&params);
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
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        // PACKAGE path: a `file://` doc that declares imports types the cursor node against the whole
        // linked package, so an IMPORTED name's type resolves (single-buffer it would read as unknown).
        // Falls back to single-buffer when not a package or the package answer is empty.
        if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
            let open = self.open_resolver();
            if let Some(h) = package_hover_at(
                &entry_path.to_string_lossy(),
                &open,
                &doc.text,
                doc.is_ml,
                pos.position,
            ) {
                return Some(h);
            }
        }
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

    /// Answer a `textDocument/documentSymbol`: the document OUTLINE — every top-level declaration
    /// (value/function/type/effect/module), backed by the `Symbols` query. `None` when the document is
    /// not open; an empty (flat) list otherwise. Returned as the flat `DocumentSymbol` form (no nesting
    /// yet — Cadenza's top-level declarations are a flat list; nested module members are a later
    /// refinement once `Symbols` reports them hierarchically).
    fn document_symbol(&self, params: &DocumentSymbolParams) -> Option<DocumentSymbolResponse> {
        let doc = self.docs.get(&params.text_document.uri)?;
        let symbols = document_symbols_for(&doc.text, doc.is_ml);
        Some(DocumentSymbolResponse::Nested(symbols))
    }

    /// Answer a `textDocument/definition`: resolve the cursor's reference to its DEFINING occurrence
    /// (backed by the `ResolveOf` query — a def name, a `let` initializer, a parameter binder), returned
    /// as a `Location` in the same document. `None` when the document is not open, the position is not a
    /// navigable reference, or the target has no source span (a prelude/built-in binding) — total.
    fn goto_definition(&self, params: &GotoDefinitionParams) -> Option<GotoDefinitionResponse> {
        let pos = &params.text_document_position_params;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        // PACKAGE path: a `file://` doc that declares imports resolves cross-file — the target may live
        // in an imported file, so `package_definition_at` returns a `Location` in THAT file. Otherwise
        // (non-file URI, or no imports) the single-buffer `definition_at`.
        let loc =
            if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
                let open = self.open_resolver();
                package_definition_at(
                    &entry_path.to_string_lossy(),
                    &open,
                    &doc.text,
                    doc.is_ml,
                    pos.position,
                )
                .or_else(|| definition_at(&doc.text, doc.is_ml, pos.position, uri))
            } else {
                definition_at(&doc.text, doc.is_ml, pos.position, uri)
            }?;
        Some(GotoDefinitionResponse::Scalar(loc))
    }

    /// Answer a `textDocument/references`: every occurrence that references the SAME definition as the
    /// name under the cursor (backed by the `UsesOf` query, keyed by the cursor's name). Honors the
    /// client's `include_declaration` flag — when set, the definition's own name occurrence is added.
    /// `None`/empty when the cursor is not on a name or the name has no references — total.
    fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let pos = &params.text_document_position;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        let include_decl = params.context.include_declaration;
        // PACKAGE path: a `file://` doc that declares imports finds references to a TOP-LEVEL name ACROSS
        // the closure — a use of an imported/exported name is referenced from other files too, each a
        // cross-file `Location`. Falls back to single-buffer when not a package or the package yields
        // nothing (e.g. a local binder — the single-buffer path applies the shadowing guard).
        if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
            let open = self.open_resolver();
            let refs = package_references_at(
                &entry_path.to_string_lossy(),
                &open,
                &doc.text,
                doc.is_ml,
                pos.position,
                include_decl,
            );
            if !refs.is_empty() {
                return Some(refs);
            }
        }
        Some(references_at(
            &doc.text,
            doc.is_ml,
            pos.position,
            uri,
            include_decl,
        ))
    }

    /// Answer a `textDocument/completion`: the names in scope at the cursor — local bindings (backed by
    /// `ScopeAt`, with their types) plus the module's top-level declarations (backed by `Symbols`),
    /// deduped by name (a local shadows a top-level of the same name). The client filters this candidate
    /// set by the typed prefix. `None` when the document is not open; an empty list otherwise (total).
    fn completion(&self, params: &CompletionParams) -> Option<CompletionResponse> {
        let pos = &params.text_document_position;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        // A `file://` document with imports completes over its package closure, so an IMPORTED name is
        // offered (it appears in neither single-buffer source); everything else is single-buffer.
        let items =
            if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
                let open = self.open_resolver();
                package_completions_at(
                    &entry_path.to_string_lossy(),
                    &open,
                    &doc.text,
                    doc.is_ml,
                    pos.position,
                )
                // A closure-load failure → the single-buffer candidates, still total.
                .unwrap_or_else(|| completions_at(&doc.text, doc.is_ml, pos.position))
            } else {
                completions_at(&doc.text, doc.is_ml, pos.position)
            };
        Some(CompletionResponse::Array(items))
    }

    /// Answer a `textDocument/codeAction`: quick-fixes for the diagnostics OVERLAPPING the request range,
    /// built from the `Diagnostics` query's structured fix columns via the SHARED `crate::fix::fix_edits`
    /// (all four kinds — replace/wrap/insert/delete — so a `cdz lsp` quick-fix applies IDENTICALLY to
    /// `cdz fix`). `None` when the document is not open; an empty list when no fix applies in range —
    /// total.
    fn code_action(&self, params: &CodeActionParams) -> Option<CodeActionResponse> {
        let uri = &params.text_document.uri;
        let doc = self.docs.get(uri)?;
        Some(code_actions_at(&doc.text, doc.is_ml, uri, params.range))
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
            Some(doc) => self.compute_diagnostics(uri, doc),
            None => Vec::new(),
        };
        self.send_diagnostics(uri, diags)
    }

    /// The diagnostics for one open document. When the document (a) has a filesystem path and (b)
    /// declares `(import …)`, analyze it as a PACKAGE — follow its import closure so cross-file names
    /// resolve — and return only THIS file's faults (an imported file's faults belong to its own
    /// document). An imported file that is itself OPEN contributes its live (unsaved) buffer text via the
    /// overlay, so editing a library is reflected immediately in its importer's diagnostics. Otherwise
    /// (a non-`file` URI like an untitled buffer, or a document with no imports) fall back to the
    /// single-buffer `diagnostics_for`.
    fn compute_diagnostics(&self, uri: &Uri, doc: &Document) -> Vec<Diagnostic> {
        // Only a `file://` document with imports takes the package path; everything else is single-buffer.
        let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) else {
            return diagnostics_for(&doc.text, doc.is_ml);
        };
        let open = self.open_resolver();
        package_diagnostics_for(&entry_path.to_string_lossy(), &open)
            // A closure-load failure (e.g. the entry path vanished) → the single-buffer path, still total.
            .unwrap_or_else(|| diagnostics_for(&doc.text, doc.is_ml))
    }

    /// Whether `doc` declares any `(import …)` — the gate for the package (cross-file) analysis path.
    fn doc_declares_import(&self, doc: &Document) -> bool {
        match parse_surface(&doc.text, doc.is_ml) {
            Ok((arenas, _spans, _errs)) => {
                !crate::closure::declared_import_paths(&arenas).is_empty()
            }
            Err(_) => false,
        }
    }

    /// The closure's source-overlay resolver: given an imported file's path, return an OPEN buffer's
    /// live (unsaved) text for it (matched by `uri_to_path`), else `None` → the closure reads disk. Lets
    /// unsaved edits to a library flow into its importer's cross-file analysis.
    fn open_resolver(&self) -> impl Fn(&std::path::Path) -> Option<String> + '_ {
        move |p: &std::path::Path| {
            self.docs
                .iter()
                .find_map(|(u, d)| (uri_to_path(u).as_deref() == Some(p)).then(|| d.text.clone()))
        }
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

/// The local filesystem path of a `file://` document URI, or `None` for a non-`file` scheme (an
/// untitled/in-memory buffer, a remote URI). `lsp-types` 0.97's `Uri` carries no `to_file_path`, so we
/// parse the `file://[host]/path` form ourselves and percent-decode the path. This is the URI→path
/// bridge the workspace `(import …)` closure needs (an imported sibling is resolved relative to the
/// entry file's directory). A `file://` with a non-empty host (a UNC share) is not handled — returns the
/// path portion as-is, which is correct for the common `file:///abs/path` (empty host) case. Used by
/// `compute_diagnostics` to find a document's on-disk path (the closure entry) + match open imported
/// siblings for the overlay.
fn uri_to_path(uri: &Uri) -> Option<std::path::PathBuf> {
    let s = uri.as_str();
    let rest = s.strip_prefix("file://")?;
    // `file:///abs` → rest = `/abs` (empty host); `file://host/abs` → rest = `host/abs`. Take from the
    // first `/` so a host is dropped and an absolute path is preserved.
    let path_part = match rest.find('/') {
        Some(i) => &rest[i..],
        None => rest, // no slash — treat the whole remainder as the path (unusual)
    };
    Some(std::path::PathBuf::from(percent_decode(path_part)))
}

/// Percent-decode a URI path component (`%20` → space, `%2F` → `/`, …). Minimal, dependency-free — just
/// what a `file://` path needs. A malformed `%`-escape (not two hex digits) is left verbatim.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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

/// The PACKAGE diagnostics for the entry file at `entry_path` — follow its `(import …)` closure (reusing
/// `crate::closure::load`, the SAME loader `cdz check` uses), run `Diagnostics` over the whole spliced
/// package, and return only the faults belonging to the ENTRY file (an imported file's faults belong to
/// its own document, published when that document is analyzed). `open(path)` supplies an open buffer's
/// live text for an imported sibling (the overlay), else the closure reads disk. `None` when the closure
/// can't be loaded (the caller falls back to single-buffer). Cross-file node ids are demuxed to the
/// entry via the compiler's `link-map` (`FileSpan` ranges) — the same demux `run_check`'s package path
/// uses — so a fault reported at a global node id maps back to the entry file's local span.
fn package_diagnostics_for(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
) -> Option<Vec<Diagnostic>> {
    let files = crate::closure::load(entry_path, open).ok()?;
    // Splice every closure file's AST + a Diagnostics request + the entry marker — exactly `run_check`'s
    // package build, so the compiler links the package and resolves cross-file names.
    let mut inputs: Vec<rcdzc::Artifact> = files
        .iter()
        .map(|f| {
            rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::Diagnostics)]),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let bytes = compiled.artifact(rcdzc::sidecar::KIND_DIAGNOSTICS)?;
    let diag_text = String::from_utf8_lossy(bytes);

    // Demux a global node id to `(file index, local id)` via the link-map. Empty map (a lone importing
    // file) → the entry with id unchanged.
    let link_map = compiled
        .artifact(rcdzc::link::KIND_LINK_MAP)
        .map(rcdzc::link::decode_link_map)
        .unwrap_or_default();
    let file_of_node = |n: u32| -> Option<usize> {
        if link_map.is_empty() {
            return Some(0);
        }
        let fs = link_map
            .iter()
            .find(|fs| n >= fs.struct_base && n < fs.struct_base + fs.struct_count)?;
        files.iter().position(|f| f.name == fs.path)
    };

    // The entry file is `files[0]`; keep only faults whose node maps to it, mapped through ITS spans.
    let entry = &files[0];
    let mut out = Vec::new();
    for line in diag_text.lines() {
        // Reuse the shared fault-line parser, but first restrict to the entry file's faults and rewrite
        // the node column to the entry-LOCAL id so `parse_diag_line` resolves it in the entry's spans.
        let Some((local_line, on_entry)) =
            rewrite_to_entry_local(line, &link_map, &files, &file_of_node)
        else {
            continue;
        };
        if !on_entry {
            continue; // a fault in an imported file — published when that document is analyzed.
        }
        if let Some(d) = parse_diag_line(&local_line, &entry.source, &entry.spans) {
            out.push(d);
        }
    }
    Some(out)
}

/// If a diagnostics line's node belongs to the ENTRY file, rewrite its node column to the entry-LOCAL id
/// and return `(rewritten_line, true)`; if it belongs to another file, return `(line, false)`; an
/// unanchored (`-`) node stays on the entry (`true`) so a package-level fault with no node is still
/// shown. `None` only on a malformed line.
fn rewrite_to_entry_local(
    line: &str,
    link_map: &[rcdzc::link::FileSpan],
    files: &[crate::closure::LoadedFile],
    file_of_node: &dyn Fn(u32) -> Option<usize>,
) -> Option<(String, bool)> {
    // The node id is the THIRD tab column (`severity  code  node  …`).
    let mut cols: Vec<&str> = line.splitn(8, '\t').collect();
    if cols.len() < 8 {
        return None;
    }
    let node = cols[2];
    let Ok(global) = node.parse::<u32>() else {
        // Unanchored (`-`) or non-numeric — treat as an entry-level fault, line unchanged.
        return Some((line.to_string(), true));
    };
    match file_of_node(global) {
        Some(0) => {
            // Entry file: rewrite the global id to the entry-local id (base 0 when no link-map).
            let local = if link_map.is_empty() {
                global
            } else {
                match link_map.iter().find(|fs| fs.path == files[0].name) {
                    Some(fs) => global - fs.struct_base,
                    None => global,
                }
            };
            let local_s = local.to_string();
            cols[2] = &local_s;
            Some((cols.join("\t"), true))
        }
        _ => Some((line.to_string(), false)), // another file, or unmapped — not the entry's fault
    }
}

/// Compute the LSP diagnostics for `text` on the given surface — the SINGLE-BUFFER "diagnostics as you
/// type" read (used for a document with no imports, or a non-`file` URI). Parses the buffer in-memory
/// (the ML reader RECOVERS from a syntax error, so a mid-edit buffer still yields a tree + its recovered
/// parse errors), drives the `Diagnostics` query, and maps each fault's node id to a source `Range` via
/// the span table. TOTAL: an un-analyzable buffer yields whatever partial set the recovering parse +
/// query produce, never a panic. A document that DECLARES imports is instead analyzed as a package by
/// `package_diagnostics_for` (so cross-file names resolve) — `compute_diagnostics` routes between them.
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

/// Cross-file go-to-definition: resolve the cursor's reference across the entry's `(import …)` closure,
/// so a use of an IMPORTED name jumps to its definition in the OTHER file. The entry is spliced FIRST
/// (`crate::closure::load` returns it at `files[0]`), so it gets `struct_base == 0` — the cursor's
/// entry-local node id IS the linked global query input, no offset needed. `ResolveOf` answers a global
/// target node id, which we demux via the link-map to the owning file, then build a `Location` with
/// THAT file's URI + span (a jump into an imported library). `None` when the closure can't load, the
/// position is not a navigable reference, or the target has no source span — the caller falls back to
/// the single-buffer `definition_at`.
fn package_definition_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    pos: Position,
) -> Option<Location> {
    // The cursor node in the ENTRY file (base 0 in the linked program, so no rebasing of the input).
    let (_entry_arenas, entry_spans, _e) = parse_surface(entry_text, entry_is_ml).ok()?;
    let byte = position_to_byte(entry_text, pos);
    let cursor = entry_spans.node_at_offset(byte)?;

    let files = crate::closure::load(entry_path, open).ok()?;
    let mut inputs: Vec<rcdzc::Artifact> = files
        .iter()
        .map(|f| {
            rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::ResolveOf {
            node: cursor.0, // entry-local == global (entry is base 0)
        })]),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let bytes = compiled.artifact(rcdzc::sidecar::KIND_RESOLVE)?;
    let target: u32 = String::from_utf8_lossy(bytes).trim().parse().ok()?;

    // Demux the global target to its owning file, then a Location in that file (its URI + local span).
    let link_map = compiled
        .artifact(rcdzc::link::KIND_LINK_MAP)
        .map(rcdzc::link::decode_link_map)
        .unwrap_or_default();
    let (file_ix, local) = if link_map.is_empty() {
        (0usize, target)
    } else {
        let fs = link_map
            .iter()
            .find(|fs| target >= fs.struct_base && target < fs.struct_base + fs.struct_count)?;
        (
            files.iter().position(|f| f.name == fs.path)?,
            target - fs.struct_base,
        )
    };
    let file = &files[file_ix];
    let span = file.spans.get(cadenza_syntax::StructId(local))?;
    Some(Location {
        uri: path_to_uri(&file.path)?,
        range: byte_range_to_range(&file.source, span.start, span.end),
    })
}

/// Cross-file hover: type the cursor node against the whole `(import …)` closure, so an IMPORTED name's
/// type resolves (single-buffer it reads as unknown). Like `package_definition_at`, the entry is spliced
/// FIRST (base 0), so the cursor's entry-local node id is the linked `TypeAt` query input unchanged. The
/// answer is a type STRING (no cross-file Location — a hover shows the type at the cursor's own range),
/// so this only needs the query result + the entry's span for the hover range. `None` when the closure
/// can't load or the answer is empty/`unknown` (the caller falls back to single-buffer hover).
fn package_hover_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    pos: Position,
) -> Option<Hover> {
    let (_entry_arenas, entry_spans, _e) = parse_surface(entry_text, entry_is_ml).ok()?;
    let byte = position_to_byte(entry_text, pos);
    let cursor = entry_spans.node_at_offset(byte)?;

    let files = crate::closure::load(entry_path, open).ok()?;
    let mut inputs: Vec<rcdzc::Artifact> = files
        .iter()
        .map(|f| {
            rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::TypeAt {
            node: cursor.0, // entry-local == global (entry is base 0)
        })]),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let bytes = compiled.artifact(rcdzc::sidecar::KIND_TYPE_AT)?;
    let ty = String::from_utf8_lossy(bytes);
    let ty = ty.trim();
    if ty.is_empty() || ty == "unknown" {
        return None; // let the single-buffer path try (or show nothing)
    }
    // Hover range = the cursor node's span in the ENTRY file (base 0, so the entry spans apply directly).
    let range = entry_spans
        .get(cursor)
        .map(|s| byte_range_to_range(entry_text, s.start, s.end));
    Some(Hover {
        contents: HoverContents::Scalar(MarkedString::String(ty.to_string())),
        range,
    })
}

/// A `file://` URI for a local filesystem path — the inverse of `uri_to_path`, for a cross-file
/// `Location` that points into an imported file. Percent-encodes every path byte that is not an
/// unreserved URI character (keeping `/` as the path separator), so a path containing a space, a `%`,
/// or a reserved char (`#`/`?`/…) yields a VALID, unambiguous `file://` URI. `None` for a non-absolute
/// path (a `Location` needs a resolvable URI).
///
/// 🪤 Encoding a literal `%` is LOAD-BEARING for the `uri_to_path`/`percent_decode` round-trip: the
/// decoder turns any `%XX` back into a byte, so a real path segment like `a%2Fb` MUST be emitted as
/// `a%252Fb` — else it would decode to `a/b` (a different path). Encoding only a space (the old behavior)
/// broke that and also produced invalid URIs for `#`/`?`.
fn path_to_uri(path: &str) -> Option<Uri> {
    use std::str::FromStr;
    let encoded = percent_encode_path(path);
    let s = if path.starts_with('/') {
        format!("file://{encoded}")
    } else {
        // A relative path (unusual for a closure file) — best-effort; prefix a slash so it's a valid URI.
        format!("file:///{encoded}")
    };
    Uri::from_str(&s).ok()
}

/// Percent-encode a filesystem path for the path component of a `file://` URI: keep `/` (the separator)
/// and the RFC 3986 unreserved set (`A-Z a-z 0-9 - . _ ~`) verbatim, `%XX`-encode every other byte
/// (space, `%`, `#`, `?`, and any non-ASCII UTF-8 byte). The exact inverse of [`percent_decode`] over the
/// path bytes (decode∘encode is the identity), so a cross-file `Location`'s URI round-trips back to the
/// same path via `uri_to_path`.
fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(
                char::from_digit((b >> 4) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((b & 0xf) as u32, 16)
                    .unwrap()
                    .to_ascii_uppercase(),
            );
        }
    }
    out
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

    // SHADOWING GUARD. `UsesOf{name}` indexes only references to the TOP-LEVEL def/type of `name` — it
    // is NAME-keyed, not node-keyed. So if the cursor is on a LOCAL binder (a parameter, a `let`, a
    // match binder) that SHADOWS a top-level symbol of the same spelling, a bare `UsesOf` would wrongly
    // return the unrelated top-level's references. Only proceed when the cursor genuinely belongs to the
    // top-level symbol: either it IS that symbol's declaration name occurrence, or it RESOLVES
    // (`ResolveOf`) to it. Otherwise return empty (a node-keyed local-uses query is a later increment).
    let top_node = top_level_symbol_node(&arenas, &name);
    let resolves_to = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::ResolveOf { node: node.0 },
        rcdzc::sidecar::KIND_RESOLVE,
    )
    .and_then(|a| a.trim().parse::<u32>().ok());
    let cursor_is_top_level =
        top_node == Some(node.0) || (top_node.is_some() && resolves_to == top_node);
    if !cursor_is_top_level {
        return Vec::new();
    }

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

    // Optionally include the DECLARATION site — the top-level symbol's own name occurrence (`UsesOf`
    // excludes the defining occurrence by design). We already confirmed the cursor belongs to that
    // top-level symbol, so its declaration node is `top_node` (falling back to the `ResolveOf` target
    // when the cursor is a reference rather than the declaration itself).
    if include_declaration {
        let decl = top_node.or(resolves_to);
        if let Some(target) = decl
            && let Some(loc) = node_location(text, &spans, uri, target)
            && !locations.contains(&loc)
        {
            locations.push(loc);
        }
    }

    locations
}

/// The node id of the TOP-LEVEL declaration named `name` (its name occurrence), or `None` if no
/// top-level symbol has that name. Reads the `Symbols` query (whose third column IS that name node) —
/// the same authority `Symbols`/`Exports` use, so a name that names no top-level declaration (a purely
/// local binder) yields `None`, which is exactly what the shadowing guard needs.
fn top_level_symbol_node(arenas: &cadenza_syntax::Arenas, name: &str) -> Option<u32> {
    let answer = run_query_text(
        arenas,
        rcdzc::sidecar::Query::Symbols,
        rcdzc::sidecar::KIND_SYMBOLS,
    )?;
    for line in answer.lines() {
        // `name<TAB>kind<TAB>name-node-id`
        let mut cols = line.splitn(3, '\t');
        if let (Some(n), Some(_kind), Some(node)) = (cols.next(), cols.next(), cols.next())
            && n == name
            && let Ok(id) = node.trim().parse::<u32>()
        {
            return Some(id);
        }
    }
    None
}

/// Cross-file find-references: every occurrence of the TOP-LEVEL name under the cursor, ACROSS the
/// `(import …)` closure — a use in an importing/imported file is a cross-file `Location`. Like def/hover,
/// the entry is spliced FIRST (base 0), so the cursor's entry-local node id feeds `ResolveOf`/`UsesOf`
/// directly; `UsesOf` answers GLOBAL node ids across all files, each demuxed via the link-map to its
/// owning file's URI + span. The shadowing guard runs against the PACKAGE `Symbols`/`ResolveOf` (a local
/// binder shadowing a top-level name → empty, single-buffer fallback applies). Empty when not a
/// navigable top-level name or the closure can't load (caller falls back to single-buffer).
fn package_references_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    pos: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let Ok((entry_arenas, entry_spans, _e)) = parse_surface(entry_text, entry_is_ml) else {
        return Vec::new();
    };
    let byte = position_to_byte(entry_text, pos);
    let Some(cursor) = entry_spans.node_at_offset(byte) else {
        return Vec::new();
    };
    // `UsesOf` is by NAME — the cursor must be a name atom.
    let Some(name) = entry_arenas.as_name(cursor).map(str::to_string) else {
        return Vec::new();
    };

    let Ok(files) = crate::closure::load(entry_path, open) else {
        return Vec::new();
    };
    // Build the spliced package inputs (entry first = base 0) — reused for all three queries below.
    let ast_inputs: Vec<rcdzc::Artifact> = files
        .iter()
        .map(|f| {
            rcdzc::Artifact::new(
                rcdzc::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    let run_query = |query: rcdzc::sidecar::Query, kind: &str| -> Option<String> {
        let mut inputs = ast_inputs.clone();
        inputs.push(rcdzc::Artifact::new(
            rcdzc::sidecar::KIND_SIDECAR,
            "drive",
            rcdzc::sidecar::encode(&[rcdzc::Request::Query(query)]),
        ));
        inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
        let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
        compiled
            .artifact(kind)
            .map(|b| String::from_utf8_lossy(b).into_owned())
    };

    // SHADOWING GUARD (package flavor). `UsesOf` is NAME-keyed, so a LOCAL binder shadowing a top-level
    // name would leak the top-level's refs. Proceed only when the cursor genuinely belongs to a
    // top-level symbol — either it IS one of the package's top-level declaration name-nodes, or it
    // RESOLVES to one. The authority is the PACKAGE `Symbols` query (global node ids, and it lists
    // IMPORTED defs too — an imported `helper`'s def appears with the id `ResolveOf` returns), run over
    // the same linked program so the ids line up. A purely-local binder resolves to its OWN occurrence,
    // which is NEVER a `Symbols` node — so it fails the guard and returns empty (the single-buffer path,
    // with its own guard, handles the local). This is the package twin of `references_at`'s guard.
    //
    // 🪤 The earlier `resolves_to.is_some()` test was too permissive: `ResolveOf` succeeds for a LOCAL
    // binder too (it resolves to itself), so a cursor on a shadowing local passed the guard and leaked
    // the top-level's uses. Requiring the resolve TARGET to be a `Symbols` node is what distinguishes a
    // genuine top-level from a shadowing local.
    let symbol_nodes: std::collections::HashSet<u32> =
        run_query(rcdzc::sidecar::Query::Symbols, rcdzc::sidecar::KIND_SYMBOLS)
            .map(|answer| {
                answer
                    .lines()
                    .filter_map(|line| {
                        line.rsplit('\t')
                            .next()
                            .and_then(|c| c.trim().parse::<u32>().ok())
                    })
                    .collect()
            })
            .unwrap_or_default();
    let resolves_to = run_query(
        rcdzc::sidecar::Query::ResolveOf { node: cursor.0 },
        rcdzc::sidecar::KIND_RESOLVE,
    )
    .and_then(|a| a.trim().parse::<u32>().ok());
    // The cursor belongs to a top-level symbol iff it IS a `Symbols` name-node (a declaration occurrence,
    // entry-local == global at base 0) or RESOLVES to one (a use of a top-level/imported name).
    let cursor_is_symbol = symbol_nodes.contains(&cursor.0);
    let resolves_to_symbol = resolves_to.is_some_and(|t| symbol_nodes.contains(&t));
    if !(cursor_is_symbol || resolves_to_symbol) {
        return Vec::new();
    }
    // The entry-local declaration node (base 0) for the include-declaration fallback below — `None` when
    // the top-level symbol is defined in an imported file (then `resolves_to` carries its global id).
    let top_node = top_level_symbol_node(&entry_arenas, &name);

    let files_ref = &files;
    let mut out: Vec<Location> = Vec::new();

    // Run UsesOf and capture the link-map from the SAME compile (so the demux matches the ids).
    let mut inputs = ast_inputs.clone();
    inputs.push(rcdzc::Artifact::new(
        rcdzc::sidecar::KIND_SIDECAR,
        "drive",
        rcdzc::sidecar::encode(&[rcdzc::Request::Query(rcdzc::sidecar::Query::UsesOf {
            name: name.clone(),
        })]),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let link_map = compiled
        .artifact(rcdzc::link::KIND_LINK_MAP)
        .map(rcdzc::link::decode_link_map)
        .unwrap_or_default();
    let loc_of_global = |global: u32| -> Option<Location> {
        let (file_ix, local) = if link_map.is_empty() {
            (0usize, global)
        } else {
            let fs = link_map
                .iter()
                .find(|fs| global >= fs.struct_base && global < fs.struct_base + fs.struct_count)?;
            (
                files_ref.iter().position(|f| f.name == fs.path)?,
                global - fs.struct_base,
            )
        };
        let file = &files_ref[file_ix];
        let span = file.spans.get(cadenza_syntax::StructId(local))?;
        Some(Location {
            uri: path_to_uri(&file.path)?,
            range: byte_range_to_range(&file.source, span.start, span.end),
        })
    };
    if let Some(bytes) = compiled.artifact(rcdzc::sidecar::KIND_USES) {
        for line in String::from_utf8_lossy(bytes).lines() {
            if let Ok(global) = line.trim().parse::<u32>()
                && let Some(loc) = loc_of_global(global)
            {
                out.push(loc);
            }
        }
    }
    // include_declaration: add the def's own name occurrence (UsesOf excludes it). Its global id is the
    // `resolves_to` target (or the entry-local `top_node` when the decl is in the entry, base 0).
    if include_declaration {
        let decl_global = resolves_to.or(top_node);
        if let Some(g) = decl_global
            && let Some(loc) = loc_of_global(g)
            && !out.contains(&loc)
        {
            out.push(loc);
        }
    }
    out
}

// ── the analysis: cursor → completion candidates, via the `ScopeAt` + `Symbols` queries ──────────────

/// Compute the completion candidates at `pos` — the names available to type there. Two sources, both
/// reads of columns the compiler fills: LOCAL bindings in scope (the `ScopeAt` query, keyed by the
/// cursor node — `let`s, parameters, match binders, each with its inferred type as the item detail),
/// and the module's TOP-LEVEL declarations (the `Symbols` query — defs / types / effects / modules).
///
/// Deduped by name — a local binding SHADOWS a top-level of the same name (the local wins), matching how
/// resolution itself would bind the name. The client filters this set by the prefix the user has typed.
/// TOTAL: never panics. On the ML surface the reader RECOVERS, so a mid-edit buffer still yields a
/// partial candidate set from its recovered tree; on the s-expr surface a buffer that does not parse
/// hard-fails, and completions are then EMPTY (there is no recovered tree to read scope/symbols from).
fn completions_at(text: &str, is_ml: bool, pos: Position) -> Vec<CompletionItem> {
    // A name → item map; INSERT top-level first, then locals OVERWRITE (a local shadows a top-level).
    let mut items: std::collections::BTreeMap<String, CompletionItem> =
        std::collections::BTreeMap::new();
    fill_completions_from(text, is_ml, pos, &mut items);
    items.into_values().collect()
}

/// Fill `items` with the candidates the SINGLE buffer `text` offers at `pos` — the module's top-level
/// declarations (`Symbols`) then the locals in scope (`ScopeAt`, which OVERWRITE to shadow a top-level of
/// the same name). Factored out of [`completions_at`] so [`package_completions_at`] can seed the same map
/// with the entry's own candidates before layering the imported names on top. A buffer that does not
/// parse contributes nothing (leaves `items` untouched).
fn fill_completions_from(
    text: &str,
    is_ml: bool,
    pos: Position,
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
) {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return;
    };

    // Top-level declarations (name<TAB>kind<TAB>node) — kind ∈ value/function/type/effect/module.
    if let Some(answer) = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::Symbols,
        rcdzc::sidecar::KIND_SYMBOLS,
    ) {
        for line in answer.lines() {
            let mut cols = line.splitn(3, '\t');
            if let (Some(name), Some(kind)) = (cols.next(), cols.next()) {
                items.insert(
                    name.to_string(),
                    CompletionItem {
                        label: name.to_string(),
                        kind: Some(symbol_kind_to_completion_kind(kind)),
                        detail: Some(kind.to_string()),
                        ..Default::default()
                    },
                );
            }
        }
    }

    // Local bindings in scope at the cursor (name<TAB>type<TAB>binder) — overwrite to shadow a top-level.
    let byte = position_to_byte(text, pos);
    if let Some(node) = spans.node_at_offset(byte)
        && let Some(answer) = run_query_text(
            &arenas,
            rcdzc::sidecar::Query::ScopeAt { node: node.0 },
            rcdzc::sidecar::KIND_SCOPE,
        )
    {
        for line in answer.lines() {
            let mut cols = line.splitn(3, '\t');
            if let (Some(name), Some(ty)) = (cols.next(), cols.next()) {
                items.insert(
                    name.to_string(),
                    CompletionItem {
                        label: name.to_string(),
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some(ty.to_string()),
                        ..Default::default()
                    },
                );
            }
        }
    }
}

/// Cross-file completion: the entry's own single-buffer candidates PLUS the names each `(import "lib"
/// (name…))` clause brings into scope. Imported names appear in neither single-buffer source — `Symbols`
/// lists only the entry's own declarations and `ScopeAt` walks only lexical binders (not the import
/// clause) — so a project buffer would otherwise never complete an imported name. For each imported name
/// we read the LIBRARY's `Exports` (for the type detail) and `Symbols` (for the kind), so the item looks
/// exactly like a locally-declared one. An imported name does NOT overwrite an entry-local of the same
/// spelling already in the map (the local binding wins, matching resolution). `None` when the closure
/// can't load (the caller falls back to the single-buffer path).
fn package_completions_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    pos: Position,
) -> Option<Vec<CompletionItem>> {
    let files = crate::closure::load(entry_path, open).ok()?;
    let mut items: std::collections::BTreeMap<String, CompletionItem> =
        std::collections::BTreeMap::new();
    // The entry's own candidates first (top-level decls + locals in scope at the cursor).
    fill_completions_from(entry_text, entry_is_ml, pos, &mut items);

    // The entry's imports: `(import "lib" (name…))` → (lib package name, [imported name]).
    let (entry_arenas, _entry_spans, _e) = parse_surface(entry_text, entry_is_ml).ok()?;
    for (lib_name, imported) in imported_names(&entry_arenas) {
        // Locate the library file in the loaded closure (skip an unresolved import — the compiler
        // reports it; here it simply contributes no completions).
        let Some(lib) = files.iter().find(|f| f.name == lib_name) else {
            continue;
        };
        // The library's exported types (name → type) and symbol kinds (name → kind), each a single-file
        // query over the library's OWN arenas — no linking needed for these per-file facts.
        let types = query_columns(
            &lib.arenas,
            rcdzc::sidecar::Query::Exports,
            rcdzc::sidecar::KIND_EXPORTS,
        );
        let kinds = query_columns(
            &lib.arenas,
            rcdzc::sidecar::Query::Symbols,
            rcdzc::sidecar::KIND_SYMBOLS,
        );
        for name in imported {
            // A local binder of the same spelling already won — don't shadow it with the import.
            if items.contains_key(&name) {
                continue;
            }
            let kind = kinds
                .get(&name)
                .map(|k| symbol_kind_to_completion_kind(k))
                .unwrap_or(CompletionItemKind::CONSTANT);
            let detail = types.get(&name).cloned();
            items.insert(
                name.clone(),
                CompletionItem {
                    label: name.clone(),
                    kind: Some(kind),
                    detail,
                    ..Default::default()
                },
            );
        }
    }
    Some(items.into_values().collect())
}

/// The names each root-level `(import "path" (name…))` clause brings into scope, grouped by the imported
/// PACKAGE name (the `"path"` string). Mirrors [`closure::declared_import_paths`]'s arena walk (peel a
/// leading comment/doc wrapper, scan a `(do …)` root's children or a lone root form) but also reads the
/// clause's name-list. Only the named-list form contributes names; the alias form `(import "path" alias)`
/// (a bare-atom spec) is a later phase and yields none. Malformed clauses are skipped (total).
fn imported_names(arenas: &cadenza_syntax::Arenas) -> Vec<(String, Vec<String>)> {
    let root = crate::unwrap_comment(arenas, arenas.root);
    let items: Vec<cadenza_syntax::StructId> = match arenas.as_form(root, "do") {
        Some(tail) => tail.to_vec(),
        None => vec![root],
    };
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for item in items {
        let item = crate::unwrap_comment(arenas, item);
        let Some(tail) = arenas.as_form(item, "import") else {
            continue;
        };
        let (Some(&path_id), Some(&spec_id)) = (tail.first(), tail.get(1)) else {
            continue;
        };
        let Some(path) = arenas.as_str(path_id) else {
            continue;
        };
        // The name spec must be a `(name…)` LIST; a bare-atom spec is the alias form (no bound names).
        let cadenza_syntax::ast::Struct::List(names) = arenas.get(spec_id) else {
            continue;
        };
        let bound: Vec<String> = names
            .iter()
            .filter_map(|&n| arenas.as_name(n).map(str::to_string))
            .collect();
        if !bound.is_empty() {
            out.push((path.to_string(), bound));
        }
    }
    out
}

/// Run a `name<TAB>value<TAB>…` query over `arenas` and collect the first two columns into a `name →
/// value` map (later lines win on a duplicate key, matching insertion). Used to read a library's
/// `Exports` (name → type) and `Symbols` (name → kind) as lookup tables. Empty on a query with no answer.
fn query_columns(
    arenas: &cadenza_syntax::Arenas,
    query: rcdzc::sidecar::Query,
    kind: &str,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(answer) = run_query_text(arenas, query, kind) {
        for line in answer.lines() {
            let mut cols = line.splitn(3, '\t');
            if let (Some(name), Some(value)) = (cols.next(), cols.next()) {
                map.insert(name.to_string(), value.to_string());
            }
        }
    }
    map
}

/// Map a `Symbols`-query kind spelling to the LSP `CompletionItemKind`. `value`→CONSTANT, `function`→
/// FUNCTION, `type`→CLASS (LSP has no "sum type"; CLASS is the nearest a theme colours as a type),
/// `effect`→EVENT, `module`→MODULE. An unknown kind → TEXT (a plain candidate).
fn symbol_kind_to_completion_kind(kind: &str) -> CompletionItemKind {
    match kind {
        "value" => CompletionItemKind::CONSTANT,
        "function" => CompletionItemKind::FUNCTION,
        "type" => CompletionItemKind::CLASS,
        "effect" => CompletionItemKind::EVENT,
        "module" => CompletionItemKind::MODULE,
        _ => CompletionItemKind::TEXT,
    }
}

// ── the analysis: source text → LSP document symbols (the outline), via the `Symbols` query ──────────

/// Compute the document outline for `text` — every TOP-LEVEL declaration (value/function/type/effect/
/// module), backed by the `Symbols` query. Each declaration becomes a `DocumentSymbol` with a
/// `SymbolKind`, its NAME occurrence's source range, and the same range as the selection range (there
/// is no separate "full body" span in the query yet, so range == selection_range — an editor still
/// navigates + highlights the name). Flat (no children): Cadenza's top-level declarations are a flat
/// list; nesting module members under their module is a later refinement once `Symbols` reports the
/// hierarchy. TOTAL: an un-analyzable buffer yields whatever partial set the query produces, never a
/// panic.
fn document_symbols_for(text: &str, is_ml: bool) -> Vec<DocumentSymbol> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let Some(answer) = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::Symbols,
        rcdzc::sidecar::KIND_SYMBOLS,
    ) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in answer.lines() {
        // `name<TAB>kind<TAB>name-node-id`
        let mut cols = line.splitn(3, '\t');
        let (Some(name), Some(kind), Some(node)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let Some(range) = node
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(|id| spans.get(cadenza_syntax::StructId(id)))
            .map(|s| byte_range_to_range(text, s.start, s.end))
        else {
            continue;
        };
        #[allow(deprecated)]
        // the `deprecated` field is deprecated but non-optional in this lsp-types.
        out.push(DocumentSymbol {
            name: name.to_string(),
            detail: Some(kind.to_string()),
            kind: symbol_kind_to_document_kind(kind),
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children: None,
        });
    }
    out
}

/// Map a `Symbols`-query kind spelling to the LSP `SymbolKind` for the outline. `value`→CONSTANT,
/// `function`→FUNCTION, `type`→ENUM (a Cadenza sum is a set of variants — ENUM is the closest LSP
/// icon), `effect`→EVENT, `module`→MODULE. An unknown kind → VARIABLE (a neutral fallback).
fn symbol_kind_to_document_kind(kind: &str) -> SymbolKind {
    match kind {
        "value" => SymbolKind::CONSTANT,
        "function" => SymbolKind::FUNCTION,
        "type" => SymbolKind::ENUM,
        "effect" => SymbolKind::EVENT,
        "module" => SymbolKind::MODULE,
        _ => SymbolKind::VARIABLE,
    }
}

// ── the analysis: diagnostics-in-range → quick-fix code actions, via `crate::fix::fix_edits` ─────────

/// Compute the quick-fix code actions available in `range` — one per diagnostic (whose fault overlaps
/// the request range) that carries a structured fix. Runs the `Diagnostics` query (the SAME read that
/// produces the squiggles), reads each fault's fix columns (`fix-kind`, `fix-node`, `fix-repl`,
/// `fix-verified`), and turns the fix into a `WorkspaceEdit` via `crate::fix::fix_edits` — the SHARED
/// builder `cdz fix`/`cdz check --json` use, so a `cdz lsp` quick-fix produces byte-IDENTICAL edits and
/// covers all four kinds (replace/wrap/insert/delete). A verified fix is marked `isPreferred`. TOTAL: an
/// un-analyzable buffer, or a fix that fails to build, yields no action for that fault — never a panic.
#[allow(clippy::mutable_key_type)] // `WorkspaceEdit.changes` is the LSP-mandated `HashMap<Uri, _>`.
fn code_actions_at(text: &str, is_ml: bool, uri: &Uri, range: Range) -> Vec<CodeActionOrCommand> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let Some(diag_text) = run_query_text(
        &arenas,
        rcdzc::sidecar::Query::Diagnostics,
        rcdzc::sidecar::KIND_DIAGNOSTICS,
    ) else {
        return Vec::new();
    };

    // The fix engine works on the homoiconic `Tree` + an origin index (built ONCE, reused per fault).
    let tree = cadenza_syntax::query::Tree::of(&arenas);
    let origins = crate::fix::OriginPaths::of(&tree);
    let surface = if is_ml {
        cadenza_syntax::convert::Format::Ml
    } else {
        cadenza_syntax::convert::Format::Sexpr
    };

    let mut actions = Vec::new();
    for line in diag_text.lines() {
        // `severity  code  node  fix-kind  fix-node  fix-repl  fix-verified  message` (8 columns).
        let mut cols = line.splitn(8, '\t');
        let (severity, code, node, fix_kind, fix_node, fix_repl, fix_verified, message) = match (
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
            cols.next(),
        ) {
            (Some(s), Some(c), Some(n), Some(fk), Some(fnode), Some(fr), Some(fv), Some(m)) => {
                (s, c, n, fk, fnode, fr, fv, m)
            }
            _ => continue,
        };
        // No fix, or the `<error>`-placeholder cascade → no action.
        if fix_kind == "-" || message.contains("`<error>`") {
            continue;
        }
        // The FAULT's own node range — used to filter to diagnostics overlapping the request range, so we
        // only offer a fix for a squiggle at/around the cursor (the client passes the cursor/selection).
        let Some(fault_range) = node
            .parse::<u32>()
            .ok()
            .and_then(|id| spans.get(cadenza_syntax::StructId(id)))
            .map(|s| byte_range_to_range(text, s.start, s.end))
        else {
            continue;
        };
        if !ranges_overlap(fault_range, range) {
            continue;
        }
        // Build the fix's primitive byte edits via the SHARED engine, then map each to an LSP TextEdit.
        let Ok(fix_target) = fix_node.parse::<u32>() else {
            continue;
        };
        let Some(edits) = crate::fix::fix_edits(
            text,
            &tree,
            &origins,
            &spans,
            fix_kind,
            cadenza_syntax::StructId(fix_target),
            fix_repl,
            surface,
        ) else {
            continue;
        };
        if edits.is_empty() {
            continue;
        }
        let text_edits: Vec<TextEdit> = edits
            .iter()
            .map(|e| TextEdit {
                range: byte_range_to_range(text, e.start, e.end),
                new_text: e.text.clone(),
            })
            .collect();

        let mut changes = HashMap::new();
        changes.insert(uri.clone(), text_edits);
        actions.push(CodeActionOrCommand::CodeAction(CodeAction {
            title: code_action_title(fix_kind, fix_repl, message),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![Diagnostic {
                range: fault_range,
                severity: Some(match severity {
                    "error" => DiagnosticSeverity::ERROR,
                    "warning" => DiagnosticSeverity::WARNING,
                    _ => DiagnosticSeverity::INFORMATION,
                }),
                code: (code != "-").then(|| lsp_types::NumberOrString::String(code.to_string())),
                source: Some("cdz".to_string()),
                message: message.to_string(),
                ..Default::default()
            }]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            // A VERIFIED fix (re-checked to clear its diagnostic without introducing new errors) is the
            // preferred one, so the editor's default quick-fix applies the trustworthy edit.
            is_preferred: Some(fix_verified == "verified"),
            ..Default::default()
        }));
    }
    actions
}

/// A human-readable code-action title. Prefers the concrete edit (`Replace with `_x``), else glosses the
/// fix kind, else falls back to the first clause of the diagnostic message.
fn code_action_title(fix_kind: &str, fix_repl: &str, message: &str) -> String {
    match fix_kind {
        "replace" if !fix_repl.is_empty() && fix_repl != "-" => {
            format!("Replace with `{fix_repl}`")
        }
        "wrap" if !fix_repl.is_empty() && fix_repl != "-" => format!("Wrap in `{fix_repl}`"),
        "insert" if !fix_repl.is_empty() && fix_repl != "-" => format!("Insert `{fix_repl}`"),
        "delete" => "Remove this element".to_string(),
        _ => {
            let short = message.split(['(', ':']).next().unwrap_or(message).trim();
            format!("Fix: {short}")
        }
    }
}

/// Whether two LSP ranges overlap (share at least a point) — the test for "this diagnostic is in the
/// code-action request range". Inclusive at the boundary so a zero-width cursor at a fault's edge counts.
fn ranges_overlap(a: Range, b: Range) -> bool {
    !(position_lt(a.end, b.start) || position_lt(b.end, a.start))
}

/// Strict less-than on LSP positions (line then character).
fn position_lt(p: Position, q: Position) -> bool {
    (p.line, p.character) < (q.line, q.character)
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
    // Sort by (line, start, length): ascending position, and at a shared start the NARROWEST token
    // first. The `Highlight` query classifies BOTH a container node and its inner leaf, which can share
    // a start position (a grouping `(` vs the binder `x` inside it) or otherwise overlap — but LSP
    // requires semantic tokens to be NON-OVERLAPPING and in ascending order, and a client seeing two
    // tokens at one position renders inconsistently (or, strictly, rejects the set). So after sorting we
    // drop any token that starts before the previously-KEPT token ends: the narrower/earlier token wins
    // (the more specific classification — a `param` over the enclosing grouping `keyword`), and each
    // painted region is claimed once. This is the token-overlap refinement the semanticTokens increment
    // deferred.
    abs.sort_by_key(|&(line, ch, len, _)| (line, ch, len));

    // Delta-encode the NON-OVERLAPPING subset: each token's line is relative to the previous token's
    // line; its start char is relative to the previous token's start when on the SAME line, else
    // absolute. `prev_end` tracks the last KEPT token's end (per line) to reject an overlapping follower.
    let mut out = Vec::with_capacity(abs.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    // The end character of the last kept token, and the line it was on — for overlap rejection.
    let mut kept_line = u32::MAX;
    let mut kept_end = 0u32;
    for (line, start, length, token_type) in abs {
        // Reject a token that OVERLAPS the previously-kept one on the same line (its start is before the
        // kept token's end). The sort put the narrowest-at-a-shared-start first, so the one we keep is
        // the most specific; a wider or duplicate token covering the same region is dropped.
        if line == kept_line && start < kept_end {
            continue;
        }
        kept_line = line;
        kept_end = start + length;
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
        // Reconstruct absolute (line, start, end) from the deltas and assert the tokens are ascending AND
        // NON-OVERLAPPING — the LSP wire-format invariant a client relies on (two tokens at one position,
        // or a token starting inside the previous one, render inconsistently / are rejected by strict
        // clients). `def double(x…)` classifies both a grouping node and its inner binder at the same
        // start; the overlap-elimination pass must leave a clean non-overlapping stream.
        assert_no_overlap(&toks);
    }

    /// Decode the delta-encoded tokens to absolute `(line, start, end)` and assert each starts at or
    /// after the previous token's end on the same line — no overlap, ascending order.
    fn assert_no_overlap(toks: &[SemanticToken]) {
        let mut line = 0u32;
        let mut start = 0u32;
        let mut prev_line = u32::MAX;
        let mut prev_end = 0u32;
        for t in toks {
            line += t.delta_line;
            start = if t.delta_line == 0 {
                start + t.delta_start
            } else {
                t.delta_start
            };
            if line == prev_line {
                assert!(
                    start >= prev_end,
                    "token at ({line},{start}) overlaps the previous token ending at {prev_end}"
                );
            }
            prev_line = line;
            prev_end = start + t.length;
        }
    }

    #[test]
    fn semantic_tokens_are_non_overlapping_when_container_and_leaf_share_a_position() {
        // The `Highlight` query classifies BOTH a grouping/container node and its inner leaf, which can
        // share a start position (a `(param…)` binder group vs the `param` inside) — the case that
        // produced overlapping/duplicate tokens before the overlap-elimination pass. Assert the emitted
        // stream is clean.
        let toks = semantic_tokens_for("def double(x: Int64) -> Int64 = x + x", true);
        assert!(!toks.is_empty());
        assert_no_overlap(&toks);
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

    #[test]
    fn references_on_a_local_shadowing_a_top_level_does_not_leak_the_top_levels_uses() {
        // `helper` is a TOP-LEVEL def AND the name of a PARAMETER of `g` that shadows it. A references
        // request on the LOCAL `helper` (the param use in g's body) must NOT return the top-level
        // `helper`'s references (the `UsesOf`-by-name bug) — the name-keyed query can't distinguish them,
        // so the shadowing guard suppresses it (empty) rather than report unrelated references.
        let text = "def helper(x: Int64) -> Int64 = x\ndef g(helper: Int64) -> Int64 = helper";
        // The `helper` param use in g's body is at line 1, col 32.
        let refs = references_at(text, true, Position::new(1, 32), &test_uri(), false);
        assert!(
            refs.is_empty(),
            "a local binder shadowing a top-level must not leak the top-level's refs, got {refs:?}"
        );
    }

    #[test]
    fn references_on_the_genuine_top_level_still_works_under_the_guard() {
        // The guard must not suppress a LEGITIMATE top-level references query: `helper` used from another
        // top-level def still finds its use.
        let text = "def helper(x: Int64) -> Int64 = x\ndef m = helper(1)";
        // The `helper` call in `m` (line 1, col 8).
        let refs = references_at(text, true, Position::new(1, 8), &test_uri(), false);
        assert_eq!(
            refs.len(),
            1,
            "the genuine top-level use should be found, got {refs:?}"
        );
        assert_eq!(refs[0].range.start.line, 1);
    }

    #[test]
    fn completion_offers_top_level_symbols() {
        // The candidate set at any point includes the module's top-level declarations, each with a kind.
        let text = "def helper(x: Int64) -> Int64 = x\ndef answer = 42\ndef main = answer";
        // Cursor in main's body (line 2, after `def main = `).
        let items = completions_at(text, true, Position::new(2, 11));
        let by_name: std::collections::HashMap<_, _> =
            items.iter().map(|i| (i.label.as_str(), i)).collect();
        assert!(by_name.contains_key("helper"), "helper missing: {items:?}");
        assert!(by_name.contains_key("answer"), "answer missing: {items:?}");
        assert_eq!(
            by_name["helper"].kind,
            Some(CompletionItemKind::FUNCTION),
            "helper is a function"
        );
        assert_eq!(
            by_name["answer"].kind,
            Some(CompletionItemKind::CONSTANT),
            "answer is a nullary value"
        );
    }

    #[test]
    fn completion_includes_local_bindings_with_their_types() {
        // Inside a function body, the parameter is a completion candidate, shown as a VARIABLE with its
        // inferred type as the detail.
        let text = "def double(x: Int64) -> Int64 = x + x";
        // Cursor at the `x + x` body (line 0, char ~32).
        let items = completions_at(text, true, Position::new(0, 33));
        let x = items.iter().find(|i| i.label == "x");
        let x =
            x.unwrap_or_else(|| panic!("param `x` should be a completion candidate: {items:?}"));
        assert_eq!(x.kind, Some(CompletionItemKind::VARIABLE));
        assert!(
            x.detail.as_deref().is_some_and(|d| d.contains("Int")),
            "the local's detail should show its type, got {:?}",
            x.detail
        );
    }

    #[test]
    fn completion_is_total_on_malformed_source() {
        // A buffer that does not parse yields a defined (possibly empty) candidate set, never a panic.
        let _ = completions_at("def (f x = (", true, Position::new(0, 5));
        let _ = completions_at("", true, Position::new(0, 0));
    }

    #[test]
    fn imported_names_reads_the_named_list_clauses() {
        // `imported_names` returns each `(import "path" (name…))` clause's package + bound names — the
        // source of the imported completion candidates. A `(do …)` root with two imports.
        let (arenas, _spans, _e) = parse_surface(
            "(do (import \"lib\" (helper twice)) (import \"more\" (extra)))",
            false,
        )
        .expect("parses");
        let imports = imported_names(&arenas);
        let by_pkg: std::collections::HashMap<_, _> =
            imports.iter().map(|(p, n)| (p.as_str(), n)).collect();
        assert_eq!(
            by_pkg.get("lib").map(|v| v.as_slice()),
            Some(["helper".to_string(), "twice".to_string()].as_slice()),
            "lib's imported names: {imports:?}"
        );
        assert_eq!(
            by_pkg.get("more").map(|v| v.as_slice()),
            Some(["extra".to_string()].as_slice()),
            "more's imported names: {imports:?}"
        );
    }

    #[test]
    fn imported_names_is_total_on_a_program_with_no_imports() {
        // A buffer with no `(import …)` yields no imported names (and never panics).
        let (arenas, _spans, _e) = parse_surface("def answer = 42", true).expect("parses");
        assert!(imported_names(&arenas).is_empty());
    }

    #[test]
    fn document_symbols_outline_every_top_level_declaration_with_its_kind() {
        // The outline lists each top-level declaration with the right SymbolKind + a navigable range.
        let text = "def answer = 42\ndef double(x: Int64) -> Int64 = x + x";
        let syms = document_symbols_for(text, true);
        let by_name: std::collections::HashMap<_, _> =
            syms.iter().map(|s| (s.name.as_str(), s)).collect();
        assert!(by_name.contains_key("answer"), "answer missing: {syms:?}");
        assert!(by_name.contains_key("double"), "double missing: {syms:?}");
        assert_eq!(by_name["answer"].kind, SymbolKind::CONSTANT);
        assert_eq!(by_name["double"].kind, SymbolKind::FUNCTION);
        // Each symbol carries a real range (its name occurrence), and range == selection_range.
        let d = by_name["double"];
        assert_eq!(d.range, d.selection_range);
        assert_eq!(d.range.start.line, 1, "double is on line 1");
    }

    #[test]
    fn document_symbols_is_total_on_malformed_source() {
        // A buffer that does not parse yields a defined (possibly empty) outline, never a panic.
        let _ = document_symbols_for("def (f x = (", true);
        let _ = document_symbols_for("", true);
    }

    /// The single code action's (title, edits), or panic.
    fn only_action(actions: &[CodeActionOrCommand]) -> (String, Vec<TextEdit>) {
        assert_eq!(
            actions.len(),
            1,
            "expected exactly one action, got {actions:?}"
        );
        let CodeActionOrCommand::CodeAction(a) = &actions[0] else {
            panic!("expected a CodeAction");
        };
        let edits = a
            .edit
            .as_ref()
            .and_then(|w| w.changes.as_ref())
            .and_then(|c| c.values().next())
            .cloned()
            .expect("a workspace edit with changes");
        (a.title.clone(), edits)
    }

    #[test]
    fn code_action_offers_a_replace_quickfix_for_an_unused_param() {
        // `def f(x) = 5` — the unused param `x` (CDZ0306) has a REPLACE fix to `_x`. A code-action over
        // the param's range offers exactly that quick-fix as a single TextEdit.
        let text = "def f(x: Int64) -> Int64 = 5";
        let at = Range::new(Position::new(0, 6), Position::new(0, 7)); // on `x`
        let (title, edits) = only_action(&code_actions_at(text, true, &test_uri(), at));
        assert!(
            title.contains("_x"),
            "title should name the replacement, got {title:?}"
        );
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].new_text, "_x");
        assert_eq!(lc(edits[0].range.start), (0, 6));
    }

    #[test]
    fn code_action_offers_a_wrap_quickfix_via_the_shared_fix_engine() {
        // A newtype-vs-inner comparison (CDZ0202) has a WRAP fix — the kind only the shared
        // `crate::fix::fix_edits` can build (multi-edit tree surgery). This is the coverage the span-only
        // version could NOT provide. The wrap inserts a `(match …` prefix + `((Mk n) n))` suffix.
        let text = "(module m (type UserId (Mk Int64)) (def (f (: u UserId)) (= u 5)) (export f))";
        let whole = Range::new(Position::new(0, 0), Position::new(0, 78));
        let (title, edits) = only_action(&code_actions_at(text, false, &test_uri(), whole));
        assert!(title.contains("Wrap"), "a wrap fix, got {title:?}");
        assert_eq!(edits.len(), 2, "wrap = a prefix + suffix edit: {edits:?}");
        let texts: Vec<&str> = edits.iter().map(|e| e.new_text.as_str()).collect();
        assert!(
            texts.iter().any(|t| t.contains("match"))
                && texts.iter().any(|t| t.contains("((Mk n) n)")),
            "the wrap fix unwraps via a match: {texts:?}"
        );
    }

    #[test]
    fn code_action_filters_to_diagnostics_in_range_and_is_total() {
        // A request far from any fixable fault offers nothing; a clean/malformed program is total.
        let text = "def f(x: Int64) -> Int64 = 5";
        let far = Range::new(Position::new(0, 27), Position::new(0, 28)); // on the `5`
        assert!(code_actions_at(text, true, &test_uri(), far).is_empty());
        let whole = Range::new(Position::new(0, 0), Position::new(0, 40));
        assert!(
            code_actions_at(
                "def double(x: Int64) -> Int64 = x + x",
                true,
                &test_uri(),
                whole
            )
            .is_empty()
        );
        let _ = code_actions_at("def (f x = (", true, &test_uri(), whole); // total, no panic
    }

    #[test]
    fn ranges_overlap_is_correct() {
        let r = |l0, c0, l1, c1| Range::new(Position::new(l0, c0), Position::new(l1, c1));
        assert!(ranges_overlap(r(0, 0, 0, 5), r(0, 3, 0, 8)), "partial");
        assert!(ranges_overlap(r(0, 0, 0, 5), r(0, 5, 0, 5)), "touching");
        assert!(!ranges_overlap(r(0, 0, 0, 5), r(0, 6, 0, 8)), "disjoint");
    }

    fn uri(s: &str) -> Uri {
        use std::str::FromStr;
        Uri::from_str(s).unwrap()
    }

    #[test]
    fn uri_to_path_handles_file_uris() {
        // The common `file:///abs/path` (empty host) → the absolute path.
        assert_eq!(
            uri_to_path(&uri("file:///home/u/prog.cdz")),
            Some(std::path::PathBuf::from("/home/u/prog.cdz"))
        );
        // A percent-encoded space in the path is decoded.
        assert_eq!(
            uri_to_path(&uri("file:///home/u/my%20prog.cdz")),
            Some(std::path::PathBuf::from("/home/u/my prog.cdz"))
        );
    }

    #[test]
    fn path_to_uri_round_trips_through_uri_to_path() {
        // `path_to_uri` (cross-file Location) is the inverse of `uri_to_path`: an absolute path → a
        // `file://` URI whose `uri_to_path` recovers the original — including a space, and the reserved
        // chars (`%`/`#`/`?`) the old space-only encoder mangled into an invalid or meaning-changed URI.
        for p in [
            "/home/u/lib.sexp",
            "/tmp/pkg/main.cdz",
            "/a b/c.sexp",
            "/tmp/weird%2Fname/lib.sexp",
            "/tmp/has#hash/lib.sexp",
            "/tmp/q?mark/lib.sexp",
        ] {
            let u = path_to_uri(p).expect("a file uri");
            assert_eq!(
                uri_to_path(&u).as_deref(),
                Some(std::path::Path::new(p)),
                "round-trip failed for {p} (uri {})",
                u.as_str()
            );
        }
    }

    #[test]
    fn uri_to_path_is_none_for_a_non_file_scheme() {
        // A non-`file` URI (untitled/in-memory buffer, remote) has no local path.
        assert_eq!(uri_to_path(&uri("untitled:Untitled-1")), None);
        assert_eq!(uri_to_path(&uri("http://example.com/x.cdz")), None);
    }

    #[test]
    fn percent_decode_handles_escapes_and_leaves_malformed_verbatim() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
        assert_eq!(percent_decode("plain"), "plain");
        // A malformed escape (`%` not followed by two hex digits) is left as-is.
        assert_eq!(percent_decode("50%"), "50%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn percent_encode_path_keeps_unreserved_and_escapes_the_rest() {
        // The path separator + the RFC 3986 unreserved set pass through; everything else is %XX.
        assert_eq!(percent_encode_path("/a/b_c.sexp"), "/a/b_c.sexp");
        assert_eq!(percent_encode_path("/a b/c"), "/a%20b/c");
        // The reserved chars that the OLD (space-only) encoder produced invalid/ambiguous URIs for:
        assert_eq!(percent_encode_path("/a#b"), "/a%23b");
        assert_eq!(percent_encode_path("/a?b"), "/a%3Fb");
        // A LITERAL `%` must be encoded (else the decoder would re-interpret a following `2F` as `/`).
        assert_eq!(percent_encode_path("/a%2Fb"), "/a%252Fb");
    }
}
