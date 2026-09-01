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
//! Capabilities: the `initialize`/`shutdown` handshake, full-document sync
//! (`didOpen`/`didChange`/`didClose`), `textDocument/publishDiagnostics` (← the `Diagnostics` query),
//! `textDocument/hover` (← the `TypeAt` query), `textDocument/semanticTokens/full` (← the `Highlight`
//! query), `textDocument/definition` (← `ResolveOf`), `textDocument/references` (← `UsesOf`),
//! `textDocument/completion` (← `ScopeAt` + `Symbols`), `textDocument/documentSymbol` (the outline, ←
//! `Symbols`), `textDocument/codeAction` (quick-fixes ← the `Diagnostics` fix columns, applied via the
//! shared `crate::fix::fix_edits` so they match `cdz fix`), and `textDocument/codeLens` (a lens above each
//! SPECIALIZED generic def listing its concrete monomorphizations, ← the `Instantiations` query). Each
//! capability is a read of a column the query engine already exposes, wired to its LSP request.
//!
//! PROJECT-AWARE: for a `file://` document that declares `(import …)`, the position/analysis features
//! (diagnostics, hover, definition, references, completion) follow the import CLOSURE — `crate::closure`
//! loads the transitive package (with an open-buffer overlay for unsaved edits) and the query runs over
//! the linked program — so a cross-file (imported) name resolves in-editor exactly as `cdz check` sees
//! it. A non-`file` URI (an untitled buffer) or a buffer with no imports takes the single-buffer path.

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
    CallHierarchyIncomingCalls, CallHierarchyOutgoingCalls, CallHierarchyPrepare,
    CodeActionRequest, CodeLensRequest, Completion, DocumentHighlightRequest,
    DocumentSymbolRequest, FoldingRangeRequest, Formatting, GotoDefinition, GotoTypeDefinition,
    HoverRequest, InlayHintRequest, References, Rename, Request as _, SelectionRangeRequest,
    SemanticTokensFullRequest, Shutdown, SignatureHelpRequest, WorkspaceSymbolRequest,
};
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyIncomingCallsParams, CallHierarchyItem,
    CallHierarchyOutgoingCall, CallHierarchyOutgoingCallsParams, CallHierarchyPrepareParams,
    CallHierarchyServerCapability, CodeAction, CodeActionKind, CodeActionOrCommand,
    CodeActionParams, CodeActionResponse, CodeLens, CodeLensOptions, CodeLensParams,
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, DocumentHighlight, DocumentHighlightKind,
    DocumentHighlightParams, DocumentSymbolParams, DocumentSymbolResponse, FoldingRange,
    FoldingRangeParams, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InlayHint,
    InlayHintKind, InlayHintLabel, InlayHintParams, Location, MarkedString, ParameterInformation,
    ParameterLabel, Position, PublishDiagnosticsParams, Range, ReferenceParams, RenameParams,
    SelectionRange, SelectionRangeParams, SemanticToken, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo,
    SignatureHelp, SignatureHelpOptions, SignatureHelpParams, SignatureInformation,
    SymbolInformation, SymbolKind, TextDocumentContentChangeEvent, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, TypeDefinitionProviderCapability, Uri, WorkDoneProgressOptions,
    WorkspaceEdit, WorkspaceSymbolParams, WorkspaceSymbolResponse,
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

/// The capabilities this server advertises: INCREMENTAL text-document sync (the client sends only the
/// changed RANGES on each edit — the server splices them into its buffer via `apply_content_change`,
/// so a keystroke ships a few bytes instead of the whole file; the recomputed analysis is byte-identical
/// to a full resend, since the spliced text equals the full document text),
/// diagnostics via `publishDiagnostics` (a push the server sends on open/change, so no explicit
/// capability flag beyond sync is required for the classic push model), `hover` (the "type at
/// cursor" read, backed by the `TypeAt` query), `definition` (go-to, backed by `ResolveOf`),
/// `references` (find-all-uses, backed by `UsesOf`), `completion` (scope bindings + top-level symbols,
/// backed by `ScopeAt` + `Symbols`), `documentSymbol` (the outline, backed by `Symbols`), and
/// `semanticTokens/full` (colour-by-meaning, backed by the `Highlight` query — the token legend is
/// `semantic_token_legend`).
fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(lsp_types::OneOf::Left(true)),
        // Go-to-TYPE-definition (jump from a value to the declaration of ITS type) — for a value whose
        // static type is a user-declared type name, jump to that `type …` decl. Backed by `TypeAt` (the
        // value's rendered type) + the `Symbols` type-name→decl-node lookup. A compound/builtin type (no
        // single user decl) declines (no jump), so it never lands somewhere wrong.
        type_definition_provider: Some(TypeDefinitionProviderCapability::Simple(true)),
        // Call hierarchy — "who calls this function" (incoming). `prepare` picks the def under the cursor;
        // `incomingCalls` finds its callers via `UsesOf`, grouped by the enclosing top-level def. (Outgoing
        // calls are a later increment.)
        call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
        references_provider: Some(lsp_types::OneOf::Left(true)),
        // Highlight every occurrence of the symbol under the cursor WITHIN the buffer (the editor's
        // subtle same-symbol highlight as you rest the caret on a name) — the single-document sibling of
        // find-references, backed by the same `UsesOf` query + shadowing guard.
        document_highlight_provider: Some(lsp_types::OneOf::Left(true)),
        // Rename a symbol across its whole scope (F2) — every reference PLUS the declaration, as a single
        // `WorkspaceEdit`. Backed by the same reference-finding path (package-aware + the same shadowing
        // guard), so a rename touches exactly what find-references would, in every file it appears.
        rename_provider: Some(lsp_types::OneOf::Left(true)),
        // The document outline (Ctrl-Shift-O / the breadcrumb bar) — every top-level declaration, backed
        // by the `Symbols` query.
        document_symbol_provider: Some(lsp_types::OneOf::Left(true)),
        // Project-wide symbol search (Ctrl-T / "Go to Symbol in Workspace") — the same `Symbols` query
        // run across every OPEN document, filtered by the query string. (An on-disk project scan of
        // unopened files is a later increment; this covers the loaded editor set.)
        workspace_symbol_provider: Some(lsp_types::OneOf::Left(true)),
        // Folding ranges — collapse each multi-line top-level form (the `(def …)`/`(type …)`/`(module
        // …)` a declaration spans). Structural, from the parse tree's top-level spans; no query needed.
        folding_range_provider: Some(lsp_types::FoldingRangeProviderCapability::Simple(true)),
        // Smart-expand selection (Ctrl+Shift+→) — from the cursor, the nested chain of enclosing syntax
        // nodes (innermost first), each the parent of the previous. Built from span containment; no query.
        selection_range_provider: Some(lsp_types::SelectionRangeProviderCapability::Simple(true)),
        // Signature help — inside a `(callee arg…)` call, show the callee's arrow type (via `TypeOf`) with
        // the argument at the cursor highlighted. Triggered on `(` (call open) and space (next arg).
        signature_help_provider: Some(SignatureHelpOptions {
            trigger_characters: Some(vec!["(".to_string(), " ".to_string()]),
            retrigger_characters: Some(vec![" ".to_string()]),
            work_done_progress_options: WorkDoneProgressOptions::default(),
        }),
        // Completion with no trigger characters — the client invokes it on the usual identifier typing /
        // Ctrl-Space. We return the full candidate set (locals + top-level symbols) and let the client
        // filter by the typed prefix (the standard "server offers, client filters" model).
        completion_provider: Some(CompletionOptions::default()),
        // Quick-fixes from the diagnostic fix columns — all 4 kinds (replace/wrap/insert/delete) via the
        // shared `crate::fix::fix_edits`, so a `cdz lsp` quick-fix applies IDENTICALLY to `cdz fix`.
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Simple(true)),
        // CodeLens above each generic/ad-hoc-polymorphic def, showing its concrete monomorphizations
        // (the `Instantiations` query — a fact NO other tool surfaces). Computed once per document (the
        // whole-program monomorphization is too costly to run per-hover), no resolve step (the query
        // yields the full title up front).
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        // Inlay hints — inline PARAMETER-NAME hints at a call site (`add(`a:`1, `b:`2)`), read from the
        // callee's DECLARED parameter names in the parse tree (not the inferred-binder type query, which is
        // still blocked). Increment 1 covers a LOCALLY-defined callee; a cross-file callee + noise
        // suppression are later increments. No resolve step (labels are computed up front).
        inlay_hint_provider: Some(lsp_types::OneOf::Left(true)),
        // Whole-document formatting (format-on-save / Shift+Alt+F) — reprints the buffer canonically in
        // its OWN surface via the same `cdz fmt` path (`convert::convert_with` same-surface), so an editor
        // format is byte-identical to the CLI. Refuses a buffer that only parses with recovered errors
        // (no half-formatted rewrite), yielding no edit.
        document_formatting_provider: Some(lsp_types::OneOf::Left(true)),
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
//= spec/capabilities/tooling-and-lsp.md#incremental-equals-batch
//# An incremental analysis result MUST equal the result a full compilation of the same source would produce.
//= spec/capabilities/tooling-and-lsp.md#incremental-equals-batch
//# An incremental analysis MUST NOT report a type, definition, or diagnostic that a full compilation would not.
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
                // A response to a request WE sent — the server initiates none, so nothing to correlate.
                Message::Response(_) => {}
            }
        }
        Ok(())
    }

    /// Dispatch a client REQUEST to the matching feature analysis, each backed by a sidecar query:
    /// `hover` (`TypeAt`), `semanticTokens/full` (`Highlight`), `definition` (`ResolveOf`), `references`
    /// (`UsesOf`), `completion` (`ScopeAt`+`Symbols`), `documentSymbol` (`Symbols`), and `codeAction`
    /// (the `Diagnostics` fix columns). An unrecognized method gets a `MethodNotFound` error so the client
    /// is not left waiting. `shutdown` is handled in `serve` via `handle_shutdown`.
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
            GotoTypeDefinition::METHOD => {
                let (id, params) = cast_request::<GotoTypeDefinition>(req)?;
                let result = self.goto_type_definition(&params);
                self.send_response(Response::new_ok(id, result))
            }
            References::METHOD => {
                let (id, params) = cast_request::<References>(req)?;
                let result = self.references(&params);
                self.send_response(Response::new_ok(id, result))
            }
            DocumentHighlightRequest::METHOD => {
                let (id, params) = cast_request::<DocumentHighlightRequest>(req)?;
                let result = self.document_highlight(&params);
                self.send_response(Response::new_ok(id, result))
            }
            Rename::METHOD => {
                let (id, params) = cast_request::<Rename>(req)?;
                let result = self.rename(&params);
                self.send_response(Response::new_ok(id, result))
            }
            WorkspaceSymbolRequest::METHOD => {
                let (id, params) = cast_request::<WorkspaceSymbolRequest>(req)?;
                let result = self.workspace_symbol(&params);
                self.send_response(Response::new_ok(id, result))
            }
            FoldingRangeRequest::METHOD => {
                let (id, params) = cast_request::<FoldingRangeRequest>(req)?;
                let result = self.folding_range(&params);
                self.send_response(Response::new_ok(id, result))
            }
            SelectionRangeRequest::METHOD => {
                let (id, params) = cast_request::<SelectionRangeRequest>(req)?;
                let result = self.selection_range(&params);
                self.send_response(Response::new_ok(id, result))
            }
            SignatureHelpRequest::METHOD => {
                let (id, params) = cast_request::<SignatureHelpRequest>(req)?;
                let result = self.signature_help(&params);
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
            CodeLensRequest::METHOD => {
                let (id, params) = cast_request::<CodeLensRequest>(req)?;
                let result = self.code_lens(&params);
                self.send_response(Response::new_ok(id, result))
            }
            InlayHintRequest::METHOD => {
                let (id, params) = cast_request::<InlayHintRequest>(req)?;
                let result = self.inlay_hint(&params);
                self.send_response(Response::new_ok(id, result))
            }
            Formatting::METHOD => {
                let (id, params) = cast_request::<Formatting>(req)?;
                let result = self.formatting(&params);
                self.send_response(Response::new_ok(id, result))
            }
            CallHierarchyPrepare::METHOD => {
                let (id, params) = cast_request::<CallHierarchyPrepare>(req)?;
                let result = self.call_hierarchy_prepare(&params);
                self.send_response(Response::new_ok(id, result))
            }
            CallHierarchyIncomingCalls::METHOD => {
                let (id, params) = cast_request::<CallHierarchyIncomingCalls>(req)?;
                let result = self.call_hierarchy_incoming(&params);
                self.send_response(Response::new_ok(id, result))
            }
            CallHierarchyOutgoingCalls::METHOD => {
                let (id, params) = cast_request::<CallHierarchyOutgoingCalls>(req)?;
                let result = self.call_hierarchy_outgoing(&params);
                self.send_response(Response::new_ok(id, result))
            }
            _ => {
                let resp = Response::new_err(
                    req.id.clone(),
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("cdz lsp: unsupported request `{}`", req.method),
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

    /// Answer a `textDocument/typeDefinition`: from the value at the cursor, jump to the declaration of its
    /// TYPE. For a value whose static type is a USER-DECLARED type name (a `type …` decl in this buffer),
    /// resolve `TypeAt` → the rendered type, and when that is a bare declared type name, jump to its `type`
    /// declaration (via the `Symbols` type-name→node lookup). Declines (no jump) for a builtin/compound
    /// type with no single user declaration — total, never a wrong landing. Single-buffer (a cross-file
    /// type-def is a later increment). `None` when the document is not open or the type is not navigable.
    /// (`GotoTypeDefinitionParams`/`Response` are type aliases of the plain go-to-definition params/
    /// response in `lsp-types`, so this reuses those types directly.)
    fn goto_type_definition(
        &self,
        params: &GotoDefinitionParams,
    ) -> Option<GotoDefinitionResponse> {
        let pos = &params.text_document_position_params;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        // PACKAGE path: a `file://` doc that declares imports types the cursor across its closure, so an
        // imported value's type resolves AND the jump can land in the declaring library. Else single-buffer.
        let loc =
            if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
                let open = self.open_resolver();
                package_type_definition_at(
                    &entry_path.to_string_lossy(),
                    &open,
                    &doc.text,
                    doc.is_ml,
                    pos.position,
                )
                .or_else(|| type_definition_at(&doc.text, doc.is_ml, pos.position, uri))
            } else {
                type_definition_at(&doc.text, doc.is_ml, pos.position, uri)
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

    /// Answer a `textDocument/documentHighlight`: every occurrence of the symbol under the cursor WITHIN
    /// THIS buffer — the editor's live same-symbol highlight (VS Code paints the caret's symbol wherever
    /// else it appears). This is the SINGLE-DOCUMENT sibling of `references`: unlike find-references it
    /// never spans files, so it uses the single-buffer `references_at` (with the declaration included, so
    /// the def site highlights too) rather than the cross-file package path. Each hit becomes a `Text`
    /// highlight (we don't distinguish read/write — Cadenza bindings are immutable, so every occurrence
    /// is a read). `None`/empty when the cursor is not on a resolvable name — total, like `references`.
    fn document_highlight(
        &self,
        params: &DocumentHighlightParams,
    ) -> Option<Vec<DocumentHighlight>> {
        let pos = &params.text_document_position_params;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        let highlights = references_at(&doc.text, doc.is_ml, pos.position, uri, true)
            .into_iter()
            .map(|loc| DocumentHighlight {
                range: loc.range,
                kind: Some(DocumentHighlightKind::TEXT),
            })
            .collect();
        Some(highlights)
    }

    /// Answer a `textDocument/rename` (F2): replace EVERY occurrence of the symbol under the cursor — all
    /// references PLUS the declaration — with `new_name`, as a single `WorkspaceEdit`. This is the WRITE
    /// counterpart of `references`: it finds exactly the same occurrences (package-aware across the file
    /// closure, under the same shadowing guard, always including the declaration since a rename must move
    /// the def too) and emits one `TextEdit` per occurrence, grouped by file URI so a cross-file rename
    /// edits every file at once. `None` when the document is not open or the cursor is not on a renamable
    /// name (a literal, an unresolved reference) — the editor then declines the rename rather than
    /// applying an empty edit.
    #[allow(clippy::mutable_key_type)] // `WorkspaceEdit.changes` is the LSP-mandated `HashMap<Uri, _>`.
    fn rename(&self, params: &RenameParams) -> Option<WorkspaceEdit> {
        let pos = &params.text_document_position;
        let uri = &pos.text_document.uri;
        let doc = self.docs.get(uri)?;
        let new_name = &params.new_name;
        // The occurrences to rewrite: the SAME set find-references returns, with the declaration included
        // (a rename must move the def site too). Package-aware first (a cross-file symbol), single-buffer
        // otherwise — mirroring the `references` handler so a rename never edits less than references shows.
        let locations =
            if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
                let open = self.open_resolver();
                let refs = package_references_at(
                    &entry_path.to_string_lossy(),
                    &open,
                    &doc.text,
                    doc.is_ml,
                    pos.position,
                    true,
                );
                if refs.is_empty() {
                    references_at(&doc.text, doc.is_ml, pos.position, uri, true)
                } else {
                    refs
                }
            } else {
                references_at(&doc.text, doc.is_ml, pos.position, uri, true)
            };
        // Not on a renamable name → decline (None), so the editor doesn't apply an empty rename.
        if locations.is_empty() {
            return None;
        }
        // One `TextEdit` per occurrence, grouped by the file the occurrence lives in (a cross-file rename
        // yields several `changes` entries; a single-buffer rename, one).
        let mut changes: HashMap<Uri, Vec<TextEdit>> = HashMap::new();
        for loc in locations {
            changes.entry(loc.uri).or_default().push(TextEdit {
                range: loc.range,
                new_text: new_name.clone(),
            });
        }
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    /// Answer a `workspace/symbol` (Ctrl-T / "Go to Symbol in Workspace"): every top-level declaration
    /// across ALL OPEN documents whose name matches the query, backed by the same `Symbols` query the
    /// document outline uses — one `SymbolInformation` per hit, each with a `Location` (the file URI + the
    /// name occurrence's range) so the editor jumps straight to it. The match is a case-insensitive
    /// SUBSTRING test (the conventional workspace-symbol filter; the client may re-rank). An empty query
    /// returns every symbol (VS Code sends "" to preload). Scoped to the loaded editor set — an on-disk
    /// scan of unopened project files is a later increment. TOTAL: an un-analyzable open buffer just
    /// contributes nothing, never a panic; result order is deterministic (documents by URI, then the
    /// query's declaration order).
    fn workspace_symbol(&self, params: &WorkspaceSymbolParams) -> Option<WorkspaceSymbolResponse> {
        let needle = params.query.to_lowercase();
        // Deterministic across docs: sort the open URIs by their string form before scanning.
        let mut uris: Vec<&Uri> = self.docs.keys().collect();
        uris.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let mut symbols: Vec<SymbolInformation> = Vec::new();
        for uri in uris {
            let doc = &self.docs[uri];
            for (name, kind, range) in top_level_symbols_of(&doc.text, doc.is_ml) {
                // Case-insensitive substring match; an empty query matches everything.
                if needle.is_empty() || name.to_lowercase().contains(&needle) {
                    #[allow(deprecated)]
                    // `deprecated` is deprecated but non-optional in this lsp-types.
                    symbols.push(SymbolInformation {
                        name,
                        kind: symbol_kind_to_document_kind(&kind),
                        tags: None,
                        deprecated: None,
                        location: Location {
                            uri: uri.clone(),
                            range,
                        },
                        container_name: None,
                    });
                }
            }
        }
        Some(WorkspaceSymbolResponse::Flat(symbols))
    }

    /// Answer a `textDocument/foldingRange`: a foldable region for each MULTI-LINE top-level form (a
    /// `(def …)`/`(type …)`/`(effect …)`/`(module …)` a declaration spans), so the editor's gutter offers
    /// a collapse toggle on each declaration. Purely structural — the top-level forms' spans from the
    /// parse tree, no query. `None` when the document is not open; an empty list when nothing spans more
    /// than one line — total, never a panic on a malformed buffer.
    fn folding_range(&self, params: &FoldingRangeParams) -> Option<Vec<FoldingRange>> {
        let doc = self.docs.get(&params.text_document.uri)?;
        Some(folding_ranges_for(&doc.text, doc.is_ml))
    }

    /// Answer a `textDocument/selectionRange`: for each requested position, the nested chain of enclosing
    /// syntax nodes (innermost node first, each the `parent` of the previous), so the editor's
    /// expand-selection (Ctrl+Shift+→) grows the selection along real syntax boundaries. Built purely from
    /// span containment (every node span covering the cursor, smallest→largest) — no query. The protocol
    /// requires one `SelectionRange` per input position, in order; a position that resolves to no node
    /// yields a degenerate empty range at that position (never fewer entries than positions). `None` when
    /// the document is not open.
    fn selection_range(&self, params: &SelectionRangeParams) -> Option<Vec<SelectionRange>> {
        let doc = self.docs.get(&params.text_document.uri)?;
        // Parse the document ONCE per request, then answer every requested position against the shared span
        // table — a multi-cursor `selectionRange` must not re-parse + re-scan per position (was
        // O(positions × parse), noticeable on large files — PR #538). If the parse fails, every position
        // gets the empty (self) range (total), matching the single-position fallback.
        match parse_surface(&doc.text, doc.is_ml) {
            Ok((_arenas, spans, _errors)) => Some(
                params
                    .positions
                    .iter()
                    .map(|&pos| selection_range_from_spans(&doc.text, &spans, pos))
                    .collect(),
            ),
            Err(_) => Some(
                params
                    .positions
                    .iter()
                    .map(|&pos| SelectionRange {
                        range: Range::new(pos, pos),
                        parent: None,
                    })
                    .collect(),
            ),
        }
    }

    /// Answer a `textDocument/signatureHelp`: inside a `(callee arg…)` call, show the CALLEE's type (its
    /// arrow signature, via the `TypeOf` query — the same authority hover uses for a def) with the argument
    /// at the cursor marked active, so the editor's signature popup tracks which parameter you're typing.
    /// `None` when the document is not open, the cursor is not inside a call whose head is a named
    /// function, or that name has no known type — the editor then shows no popup (total, never an error).
    fn signature_help(&self, params: &SignatureHelpParams) -> Option<SignatureHelp> {
        let pos = &params.text_document_position_params;
        let doc = self.docs.get(&pos.text_document.uri)?;
        signature_help_at(&doc.text, doc.is_ml, pos.position)
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

    /// Answer a `textDocument/codeLens`: a lens above each generic/ad-hoc-polymorphic definition showing
    /// its concrete monomorphizations (the `Instantiations` query). `None` when the document is not open;
    /// an empty list when the program has no specialized def — total.
    fn code_lens(&self, params: &CodeLensParams) -> Option<Vec<CodeLens>> {
        let doc = self.docs.get(&params.text_document.uri)?;
        Some(code_lenses_for(&doc.text, doc.is_ml))
    }

    /// Answer a `textDocument/inlayHint`: inline PARAMETER-NAME hints at each call site to a defined
    /// function within the requested range — `add(`a:`1, `b:`2)` renders each positional argument prefixed
    /// with the callee's declared parameter name. Read purely from the parse tree (the callee's `(def
    /// (name param…) …)` signature), so it does NOT depend on the still-blocked inferred-binder type query.
    /// A `file://` document with imports resolves an IMPORTED callee's params over its package closure
    /// (increment 2); everything else is single-buffer. `None` when the document is not open; an empty list
    /// when the range holds no matching call — total.
    fn inlay_hint(&self, params: &InlayHintParams) -> Option<Vec<InlayHint>> {
        let uri = &params.text_document.uri;
        let doc = self.docs.get(uri)?;
        let hints =
            if let Some(entry_path) = uri_to_path(uri).filter(|_| self.doc_declares_import(doc)) {
                let open = self.open_resolver();
                package_inlay_hints_at(
                    &entry_path.to_string_lossy(),
                    &open,
                    &doc.text,
                    doc.is_ml,
                    params.range,
                )
                // A closure-load failure → the single-buffer hints, still total.
                .unwrap_or_else(|| inlay_hints_at(&doc.text, doc.is_ml, params.range))
            } else {
                inlay_hints_at(&doc.text, doc.is_ml, params.range)
            };
        Some(hints)
    }

    /// Answer a `textDocument/formatting`: reprint the whole buffer canonically in its OWN surface (the
    /// same `cdz fmt` path), returned as a SINGLE full-document `TextEdit`. `None` when the document is not
    /// open, the buffer does not parse cleanly (a broken file is never rewritten to a patched-up form —
    /// matching `cdz fmt`), or it is already canonical (no edit needed). Total — never a panic.
    fn formatting(&self, params: &DocumentFormattingParams) -> Option<Vec<TextEdit>> {
        let doc = self.docs.get(&params.text_document.uri)?;
        let formatted = format_document(&doc.text, doc.is_ml)?;
        if formatted == doc.text {
            // Already canonical — no edit (the editor leaves the buffer untouched).
            return Some(Vec::new());
        }
        // Replace the ENTIRE document. The end position is one past the last line, column 0 — an
        // end-exclusive range covering every existing byte, which the client overwrites with `formatted`.
        let end = full_document_end(&doc.text);
        Some(vec![TextEdit {
            range: Range::new(Position::new(0, 0), end),
            new_text: formatted,
        }])
    }

    /// Answer a `callHierarchy/prepare`: the call-hierarchy item for the top-level definition whose NAME
    /// the cursor sits on — the anchor the client then asks incoming/outgoing calls for. `None` when the
    /// document is not open or the cursor is not on a top-level definition's name.
    fn call_hierarchy_prepare(
        &self,
        params: &CallHierarchyPrepareParams,
    ) -> Option<Vec<CallHierarchyItem>> {
        let pos = &params.text_document_position_params;
        let doc = self.docs.get(&pos.text_document.uri)?;
        let item =
            call_hierarchy_item_at(&doc.text, doc.is_ml, pos.position, &pos.text_document.uri)?;
        Some(vec![item])
    }

    /// Answer a `callHierarchy/incomingCalls`: the callers of the prepared item — every top-level def that
    /// references its name, each with the ranges of the calls. Backed by `UsesOf` (the reference index),
    /// grouped by the enclosing top-level def. `None` when the document is not open; an empty list when the
    /// definition has no callers — total.
    fn call_hierarchy_incoming(
        &self,
        params: &CallHierarchyIncomingCallsParams,
    ) -> Option<Vec<CallHierarchyIncomingCall>> {
        let doc = self.docs.get(&params.item.uri)?;
        Some(incoming_calls_for(
            &doc.text,
            doc.is_ml,
            &params.item.name,
            &params.item.uri,
        ))
    }

    /// Answer a `callHierarchy/outgoingCalls`: the callees of the prepared item — every top-level def that
    /// the item's OWN body calls, each with the ranges of those call sites (within the item). Walks the
    /// item def's body for name-headed call lists whose head names a top-level def. `None` when the document
    /// is not open; an empty list when the def calls nothing local — total.
    fn call_hierarchy_outgoing(
        &self,
        params: &CallHierarchyOutgoingCallsParams,
    ) -> Option<Vec<CallHierarchyOutgoingCall>> {
        let doc = self.docs.get(&params.item.uri)?;
        Some(outgoing_calls_for(
            &doc.text,
            doc.is_ml,
            &params.item.name,
            &params.item.uri,
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
                // Opening a LIBRARY that other open docs import must refresh their diagnostics too (its
                // live buffer now overlays the on-disk version in their package analysis).
                if let Some(path) = uri_to_path(&uri) {
                    self.republish_importers_of(&path)?;
                }
            }
            DidChangeTextDocument::METHOD => {
                let params: DidChangeTextDocumentParams = extract_note(note)?;
                let uri = params.text_document.uri;
                // INCREMENTAL sync: `content_changes` is an ORDERED list of edits; apply each to the
                // current buffer in turn (each change's range is relative to the text AFTER the prior
                // changes in the same notification, per the LSP spec). A change with no `range` is a
                // whole-document replace (a client may still send one) — `apply_content_change` handles
                // both. If the doc is somehow unknown (a change before an open), start from an empty
                // buffer so the range edits still apply against a defined base rather than being dropped.
                let is_ml = uri_is_ml(&uri);
                let mut text = self
                    .docs
                    .get(&uri)
                    .map(|d| d.text.clone())
                    .unwrap_or_default();
                for change in params.content_changes {
                    apply_content_change(&mut text, change);
                }
                self.docs.insert(uri.clone(), Document { text, is_ml });
                self.publish(&uri)?;
                // Reverse-dependency invalidation: if the edited doc is a LIBRARY, re-lint every open
                // importer so its cross-file diagnostics track the live edit (not just the lib itself).
                if let Some(path) = uri_to_path(&uri) {
                    self.republish_importers_of(&path)?;
                }
            }
            DidCloseTextDocument::METHOD => {
                let params: DidCloseTextDocumentParams = extract_note(note)?;
                let uri = params.text_document.uri;
                self.docs.remove(&uri);
                // Clear the document's diagnostics on close (an empty list), so a client does not keep
                // showing stale errors for a file no longer open.
                self.send_diagnostics(&uri, Vec::new())?;
                // Closing a LIBRARY reverts its importers to the ON-DISK version (the overlay is gone), so
                // re-lint any open importer whose analysis was using this buffer's live text.
                if let Some(path) = uri_to_path(&uri) {
                    self.republish_importers_of(&path)?;
                }
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

    /// Re-publish every OTHER open document whose import closure includes `changed` — so editing an open
    /// LIBRARY refreshes its IMPORTERS' diagnostics live (reverse-dependency invalidation). Without this,
    /// a `didChange` to a library re-lints only the library itself, leaving a stale squiggle (or a
    /// stale-clean) on an importer until the importer is itself touched. `changed` is the just-edited
    /// document's on-disk path; an open doc depends on it iff `changed` appears in that doc's TRANSITIVE
    /// import closure (`closure::load` with the open-buffer overlay), so an indirect dependency (A imports
    /// B imports the edited C) is refreshed too, not just direct importers. Best-effort + total: an open
    /// doc that has no path, or whose closure does not load / does not include `changed`, is skipped.
    fn republish_importers_of(
        &mut self,
        changed: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error + Sync + Send>> {
        // Collect the importer URIs first (an immutable borrow of `docs` via `open`), then publish (a
        // mutable borrow). The `open` resolver borrows `self.docs`, so scope it in a block that ends
        // BEFORE the publish loop — otherwise the immutable borrow would outlive the mutable one.
        let importers: Vec<Uri> = {
            let open = self.open_resolver();
            self.docs
                .iter()
                .filter(|(uri, doc)| {
                    let Some(path) = uri_to_path(uri) else {
                        return false;
                    };
                    if path == changed {
                        return false; // the changed doc itself is already (re)published by the caller
                    }
                    // PERF: a doc that declares NO `(import …)` cannot possibly depend on `changed`, so
                    // skip it BEFORE the (parse-every-file) closure load — this runs per keystroke, and
                    // `closure::load` walks + reads + parses the whole transitive closure, so paying it for
                    // every single-file buffer open in the editor would be O(open_docs × closure) per edit.
                    if !self.doc_declares_import(doc) {
                        return false;
                    }
                    // Load this open doc's TRANSITIVE closure (overlay-aware); is `changed` in it?
                    match crate::closure::load(&path.to_string_lossy(), &open) {
                        Ok(files) => files
                            .iter()
                            .any(|f| std::path::Path::new(&f.path) == changed),
                        Err(_) => false,
                    }
                })
                .map(|(uri, _)| uri.clone())
                .collect()
        };
        for uri in importers {
            self.publish(&uri)?;
        }
        Ok(())
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
    let mut inputs: Vec<cadenza_compile_abi::Artifact> = files
        .iter()
        .map(|f| {
            cadenza_compile_abi::Artifact::new(
                cadenza_compile_abi::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    inputs.push(cadenza_compile_abi::Artifact::new(
        cadenza_compile_abi::sidecar::KIND_SIDECAR,
        "drive",
        cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::Query(
            cadenza_compile_abi::sidecar::Query::Diagnostics,
        )]),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    // Single-result package query — delegated to `cdz-compile` under `!standalone`, in-process otherwise.
    let compiled =
        crate::dispatch_query_over_inputs(inputs, cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS);
    let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS) else {
        // The `Diagnostics` query produced NO artifact — the package failed to LINK before the query
        // could run (e.g. a CYCLIC import: `link()` rejects the import graph up front). The compile still
        // carries the link faults in `compiled.diagnostics`; surface THOSE (mirroring the CLI's
        // `report_errors`) instead of returning `None`, which would fall back to single-buffer analysis
        // and emit a MISLEADING "`import` is not modeled" + false "unbound name" pair rather than the
        // accurate `CDZ0201 cyclic module imports`.
        return Some(package_link_faults_as_diagnostics(&compiled, &files));
    };

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
    for d in cadenza_compile_abi::decode_diagnostics(bytes) {
        // Restrict to the entry file's faults + remap the node to the entry-LOCAL id so `parse_diag_line`
        // resolves it in the entry's spans (the KIND_DIAGNOSTICS wire is decoded structs now).
        let Some((local_d, on_entry)) =
            rewrite_to_entry_local(&d, &link_map, &files, &file_of_node)
        else {
            continue;
        };
        if !on_entry {
            continue; // a fault in an imported file — published when that document is analyzed.
        }
        if let Some(diag) = parse_diag_line(&local_d, &entry.source, &entry.spans) {
            out.push(diag);
        }
    }
    Some(out)
}

/// Surface a package compile's own error diagnostics (`compiled.diagnostics`) as LSP diagnostics on the
/// ENTRY file, for the case where the `Diagnostics` QUERY produced no artifact because the package failed
/// to LINK first (a CYCLIC import is the canonical case — `link()` rejects the import graph before the
/// query runs). Mirrors the CLI's `report_errors`, but maps each fault to a source `Range` via the entry's
/// span table. Only error-severity faults are surfaced (a link failure is an error), and only those that
/// belong to (or are unanchored on) the entry file — a fault anchored in an imported sibling is published
/// when that document is analyzed, matching the artifact path's per-file demux. An anchored node whose id
/// is out of the entry's span range (a global package id, or a sibling's) is shown at the document start
/// rather than dropped, so a real package-level fault is never silently lost.
fn package_link_faults_as_diagnostics(
    compiled: &cadenza_compile_abi::CompileOutput,
    files: &[crate::closure::LoadedFile],
) -> Vec<Diagnostic> {
    let entry = &files[0];
    compiled
        .diagnostics
        .iter()
        .filter(|d| d.severity == cadenza_compile_abi::Severity::Error)
        .map(|d| {
            let range = d
                .node
                .and_then(|id| entry.spans.get(cadenza_syntax::StructId(id)))
                .map(|s| byte_range_to_range(&entry.source, s.start, s.end))
                .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0)));
            Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                code: d.code.clone().map(lsp_types::NumberOrString::String),
                source: Some("cdz".to_string()),
                message: d.message.clone(),
                ..Default::default()
            }
        })
        .collect()
}

/// If a diagnostics line's node belongs to the ENTRY file, rewrite its node column to the entry-LOCAL id
/// and return `(rewritten_line, true)`; if it belongs to another file, return `(line, false)`; an
/// unanchored (`-`) node stays on the entry (`true`) so a package-level fault with no node is still
/// shown. `None` only on a malformed line.
fn rewrite_to_entry_local(
    d: &cadenza_compile_abi::Diagnostic,
    link_map: &[cadenza_compile_abi::FileSpan],
    files: &[crate::closure::LoadedFile],
    file_of_node: &dyn Fn(u32) -> Option<usize>,
) -> Option<(cadenza_compile_abi::Diagnostic, bool)> {
    // Remap the fault's GLOBAL node id to the entry-LOCAL id (on the decoded struct, not a tab column).
    let Some(global) = d.node else {
        // Unanchored — treat as an entry-level fault, unchanged.
        return Some((d.clone(), true));
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
            let mut local_d = d.clone();
            local_d.node = Some(local);
            Some((local_d, true))
        }
        _ => Some((d.clone(), false)), // another file, or unmapped — not the entry's fault
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
        cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::Query(
            cadenza_compile_abi::sidecar::Query::Diagnostics,
        )]);
    let inputs = vec![
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            "main",
            ast_bytes,
        ),
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            sidecar_bytes,
        ),
    ];
    // Single-result Diagnostics query — delegated to `cdz-compile` under `!standalone`, in-process otherwise.
    let compiled =
        crate::dispatch_query_over_inputs(inputs, cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS);
    if let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS) {
        // The KIND_DIAGNOSTICS wire is canonical binary AST — decode to the fault structs, no tab parsing.
        for d in cadenza_compile_abi::decode_diagnostics(bytes) {
            if let Some(diag) = parse_diag_line(&d, text, &spans) {
                out.push(diag);
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
        let (raw_arenas, raw_spans) = match cadenza_syntax::sexpr::read_spanned(text) {
            Ok(pair) => pair,
            Err(_) => match cadenza_syntax::sexpr::read_all_spanned(text) {
                Ok(pair) => pair,
                Err(e) => return Err(((0, text.len().min(1)), format!("s-expr parse: {}", e.0))),
            },
        };
        // CANONICALIZE + REMAP — the SAME step the ML branch (and the CLI loader
        // `parse_program_spanned_counted`) does, for the SAME reason: the compiler reports over
        // `codec::encode`'d (canonical) node ids, so a NODE-ID-keyed query (definition/references/hover/
        // tokens) needs the canonical arena + a span table keyed by canonical ids. A LONE s-expr form is
        // already canonical (identity map, no-op), but the MULTI-form `read_all_spanned` fallback wraps
        // the roots in a synthetic `(do …)` whose head is built LAST — canonicalization then REORDERS the
        // ids, and an un-remapped span table would map every answer to a NEIGHBOUR's node (a
        // go-to-definition on a multi-form `.sexp` buffer jumped to the wrong line).
        let (arenas, id_map) = cadenza_syntax::canon::canonicalize_with_map(&raw_arenas);
        let spans = raw_spans.remap(&id_map, arenas.structure.len());
        Ok((arenas, spans, Vec::new()))
    }
}

/// Parse one TAB-separated `Diagnostics`-query line into an LSP [`Diagnostic`], or `None` to drop it.
/// The line shape is `severity  code  node-id  fix-kind  fix-node  fix-repl  fix-verified  message`
/// (eight columns, message last). Drops a cascade artifact — an `unbound name `<error>`` on a recovered
/// `<error>` placeholder (the parse error already said what to fix). An unanchored (`-`) node maps to the
/// document start so the diagnostic is still shown (never silently dropped).
fn parse_diag_line(
    d: &cadenza_compile_abi::Diagnostic,
    text: &str,
    spans: &cadenza_syntax::spans::SpanTable,
) -> Option<Diagnostic> {
    // Reads the decoded `Diagnostic` STRUCT (the KIND_DIAGNOSTICS wire is canonical binary AST now — the
    // caller `decode_diagnostics`d it; no per-line tab parsing). The fix fields are unused by the LSP
    // surface (diagnostics only, no code-action here), so only severity/code/node/message are read.

    // Drop the `<error>`-placeholder cascade: a recovered parse placeholder reduces to a bare name
    // `<error>`, which the checker reports as an unbound-name fault referencing a token the user never
    // wrote. `<error>` is unlexable on either surface, so such a message is always the placeholder.
    if d.message.contains("`<error>`") {
        return None;
    }

    let severity = match d.severity {
        cadenza_compile_abi::Severity::Error => DiagnosticSeverity::ERROR,
        cadenza_compile_abi::Severity::Warning => DiagnosticSeverity::WARNING,
    };

    // The node's source range via the span table; an unanchored/unmapped node → the document start.
    let range = d
        .node
        .and_then(|id| spans.get(cadenza_syntax::StructId(id)))
        .map(|s| byte_range_to_range(text, s.start, s.end))
        .unwrap_or_else(|| Range::new(Position::new(0, 0), Position::new(0, 0)));

    let code = d.code.clone().map(lsp_types::NumberOrString::String);

    Some(Diagnostic {
        range,
        severity: Some(severity),
        code,
        source: Some("cdz".to_string()),
        message: d.message.clone(),
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

/// Apply one incremental `didChange` content change to `text` IN PLACE. A change with a `range` splices
/// its `text` over the `[start, end)` byte span the range maps to (via [`position_to_byte`], so the
/// UTF-16 columns the client sends resolve to UTF-8 byte offsets); a change with NO range is a
/// whole-document replace (the LSP spec permits either, even under incremental sync). Ranges are clamped
/// and ordered so a malformed change (end before start, out-of-range) can never panic — a tooling read
/// stays total. Called for each change in order; each range is relative to the text as left by the prior
/// change in the same notification, which holds because we mutate `text` before the next iteration.
fn apply_content_change(text: &mut String, change: TextDocumentContentChangeEvent) {
    let Some(range) = change.range else {
        // No range: the whole document was replaced.
        *text = change.text;
        return;
    };
    let start = position_to_byte(text, range.start);
    let end = position_to_byte(text, range.end);
    // Clamp to a well-ordered span: a degenerate range (end < start) collapses to an insertion at start
    // rather than a panic on `replace_range`.
    let (lo, hi) = (start.min(end), start.max(end));
    text.replace_range(lo..hi, &change.text);
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

    // Read the TYPE (`TypeAt`) and the DOCSTRING (`DocAt`) of the node in ONE compile — a hover shows both
    // (rust-analyzer-style: the signature, then its doc prose). Distinct query kinds → distinct artifacts.
    let ast_bytes = cadenza_syntax::codec::encode(&arenas);
    let sidecar_bytes = cadenza_compile_abi::sidecar::encode(&[
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
            node: node.0,
        }),
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::DocAt {
            node: node.0,
        }),
    ]);
    let inputs = vec![
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            "main",
            ast_bytes,
        ),
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            sidecar_bytes,
        ),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let ty = compiled
        .artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT)
        .map(|b| crate::render_type_at(&cadenza_compile_abi::decode_type_at(b)))
        .unwrap_or_default();
    // A total-but-uninformative answer ("unknown", or empty) is not worth a hover popup — return None so
    // the editor shows nothing rather than a meaningless box.
    if ty.is_empty() || ty == "unknown" {
        return None;
    }
    // The hovered node's documentation, decoded from the structured binary-AST wire — only a `Doc`
    // outcome yields hover prose (a no-answer verdict → no doc section); ZERO string parsing.
    let doc = compiled
        .artifact(cadenza_compile_abi::sidecar::KIND_DOC)
        .and_then(|b| match cadenza_compile_abi::decode_doc(b) {
            cadenza_compile_abi::DocAnswer::Doc(text) => Some(text.trim().to_string()),
            _ => None,
        })
        .filter(|d| !d.is_empty());
    // The hovered node's source range, so the editor underlines exactly the sub-expression it typed.
    let range = spans
        .get(node)
        .map(|s| byte_range_to_range(text, s.start, s.end));
    Some(Hover {
        contents: hover_contents(&ty, doc.as_deref()),
        range,
    })
}

/// Assemble a hover's contents from the type and an optional docstring. With a doc, render Markdown — the
/// type as a Cadenza code fence, then the doc prose below a rule (rust-analyzer's shape). Without a doc,
/// keep the plain type string (a `MarkedString`), so a bare hover is unchanged.
fn hover_contents(ty: &str, doc: Option<&str>) -> HoverContents {
    match doc {
        Some(doc) => HoverContents::Markup(lsp_types::MarkupContent {
            kind: lsp_types::MarkupKind::Markdown,
            value: format!("```cadenza\n{ty}\n```\n\n---\n\n{doc}"),
        }),
        None => HoverContents::Scalar(MarkedString::String(ty.to_string())),
    }
}

// ── the analysis: cursor → definition / references, via the `ResolveOf` / `UsesOf` queries ───────────

/// The RAW artifact bytes of a single sidecar `query` over `arenas` — every LSP single-query answer wire
/// is canonical binary AST now (e.g. `KIND_DIAGNOSTICS` → `decode_diagnostics`, `KIND_USES` →
/// `decode_uses`, `KIND_TYPE_INFO` → `decode_type_info`), so the caller decodes the structured payload
/// rather than reading text. Routes through the shared `run_sidecar` chokepoint (which builds the same
/// `[ast "main", sidecar "drive"]` inputs) so a `!standalone` build DELEGATES the query to `cdz-compile`;
/// under `standalone` it is the in-process compile. `None` if the compile produced no such artifact.
fn run_query_bytes(
    arenas: &cadenza_syntax::Arenas,
    query: cadenza_compile_abi::sidecar::Query,
    kind: &str,
) -> Option<Vec<u8>> {
    let compiled = crate::run_sidecar(arenas, cadenza_compile_abi::Request::Query(query));
    compiled.artifact(kind).map(|b| b.to_vec())
}

/// Map a decoded [`cadenza_compile_abi::FixKind`] to the kind string the shared `fix::fix_edits`/
/// `code_action_title` engine matches on (`replace`/`insert`/`wrap`/`delete`; `InsertInto` → `insert`).
fn fix_kind_str(kind: cadenza_compile_abi::FixKind) -> &'static str {
    match kind {
        cadenza_compile_abi::FixKind::Replace => "replace",
        cadenza_compile_abi::FixKind::InsertInto => "insert",
        cadenza_compile_abi::FixKind::Wrap => "wrap",
        cadenza_compile_abi::FixKind::Delete => "delete",
    }
}

/// The decoded `Symbols` outline records `(name, kind, name-node-id)` for `arenas` — the `Symbols` query
/// answer on the canonical binary-AST wire (`symbols_wire`) run through the shared codec, for the LSP
/// handlers that read the document outline. ZERO string parsing.
fn query_symbols(arenas: &cadenza_syntax::Arenas) -> Vec<(String, String, u32)> {
    run_query_bytes(
        arenas,
        cadenza_compile_abi::sidecar::Query::Symbols,
        cadenza_compile_abi::sidecar::KIND_SYMBOLS,
    )
    .map(|b| cadenza_compile_abi::decode_symbols(&b))
    .unwrap_or_default()
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
    let bytes = run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::ResolveOf { node: node.0 },
        cadenza_compile_abi::sidecar::KIND_RESOLVE,
    )?;
    // The `ResolveOf` answer is the defining occurrence's node id (none = not a navigable reference),
    // decoded from the binary-AST wire — ZERO string parsing.
    let target = cadenza_compile_abi::decode_resolve(&bytes)?;
    node_location(text, &spans, uri, target)
}

/// Go-to-TYPE-definition: from the node at `pos`, the source location of the declaration of its TYPE.
/// Reads `TypeAt` (the node's rendered type), and when that type is a BARE user-declared type NAME —
/// resolvable via the `Symbols` type-name→decl-node lookup (`top_level_symbol_node`) — returns that
/// declaration's location. Declines (`None`) when the node has no type, the type is not a bare name (a
/// builtin scalar like `Int64`, or a compound like `(List Color)` / `(-> …)` — no single user decl to
/// jump to), or the name is not a declared symbol. Total: never a panic, never a wrong landing.
fn type_definition_at(text: &str, is_ml: bool, pos: Position, uri: &Uri) -> Option<Location> {
    let (arenas, spans, _errors) = parse_surface(text, is_ml).ok()?;
    let byte = position_to_byte(text, pos);
    let node = spans.node_at_offset(byte)?;
    let out = crate::run_sidecar(
        &arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
            node: node.0,
        }),
    );
    let ty = crate::render_type_at(&cadenza_compile_abi::decode_type_at(
        out.artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT)?,
    ));
    let name = navigable_type_name(&ty)?;
    // Map the type NAME to its top-level declaration node (the same `Symbols` lookup references uses),
    // then to a source location. A name that names no declared type (a builtin) yields None.
    let decl = top_level_symbol_node(&arenas, name)?;
    node_location(text, &spans, uri, decl)
}

/// The NAVIGABLE type name in a rendered `TypeAt` answer — a BARE type name (a single identifier token)
/// that a go-to-type-definition can jump to, or `None`. Excludes the uninformative `unknown`, and any
/// COMPOUND type (`(List …)`, `(-> …)` — contains a space/paren) which has no single declaration to land
/// on. A builtin scalar name (`Int64`) passes this shape filter but simply won't resolve to a user
/// `type` declaration downstream, so it declines there. Shared by the single-buffer + package paths.
fn navigable_type_name(rendered: &str) -> Option<&str> {
    let ty = rendered.trim();
    if ty.is_empty()
        || ty == "unknown"
        || ty.contains(|c: char| c.is_whitespace() || c == '(' || c == ')')
    {
        return None;
    }
    Some(ty)
}

/// Cross-file go-to-TYPE-definition: type the cursor node against the whole `(import …)` closure (so an
/// IMPORTED value's type resolves), then jump to that type's `type …` declaration WHEREVER it lives —
/// the entry file OR an imported library. Mirrors `package_hover_at` for the linked `TypeAt`, then locates
/// the type NAME's declaration by scanning each loaded file's own `Symbols` (like `package_completions_at`
/// reads each lib's columns). `None` when the closure can't load, the type is not a bare navigable name,
/// or no loaded file declares it (a builtin) — the caller falls back to the single-buffer path.
/// The shared cross-file linked-query preamble for the `package_*_at` handlers: load the cursor's
/// `(import …)` closure, splice each file as a `KIND_AST` artifact plus a `KIND_SIDECAR` "drive" request
/// list built by `build_requests(cursor_node)`, and compile the linked program. The entry is spliced FIRST
/// (`crate::closure::load` returns it at `files[0]`, `struct_base == 0`), so the cursor's entry-local node
/// id IS the global query input — pass it as `cursor_node`. Returns the loaded files (for span/link-map
/// demux) and the compile output (for the caller to decode its own answer artifact); `None` if the closure
/// can't load. The caller parses the entry itself (it needs `entry_spans` for the cursor, and hover for its
/// range), so only the load→splice→compile body is shared.
fn linked_query_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    cursor_node: u32,
    build_requests: impl FnOnce(u32) -> Vec<cadenza_compile_abi::Request>,
) -> Option<(
    Vec<crate::closure::LoadedFile>,
    cadenza_compile_abi::CompileOutput,
)> {
    let files = crate::closure::load(entry_path, open).ok()?;
    let mut inputs: Vec<cadenza_compile_abi::Artifact> = files
        .iter()
        .map(|f| {
            cadenza_compile_abi::Artifact::new(
                cadenza_compile_abi::Artifact::KIND_AST,
                f.name.clone(),
                cadenza_syntax::codec::encode(&f.arenas),
            )
        })
        .collect();
    inputs.push(cadenza_compile_abi::Artifact::new(
        cadenza_compile_abi::sidecar::KIND_SIDECAR,
        "drive",
        cadenza_compile_abi::sidecar::encode(&build_requests(cursor_node)),
    ));
    inputs.push(rcdzc::cli::entry_artifact(&files[0].name));
    Some((
        files,
        rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[])),
    ))
}

fn package_type_definition_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    pos: Position,
) -> Option<Location> {
    let (_entry_arenas, entry_spans, _e) = parse_surface(entry_text, entry_is_ml).ok()?;
    let byte = position_to_byte(entry_text, pos);
    let cursor = entry_spans.node_at_offset(byte)?;
    // Entry is spliced FIRST (base 0), so the cursor's entry-local node id is the linked query input.
    let (files, compiled) = linked_query_at(entry_path, open, cursor.0, |node| {
        vec![cadenza_compile_abi::Request::Query(
            cadenza_compile_abi::sidecar::Query::TypeAt { node },
        )]
    })?;
    let ty = compiled
        .artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT)
        .map(|b| crate::render_type_at(&cadenza_compile_abi::decode_type_at(b)))?;
    let name = navigable_type_name(&ty)?;

    // Locate the type NAME's declaration in whichever loaded file declares it — its own `Symbols` gives a
    // FILE-LOCAL node id, mapped to a Location in THAT file (a jump into the declaring library, or the
    // entry itself). First match wins (a type name is unique across a well-formed closure).
    for file in &files {
        if let Some(decl) = top_level_symbol_node(&file.arenas, name) {
            let span = file.spans.get(cadenza_syntax::StructId(decl))?;
            return Some(Location {
                uri: path_to_uri(&file.path)?,
                range: byte_range_to_range(&file.source, span.start, span.end),
            });
        }
    }
    None
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
    let (files, compiled) = linked_query_at(entry_path, open, cursor.0, |node| {
        vec![cadenza_compile_abi::Request::Query(
            cadenza_compile_abi::sidecar::Query::ResolveOf { node }, // entry-local == global (entry is base 0)
        )]
    })?;
    let bytes = compiled.artifact(cadenza_compile_abi::sidecar::KIND_RESOLVE)?;
    // The defining occurrence's global node id, decoded from the binary-AST wire — ZERO string parsing.
    let target = cadenza_compile_abi::decode_resolve(bytes)?;

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
    // TYPE + DOCSTRING of the cursor node in one linked compile (entry-local == global at base 0).
    let (_files, compiled) = linked_query_at(entry_path, open, cursor.0, |node| {
        vec![
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
                node,
            }),
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::DocAt {
                node,
            }),
        ]
    })?;
    let ty = compiled
        .artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT)
        .map(|b| crate::render_type_at(&cadenza_compile_abi::decode_type_at(b)))
        .unwrap_or_default();
    if ty.is_empty() || ty == "unknown" {
        return None; // let the single-buffer path try (or show nothing)
    }
    // The hovered node's documentation, decoded from the structured binary-AST wire — only a `Doc`
    // outcome yields hover prose (a no-answer verdict → no doc section); ZERO string parsing.
    let doc = compiled
        .artifact(cadenza_compile_abi::sidecar::KIND_DOC)
        .and_then(|b| match cadenza_compile_abi::decode_doc(b) {
            cadenza_compile_abi::DocAnswer::Doc(text) => Some(text.trim().to_string()),
            _ => None,
        })
        .filter(|d| !d.is_empty());
    // Hover range = the cursor node's span in the ENTRY file (base 0, so the entry spans apply directly).
    let range = entry_spans
        .get(cursor)
        .map(|s| byte_range_to_range(entry_text, s.start, s.end));
    Some(Hover {
        contents: hover_contents(&ty, doc.as_deref()),
        range,
    })
}

/// A `file://` URI for a local filesystem path — the inverse of `uri_to_path`, for a cross-file
/// `Location` that points into an imported file. Percent-encodes every path byte that is not an
/// unreserved URI character (keeping `/` as the path separator), so a path containing a space, a `%`,
/// or a reserved char (`#`/`?`/…) yields a VALID, unambiguous `file://` URI. An ABSOLUTE path becomes
/// `file://<encoded>`; a relative path (unusual for a closure file, which resolves siblings against the
/// entry's dir) is handled best-effort by prefixing a slash (`file:///<encoded>`). `None` only if the
/// assembled string still does not parse as a `Uri` (it does for any real path).
///
/// TRAP: Encoding a literal `%` is LOAD-BEARING for the `uri_to_path`/`percent_decode` round-trip: the
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
    let resolves_to = run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::ResolveOf { node: node.0 },
        cadenza_compile_abi::sidecar::KIND_RESOLVE,
    )
    .and_then(|b| cadenza_compile_abi::decode_resolve(&b));
    let cursor_is_top_level =
        top_node == Some(node.0) || (top_node.is_some() && resolves_to == top_node);
    if !cursor_is_top_level {
        return Vec::new();
    }

    let mut locations: Vec<Location> = Vec::new();
    if let Some(bytes) = run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::UsesOf { name: name.clone() },
        cadenza_compile_abi::sidecar::KIND_USES,
    ) {
        for id in cadenza_compile_abi::decode_uses(&bytes) {
            if let Some(loc) = node_location(text, &spans, uri, id) {
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

/// Every top-level `(def …)` / `(type …)` / `(effect …)` form in `text`, as `(name, full_range,
/// name_range)` — the FULL form span (for "which caller encloses this use") plus the NAME occurrence span
/// (the call-hierarchy `selection_range`). Walks the arena root through a `(module …)` / `(do …)` wrapper
/// (like the folding/outline walk), reading the declared name from a function signature `(name param…)`
/// or a bare `(def name …)`. Used by the call-hierarchy handlers to build items + attribute a reference
/// to its enclosing definition. Total: an unparseable buffer yields empty.
fn top_level_defs_with_spans(text: &str, is_ml: bool) -> Vec<(String, Range, Range)> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let root = crate::unwrap_comment(&arenas, arenas.root);
    // Top-level items: a `(module name member…)`'s members, a `(do …)`'s children, else the lone root.
    let items: Vec<cadenza_syntax::StructId> = if let Some(tail) = arenas.as_form(root, "module") {
        tail.iter().skip(1).copied().collect() // skip the module NAME
    } else if let Some(tail) = arenas.as_form(root, "do") {
        tail.to_vec()
    } else {
        vec![root]
    };
    let mut out = Vec::new();
    for item in items {
        let item = crate::unwrap_comment(&arenas, item);
        // Only definitional forms carry a callable/declared name we hang a hierarchy item on.
        let tail = arenas
            .as_form(item, "def")
            .or_else(|| arenas.as_form(item, "type"))
            .or_else(|| arenas.as_form(item, "effect"));
        let Some(tail) = tail else { continue };
        let Some(&target) = tail.first() else {
            continue;
        };
        // `(def (name param…) body)` → name is the sig-list head; `(def name body)` → the bare name.
        let (name, name_node) = match arenas.get(target) {
            cadenza_syntax::ast::Struct::List(sig) => match sig.first() {
                Some(&h) => match arenas.as_name(h) {
                    Some(n) => (n.to_string(), h),
                    None => continue,
                },
                None => continue,
            },
            cadenza_syntax::ast::Struct::Atom(_) => match arenas.as_name(target) {
                Some(n) => (n.to_string(), target),
                None => continue,
            },
        };
        let (Some(full), Some(name_span)) = (spans.get(item), spans.get(name_node)) else {
            continue;
        };
        out.push((
            name,
            byte_range_to_range(text, full.start, full.end),
            byte_range_to_range(text, name_span.start, name_span.end),
        ));
    }
    out
}

/// The `CallHierarchyItem` for the top-level definition the cursor refers to — the anchor a
/// `callHierarchy/prepare` returns. Prepares from EITHER the definition's own name OR a USE of it
/// (rust-analyzer prepares from a call site too): the cursor's name atom is matched against the top-level
/// defs by name, so a cursor on `helper` anywhere — its `def` or any `helper(…)` call — yields helper's
/// item. `None` when the cursor is not on a name that names a top-level definition.
fn call_hierarchy_item_at(
    text: &str,
    is_ml: bool,
    pos: Position,
    uri: &Uri,
) -> Option<CallHierarchyItem> {
    let (arenas, spans, _errors) = parse_surface(text, is_ml).ok()?;
    // The name atom under the cursor (a def name, or a use of one). A cursor off any name → None.
    let byte = position_to_byte(text, pos);
    let cursor_name = spans
        .node_at_offset(byte)
        .and_then(|n| arenas.as_name(n))
        .map(str::to_string);
    let kinds: std::collections::HashMap<String, String> = top_level_symbols_of(text, is_ml)
        .into_iter()
        .map(|(n, k, _)| (n, k))
        .collect();
    // Match a top-level def either by NAME-range containment (cursor on the decl) OR by the cursor's name
    // atom (cursor on a use elsewhere). The decl-range check wins first (exact), then the name match.
    let defs = top_level_defs_with_spans(text, is_ml);
    let hit = defs
        .iter()
        .find(|(_, _, name_range)| range_contains(name_range, pos))
        .or_else(|| {
            cursor_name
                .as_ref()
                .and_then(|cn| defs.iter().find(|(n, _, _)| n == cn))
        })?;
    let (name, full_range, name_range) = hit.clone();
    let kind = kinds
        .get(&name)
        .map(|k| symbol_kind_to_document_kind(k))
        .unwrap_or(SymbolKind::FUNCTION);
    Some(CallHierarchyItem {
        name,
        kind,
        tags: None,
        detail: None,
        uri: uri.clone(),
        range: full_range,
        selection_range: name_range,
        data: None,
    })
}

/// The incoming calls to the definition `name` in `text` — every top-level def that references it, each as
/// a `CallHierarchyIncomingCall{from: caller-item, from_ranges: [use ranges within that caller]}`. Backed
/// by `UsesOf{name}` (the reference index); each reference range is attributed to the enclosing top-level
/// def by span containment. A reference NOT inside any def (e.g. an `(export …)` clause) is skipped — it is
/// not a call site. Total: no callers → empty.
fn incoming_calls_for(
    text: &str,
    is_ml: bool,
    name: &str,
    uri: &Uri,
) -> Vec<CallHierarchyIncomingCall> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let defs = top_level_defs_with_spans(text, is_ml);
    let kinds: std::collections::HashMap<String, String> = top_level_symbols_of(text, is_ml)
        .into_iter()
        .map(|(n, k, _)| (n, k))
        .collect();
    // Each reference's range, from `UsesOf` (node-id-keyed → source range).
    let Some(bytes) = run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::UsesOf {
            name: name.to_string(),
        },
        cadenza_compile_abi::sidecar::KIND_USES,
    ) else {
        return Vec::new();
    };
    // Group use ranges by the enclosing caller def (by name), preserving encounter order.
    let mut order: Vec<String> = Vec::new();
    let mut by_caller: std::collections::HashMap<String, Vec<Range>> =
        std::collections::HashMap::new();
    for id in cadenza_compile_abi::decode_uses(&bytes) {
        let Some(span) = spans.get(cadenza_syntax::StructId(id)) else {
            continue;
        };
        let use_range = byte_range_to_range(text, span.start, span.end);
        // The caller is the def whose FULL range encloses this use (and isn't the def's own name — a
        // self-recursive call still counts as an incoming call from itself, which is correct).
        let Some((caller, _, _)) = defs
            .iter()
            .find(|(_, full, _)| range_contains(full, use_range.start))
        else {
            continue; // a reference outside any def (e.g. an export clause) is not a call site
        };
        if !by_caller.contains_key(caller) {
            order.push(caller.clone());
        }
        by_caller.entry(caller.clone()).or_default().push(use_range);
    }
    order
        .into_iter()
        .filter_map(|caller| {
            let ranges = by_caller.remove(&caller)?;
            let (_, full_range, name_range) = defs.iter().find(|(n, _, _)| n == &caller)?.clone();
            let kind = kinds
                .get(&caller)
                .map(|k| symbol_kind_to_document_kind(k))
                .unwrap_or(SymbolKind::FUNCTION);
            Some(CallHierarchyIncomingCall {
                from: CallHierarchyItem {
                    name: caller,
                    kind,
                    tags: None,
                    detail: None,
                    uri: uri.clone(),
                    range: full_range,
                    selection_range: name_range,
                    data: None,
                },
                from_ranges: ranges,
            })
        })
        .collect()
}

/// The outgoing calls FROM the definition `name` in `text` — every top-level def that `name`'s own body
/// calls, each as `CallHierarchyOutgoingCall{to: callee-item, from_ranges: [call-site ranges in name]}`.
/// Walks every name-headed call list whose span is INSIDE `name`'s def form and whose head names a
/// top-level def (the callee); groups the call-site ranges by callee. A call to a builtin / unknown name
/// (not a top-level def) is skipped. The def's OWN signature list `(name param…)` is excluded (it is not a
/// call). Total: a def that calls nothing local → empty.
fn outgoing_calls_for(
    text: &str,
    is_ml: bool,
    name: &str,
    uri: &Uri,
) -> Vec<CallHierarchyOutgoingCall> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let defs = top_level_defs_with_spans(text, is_ml);
    // The caller def's FULL byte span (so we only look at call sites inside its body). No such def → empty.
    let Some((_, caller_full, _)) = defs.iter().find(|(n, _, _)| n == name).cloned() else {
        return Vec::new();
    };
    let caller_lo = position_to_byte(text, caller_full.start);
    let caller_hi = position_to_byte(text, caller_full.end);
    let def_names: std::collections::HashSet<&str> =
        defs.iter().map(|(n, _, _)| n.as_str()).collect();
    let kinds: std::collections::HashMap<String, String> = top_level_symbols_of(text, is_ml)
        .into_iter()
        .map(|(n, k, _)| (n, k))
        .collect();
    // A def's own signature list `(name param…)` is call-shaped but is a declaration, not a call — exclude
    // it (mirrors the inlay-hint signature-leak guard).
    let mut def_sigs: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        if let cadenza_syntax::ast::Struct::List(kids) = arenas.get(id)
            && kids.first().and_then(|&h| arenas.as_name(h)) == Some("def")
            && let Some(&sig) = kids.get(1)
            && matches!(arenas.get(sig), cadenza_syntax::ast::Struct::List(_))
        {
            def_sigs.insert(sig.0);
        }
    }
    let mut order: Vec<String> = Vec::new();
    let mut by_callee: std::collections::HashMap<String, Vec<Range>> =
        std::collections::HashMap::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        if def_sigs.contains(&id.0) {
            continue;
        }
        let cadenza_syntax::ast::Struct::List(children) = arenas.get(id) else {
            continue;
        };
        let Some(callee) = children.first().and_then(|&h| arenas.as_name(h)) else {
            continue;
        };
        // Only a call to a TOP-LEVEL def (not the caller itself → a self-recursive call is an outgoing
        // call to itself, which is correct and useful), and only within the caller's body span.
        if !def_names.contains(callee) {
            continue;
        }
        let Some(span) = spans.get(id) else { continue };
        if span.start < caller_lo || span.start >= caller_hi {
            continue;
        }
        let range = byte_range_to_range(text, span.start, span.end);
        if !by_callee.contains_key(callee) {
            order.push(callee.to_string());
        }
        by_callee.entry(callee.to_string()).or_default().push(range);
    }
    order
        .into_iter()
        .filter_map(|callee| {
            let ranges = by_callee.remove(&callee)?;
            let (_, full_range, name_range) = defs.iter().find(|(n, _, _)| n == &callee)?.clone();
            let kind = kinds
                .get(&callee)
                .map(|k| symbol_kind_to_document_kind(k))
                .unwrap_or(SymbolKind::FUNCTION);
            Some(CallHierarchyOutgoingCall {
                to: CallHierarchyItem {
                    name: callee,
                    kind,
                    tags: None,
                    detail: None,
                    uri: uri.clone(),
                    range: full_range,
                    selection_range: name_range,
                    data: None,
                },
                from_ranges: ranges,
            })
        })
        .collect()
}

/// Whether `range` covers `pos` (inclusive of start, exclusive of end at the line/char granularity LSP
/// positions use) — the containment test the call-hierarchy handlers use to attribute a cursor/use to a
/// def. A simple line/column lexicographic comparison.
fn range_contains(range: &Range, pos: Position) -> bool {
    let after_start = (pos.line, pos.character) >= (range.start.line, range.start.character);
    let before_end = (pos.line, pos.character) < (range.end.line, range.end.character);
    after_start && before_end
}

/// The node id of the TOP-LEVEL declaration named `name` (its name occurrence), or `None` if no
/// top-level symbol has that name. Reads the `Symbols` query (whose third column IS that name node) —
/// the same authority `Symbols`/`Exports` use, so a name that names no top-level declaration (a purely
/// local binder) yields `None`, which is exactly what the shadowing guard needs.
fn top_level_symbol_node(arenas: &cadenza_syntax::Arenas, name: &str) -> Option<u32> {
    // The name-node-id of the top-level declaration named `name`, from the binary-AST `Symbols` outline.
    query_symbols(arenas)
        .into_iter()
        .find(|(n, _, _)| n == name)
        .map(|(_, _, node)| node)
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

    // ALL THREE fact reads in ONE linked compile (entry first = base 0): `Symbols` (the shadowing-guard
    // authority + the declaration node), `ResolveOf` (does the cursor resolve to a top-level symbol), and
    // `UsesOf` (the references themselves). A query is TOTAL and rides alongside the others, so one linked
    // compile answers all three (and carries the `link-map` for the demux). Distinct query kinds → distinct
    // artifacts, retrieved by `KIND_*` below.
    let Some((files, compiled)) = linked_query_at(entry_path, open, cursor.0, |node| {
        vec![
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Symbols),
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ResolveOf {
                node,
            }),
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::UsesOf {
                name: name.clone(),
            }),
        ]
    }) else {
        return Vec::new();
    };
    // Read a query answer's RAW artifact bytes off the package compile, decoded via the shared
    // binary-AST codec (`KIND_SYMBOLS`/`KIND_RESOLVE`) — ZERO string parsing.
    let artifact_bytes = |kind: &str| -> Option<&[u8]> { compiled.artifact(kind) };

    // SHADOWING GUARD (package flavor). `UsesOf` is NAME-keyed, so a LOCAL binder shadowing a top-level
    // name would leak the top-level's refs. Proceed only when the cursor genuinely belongs to a
    // top-level symbol — either it IS one of the package's top-level declaration name-nodes, or it
    // RESOLVES to one. The authority is the PACKAGE `Symbols` query (global node ids, and it lists
    // IMPORTED defs too — an imported `helper`'s def appears with the id `ResolveOf` returns), run over
    // the same linked program so the ids line up. A purely-local binder resolves to its OWN occurrence,
    // which is NEVER a `Symbols` node — so it fails the guard and returns empty (the single-buffer path,
    // with its own guard, handles the local). This is the package twin of `references_at`'s guard.
    //
    // TRAP: The earlier `resolves_to.is_some()` test was too permissive: `ResolveOf` succeeds for a LOCAL
    // binder too (it resolves to itself), so a cursor on a shadowing local passed the guard and leaked
    // the top-level's uses. Requiring the resolve TARGET to be a `Symbols` node is what distinguishes a
    // genuine top-level from a shadowing local. `symbols_lines` is parsed once and reused for the
    // declaration-node lookup below (avoiding a separate entry-only `Symbols` compile).
    // Decode the package `Symbols` outline ONCE (reused for the symbol-node set AND the declaration-node
    // lookup below — no extra compile).
    let symbols = artifact_bytes(cadenza_compile_abi::sidecar::KIND_SYMBOLS)
        .map(cadenza_compile_abi::decode_symbols)
        .unwrap_or_default();
    let symbol_nodes: std::collections::HashSet<u32> =
        symbols.iter().map(|(_, _, node)| *node).collect();
    let resolves_to = artifact_bytes(cadenza_compile_abi::sidecar::KIND_RESOLVE)
        .and_then(cadenza_compile_abi::decode_resolve);
    // The cursor belongs to a top-level symbol iff it IS a `Symbols` name-node (a declaration occurrence,
    // entry-local == global at base 0) or RESOLVES to one (a use of a top-level/imported name).
    let cursor_is_symbol = symbol_nodes.contains(&cursor.0);
    let resolves_to_symbol = resolves_to.is_some_and(|t| symbol_nodes.contains(&t));
    if !(cursor_is_symbol || resolves_to_symbol) {
        return Vec::new();
    }
    // The declaration node named `name` for the include-declaration fallback below — read from the SAME
    // package `Symbols` answer (its GLOBAL id, whichever file the def lives in), so no extra compile.
    // `None` when `name` names no top-level declaration (then `resolves_to` carries the target).
    let top_node = symbols
        .iter()
        .find(|(n, _, _)| n == &name)
        .map(|(_, _, node)| *node);

    let files_ref = &files;
    let mut out: Vec<Location> = Vec::new();

    // The `link-map` came from the SAME compile as `UsesOf` (so the demux ids match).
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
    if let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_USES) {
        for global in cadenza_compile_abi::decode_uses(bytes) {
            if let Some(loc) = loc_of_global(global) {
                out.push(loc);
            }
        }
    }
    // include_declaration: add the def's own name occurrence (UsesOf excludes it). Its GLOBAL id is the
    // `resolves_to` target (or `top_node`, the package `Symbols` name-node for `name` — also global).
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

    // Top-level declarations — kind ∈ value/function/type/effect/module (from the binary-AST outline).
    for (name, kind, _) in query_symbols(&arenas) {
        items.insert(
            name.clone(),
            CompletionItem {
                label: name.clone(),
                kind: Some(symbol_kind_to_completion_kind(&kind)),
                detail: Some(kind),
                ..Default::default()
            },
        );
    }

    // Local bindings in scope at the cursor — overwrite to shadow a top-level. The KIND_SCOPE wire is now
    // binary AST (`decode_scope`); each binding's type detail is rendered from its FULL structured Ty
    // payload via the shared cadenza-syntax renderer (`render_ty_scheme`), not a wire render_name string.
    let byte = position_to_byte(text, pos);
    if let Some(node) = spans.node_at_offset(byte) {
        let compiled = crate::run_sidecar(
            &arenas,
            cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::ScopeAt {
                node: node.0,
            }),
        );
        if let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_SCOPE) {
            for b in cadenza_compile_abi::decode_scope(bytes) {
                let detail = cadenza_syntax::render_ty::render_ty_scheme(&b.ty, b.ty.root);
                items.insert(
                    b.name.clone(),
                    CompletionItem {
                        label: b.name,
                        kind: Some(CompletionItemKind::VARIABLE),
                        detail: Some(detail),
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
        // The library's exported types (name → rendered type-name, from the binary-AST KIND_EXPORTS
        // value) and symbol kinds (name → kind, still the TAB-text Symbols query), each a single-file
        // query over the library's OWN arenas — no linking needed for these per-file facts.
        let types = export_type_details(&lib.arenas);
        let kinds: std::collections::HashMap<String, String> = query_symbols(&lib.arenas)
            .into_iter()
            .map(|(n, k, _)| (n, k))
            .collect();
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

/// The library's exported `name → rendered type-name` map, from the binary-AST `KIND_EXPORTS` value.
/// Decodes the structured exports value
/// (`cadenza_compile_abi::decode_exports`) and renders each RESOLVED type via the shared cadenza-syntax
/// renderer (`render_ty_scheme` — an export signature may be polymorphic, so it gets stable Var-lettering).
/// An export whose type did not resolve carries no payload and is OMITTED (no completion detail for it) —
/// the same graceful degrade the TAB reader gave a missing "unknown" column. Empty on no answer.
fn export_type_details(
    arenas: &cadenza_syntax::Arenas,
) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let compiled = crate::run_sidecar(
        arenas,
        cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::Exports),
    );
    if let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_EXPORTS) {
        for e in cadenza_compile_abi::decode_exports(bytes) {
            if let Some(a) = &e.ty {
                map.insert(
                    e.name,
                    cadenza_syntax::render_ty::render_ty_scheme(a, a.root),
                );
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

// ── the analysis: source text → LSP code lenses (monomorphizations), via the `Instantiations` query ──

/// The command id the instantiation CodeLens carries. LSP requires a `Command.command` to be NON-EMPTY
/// (some clients drop a lens with an empty id), so the lens names this stable, namespaced id — the
/// editor extension registers it as a NO-OP so the lens is a valid, non-actionable informational label.
const LENS_INSTANTIATIONS_COMMAND: &str = "cadenza.showInstantiations";

/// Compute the CodeLenses for `text` — one lens above each generic/ad-hoc-polymorphic top-level
/// definition that the compiler SPECIALIZED, titled with its concrete monomorphizations (the
/// `Instantiations` query — a fact no other tool surfaces, e.g. `loopn` → `[n: Int64, x: Int64]`,
/// `[n: Int64, x: String]`). A def that is not specialized (a plain monomorphic function, or a generic
/// inlined at every call) gets NO lens. TOTAL: an un-analyzable buffer yields no lenses, never a panic.
///
/// Cost: the `Instantiations` query forces WHOLE-PROGRAM monomorphization, so all defs' queries are
/// batched into ONE compile (each answer rides its own `KIND_INSTANTIATIONS` artifact, disambiguated by
/// the artifact's `name` = the queried def name) — monomorphization runs once, not once per def. This is
/// why the feature is a per-document CodeLens, not a per-cursor hover.
fn code_lenses_for(text: &str, is_ml: bool) -> Vec<CodeLens> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    // The top-level declaration names + their name-node ids (for lens placement) — the `Symbols` query,
    // decoded from the binary-AST outline.
    let names: Vec<(String, u32)> = query_symbols(&arenas)
        .into_iter()
        .map(|(name, _kind, node)| (name, node))
        .collect();
    if names.is_empty() {
        return Vec::new();
    }

    // Batch an `Instantiations` query for EVERY top-level name into one compile (monomorphization is
    // whole-program, so it runs once); each answer rides a distinct `KIND_INSTANTIATIONS` artifact keyed by
    // its POSITIONAL request index (`"0"`, `"1"`, …), so we recover the i-th answer by index below.
    let ast_bytes = cadenza_syntax::codec::encode(&arenas);
    let requests: Vec<cadenza_compile_abi::Request> = names
        .iter()
        .map(|(name, _)| {
            cadenza_compile_abi::Request::Query(
                cadenza_compile_abi::sidecar::Query::Instantiations { name: name.clone() },
            )
        })
        .collect();
    let inputs = vec![
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            "main",
            ast_bytes,
        ),
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            cadenza_compile_abi::sidecar::encode(&requests),
        ),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));

    let mut out = Vec::new();
    for (i, (_name, node)) in names.iter().enumerate() {
        // The `Instantiations` answer for THIS request, matched by REQUEST INDEX. `rcdzc::compile` names each
        // query answer's artifact by its POSITIONAL request index (`"0"`, `"1"`, … — see compile.rs "artifact
        // NAME is its REQUEST INDEX"), NOT the query's semantic def name; we built one request per `names`
        // entry in order, so request `i` answers `names[i]`. (Matching by `a.name == def_name` — the old
        // semantic-name contract — silently found nothing here, so the whole batched lens path returned 0.)
        // Decoded from the canonical binary-AST wire (operator P0 seq-284) — no string parsing.
        let idx = i.to_string();
        let Some(report) = compiled
            .artifacts
            .iter()
            .find(|a| a.kind == cadenza_compile_abi::sidecar::KIND_INSTANTIATIONS && a.name == idx)
            .and_then(|a| cadenza_compile_abi::instantiations_wire::decode(&a.bytes))
        else {
            continue;
        };
        let Some(title) = instantiations_lens_title(&report) else {
            continue; // not specialized → no lens
        };
        let Some(range) = spans
            .get(cadenza_syntax::StructId(*node))
            .map(|s| byte_range_to_range(text, s.start, s.end))
        else {
            continue;
        };
        // The title IS the information (the monomorphizations). A `Command` needs a NON-EMPTY `command`
        // id per LSP (some clients drop/reject an empty-id lens), so we name a stable, NAMESPACED id the
        // editor extension registers as a NO-OP handler (`cadenza.showInstantiations`) — clicking the lens
        // does nothing, which is the intended "informational label" behavior, but the lens is now valid
        // and renders everywhere. A future increment could make the handler jump to an instance.
        out.push(CodeLens {
            range,
            command: Some(lsp_types::Command {
                title,
                command: LENS_INSTANTIATIONS_COMMAND.to_string(),
                arguments: None,
            }),
            data: None,
        });
    }
    out
}

/// Build a CodeLens title from an `Instantiations` query answer, or `None` if the def is not SPECIALIZED
/// (no lens then). The answer is TSV: a `disp\t<node>\t<dispositions>` line then one
/// `inst\t<spec_name>\t<node>\t<arg;arg;…>` line per monomorphization. Only a `specialized` disposition
/// with ≥1 instance yields a title like `2 instances: [n: Int64, x: Int64] · [n: Int64, x: String]`.
fn instantiations_lens_title(
    report: &cadenza_compile_abi::instantiations_wire::Instantiations,
) -> Option<String> {
    // Only SPECIALIZED defs (a `specialized` disposition + ≥1 instance) get a lens — its title lists the
    // monomorphizations. Reads the STRUCTURED report (operator P0 seq-284: binary AST everywhere) directly;
    // each instance's args render as `[a, b]`.
    let specialized = report.dispositions.iter().any(|d| d == "specialized");
    if !specialized || report.instances.is_empty() {
        return None;
    }
    let instances: Vec<String> = report
        .instances
        .iter()
        .map(|inst| format!("[{}]", inst.args.join(", ")))
        .collect();
    let n = instances.len();
    let noun = if n == 1 { "instance" } else { "instances" };
    Some(format!("{n} {noun}: {}", instances.join(" · ")))
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
    top_level_symbols_of(text, is_ml)
        .into_iter()
        .map(|(name, kind, range)| {
            #[allow(deprecated)]
            // the `deprecated` field is deprecated but non-optional in this lsp-types.
            DocumentSymbol {
                detail: Some(kind.clone()),
                kind: symbol_kind_to_document_kind(&kind),
                name,
                tags: None,
                deprecated: None,
                range,
                selection_range: range,
                children: None,
            }
        })
        .collect()
}

/// The shared core of the outline / workspace-symbol reads: every TOP-LEVEL declaration in `text` as
/// `(name, kind-spelling, name-range)`, backed by the `Symbols` query. The kind is the raw wire spelling
/// (`value`/`function`/`type`/`effect`/`module`) — callers map it to an LSP `SymbolKind`. Declaration
/// order preserved (a deterministic function of the source). TOTAL: an un-analyzable buffer, or a row
/// whose node has no span, contributes nothing rather than panicking.
fn top_level_symbols_of(text: &str, is_ml: bool) -> Vec<(String, String, Range)> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (name, kind, node) in query_symbols(&arenas) {
        // Map the name-node id to its source range; a row whose node has no span contributes nothing.
        let Some(range) = spans
            .get(cadenza_syntax::StructId(node))
            .map(|s| byte_range_to_range(text, s.start, s.end))
        else {
            continue;
        };
        out.push((name, kind, range));
    }
    out
}

/// The folding ranges for `text`: one per MULTI-LINE top-level form. Enumerates the parse tree's
/// top-level items (the children of the `(do …)` root, or the lone root form — the SAME walk
/// `imported_names`/`declared_import_paths` use, after peeling a leading comment/doc wrapper), takes each
/// item's source span, and emits a `FoldingRange` from its first to its last line for any item that spans
/// ≥2 lines (a single-line form has nothing to fold). Line-based (no `end_character`), so the client
/// folds whole lines — the conventional declaration fold. TOTAL: an un-analyzable buffer yields no
/// ranges rather than panicking; a form with no span is skipped.
fn folding_ranges_for(text: &str, is_ml: bool) -> Vec<FoldingRange> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    let root = crate::unwrap_comment(&arenas, arenas.root);
    let items: Vec<cadenza_syntax::StructId> = match arenas.as_form(root, "do") {
        Some(tail) => tail.to_vec(),
        None => vec![root],
    };
    let mut out = Vec::new();
    for item in items {
        fold_item(&arenas, &spans, text, item, &mut out);
    }
    out
}

/// Emit a `FoldingRange` for `item` if it spans ≥2 lines, then — if `item` is a `(module …)` — RECURSE
/// into its members so a multi-line declaration NESTED in a module folds too (the "nesting module members
/// is a later refinement" the outline doc-comment promised). One level of nesting matches Cadenza's
/// structure (top-level declarations, optionally grouped under a module); a member that is itself a module
/// recurses again. The module's own fold (emitted first) covers the whole block; each member fold is a
/// sub-region an editor nests under it.
fn fold_item(
    arenas: &cadenza_syntax::Arenas,
    spans: &cadenza_syntax::spans::SpanTable,
    text: &str,
    item: cadenza_syntax::StructId,
    out: &mut Vec<FoldingRange>,
) {
    // Fold the form INCLUDING a leading doc/comment wrapper (so the fold covers the whole declaration),
    // so take the span of the item as-parsed, not the comment-unwrapped inner node.
    if let Some(span) = spans.get(item) {
        let range = byte_range_to_range(text, span.start, span.end);
        // Only a genuinely multi-line form is foldable.
        if range.end.line > range.start.line {
            out.push(FoldingRange {
                start_line: range.start.line,
                end_line: range.end.line,
                start_character: None,
                end_character: None,
                kind: None,
                collapsed_text: None,
            });
        }
    }
    // A `(module name member…)` groups declarations — recurse into its members (the tail past the name)
    // so a multi-line member folds on its own. `as_form` reads through the comment wrapper.
    let inner = crate::unwrap_comment(arenas, item);
    if let Some(tail) = arenas.as_form(inner, "module") {
        // Skip element 0 (the module NAME); the rest are its member declarations.
        for &member in tail.iter().skip(1) {
            fold_item(arenas, spans, text, member, out);
        }
    }
}

/// The selection-range chain at `pos`: the nested enclosing syntax nodes, innermost first, each linked as
/// the `parent` of the previous. Built from SPAN CONTAINMENT — every node span covering the cursor byte,
/// deduped by `(start, end)` and ordered smallest→largest — the same containment model `node_at_offset`
/// uses, so the innermost entry matches a go-to-definition/hover resolution. Distinct-width ordering makes
/// the chain strictly nested (an editor's expand-selection steps out one syntactic level at a time). A
/// cursor over no node (an empty/unparseable buffer, or a position past the end) yields a degenerate
/// empty range AT the cursor — the protocol wants one entry per requested position, never a gap.
///
/// TEST-ONLY: the production `selection_range` handler parses ONCE per request and drives
/// `selection_range_from_spans` per position (PR #538 — a multi-cursor request must not re-parse per
/// cursor), so this parse-then-delegate single-position wrapper is used only by the unit tests that pin
/// the chain shape from raw text.
#[cfg(test)]
fn selection_range_at(text: &str, is_ml: bool, pos: Position) -> SelectionRange {
    // Single-position convenience: parse once here then delegate. The MULTI-position `selection_range`
    // handler does NOT use this — it parses ONCE for the whole request and calls `selection_range_from_spans`
    // per position (a multi-cursor selectionRange must not re-parse per cursor — PR #538).
    let empty = SelectionRange {
        range: Range::new(pos, pos),
        parent: None,
    };
    let Ok((_arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return empty;
    };
    selection_range_from_spans(text, &spans, pos)
}

/// Build the nested `SelectionRange` chain at `pos` from an ALREADY-PARSED span table — the per-position
/// core of `selection_range`, split out so a multi-position (multi-cursor) request parses the document
/// ONCE and answers every position against the SAME `spans`, instead of re-parsing + re-scanning per
/// position (was O(positions × parse) — PR #538). Total: a position inside no node span yields the empty
/// (self) range.
fn selection_range_from_spans(
    text: &str,
    spans: &cadenza_syntax::spans::SpanTable,
    pos: Position,
) -> SelectionRange {
    let byte = position_to_byte(text, pos);
    let empty = SelectionRange {
        range: Range::new(pos, pos),
        parent: None,
    };
    // Collect every node span that contains the cursor, as `(start, end)` byte pairs.
    let mut ranges: Vec<(usize, usize)> = (0..spans.len())
        .filter_map(|i| spans.get(cadenza_syntax::StructId(i as u32)))
        .filter(|s| s.contains(byte))
        .map(|s| (s.start, s.end))
        .collect();
    if ranges.is_empty() {
        return empty;
    }
    // Dedup identical spans (several nodes can share one span), then order smallest→largest so the chain
    // is strictly nested from the innermost node outward.
    ranges.sort_by_key(|&(s, e)| (e - s, s));
    ranges.dedup();
    // Build the chain from the OUTERMOST inward so each inner range boxes the outer as its `parent`.
    let mut chain: Option<Box<SelectionRange>> = None;
    for &(start, end) in ranges.iter().rev() {
        chain = Some(Box::new(SelectionRange {
            range: byte_range_to_range(text, start, end),
            parent: chain,
        }));
    }
    // `chain` is non-None (ranges was non-empty) — unbox the innermost entry as the returned root.
    *chain.expect("ranges was non-empty, so the chain has at least one entry")
}

/// Signature help at `pos`: find the innermost `(callee arg…)` call the cursor is inside, then show the
/// callee's type. The enclosing call is the SMALLEST-span `List` node containing the cursor whose head
/// (child 0) is a NAME — found by span containment (the same model `node_at_offset`/`selection_range_at`
/// use), so no parent index is needed. The callee's signature comes from the `TypeOf` query (the arrow
/// the def resolves to — the same authority hover uses). The active parameter is how many argument forms
/// (children past the head) END at or before the cursor — i.e. how many args are already typed. `None`
/// when not inside such a call, the head is not a named function, or that name has no known type.
fn signature_help_at(text: &str, is_ml: bool, pos: Position) -> Option<SignatureHelp> {
    let (arenas, spans, _errors) = parse_surface(text, is_ml).ok()?;
    let byte = position_to_byte(text, pos);
    // The innermost enclosing CALL: smallest-span List node covering the cursor whose head is a name.
    let mut best: Option<(
        cadenza_syntax::StructId,
        usize,
        Vec<cadenza_syntax::StructId>,
    )> = None;
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        let cadenza_syntax::ast::Struct::List(children) = arenas.get(id) else {
            continue;
        };
        // Head must be a name (a call `(f …)`, not a bare list) and there must be a head to name.
        if children.first().and_then(|&h| arenas.as_name(h)).is_none() {
            continue;
        }
        let Some(span) = spans.get(id) else { continue };
        if !span.contains(byte) {
            continue;
        }
        let width = span.end - span.start;
        if best.as_ref().is_none_or(|(_, w, _)| width < *w) {
            best = Some((id, width, children.clone()));
        }
    }
    let (_call, _w, children) = best?;
    // The callee name (head), and its type via `TypeOf` (the arrow the def resolves to).
    let callee = arenas.as_name(children[0])?.to_string();
    // The callee's type comes back as a STRUCTURED binary-AST verdict — `KIND_TYPE_INFO` is canonical
    // binary AST since the sidecar-wire conversion (a raw `String::from_utf8_lossy` would hand back the
    // undecoded `cdzast…` payload, which merely HAPPENS to contain `->` bytes). Decode it and render via
    // the shared cadenza-syntax type renderer (`render_ty_scheme`) — the SAME path `cdz type` uses.
    let arrow = match cadenza_compile_abi::decode_type_info(&run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::TypeOf {
            name: callee.clone(),
        },
        cadenza_compile_abi::sidecar::KIND_TYPE_INFO,
    )?) {
        cadenza_compile_abi::TypeInfo::Found(ty) => {
            cadenza_syntax::render_ty::render_ty_scheme(&ty, ty.root)
        }
        // `NoDef` (a SPECIAL FORM head `def`/`if`/`let`/`do`/… or a typo) and `Unknown` (a real but
        // unsolved type) are not callable arrows — no signature. The `NoDef` arm is also the guard that
        // keeps signature help from firing on a special-form head.
        cadenza_compile_abi::TypeInfo::NoDef(_) | cadenza_compile_abi::TypeInfo::Unknown => {
            return None;
        }
    };
    // A signature must be a FUNCTION type — an arrow `(-> …)`. A nullary value (`answer : Int64`, no arrow)
    // is not a callable signature.
    if !arrow.contains("->") {
        return None;
    }
    // Active parameter: how many ARGUMENT forms (children past the head) end at/before the cursor — the
    // args already typed. The current (in-progress) arg is that count (0-based index of the arg being
    // typed). Clamp so a cursor past the last arg still points at the last parameter slot.
    let arg_spans: Vec<_> = children[1..].iter().filter_map(|&c| spans.get(c)).collect();
    let typed = arg_spans.iter().filter(|s| s.end <= byte).count();
    let active = typed as u32;
    // The signature label is the callee with its arrow type — `callee : (-> A B Ret)`.
    let label = format!("{callee} : {arrow}");
    // Per-parameter labels: split the arrow `(-> P1 P2 … Ret)` into its top-level components so the client
    // can BOLD the active parameter's type as you type. The last component is the RETURN type (not a
    // parameter), so only the leading components are parameters. Offsets are code-UNIT ranges into `label`
    // (LSP `LabelOffsets` are UTF-16, exclusive-end) located by finding each component's substring at/after
    // the arrow's position in the label — robust to a type spelling repeating (e.g. `(-> Int64 Int64)`).
    let params = arrow_parameter_components(&arrow);
    let parameters: Vec<ParameterInformation> = if params.len() >= 2 {
        // Drop the final component (the return type); the rest are parameter slots.
        let param_types = &params[..params.len() - 1];
        let label_utf16: Vec<u16> = label.encode_utf16().collect();
        // Byte cursor into `label` for locating each component left-to-right (types can repeat).
        let mut search_from = callee.len(); // start scanning past the callee name
        param_types
            .iter()
            .filter_map(|ty| {
                let at = label[search_from..].find(ty.as_str())? + search_from;
                search_from = at + ty.len();
                // Convert the byte range [at, at+ty.len()) to UTF-16 code-unit offsets into `label`.
                let start = label[..at].encode_utf16().count() as u32;
                let end = start + ty.encode_utf16().count() as u32;
                // Guard against a malformed offset past the label (LabelOffsets must index `label`).
                if (end as usize) <= label_utf16.len() {
                    Some(ParameterInformation {
                        label: ParameterLabel::LabelOffsets([start, end]),
                        documentation: None,
                    })
                } else {
                    None
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    // Clamp the active parameter to a real slot so the client never highlights past the last parameter
    // (a cursor past the final arg, or a variadic-looking over-count, points at the last slot).
    let active = if parameters.is_empty() {
        active
    } else {
        active.min(parameters.len() as u32 - 1)
    };
    let parameters = if parameters.is_empty() {
        None
    } else {
        Some(parameters)
    };
    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label,
            documentation: None,
            parameters,
            active_parameter: Some(active),
        }],
        active_signature: Some(0),
        active_parameter: Some(active),
    })
}

/// The NAME a parameter binder introduces: a BARE name `x`, or the name of an ANNOTATED binder
/// `(: name Type)` (a `:`-headed list whose second child is the name). `None` for any other shape.
fn param_binder_name(
    arenas: &cadenza_syntax::Arenas,
    p: cadenza_syntax::StructId,
) -> Option<String> {
    if let Some(n) = arenas.as_name(p) {
        return Some(n.to_string());
    }
    // `(: name Type)` — the annotated-parameter form; the binder name is the child after the `:` head.
    arenas
        .as_form(p, ":")
        .and_then(|tail| tail.first())
        .and_then(|&n| arenas.as_name(n))
        .map(str::to_string)
}

/// Map each top-level `(def (name param…) …)` in `arenas` to its DECLARED parameter names, in order — the
/// callee→params lookup the parameter-name inlay hints ride on. A bare-value `def name body` (no signature
/// list) or a param-less `(def (name) …)` contributes nothing. Read purely from the parse tree, so it does
/// NOT depend on the still-blocked inferred-binder TYPE query — a parameter's NAME is written in the source
/// even when its type is not. Handles both bare `x` and annotated `(: x Type)` params via
/// [`param_binder_name`].
fn local_def_params(
    arenas: &cadenza_syntax::Arenas,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        let cadenza_syntax::ast::Struct::List(kids) = arenas.get(id) else {
            continue;
        };
        // `(def (name p1 p2 …) body)` — head is `def`, second child is the signature list.
        if kids.first().and_then(|&h| arenas.as_name(h)) != Some("def") {
            continue;
        }
        let Some(&sig) = kids.get(1) else { continue };
        let cadenza_syntax::ast::Struct::List(sig_kids) = arenas.get(sig) else {
            continue;
        };
        let Some(name) = sig_kids.first().and_then(|&h| arenas.as_name(h)) else {
            continue;
        };
        // A parameter is either a BARE name `x` or an ANNOTATED binder `(: name Type)` (a list headed by
        // `:` whose second child is the name). Extract the name from both — an annotated param still has a
        // written NAME, which is all the hint needs (the TYPE half is irrelevant here).
        let param_names: Vec<String> = sig_kids[1..]
            .iter()
            .filter_map(|&p| param_binder_name(arenas, p))
            .collect();
        if !param_names.is_empty() {
            out.insert(name.to_string(), param_names);
        }
    }
    out
}

/// Emit the parameter-name hints for every call in `arenas` whose head names a callee in `params_by_callee`
/// and whose argument START falls within `[range]`. Each positional argument gets an inline `name:` hint,
/// for as many args as the callee has parameters (a mismatched arg count just hints the overlap). Shared by
/// the single-buffer and package (cross-file) entry points — the difference is only WHICH callees populate
/// the map (local defs, plus imported defs in the package case).
fn emit_param_hints(
    text: &str,
    arenas: &cadenza_syntax::Arenas,
    spans: &cadenza_syntax::spans::SpanTable,
    params_by_callee: &std::collections::HashMap<String, Vec<String>>,
    range: Range,
) -> Vec<InlayHint> {
    if params_by_callee.is_empty() {
        return Vec::new();
    }
    // A def's SIGNATURE list `(name param…)` is a name-headed list of the SAME shape as a call, and its
    // head is a known callee — so the walk below would wrongly hint the PARAMETER DECLARATIONS as if they
    // were call arguments (`(scale (factor: (: factor Int64)))`). Collect the signature-list node ids (the
    // second child of every `(def <sig> body)`) and skip them: they are declarations, not calls.
    let mut def_sigs: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        let cadenza_syntax::ast::Struct::List(kids) = arenas.get(id) else {
            continue;
        };
        if kids.first().and_then(|&h| arenas.as_name(h)) == Some("def")
            && let Some(&sig) = kids.get(1)
            && matches!(arenas.get(sig), cadenza_syntax::ast::Struct::List(_))
        {
            def_sigs.insert(sig.0);
        }
    }
    let range_lo = position_to_byte(text, range.start);
    let range_hi = position_to_byte(text, range.end);
    let mut hints: Vec<InlayHint> = Vec::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        // Skip a def's own signature list — it is a declaration, not a call.
        if def_sigs.contains(&id.0) {
            continue;
        }
        let cadenza_syntax::ast::Struct::List(children) = arenas.get(id) else {
            continue;
        };
        let Some(&head) = children.first() else {
            continue;
        };
        let Some(callee) = arenas.as_name(head) else {
            continue;
        };
        let Some(param_names) = params_by_callee.get(callee) else {
            continue;
        };
        for (arg, name) in children[1..].iter().zip(param_names.iter()) {
            // NOISE SUPPRESSION (rust-analyzer's rule): when the argument is itself a bare name IDENTICAL
            // to the parameter, the `name:` hint is pure redundancy (`add(a: a, b: b)`) — skip it. Only a
            // name atom triggers this; a literal / compound arg still gets its hint.
            if arenas.as_name(*arg) == Some(name.as_str()) {
                continue;
            }
            let Some(arg_span) = spans.get(*arg) else {
                continue;
            };
            // Only hint arguments whose START is inside the requested range (the visible viewport).
            if arg_span.start < range_lo || arg_span.start >= range_hi {
                continue;
            }
            hints.push(InlayHint {
                position: byte_to_position(text, arg_span.start),
                label: InlayHintLabel::String(format!("{name}:")),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip: None,
                // Pad the right so the hint reads `name: arg`, not `name:arg`, against the argument.
                padding_left: None,
                padding_right: Some(true),
                data: None,
            });
        }
    }
    hints
}

/// The PARAMETER-NAME inlay hints for the calls in `text` that fall within `range` — each positional
/// argument of a call to a LOCALLY-defined function gets an inline `name:` hint (rust-analyzer's default
/// call hint). Single-buffer: only callees defined in THIS buffer are hinted (a cross-file/imported callee
/// needs the closure — see [`package_inlay_hints_at`]). TOTAL: a buffer that does not parse, or a range
/// with no local call, yields the empty list — never a panic.
fn inlay_hints_at(text: &str, is_ml: bool, range: Range) -> Vec<InlayHint> {
    let Ok((arenas, spans, _errors)) = parse_surface(text, is_ml) else {
        return Vec::new();
    };
    // Two hint modes, both emitted: (1) PARAMETER-NAME hints at call sites (`add(`a:`1)`), and (2) TYPE
    // hints on an un-annotated but INFERABLE parameter binder (`def f(x`: Int64`) = …`) — the latter now
    // that v-inference's TypeAt returns the solved type for such a binder (was `unknown`). A generic
    // param (no single monomorphic type) still answers `unknown` and is skipped.
    let params_by_callee = local_def_params(&arenas);
    let mut hints = emit_param_hints(text, &arenas, &spans, &params_by_callee, range);
    hints.extend(emit_param_type_hints(text, &arenas, &spans, range));
    hints
}

/// TYPE inlay hints on un-annotated, inferable PARAMETER binders — `def f(x`: Int64`) = x + 1` renders
/// the solved type after the binder. For each top-level `(def (name param…) …)`, a param that is a BARE
/// name (not an already-annotated `(: name Type)`) gets its node typed via `TypeAt`; a SOLVED answer (not
/// `unknown`) yields a `: <type>` hint at the binder's end, an `unknown` (a fully-generic param with no
/// single monomorphic type) is skipped. Definition-site hints (distinct from the call-site name hints).
/// Only binders whose start is in `range` are emitted. TypeAt on an un-annotated inferable binder returns
/// the solved type as of v-inference `071ed9642`.
fn emit_param_type_hints(
    text: &str,
    arenas: &cadenza_syntax::Arenas,
    spans: &cadenza_syntax::spans::SpanTable,
    range: Range,
) -> Vec<InlayHint> {
    let range_lo = position_to_byte(text, range.start);
    let range_hi = position_to_byte(text, range.end);
    let mut hints: Vec<InlayHint> = Vec::new();
    for i in 0..arenas.structure.len() {
        let id = cadenza_syntax::StructId(i as u32);
        let cadenza_syntax::ast::Struct::List(kids) = arenas.get(id) else {
            continue;
        };
        // `(def (name p1 p2 …) body)` — head `def`, second child the signature list.
        if kids.first().and_then(|&h| arenas.as_name(h)) != Some("def") {
            continue;
        }
        let Some(&sig) = kids.get(1) else { continue };
        let cadenza_syntax::ast::Struct::List(sig_kids) = arenas.get(sig) else {
            continue;
        };
        // Each param past the sig head. Only a BARE-name param (already-annotated `(: n T)` shows its own
        // type) is a hint candidate.
        for &param in sig_kids.iter().skip(1) {
            if arenas.as_name(param).is_none() {
                continue; // annotated `(: name T)` (a List) or a non-name — skip
            }
            let Some(span) = spans.get(param) else {
                continue;
            };
            if span.start < range_lo || span.start >= range_hi {
                continue;
            }
            let out = crate::run_sidecar(
                arenas,
                cadenza_compile_abi::Request::Query(cadenza_compile_abi::sidecar::Query::TypeAt {
                    node: param.0,
                }),
            );
            let Some(bytes) = out.artifact(cadenza_compile_abi::sidecar::KIND_TYPE_AT) else {
                continue;
            };
            let ty = crate::render_type_at(&cadenza_compile_abi::decode_type_at(bytes));
            let ty = ty.trim();
            // A generic param answers `unknown` (no single monomorphic type) — emit no hint there.
            if ty.is_empty() || ty == "unknown" {
                continue;
            }
            hints.push(InlayHint {
                // The hint sits AFTER the binder name (`x`: Int64`).
                position: byte_to_position(text, span.end),
                label: InlayHintLabel::String(format!(": {ty}")),
                kind: Some(InlayHintKind::TYPE),
                text_edits: None,
                tooltip: None,
                padding_left: None,
                padding_right: None,
                data: None,
            });
        }
    }
    hints
}

/// Cross-file parameter-name inlay hints: like [`inlay_hints_at`] but a callee IMPORTED from a sibling
/// library is also hinted, using that library's `(def (name param…) …)` signature. The entry's own local
/// defs still win a name collision (they're inserted last). `None` when the closure can't load (the caller
/// falls back to the single-buffer path). Increment 2 of the parameter-name inlay-hint feature.
fn package_inlay_hints_at(
    entry_path: &str,
    open: &dyn Fn(&std::path::Path) -> Option<String>,
    entry_text: &str,
    entry_is_ml: bool,
    range: Range,
) -> Option<Vec<InlayHint>> {
    let files = crate::closure::load(entry_path, open).ok()?;
    let (entry_arenas, entry_spans, _e) = parse_surface(entry_text, entry_is_ml).ok()?;
    let mut params_by_callee: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    // Imported callees FIRST, then the entry's own local defs OVERWRITE (a local def of the same spelling
    // wins the hint, matching resolution — same precedence as package completion).
    for (lib_name, imported) in imported_names(&entry_arenas) {
        let Some(lib) = files.iter().find(|f| f.name == lib_name) else {
            continue;
        };
        let lib_params = local_def_params(&lib.arenas);
        for name in imported {
            if let Some(ps) = lib_params.get(&name) {
                params_by_callee.insert(name, ps.clone());
            }
        }
    }
    for (name, ps) in local_def_params(&entry_arenas) {
        params_by_callee.insert(name, ps);
    }
    Some(emit_param_hints(
        entry_text,
        &entry_arenas,
        &entry_spans,
        &params_by_callee,
        range,
    ))
}

/// Reprint `text` canonically in its OWN surface (ML for `.cdz`/`.ml`, s-expr for `.sexp`/`.sexpr`) — the
/// in-memory core of `cdz fmt`, so an editor format is byte-identical to the CLI. Uses
/// `convert::convert_with(from, from)`: a SAME-surface round-trip that reprints the parse tree canonically
/// (never a cross-surface conversion). Returns `None` when the buffer does not parse cleanly — `convert`'s
/// reader rejects a program that only survives with recovered errors, so a broken buffer is NOT rewritten
/// to a patched-up shape (matching `cdz fmt`'s fail-safe). The result carries no trailing newline from the
/// printer; a single `\n` is appended so a formatted buffer is newline-terminated + stable under re-format
/// (the CLI's convention).
fn format_document(text: &str, is_ml: bool) -> Option<String> {
    let surface = if is_ml {
        cadenza_syntax::convert::Format::Ml
    } else {
        cadenza_syntax::convert::Format::Sexpr
    };
    let out = cadenza_syntax::convert::convert_with(
        text.as_bytes(),
        surface,
        surface,
        cadenza_syntax::convert::Options::default(),
    )
    .ok()?;
    let mut formatted = String::from_utf8(out).ok()?;
    if !formatted.ends_with('\n') {
        formatted.push('\n');
    }
    Some(formatted)
}

/// The LSP `Position` one past the END of `text` — line = the number of line-breaks, column 0. Paired with
/// `(0,0)` it is an end-exclusive range covering every existing byte, so a full-document `TextEdit` over it
/// replaces the whole buffer regardless of the final line's content.
fn full_document_end(text: &str) -> Position {
    let line = text.matches('\n').count() as u32;
    Position::new(line, 0)
}

/// Split a rendered arrow type into `[Param1, …, ParamN, Ret]`. The `TypeOf` query renders function types
/// CURRIED — a 2-arg function is `(-> Int64 (-> Int64 Int64))`, not `(-> Int64 Int64 Int64)` — so this
/// UNFOLDS the curry: each `(-> Head Rest)` contributes `Head` as a parameter, then recurses into `Rest`
/// when `Rest` is itself an arrow; the final non-arrow `Rest` is the return type. Paren depth is tracked so
/// a nested NON-arrow type stays one component (`(List a)` is one param). A curried tail is fully unfolded
/// because the s-expr call `(f a b)` applies every level, so `(-> (List a) (-> A B))` → `["(List a)",
/// "A", "B"]` (two fillable params + a return). Returns empty if `arrow` is not a `(-> …)` form or is a
/// nullary `(-> Ret)`. The caller treats the last element as the return type and the leading ones as
/// parameter slots.
fn arrow_parameter_components(arrow: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = arrow.to_string();
    loop {
        let Some(inner) = rest.strip_prefix("(->").and_then(|s| s.strip_suffix(')')) else {
            // A non-arrow tail is the return type (or the whole thing was not an arrow at all).
            if !out.is_empty() {
                out.push(rest);
            }
            return out;
        };
        // Split the inner into TOP-LEVEL components (depth-aware). A curried arrow has exactly two: the
        // parameter type and the rest. A rendered arrow with >2 top-level components (a defensive path for
        // any un-curried render) is treated as flat: all but the last are parameters.
        let comps = split_top_level(inner);
        if comps.len() < 2 {
            // Degenerate `(-> Ret)` (nullary) — no parameters, just a return; nothing to highlight.
            return out;
        }
        if comps.len() > 2 {
            // Flat render fallback: every leading component is a parameter, the last is the return.
            for c in comps {
                out.push(c);
            }
            return out;
        }
        // Curried: first component is a parameter, second is the rest to unfold.
        out.push(comps[0].clone());
        rest = comps[1].clone();
    }
}

/// Split a whitespace-separated component list at TOP-LEVEL only (paren-depth aware) so a nested
/// parenthesised type stays a single component.
fn split_top_level(inner: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
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
    let Some(diag_bytes) = run_query_bytes(
        &arenas,
        cadenza_compile_abi::sidecar::Query::Diagnostics,
        cadenza_compile_abi::sidecar::KIND_DIAGNOSTICS,
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
    // The KIND_DIAGNOSTICS wire is canonical binary AST — decode to fault STRUCTS + read fields directly.
    for d in cadenza_compile_abi::decode_diagnostics(&diag_bytes) {
        // No fix, or the `<error>`-placeholder cascade → no action.
        let Some(fix) = &d.fix else {
            continue;
        };
        if d.message.contains("`<error>`") {
            continue;
        }
        let fix_kind = fix_kind_str(fix.kind);
        // The FAULT's own node range — used to filter to diagnostics overlapping the request range, so we
        // only offer a fix for a squiggle at/around the cursor (the client passes the cursor/selection).
        let Some(fault_range) = d
            .node
            .and_then(|id| spans.get(cadenza_syntax::StructId(id)))
            .map(|s| byte_range_to_range(text, s.start, s.end))
        else {
            continue;
        };
        if !ranges_overlap(fault_range, range) {
            continue;
        }
        // Build the fix's primitive byte edits via the SHARED engine, then map each to an LSP TextEdit.
        let Some(edits) = crate::fix::fix_edits(
            text,
            &tree,
            &origins,
            &spans,
            fix_kind,
            cadenza_syntax::StructId(fix.node),
            &fix.replacement,
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
            title: code_action_title(fix_kind, &fix.replacement, &d.message),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![Diagnostic {
                range: fault_range,
                severity: Some(match d.severity {
                    cadenza_compile_abi::Severity::Error => DiagnosticSeverity::ERROR,
                    cadenza_compile_abi::Severity::Warning => DiagnosticSeverity::WARNING,
                }),
                code: d.code.clone().map(lsp_types::NumberOrString::String),
                source: Some("cdz".to_string()),
                message: d.message.clone(),
                ..Default::default()
            }]),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            // A VERIFIED fix (re-checked to clear its diagnostic without introducing new errors) is the
            // preferred one, so the editor's default quick-fix applies the trustworthy edit.
            is_preferred: Some(fix.verified),
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
        cadenza_compile_abi::sidecar::encode(&[cadenza_compile_abi::Request::Query(
            cadenza_compile_abi::sidecar::Query::Highlight,
        )]);
    let inputs = vec![
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::Artifact::KIND_AST,
            "main",
            ast_bytes,
        ),
        cadenza_compile_abi::Artifact::new(
            cadenza_compile_abi::sidecar::KIND_SIDECAR,
            "drive",
            sidecar_bytes,
        ),
    ];
    let compiled = rcdzc::run_with_compiler_stack(|| rcdzc::compile(&inputs, &[]));
    let Some(bytes) = compiled.artifact(cadenza_compile_abi::sidecar::KIND_HIGHLIGHT) else {
        return Vec::new();
    };

    // Gather each classified leaf as an absolute (line, start-char, length, token-type) tuple, decoded
    // from the canonical binary-AST wire (`highlight_wire`, ZERO string parsing). The Highlight query
    // already emits leaves in ascending node-id order, but node id is not source order, so sort by
    // (line, start) before delta-encoding (LSP requires ascending position).
    let mut abs: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (id, kind) in cadenza_compile_abi::decode_highlight(bytes) {
        let Some(token_type) = highlight_kind_to_token_index(&kind) else {
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

    /// Build one incremental content change over a `[(l0,c0)..(l1,c1))` range with replacement `text`.
    fn ranged_change(
        l0: u32,
        c0: u32,
        l1: u32,
        c1: u32,
        text: &str,
    ) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: Some(Range::new(Position::new(l0, c0), Position::new(l1, c1))),
            range_length: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn apply_content_change_splices_a_ranged_edit() {
        // A single-line replace: swap `cd` (cols 0..2 on line 1) for `XY`.
        let mut text = "ab\ncd".to_string();
        apply_content_change(&mut text, ranged_change(1, 0, 1, 2, "XY"));
        assert_eq!(text, "ab\nXY");
        // A pure INSERTION is a zero-width range: insert `!` at (0,2), pushing the newline right.
        let mut text = "ab\ncd".to_string();
        apply_content_change(&mut text, ranged_change(0, 2, 0, 2, "!"));
        assert_eq!(text, "ab!\ncd");
        // A pure DELETION is an empty replacement over a span: delete `b\nc` (0,1)..(1,1).
        let mut text = "ab\ncd".to_string();
        apply_content_change(&mut text, ranged_change(0, 1, 1, 1, ""));
        assert_eq!(text, "ad");
    }

    #[test]
    fn apply_content_change_without_a_range_replaces_the_whole_document() {
        // A change with no range is a full-document replace (a client may still send one under INCREMENTAL).
        let mut text = "old contents".to_string();
        apply_content_change(
            &mut text,
            TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "brand new".to_string(),
            },
        );
        assert_eq!(text, "brand new");
    }

    #[test]
    fn apply_content_change_applies_a_sequence_relative_to_prior_edits() {
        // The LSP spec: within one notification, each change's range is relative to the text as left by
        // the PRIOR change. Insert `X` at (0,1) → `aXbc`, then a second edit at (0,3) (now the `c`) → `aXbY`.
        let mut text = "abc".to_string();
        apply_content_change(&mut text, ranged_change(0, 1, 0, 1, "X")); // aXbc
        apply_content_change(&mut text, ranged_change(0, 3, 0, 4, "Y")); // replace `c` → aXbY
        assert_eq!(text, "aXbY");
    }

    #[test]
    fn apply_content_change_clamps_a_degenerate_range_without_panicking() {
        // A malformed range with end BEFORE start collapses to an insertion at the lower offset — a tooling
        // read stays total (never a `replace_range` panic on an inverted span).
        let mut text = "abcd".to_string();
        apply_content_change(&mut text, ranged_change(0, 3, 0, 1, "!"));
        // The `[1,3)` span (`bc`) is replaced by `!` — `a!d`.
        assert_eq!(text, "a!d");
        // An entirely out-of-range span clamps to the text end and appends.
        let mut text = "ab".to_string();
        apply_content_change(&mut text, ranged_change(9, 0, 9, 0, "Z"));
        assert_eq!(text, "abZ");
    }

    #[test]
    fn apply_content_change_maps_utf16_columns_to_utf8_bytes() {
        // A range column counts UTF-16 units (LSP), so an edit after a multibyte char resolves to the right
        // byte. `€` is 3 UTF-8 bytes but 1 UTF-16 unit; replace the `x` at column 1 with `Y`.
        let mut text = "€x".to_string();
        apply_content_change(&mut text, ranged_change(0, 1, 0, 2, "Y"));
        assert_eq!(text, "€Y");
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
        // EXACT render, not a `contains("Int")` substring: `KIND_TYPE_AT` is a binary-AST wire, and the
        // signature-help sibling regressed for a release when its render fell back to the raw `cdzast…`
        // payload — which still CONTAINS the type bytes, so a substring check passed on garbage. Pin the
        // clean render so a decoder-skipping regression fails loudly here too.
        assert_eq!(
            rendered, "answer : Int64",
            "hover reports the callee name + its cleanly-rendered inferred type"
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
    fn hover_resolves_correctly_when_a_multibyte_char_precedes_the_cursor_on_the_same_line() {
        // End-to-end position-bridge pin THROUGH a real provider (hover), not just the byte<->position
        // unit tests. `position_to_byte` resets its column counter per line, so the divergence between a
        // cursor's UTF-16 column and its UTF-8 byte offset must come from a multibyte char EARLIER ON THE
        // SAME LINE. Here `café` (`é` = 2 UTF-8 bytes / 1 UTF-16 unit) appears twice on line 1; the SECOND
        // use sits at UTF-16 col 21 but byte offset 22 (the first `café`'s `é` shifts bytes past columns).
        // Hover that second use: if the bridge mis-walked the multibyte char it would land on the wrong
        // node (or none) and fail to report the Int type. Multibyte identifiers are accepted by the
        // front-end (`cdz check` passes on this source).
        let text = "def café = 42\ndef total() = café + café";
        // The SECOND `café` use on line 1 starts at UTF-16 col 21 (byte 22 — the divergence proves the walk).
        let h = hover_at(text, true, Position::new(1, 21))
            .expect("a hover on the second multibyte-preceded use");
        let rendered = match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        assert!(
            rendered.contains("Int"),
            "hover on a use preceded by a same-line multibyte char should still report the Int type, \
             got: {rendered}"
        );
    }

    // A hover result is WELL-FORMED when it is either `None` (no node) or a `Some` whose range (if
    // present) has start <= end AND both endpoints map back to CHAR-BOUNDARY byte offsets in `text`. This
    // is the POSITIVE contract for the multibyte no-panic tests below: asserting it (not merely "the call
    // returned") means a silently-corrupt result — a range split across a multibyte char, or a crash that
    // aborts before returning — fails the test, per the PR #1173/#1207 review that a bare "did not panic"
    // is also satisfied by a silent abort (#1263).
    fn assert_hover_is_well_formed(text: &str, h: &Option<Hover>) {
        if let Some(hover) = h
            && let Some(range) = hover.range
        {
            let start = position_to_byte(text, range.start);
            let end = position_to_byte(text, range.end);
            assert!(
                start <= end,
                "hover range start {start} must be <= end {end}"
            );
            assert!(
                text.is_char_boundary(start) && text.is_char_boundary(end),
                "hover range must map to char boundaries (start {start}, end {end}) in {text:?}"
            );
        }
    }

    #[test]
    fn hover_on_a_column_at_and_past_a_multibyte_char_is_well_formed() {
        // The LSP analogue of the cdz-type-at multibyte-cursor pin (#1173): a column AT and PAST a
        // multibyte char must map to a char-boundary byte offset (`position_to_byte` advances by
        // `len_utf16` per char), so no downstream `&text[off..]` slice can split a UTF-8 sequence and
        // panic. `é` is a BMP char (2 UTF-8 bytes, 1 UTF-16 unit), so an LSP client can only address a
        // column AT or AFTER it — not inside it (that needs a surrogate pair; see the astral test below).
        // Assert the POSITIVE contract (per PR #1263 review): the cursor ON the `café` name (col 7, the
        // `é`) resolves to a hover reporting the Int type, and a column just PAST it stays well-formed
        // (None or a valid range) — a silent crash would abort before returning and fail these asserts.
        let text = "def café = 42";
        // Col 7 is the `é` inside the `café` NAME — hover must resolve the def and report its type.
        let on_name = hover_at(text, true, Position::new(0, 7));
        assert_hover_is_well_formed(text, &on_name);
        let rendered = match &on_name.expect("hover on the café name resolves").contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        assert!(
            rendered.contains("Int"),
            "hover on the multibyte name must report the Int type, got: {rendered}"
        );
        // Col 8 is the SPACE right after `café` (byte 9; the `=` is col 9) — total: whatever it returns
        // must be well-formed.
        assert_hover_is_well_formed(text, &hover_at(text, true, Position::new(0, 8)));
    }

    #[test]
    fn hover_on_a_column_inside_an_astral_surrogate_pair_is_well_formed() {
        // The genuine "column inside a scalar" edge Copilot flagged (PR #1207): only a NON-BMP char
        // (`𝟙`, U+1D7D9 — 4 UTF-8 bytes, 2 UTF-16 code units = a surrogate pair) has a UTF-16 column
        // strictly INSIDE it that an LSP client can send. `position_to_byte` counts UTF-16 units per char,
        // so a column landing between the pair's two units must still resolve to a CHAR BOUNDARY (the
        // start or end of the scalar), never a byte splitting the 4-byte sequence — no `&text[off..]`
        // panic. The astral char sits in a `///` doc comment (accepted by the front-end; `cdz check`
        // passes). The `𝟙` starts at UTF-16 col 8 and spans [8, 10); col 9 is strictly inside it. Assert
        // the POSITIVE contract (per PR #1263 review): each result is well-formed (None or a valid
        // char-boundary range), so a silent abort on a mid-surrogate column fails the test.
        let text = "/// doc 𝟙 more\ndef answer = 42";
        assert_hover_is_well_formed(text, &hover_at(text, true, Position::new(0, 9))); // inside the pair
        assert_hover_is_well_formed(text, &hover_at(text, true, Position::new(0, 8))); // at the pair start
        assert_hover_is_well_formed(text, &hover_at(text, true, Position::new(0, 10))); // just past the pair
    }

    #[test]
    fn hover_on_a_documented_def_shows_the_type_and_the_docstring() {
        // Hovering a use of a DOCUMENTED definition shows both its type (`TypeAt`) and its doc prose
        // (`DocAt`) as Markdown — the `///` doc comment is surfaced in the hover popup.
        let text = "/// Doubles its argument.\ndef double(x: Int64) -> Int64 = x + x\ndef use = double(21)";
        // Cursor on the `double` USE in the last line (col 10 of `def use = double(21)`).
        let h = hover_at(text, true, Position::new(2, 10)).expect("a hover");
        let rendered = match &h.contents {
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("expected Markdown hover with a doc, got: {other:?}"),
        };
        assert!(
            rendered.contains("Doubles its argument."),
            "the hover should include the docstring, got: {rendered}"
        );
        assert!(
            rendered.contains("->") || rendered.contains("Int"),
            "the hover should still include the type, got: {rendered}"
        );
    }

    #[test]
    fn hover_contents_without_a_doc_stays_a_plain_type_string() {
        // No docstring → the hover keeps the plain type `MarkedString` (bare hover unchanged, no Markdown).
        match hover_contents("Int64", None) {
            HoverContents::Scalar(MarkedString::String(s)) => assert_eq!(s, "Int64"),
            other => panic!("expected a plain scalar type, got: {other:?}"),
        }
        // With a doc → Markdown carrying both.
        match hover_contents("Int64", Some("The answer.")) {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("Int64") && m.value.contains("The answer."));
            }
            other => panic!("expected Markdown, got: {other:?}"),
        }
    }

    #[test]
    fn hover_on_the_definition_itself_shows_its_docstring() {
        // The LSP hover-doc analogue of the cdz-doc-at "on the definition" case (#1291): the existing
        // documented-def test hovers a USE (call site); pin the DEFINITION SITE too — hovering the def's
        // own NAME must also surface its `///` doc, not only a downstream reference. Cursor on `double`
        // in the def line (line 1, col 4).
        let text = "/// Doubles its argument.\ndef double(x: Int64) -> Int64 = x + x\ndef use = double(21)";
        let h = hover_at(text, true, Position::new(1, 4)).expect("a hover on the definition name");
        let rendered = match &h.contents {
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("expected Markdown hover with a doc on the def site, got: {other:?}"),
        };
        assert!(
            rendered.contains("Doubles its argument."),
            "hovering the definition itself should surface its docstring, got: {rendered}"
        );
    }

    #[test]
    fn hover_on_an_undocumented_def_is_a_plain_type_without_markdown() {
        // The LSP hover-doc analogue of the cdz-doc-at "undocumented node → no documentation" case
        // (#1291): pin END-TO-END through `hover_at` (not just the `hover_contents` helper) that an
        // undocumented-but-valid def hovers to a PLAIN type string, never an empty/spurious Markdown doc
        // block. Cursor on the `double` def name (line 0, col 4) — no `///` precedes it.
        let text = "def double(x: Int64) -> Int64 = x + x\ndef use = double(21)";
        let h = hover_at(text, true, Position::new(0, 4)).expect("a hover on the undocumented def");
        match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => assert!(
                s.contains("->") || s.contains("Int"),
                "an undocumented def hovers to a plain type string, got: {s}"
            ),
            other => panic!("expected a plain scalar type (no doc Markdown), got: {other:?}"),
        }
    }

    #[test]
    fn hover_on_a_use_of_a_let_local_reports_its_inferred_type() {
        // A `let`-bound local has NO written annotation, yet a USE of it hovers to its INFERRED type: the
        // `TypeAt` query resolves a use-site name to the solved type of what it refers to. This is the one
        // inferred-type read that works today, and it's the near edge of the inlayHint frontier — inlayHint
        // needs the BINDER's inferred type (still `unknown`, tracked in the query-typeat-on-unannotated-binder
        // issue), but a use already resolves. Pin it so a change to the type-query can't silently drop it.
        let text = "def main() =\n  let n = 5\n  n + 1";
        // Cursor on the `n` USE in `n + 1` (line 2, col 2).
        let h = hover_at(text, true, Position::new(2, 2)).expect("a hover on the let-local use");
        let rendered = match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        // EXACT render (see `hover_on_a_definition_reports_its_type` for why substring checks are unsafe on
        // a binary-AST wire): the inferred scalar renders cleanly as `Int64`, not a `cdzast…`-embedded `Int`.
        assert_eq!(
            rendered, "Int64",
            "hover on a let-local use reports its cleanly-rendered inferred Int64 type"
        );
    }

    #[test]
    fn hover_on_a_use_of_a_let_local_reports_a_compound_inferred_type() {
        // The near-edge inferred read resolves for a COMPOUND type, not just a scalar: a `let`-bound local
        // whose RHS is a list literal hovers to its full inferred `(List Int64)` at a use site. This pins
        // that the inferred-type render survives structure (the type string is the compound form, not a
        // flattened scalar or "unknown") — the shape inlayHint will ultimately surface for a let binder.
        let text = "def main() =\n  let xs = [1, 2, 3]\n  xs";
        // Cursor on the `xs` USE on the last line (line 2, col 2).
        let h = hover_at(text, true, Position::new(2, 2))
            .expect("a hover on the compound let-local use");
        let rendered = match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        // EXACT compound render: pins that the structured type survives as `(List Int64)`, not a flattened
        // scalar, an `unknown`, or the raw `cdzast…` wire (a substring `contains("List") && contains("Int")`
        // would pass on the undecoded payload — the binary-wire regression class this whole family guards).
        assert_eq!(
            rendered, "(List Int64)",
            "hover on a compound let-local use reports its cleanly-rendered inferred (List Int64) type"
        );
    }

    #[test]
    fn hover_on_an_unannotated_but_inferable_parameter_shows_the_solved_type() {
        // FLIPPED (was `..._is_none_not_a_misleading_type`) when v-inference landed the inferred-binder
        // `TypeAt` query (rcdzc `query_param_ty`): an UN-ANNOTATED but locally-inferable parameter now
        // hovers as its SOLVED type — the unblock signal for inlayHint (inline `param: <inferred>`). In
        // `def f(x) = x + 1`, `x`'s use as `(+ x 1)` pins it to `Int64`, so BOTH the binder and its use
        // hover `Int64` (the binder via the new inferred-param query; the use via ordinary inlining). A
        // genuinely-generic param (`def id(x) = x`) still hovers nothing — covered by the rcdzc-side
        // `hover_on_a_fully_generic_param_binder_stays_unknown` test; here we pin the inferable case.
        let text = "def f(x) = x + 1";
        let rendered_at = |col: u32, what: &str| -> String {
            let h = hover_at(text, true, Position::new(0, col))
                .unwrap_or_else(|| panic!("{what} now hovers its inferred type"));
            match &h.contents {
                HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
                other => panic!("unexpected hover contents at {what}: {other:?}"),
            }
        };
        // Binder `x` in `f(x)` (col 6) and its use in `x + 1` (col 11) — both now report the inferred Int64.
        // EXACT render on both the binder and the use (substring checks are unsafe on the binary-AST wire —
        // see `hover_on_a_definition_reports_its_type`): each is exactly `Int64`, not a `cdzast…`-embedded one.
        let binder = rendered_at(6, "the unannotated param binder");
        assert_eq!(
            binder, "Int64",
            "the binder hovers its cleanly-rendered inferred Int64"
        );
        let use_ = rendered_at(11, "the use of the unannotated param");
        assert_eq!(
            use_, "Int64",
            "the use hovers its cleanly-rendered inferred Int64"
        );
    }

    #[test]
    fn highlight_kind_map_covers_the_whole_query_vocabulary() {
        // Every `HighlightKind` the query can emit must map to a legend index whose value is in range of
        // the published legend — otherwise that kind renders unclassified. Iterate the PRODUCER's canonical
        // vocabulary (`rcdzc::sidecar::HighlightKind::ALL`) rather than a hand-maintained spelling list, so
        // a NEW kind added upstream is automatically exercised here and FAILS this test until it gets a
        // legend mapping — closing the producer→consumer loop (ALL guarantees completeness upstream; this
        // proves the semantic-token legend consumes the whole of it). The wire spelling is the query's
        // per-token second column, `HighlightKind::as_str`.
        for kind in rcdzc::sidecar::HighlightKind::ALL {
            let spelling = kind.as_str();
            let idx = highlight_kind_to_token_index(spelling).unwrap_or_else(|| {
                panic!("highlight kind `{spelling}` has no semantic-token legend index")
            });
            assert!(
                (idx as usize) < SEMANTIC_TOKEN_TYPES.len(),
                "index {idx} for `{spelling}` is out of legend range"
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
    fn semantic_tokens_skip_a_leaf_that_crosses_a_line_boundary() {
        // LSP semantic tokens are SINGLE-LINE (delta-encoded per line); a leaf whose span crosses a line
        // boundary — a multi-line string literal — must be SKIPPED (left to the editor's lexical fallback)
        // rather than emitted as a token whose length would run past the end of its start line. Pins the
        // `start.line != end.line` guard so a regression can't emit an out-of-line token that a strict
        // client rejects. Every EMITTED token must be single-line; the tokens on the other lines still
        // appear (the skip is surgical to the crossing leaf).
        let src = "(module m (def (msg) \"line one\nline two\") (export msg))";
        let toks = semantic_tokens_for(src, false);
        assert!(
            !toks.is_empty(),
            "the single-line tokens (keyword/name/…) are still emitted around the skipped string"
        );
        // Reconstruct each token's absolute (line, start, length) and confirm none is the multi-line
        // string: the string starts on line 0 at the `\"` and ends on line 1, so if it were NOT skipped it
        // would appear as a line-0 token whose start.character + length overruns line 0's content. The
        // strongest invariant: the delta stream stays ascending + non-overlapping AND every token's length
        // fits within its line — `assert_no_overlap` already checks ascending/non-overlap; here we assert
        // no token claims to start on the line where the multi-line string BEGINS at its column.
        assert_no_overlap(&toks);
        // The `msg` name appears twice (def + export) and both are single-line `variable` tokens — proving
        // the walk continued past the skipped string rather than aborting.
        let string_line0_col = {
            // Column (UTF-16) of the opening quote on line 0.
            let byte = src.find('"').expect("a string literal");
            byte_to_position(src, byte).character
        };
        let mut line = 0u32;
        let mut start = 0u32;
        for t in &toks {
            line += t.delta_line;
            start = if t.delta_line == 0 {
                start + t.delta_start
            } else {
                t.delta_start
            };
            assert!(
                !(line == 0 && start == string_line0_col),
                "the multi-line string literal at line 0 col {string_line0_col} must be SKIPPED, not emitted"
            );
        }
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
    fn lsp_handles_a_forall_binder_annotation_now_that_it_is_semantically_live() {
        // The `forall a. T` type-annotation surface became semantically live (compile+run+monomorphize,
        // rcdzc 363e57f53) — it was syntax-only before and the checker rejected it (CDZ0101). Now that it
        // is a STABLE surface, pin that the LSP handles a forall-annotated def cleanly on the ML surface:
        // (1) diagnostics are EMPTY (it typechecks, no false "unbound name `a`"); (2) semantic tokens are
        // total (a token stream, no panic); (3) hover on the forall-quantified parameter yields a type,
        // not an error. Guards against a future change silently regressing the editor's forall support.
        let text = "def id(x: forall a. a) -> a = x\ndef main = 0";
        let diags = diagnostics_for(text, true);
        assert!(
            diags.is_empty(),
            "a valid forall-annotated def must typecheck clean in-editor (no false CDZ0101): {diags:?}"
        );
        let toks = semantic_tokens_for(text, true);
        assert!(
            !toks.is_empty(),
            "semantic tokens over a forall def must be a total, non-empty stream"
        );
        // Hover on the `x` parameter (line 0, char 7 — `def id(` is 7 chars).
        assert!(
            hover_at(text, true, Position::new(0, 7)).is_some(),
            "hover on a forall-quantified parameter must yield a type, not None/error"
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
        let d = cadenza_compile_abi::Diagnostic {
            severity: cadenza_compile_abi::Severity::Error,
            code: Some("CDZ0101".into()),
            message: "unbound name `<error>`".into(),
            node: None,
            fix: None,
        };
        assert!(parse_diag_line(&d, "x", &spans).is_none());
    }

    #[test]
    fn parse_diag_line_maps_a_warning_with_no_code() {
        // A warning-severity, uncoded fault maps to a WARNING diagnostic with no code, at the doc start
        // when its node is unanchored (`-`).
        let spans = cadenza_syntax::parser::read_ml("x").spans;
        let fault = cadenza_compile_abi::Diagnostic {
            severity: cadenza_compile_abi::Severity::Warning,
            code: None,
            message: "something is unused".into(),
            node: None,
            fix: None,
        };
        let d = parse_diag_line(&fault, "x", &spans).expect("a diagnostic");
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

    /// A `Server` wired to an in-memory connection, plus the CLIENT end so a test can drain the
    /// notifications the server publishes (e.g. `publishDiagnostics` via `client.receiver`). Drives the
    /// real notification dispatch (`handle_notification`), not just the pure helpers — so it exercises the
    /// didOpen/didChange wiring end to end.
    fn memory_server() -> (Server, Connection) {
        let (server_conn, client_conn) = Connection::memory();
        (Server::new(server_conn), client_conn)
    }

    /// Build a `didChange` notification with a single ranged content change over `[(l0,c0)..(l1,c1))`.
    fn did_change_note(uri: &Uri, l0: u32, c0: u32, l1: u32, c1: u32, text: &str) -> Notification {
        let params = DidChangeTextDocumentParams {
            text_document: lsp_types::VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 0,
            },
            content_changes: vec![ranged_change(l0, c0, l1, c1, text)],
        };
        Notification::new(
            DidChangeTextDocument::METHOD.to_string(),
            serde_json::to_value(params).unwrap(),
        )
    }

    /// Build a `didOpen` notification for `text` at `uri`.
    fn did_open_note(uri: &Uri, text: &str) -> Notification {
        let params = DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri: uri.clone(),
                language_id: "cadenza".to_string(),
                version: 0,
                text: text.to_string(),
            },
        };
        Notification::new(
            DidOpenTextDocument::METHOD.to_string(),
            serde_json::to_value(params).unwrap(),
        )
    }

    #[test]
    fn capabilities_advertise_incremental_sync() {
        // The server must advertise INCREMENTAL text-document sync (not FULL) so clients send only the
        // changed ranges — the wire contract the `apply_content_change` splice path depends on. Assert on
        // the serialized capabilities (exactly what the client reads).
        let value = serde_json::to_value(capabilities()).expect("serializes");
        // `textDocumentSync` serializes to the numeric SyncKind: INCREMENTAL == 2 (FULL == 1, NONE == 0).
        assert_eq!(
            value.get("textDocumentSync").and_then(|v| v.as_u64()),
            Some(2),
            "the server must advertise INCREMENTAL (2) text-document sync: {value}"
        );
    }

    #[test]
    fn did_change_applies_incremental_edits_through_the_handler() {
        // End-to-end through the real notification dispatch: open a document, then send a SEQUENCE of
        // incremental `didChange` edits, and confirm the server's in-memory buffer ends in the spliced
        // state a subsequent query would read. This pins the didChange HANDLER wiring (each change applied
        // in order to the live buffer), not just the `apply_content_change` helper in isolation.
        let (mut server, client) = memory_server();
        let uri = test_uri();
        // Open `def main = 1` (a clean nullary def).
        server
            .handle_notification(did_open_note(&uri, "def main = 1"))
            .expect("didOpen dispatches");
        // Edit 1: replace the `1` (col 11..12) with `2` → `def main = 2`.
        server
            .handle_notification(did_change_note(&uri, 0, 11, 0, 12, "2"))
            .expect("didChange 1 dispatches");
        // Edit 2: insert ` + 3` at end (col 12) → `def main = 2 + 3`.
        server
            .handle_notification(did_change_note(&uri, 0, 12, 0, 12, " + 3"))
            .expect("didChange 2 dispatches");
        assert_eq!(
            server.docs.get(&uri).map(|d| d.text.as_str()),
            Some("def main = 2 + 3"),
            "the incremental edits splice into the live buffer in order"
        );
        // Each open/change publishes diagnostics — drain the client end and confirm we got at least one
        // `publishDiagnostics` (the wiring reaches the transport), and that the final buffer is clean
        // (no diagnostics on the last publish).
        let mut last_diags: Option<usize> = None;
        while let Ok(Message::Notification(n)) = client.receiver.try_recv() {
            if n.method == PublishDiagnostics::METHOD {
                let p: PublishDiagnosticsParams = serde_json::from_value(n.params).unwrap();
                last_diags = Some(p.diagnostics.len());
            }
        }
        assert_eq!(
            last_diags,
            Some(0),
            "the final `def main = 2 + 3` is clean → the last publishDiagnostics carries zero diagnostics"
        );
    }

    #[test]
    fn did_change_before_open_starts_from_an_empty_buffer() {
        // A `didChange` for a URI the server has not seen (a change before an open — shouldn't happen per
        // the protocol, but a robust server must not drop it or panic). The handler starts from an empty
        // buffer, so an insertion-at-origin change becomes the whole content. Total, never a panic.
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        server
            .handle_notification(did_change_note(&uri, 0, 0, 0, 0, "def main = 7"))
            .expect("a didChange before open is handled, not dropped");
        assert_eq!(
            server.docs.get(&uri).map(|d| d.text.as_str()),
            Some("def main = 7"),
            "a pre-open change applies against an empty base rather than being lost"
        );
    }

    /// Build a `didClose` notification for `uri`.
    fn did_close_note(uri: &Uri) -> Notification {
        let params = DidCloseTextDocumentParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
        };
        Notification::new(
            DidCloseTextDocument::METHOD.to_string(),
            serde_json::to_value(params).unwrap(),
        )
    }

    #[test]
    fn did_close_drops_the_buffer_and_clears_its_diagnostics() {
        // Closing a document must (a) forget its buffer (so a later query can't read stale text) and
        // (b) publish an EMPTY diagnostic list for its URI, so the editor clears any squiggle it was
        // showing for a file no longer open. Drive it through the real handler and confirm both.
        let (mut server, client) = memory_server();
        let uri = test_uri();
        // Open a program WITH an error (an unbound name) so there's a diagnostic to clear on close.
        server
            .handle_notification(did_open_note(&uri, "def main = nope"))
            .expect("didOpen dispatches");
        // The open published at least one diagnostic (the unbound `nope`); drain up to the last publish.
        let mut open_diags: Option<usize> = None;
        while let Ok(Message::Notification(n)) = client.receiver.try_recv() {
            if n.method == PublishDiagnostics::METHOD {
                let p: PublishDiagnosticsParams = serde_json::from_value(n.params).unwrap();
                open_diags = Some(p.diagnostics.len());
            }
        }
        assert_eq!(
            open_diags,
            Some(1),
            "opening `def main = nope` publishes one diagnostic (the unbound name)"
        );
        // Now close it.
        server
            .handle_notification(did_close_note(&uri))
            .expect("didClose dispatches");
        // (a) the buffer is forgotten.
        assert!(
            !server.docs.contains_key(&uri),
            "a closed document is removed from the open set"
        );
        // (b) the close published an EMPTY diagnostic list for the URI (clears the squiggle).
        let mut close_diags: Option<usize> = None;
        while let Ok(Message::Notification(n)) = client.receiver.try_recv() {
            if n.method == PublishDiagnostics::METHOD {
                let p: PublishDiagnosticsParams = serde_json::from_value(n.params).unwrap();
                assert_eq!(p.uri, uri, "the clear is published for the closed URI");
                close_diags = Some(p.diagnostics.len());
            }
        }
        assert_eq!(
            close_diags,
            Some(0),
            "closing publishes an empty diagnostic list so the editor clears stale errors"
        );
    }

    #[test]
    fn serve_dispatches_a_request_and_terminates_on_shutdown() {
        // End-to-end over the REAL receive loop (`serve`), not a direct handler call: the client sends a
        // `didOpen`, a `hover` REQUEST, then the `shutdown` request + `exit` notification. Asserts (1) the
        // request is routed to the handler and a Response comes back on the wire (the request→response
        // cycle `serve`+`handle_request` drive — untested before; every other test calls handlers
        // directly); (2) `serve` RETURNS after `shutdown`/`exit` (the loop terminates, not hangs). Runs the
        // server on a thread so the single-threaded memory channel does not deadlock.
        let (server_conn, client) = Connection::memory();
        let handle = std::thread::spawn(move || {
            let mut server = Server::new(server_conn);
            server.serve()
        });
        let uri = test_uri();
        // Open a doc so the hover has something to resolve.
        client
            .sender
            .send(Message::Notification(did_open_note(
                &uri,
                "def answer = 42",
            )))
            .unwrap();
        // A hover request on the `answer` name (line 0, col 4).
        let hover_params = HoverParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 4),
            },
            work_done_progress_params: Default::default(),
        };
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                HoverRequest::METHOD.to_string(),
                hover_params,
            )))
            .unwrap();
        // Drain until we see the hover Response (the server also pushes a publishDiagnostics notification
        // on didOpen — skip notifications, wait for the response to our request id).
        let mut got_hover = false;
        loop {
            match client.receiver.recv() {
                Ok(Message::Response(r)) if r.id == RequestId::from(1) => {
                    // An Ok response with a non-null result — `answer` has a type. (Exact rendering is
                    // covered by hover_at tests; here we only prove the request reached the handler and
                    // returned a result through the real serve loop.)
                    match r.response_kind {
                        lsp_server::ResponseKind::Ok { result } => assert!(
                            !result.is_null(),
                            "hover over a typed definition should return a result through serve"
                        ),
                        lsp_server::ResponseKind::Err { error } => {
                            panic!("hover request errored: {error:?}")
                        }
                    }
                    got_hover = true;
                    break;
                }
                // The didOpen diagnostics push + any other notification — ignore.
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(
            got_hover,
            "the hover request was dispatched and answered by serve"
        );
        // Now shut the server down: `shutdown` request (server replies via handle_shutdown) then `exit`.
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(2),
                Shutdown::METHOD.to_string(),
                serde_json::Value::Null,
            )))
            .unwrap();
        client
            .sender
            .send(Message::Notification(Notification::new(
                "exit".to_string(),
                serde_json::Value::Null,
            )))
            .unwrap();
        // `serve` must RETURN (Ok) once shutdown+exit arrive — join proves the loop terminated, not hung.
        let served = handle.join().expect("the serve thread did not panic");
        assert!(served.is_ok(), "serve returned an error: {served:?}");
    }

    #[test]
    fn serve_answers_an_unknown_request_with_method_not_found() {
        // The `_ =>` arm of `handle_request`: an unrecognized method must come back as a `MethodNotFound`
        // ERROR response (not a panic, not silence) so the client is not left waiting on its request id.
        // Exercised end-to-end over the real `serve` loop, the one path the happy-path serve test skips.
        let (server_conn, client) = Connection::memory();
        let handle = std::thread::spawn(move || {
            let mut server = Server::new(server_conn);
            server.serve()
        });
        // A method the server does not implement. No didOpen needed — dispatch rejects before any doc work.
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(1),
                "textDocument/theOneWeDoNotSupport".to_string(),
                serde_json::Value::Null,
            )))
            .unwrap();
        let mut got_err = false;
        loop {
            match client.receiver.recv() {
                Ok(Message::Response(r)) if r.id == RequestId::from(1) => {
                    match r.response_kind {
                        lsp_server::ResponseKind::Err { error } => {
                            assert_eq!(
                                error.code,
                                lsp_server::ErrorCode::MethodNotFound as i32,
                                "an unknown method must be MethodNotFound, got {error:?}"
                            );
                        }
                        lsp_server::ResponseKind::Ok { result } => {
                            panic!("an unknown method should error, not succeed: {result:?}")
                        }
                    }
                    got_err = true;
                    break;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        assert!(
            got_err,
            "the unknown request was answered with an error through serve"
        );
        // Clean shutdown so the serve thread returns (proves the loop survives an error response).
        client
            .sender
            .send(Message::Request(Request::new(
                RequestId::from(2),
                Shutdown::METHOD.to_string(),
                serde_json::Value::Null,
            )))
            .unwrap();
        client
            .sender
            .send(Message::Notification(Notification::new(
                "exit".to_string(),
                serde_json::Value::Null,
            )))
            .unwrap();
        let served = handle.join().expect("the serve thread did not panic");
        assert!(served.is_ok(), "serve returned an error: {served:?}");
    }

    #[test]
    fn read_handlers_return_none_on_a_document_that_is_not_open() {
        // Every read handler opens with `self.docs.get(uri)?` — a request over a URI the server has never
        // seen a `didOpen` for (a client racing a query ahead of open, or querying a closed buffer) must
        // return `None`/JSON-null, NEVER panic and never query stale/empty state. This is a DISTINCT branch
        // from "cursor lands on no node in an OPEN doc" (the `*_at` totality tests): here the document map
        // has no entry at all. Drives the handler methods directly on a fresh server with an empty doc map.
        let (server, _client) = memory_server();
        let uri = test_uri();
        let doc_pos = lsp_types::TextDocumentPositionParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            position: Position::new(0, 0),
        };
        // Position-based reads.
        assert!(
            server
                .hover(&HoverParams {
                    text_document_position_params: doc_pos.clone(),
                    work_done_progress_params: Default::default(),
                })
                .is_none(),
            "hover on an unopened document must be None"
        );
        assert!(
            server
                .goto_definition(&GotoDefinitionParams {
                    text_document_position_params: doc_pos.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                })
                .is_none(),
            "definition on an unopened document must be None"
        );
        assert!(
            server
                .references(&ReferenceParams {
                    text_document_position: doc_pos.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    context: lsp_types::ReferenceContext {
                        include_declaration: true,
                    },
                })
                .is_none(),
            "references on an unopened document must be None"
        );
        assert!(
            server
                .completion(&CompletionParams {
                    text_document_position: doc_pos.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    context: None,
                })
                .is_none(),
            "completion on an unopened document must be None"
        );
        // Whole-document reads.
        assert!(
            server
                .semantic_tokens(&SemanticTokensParams {
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                })
                .is_none(),
            "semantic tokens on an unopened document must be None"
        );
        assert!(
            server
                .document_symbol(&DocumentSymbolParams {
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                })
                .is_none(),
            "document symbols on an unopened document must be None"
        );
        assert!(
            server
                .code_lens(&CodeLensParams {
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                })
                .is_none(),
            "code lens on an unopened document must be None"
        );
        assert!(
            server
                .code_action(&CodeActionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                    range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                    context: Default::default(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                })
                .is_none(),
            "code action on an unopened document must be None"
        );
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
    fn definition_on_a_builtin_reference_is_none() {
        // The LSP go-to-def analogue of the cdz-def "no navigable definition (a literal/builtin) fails"
        // contrast (#1312): a reference to a BUILTIN / prelude operator has no USER definition in the
        // buffer to jump to, so go-to-definition declines (None) rather than landing somewhere wrong.
        // This is the deliberate contrast with hover-doc (which surfaces a type even for a builtin) — pin
        // it so a refactor can't make go-to-def resolve a builtin to a spurious location.
        let text = "def main = 1 + 2";
        // Cursor on the `+` operator (col 13 of `def main = 1 + 2`).
        assert!(
            definition_at(text, true, Position::new(0, 13), &test_uri()).is_none(),
            "go-to-definition on a builtin operator has no navigable user-def → None"
        );
    }

    #[test]
    fn type_definition_jumps_from_a_value_to_its_type_declaration() {
        // Go-to-TYPE-definition: from a value whose static type is the user-declared `Color`, jump to the
        // `type Color = …` declaration (line 0), NOT the value's own definition. `favorite` returns Color;
        // the cursor on its USE in `main` lands on the `Color` type decl.
        let text = "type Color = Red | Green | Blue\ndef favorite() -> Color = Green\ndef main() -> Color = favorite()";
        // Cursor on the `favorite` use in main (line 2, col 22).
        let loc = type_definition_at(text, true, Position::new(2, 22), &test_uri())
            .expect("a type definition");
        assert_eq!(
            loc.range.start.line, 0,
            "the type Color is declared on line 0, got {loc:?}"
        );
    }

    #[test]
    fn type_definition_declines_for_a_builtin_scalar_type() {
        // A value of a BUILTIN type (`Int64`) has no user `type` declaration to jump to — decline (None),
        // never a wrong landing. `answer : Int64` → go-to-type-definition on its use yields nothing.
        let text = "def answer = 42\ndef main() -> Int64 = answer";
        // Cursor on the `answer` use in main (line 1). "def main() -> Int64 = " is 22 chars → col 22.
        assert!(
            type_definition_at(text, true, Position::new(1, 22), &test_uri()).is_none(),
            "a builtin-typed value has no type declaration to jump to"
        );
    }

    #[test]
    fn type_definition_jumps_across_files_to_an_imported_type_decl() {
        // Cross-file go-to-type-definition: the entry imports type `Color` (and `favorite`) from a library
        // and holds a `Color`-typed value; go-to-type-definition lands on the `type Color` decl in the
        // LIBRARY file, not the entry. Exercises package_type_definition_at (linked TypeAt + per-file
        // Symbols scan → a Location in the declaring lib).
        let dir = std::env::temp_dir().join(format!("cdz-lsp-typedef-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("lib.sexp"),
            "(module lib (type Color (Red) (Green) (Blue)) (def (favorite) Green) (export Color favorite))",
        )
        .expect("write lib");
        let main_path = dir.join("main.sexp");
        let main_text =
            "(do (import \"lib\" (Color favorite)) (def (main) (favorite)) (export main))";
        std::fs::write(&main_path, main_text).expect("write main");
        let open = |p: &std::path::Path| std::fs::read_to_string(p).ok();
        // Cursor on the `favorite` use in main — its type is the imported `Color`.
        let byte = main_text.find("(favorite))").unwrap() + 1;
        let pos = crate::lsp::byte_to_position(main_text, byte);
        let loc =
            package_type_definition_at(&main_path.to_string_lossy(), &open, main_text, false, pos)
                .expect("a cross-file type definition");
        assert!(
            loc.uri.as_str().ends_with("lib.sexp"),
            "the type Color is declared in lib.sexp — jump should land there, got {:?}",
            loc.uri
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn type_definition_is_total_on_malformed_source() {
        // Total like the other node-id-keyed queries — an unparseable / out-of-range buffer yields None,
        // never a panic.
        assert!(
            type_definition_at("def (f x = (", true, Position::new(0, 5), &test_uri()).is_none()
        );
        assert!(type_definition_at("", true, Position::new(9, 9), &test_uri()).is_none());
    }

    #[test]
    fn call_hierarchy_prepare_returns_the_def_under_the_cursor() {
        // Preparing call hierarchy from a def's NAME yields an item for that def (a FUNCTION `helper`),
        // with its name as the selection range — the anchor the client asks incoming/outgoing calls for.
        let text = "def helper(x: Int64) -> Int64 = x + 1\ndef main() -> Int64 = helper(2)";
        // Cursor on the `helper` NAME in its declaration (line 0, col 4).
        let item = call_hierarchy_item_at(text, true, Position::new(0, 4), &test_uri())
            .expect("an item on the def name");
        assert_eq!(item.name, "helper");
        assert_eq!(item.kind, SymbolKind::FUNCTION);
        assert_eq!(
            item.selection_range.start.line, 0,
            "selection is the name on line 0"
        );
    }

    #[test]
    fn call_hierarchy_prepare_works_from_a_use_site() {
        // rust-analyzer prepares call hierarchy from a CALL SITE too, not just the declaration. A cursor on
        // the `helper` USE in main's body yields helper's item (its DECL range, on line 0), so the client
        // can ask "who else calls helper" starting from a call it's looking at.
        let text = "def helper(x: Int64) -> Int64 = x + 1\ndef main() -> Int64 = helper(2)";
        // Cursor on the `helper` USE in main (line 1). "def main() -> Int64 = " is 22 chars → col 22.
        let item = call_hierarchy_item_at(text, true, Position::new(1, 22), &test_uri())
            .expect("an item from the use site");
        assert_eq!(item.name, "helper");
        assert_eq!(
            item.selection_range.start.line, 0,
            "the item anchors on helper's DECLARATION (line 0), not the use: {item:?}"
        );
    }

    #[test]
    fn call_hierarchy_prepare_is_none_off_a_definition_name() {
        // A cursor NOT on a top-level def's name (here in the body) prepares nothing.
        let text = "def answer = 42";
        assert!(
            call_hierarchy_item_at(text, true, Position::new(0, 13), &test_uri()).is_none(),
            "cursor on the `42` body is not a def name"
        );
    }

    #[test]
    fn call_hierarchy_incoming_finds_the_callers_grouped_by_def() {
        // Incoming calls to `helper`: both `main` and `twice` call it, so incomingCalls returns two caller
        // items (`main`, `twice`), each with the range(s) of its call(s). `helper` itself is not a caller.
        let text = "def helper(x: Int64) -> Int64 = x + 1\ndef main() -> Int64 = helper(2)\ndef twice() -> Int64 = helper(helper(3))";
        let calls = incoming_calls_for(text, true, "helper", &test_uri());
        let callers: std::collections::HashSet<&str> =
            calls.iter().map(|c| c.from.name.as_str()).collect();
        assert!(
            callers.contains("main") && callers.contains("twice"),
            "helper's callers are main + twice, got {callers:?}"
        );
        assert!(
            !callers.contains("helper"),
            "helper's own declaration is not one of its callers: {callers:?}"
        );
        // `twice` calls helper TWICE (nested) → its entry carries 2 call ranges.
        let twice = calls.iter().find(|c| c.from.name == "twice").unwrap();
        assert_eq!(
            twice.from_ranges.len(),
            2,
            "twice calls helper twice (helper(helper(3))): {:?}",
            twice.from_ranges
        );
    }

    #[test]
    fn call_hierarchy_incoming_is_empty_for_an_uncalled_def() {
        // A def nobody calls has no incoming calls — empty (never a panic).
        let text = "def lonely() -> Int64 = 1\ndef main() -> Int64 = 2";
        assert!(
            incoming_calls_for(text, true, "lonely", &test_uri()).is_empty(),
            "an uncalled def has no incoming calls"
        );
    }

    #[test]
    fn call_hierarchy_works_on_an_sexpr_module_rooted_program() {
        // The s-expr `(module m …)` root is a DISTINCT path in top_level_defs_with_spans (it skips the
        // module NAME and walks the members), separate from the ML do-root/bare-def cases the other
        // call-hierarchy tests use. Pin both directions on a module-rooted program: incoming to `helper`
        // = [main]; outgoing from `main` = [helper].
        let text = "(module m (def (helper x) (+ x 1)) (def (main) (helper 2)) (export main))";
        let incoming = incoming_calls_for(text, false, "helper", &test_uri());
        let callers: Vec<&str> = incoming.iter().map(|c| c.from.name.as_str()).collect();
        assert_eq!(
            callers,
            vec!["main"],
            "helper's caller in the module is main: {callers:?}"
        );
        let outgoing = outgoing_calls_for(text, false, "main", &test_uri());
        let callees: Vec<&str> = outgoing.iter().map(|c| c.to.name.as_str()).collect();
        assert_eq!(
            callees,
            vec!["helper"],
            "main's callee in the module is helper: {callees:?}"
        );
    }

    #[test]
    fn call_hierarchy_outgoing_finds_the_callees_of_a_def() {
        // Outgoing calls FROM `main`: it calls `helper` and `other`, so outgoingCalls returns those two
        // callee items with their call-site ranges. `main`'s own signature is not a call.
        let text = "def helper(x: Int64) -> Int64 = x\ndef other() -> Int64 = 7\ndef main() -> Int64 = helper(other())";
        let calls = outgoing_calls_for(text, true, "main", &test_uri());
        let callees: std::collections::HashSet<&str> =
            calls.iter().map(|c| c.to.name.as_str()).collect();
        assert!(
            callees.contains("helper") && callees.contains("other"),
            "main calls helper + other, got {callees:?}"
        );
        assert!(
            !callees.contains("main"),
            "main's own signature is not an outgoing call: {callees:?}"
        );
    }

    #[test]
    fn call_hierarchy_outgoing_counts_a_self_recursive_call() {
        // A self-recursive def calls ITSELF — an outgoing call to itself (useful: shows the recursion).
        // `countdown` calls `countdown` once in its body.
        let text = "def countdown(n: Int64) -> Int64 = countdown(n)";
        let calls = outgoing_calls_for(text, true, "countdown", &test_uri());
        let self_call = calls.iter().find(|c| c.to.name == "countdown");
        assert!(
            self_call.is_some(),
            "a self-recursive call is an outgoing call to itself: {:?}",
            calls.iter().map(|c| &c.to.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn call_hierarchy_outgoing_is_empty_for_a_leaf_def() {
        // A def that calls no top-level def (only a builtin `+`) has no outgoing calls — empty.
        let text = "def leaf(x: Int64) -> Int64 = x + 1\ndef main() -> Int64 = leaf(2)";
        assert!(
            outgoing_calls_for(text, true, "leaf", &test_uri()).is_empty(),
            "leaf calls only the builtin `+`, no top-level callee"
        );
    }

    #[test]
    fn call_hierarchy_is_total_on_malformed_source() {
        // Total on incomplete source, like the other queries.
        let _ = call_hierarchy_item_at("def (f x = (", true, Position::new(0, 5), &test_uri());
        assert!(incoming_calls_for("(def (f x", false, "f", &test_uri()).is_empty());
        assert!(outgoing_calls_for("(def (f x", false, "f", &test_uri()).is_empty());
    }

    #[test]
    fn definition_and_references_are_total_on_malformed_mid_edit_source() {
        // "Queries over incomplete source are TOTAL" (tooling-and-lsp.md): the editor fires
        // definition/references CONSTANTLY as the user types, so both must return (never panic) on a
        // half-typed / unparseable buffer, on BOTH surfaces and at an out-of-range cursor. These are the
        // node-id-keyed queries most exposed to a mid-edit arena — the other analyses have such a test,
        // these two did not.
        let cases: &[(&str, bool)] = &[
            ("def (f x = (", true), // ML: unbalanced, recovered arena
            ("def f(x:", true),     // ML: truncated signature mid-type
            ("(def (f x", false),   // s-expr: unclosed forms
            ("(do (import", false), // s-expr: truncated import
            ("", true),             // empty
            ("   ", false),         // whitespace-only
        ];
        for &(text, is_ml) in cases {
            // In-range AND out-of-range cursors; include_declaration both ways. Just must not panic.
            for pos in [Position::new(0, 3), Position::new(9, 9)] {
                let _ = definition_at(text, is_ml, pos, &test_uri());
                let _ = references_at(text, is_ml, pos, &test_uri(), true);
                let _ = references_at(text, is_ml, pos, &test_uri(), false);
            }
        }
    }

    // ── s-expr surface (is_ml = false) — the OTHER reader/canonicalization path ──────────────────────
    // Almost every single-buffer unit test drives the ML surface; `parse_surface`'s s-expr branch is a
    // DISTINCT reader (`sexpr::read_spanned` with a `read_all_spanned` multi-form fallback, no `canon`
    // remap) whose node ids must ALSO line up with what the queries answer. These pin the node-id-keyed
    // analyses (definition/hover/completion) on `.sexp` so a regression in that branch can't slip past.

    #[test]
    fn sexpr_definition_jumps_from_a_use_to_the_defining_name() {
        // Multi-form s-expr program (exercises the `read_all_spanned` fallback): `double` is defined on
        // line 0 and used on line 1; go-to-definition from the use lands on the definition's name.
        let text = "(def (double x) (+ x x))\n(def use (double 21))";
        // The `double` USE in `use`'s body: line 1, col 10 ("(def use (" = 10 chars before `double`).
        let loc =
            definition_at(text, false, Position::new(1, 10), &test_uri()).expect("a definition");
        assert_eq!(
            loc.range.start.line, 0,
            "definition is on line 0, got {loc:?}"
        );
    }

    #[test]
    fn sexpr_hover_reports_the_type() {
        // Hover on the s-expr surface reads TypeAt over the s-expr arena — the node ids must match.
        let text = "(def (double x) (+ x x))\n(def use (double 21))";
        let h = hover_at(text, false, Position::new(1, 10)).expect("a hover");
        let rendered = match &h.contents {
            HoverContents::Scalar(MarkedString::String(s)) => s.clone(),
            HoverContents::Markup(m) => m.value.clone(),
            other => panic!("unexpected hover contents: {other:?}"),
        };
        assert!(
            rendered.contains("->") || rendered.contains("Int"),
            "hover should report the function type, got: {rendered}"
        );
    }

    #[test]
    fn sexpr_completion_offers_top_level_symbols() {
        // Completion on the s-expr surface offers the module's top-level declarations (`Symbols` read).
        let text = "(def (double x) (+ x x))\n(def use (double 21))";
        let items = completions_at(text, false, Position::new(1, 10));
        assert!(
            items.iter().any(|i| i.label == "double"),
            "completion should offer the top-level `double`: {items:?}"
        );
    }

    #[test]
    fn sexpr_references_finds_every_use_across_multiple_forms() {
        // References on the s-expr surface (multi-form → the `read_all_spanned` fallback + canon remap):
        // `helper` is defined on line 0 and used on lines 1 and 2. References from a use finds BOTH uses.
        // This shares the node-id/canonicalization concern the s-expr canon fix addressed — pin it so a
        // regression that mis-anchors the `UsesOf` answers to neighbour nodes is caught.
        let text = "(def (helper x) (+ x 1))\n(def a (helper 1))\n(def b (helper 2))";
        // The `helper` use in `a` (line 1, col 8: "(def a (" = 8 chars before `helper`).
        let refs = references_at(text, false, Position::new(1, 8), &test_uri(), false);
        assert_eq!(refs.len(), 2, "two uses expected, got {refs:?}");
        let lines: Vec<u32> = refs.iter().map(|l| l.range.start.line).collect();
        assert!(
            lines.contains(&1) && lines.contains(&2),
            "both use sites (lines 1 and 2) expected, got {lines:?}"
        );
    }

    #[test]
    fn sexpr_semantic_tokens_are_valid_and_non_overlapping() {
        // Semantic tokens on the s-expr surface: a whole-document leaf walk over the canonicalized arena.
        // Every token references a legend index, has positive length, and the delta-encoded stream is
        // ascending + non-overlapping (the LSP wire invariant). Pins the s-expr `Highlight` path (the
        // single-buffer token tests were ML-only, like the rest of the surface before the canon fix).
        let toks = semantic_tokens_for("(def (double x) (+ x x))\n(def use (double 21))", false);
        assert!(
            !toks.is_empty(),
            "expected some semantic tokens on the s-expr surface"
        );
        for t in &toks {
            assert!(t.length > 0, "a token must have positive length");
            assert!(
                (t.token_type as usize) < SEMANTIC_TOKEN_TYPES.len(),
                "token_type {} out of legend range",
                t.token_type
            );
        }
        assert_no_overlap(&toks);
    }

    #[test]
    fn sexpr_document_symbols_outline_multiple_forms() {
        // The outline on the s-expr surface (multi-form): each top-level `def` becomes a symbol with the
        // right kind + a range on its own line. Pins the last node-id-keyed analysis on the s-expr path.
        let syms = document_symbols_for("(def (double x) (+ x x))\n(def use (double 21))", false);
        let by_name: std::collections::HashMap<_, _> =
            syms.iter().map(|s| (s.name.as_str(), s)).collect();
        assert!(by_name.contains_key("double"), "double missing: {syms:?}");
        assert!(by_name.contains_key("use"), "use missing: {syms:?}");
        assert_eq!(
            by_name["double"].kind,
            SymbolKind::FUNCTION,
            "double is a function"
        );
        // Each symbol's range lands on its own declaration line (0 for double, 1 for use) — proving the
        // remapped span table anchors the outline to the right form, not a neighbour.
        assert_eq!(by_name["double"].range.start.line, 0);
        assert_eq!(by_name["use"].range.start.line, 1);
    }

    #[test]
    fn sexpr_code_action_anchors_the_quickfix_to_the_right_node_in_a_multi_form_program() {
        // A MULTI-form s-expr program where the 2nd def has an unused param — the exact case that
        // mis-anchored in the CLI `cdz fix` path (its `reparse_spans` s-expr arm skipped canonicalization,
        // so the canonical fix-node indexed a NEIGHBOUR span → the fix rewrote the TYPE, not the param;
        // filed to v-cdz-tooling). The LSP code_action goes through `parse_surface`, which DOES canonicalize
        // the s-expr arena (fix 281291b36), so its quick-fix must anchor to the `y` PARAM, not its type.
        // Pins that the LSP path is (and stays) correct on the case that broke the sibling.
        let text = "(def (a (: x Int64)) x)\n(def (b (: y Int64)) 5)\n(export a b)";
        let whole = Range::new(Position::new(0, 0), Position::new(2, 20));
        let (title, edits) = only_action(&code_actions_at(text, false, &test_uri(), whole));
        assert!(
            title.contains("_y"),
            "title should name the `_y` replacement, got {title:?}"
        );
        assert_eq!(
            edits.len(),
            1,
            "the unused-param fix is a single replace: {edits:?}"
        );
        let e = &edits[0];
        assert_eq!(e.new_text, "_y", "replaces the param name with `_y`");
        // The edit must cover the PARAM `y` on line 1 (col 11..12), NOT its `Int64` type — the
        // neighbour-mis-anchor bug would have landed the edit on the type instead.
        assert_eq!(
            (
                e.range.start.line,
                e.range.start.character,
                e.range.end.character
            ),
            (1, 11, 12),
            "the edit must cover the `y` param, not its type: {e:?}"
        );
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
    fn document_highlight_marks_every_occurrence_including_the_declaration() {
        // documentHighlight is the single-document sibling of references, but ALWAYS includes the
        // declaration (the editor highlights the def site too, unlike the default find-references). On a
        // `helper` used twice + declared once, a highlight from any occurrence marks all three lines.
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        let text = "def helper(x: Int64) -> Int64 = x\n\
                    def a = helper(1)\n\
                    def b = helper(2)";
        server
            .handle_notification(did_open_note(&uri, text))
            .expect("didOpen dispatches");
        // Cursor on the `helper` use in `a` (line 1, col 8).
        let params = DocumentHighlightParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(1, 8),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let hls = server
            .document_highlight(&params)
            .expect("a highlight list");
        let lines: Vec<u32> = hls.iter().map(|h| h.range.start.line).collect();
        assert!(
            lines.contains(&0) && lines.contains(&1) && lines.contains(&2),
            "declaration (0) + both uses (1,2) highlighted: {lines:?}"
        );
        // Every hit is a plain TEXT highlight (Cadenza bindings are immutable — no read/write split).
        assert!(
            hls.iter()
                .all(|h| h.kind == Some(DocumentHighlightKind::TEXT)),
            "all highlights are TEXT kind: {hls:?}"
        );
    }

    #[test]
    fn document_highlight_off_a_name_is_empty_not_none() {
        // A cursor on a literal (not a resolvable name) yields an EMPTY list, not None and not a panic —
        // total, mirroring references_off_a_name_is_empty. (None is reserved for an unopened document.)
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        server
            .handle_notification(did_open_note(&uri, "def answer = 42"))
            .expect("didOpen dispatches");
        // Cursor on the `42` literal (col 13).
        let params = DocumentHighlightParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: Position::new(0, 13),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let hls = server
            .document_highlight(&params)
            .expect("Some(empty), not None");
        assert!(
            hls.is_empty(),
            "a literal has no symbol to highlight: {hls:?}"
        );
    }

    #[test]
    fn document_highlight_handler_returns_none_on_an_unopened_document() {
        // The unopened-document branch (`self.docs.get(uri)?`): a highlight request over a URI the server
        // never saw a `didOpen` for must be None — NOT the empty-list that a resolved-but-unreferenced
        // cursor yields, and never a panic. This is the None/empty distinction the sibling test documents;
        // pins it directly (document_highlight was the one read handler missing an explicit unopened test,
        // unlike the 8 in read_handlers_return_none_on_a_document_that_is_not_open and the folding/selection/
        // signatureHelp handlers, which each have one).
        let (server, _client) = memory_server();
        let params = DocumentHighlightParams {
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: test_uri() },
                position: Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        assert!(
            server.document_highlight(&params).is_none(),
            "documentHighlight on an unopened document must be None"
        );
    }

    #[test]
    fn document_highlight_capability_is_advertised() {
        // The server must advertise documentHighlight so a client requests it (the caret same-symbol
        // highlight is off by default unless the server offers the provider).
        let value = serde_json::to_value(capabilities()).expect("serializes");
        assert_eq!(
            value
                .get("documentHighlightProvider")
                .and_then(|v| v.as_bool()),
            Some(true),
            "documentHighlightProvider must be advertised: {value}"
        );
    }

    /// Build a `RenameParams` for the cursor at `pos` renaming to `new_name`.
    fn rename_params(uri: &Uri, pos: Position, new_name: &str) -> RenameParams {
        RenameParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
                position: pos,
            },
            new_name: new_name.to_string(),
            work_done_progress_params: Default::default(),
        }
    }

    #[test]
    #[allow(clippy::mutable_key_type)] // `WorkspaceEdit.changes` is the LSP-mandated `HashMap<Uri, _>`.
    fn rename_rewrites_every_occurrence_including_the_declaration() {
        // Rename is the WRITE counterpart of references: every use PLUS the declaration is replaced. On
        // `helper` (declared line 0, used lines 1 & 2), a rename from any occurrence edits all three.
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        let text = "def helper(x: Int64) -> Int64 = x\n\
                    def a = helper(1)\n\
                    def b = helper(2)";
        server
            .handle_notification(did_open_note(&uri, text))
            .expect("didOpen dispatches");
        // Cursor on the `helper` use in `a` (line 1, col 8).
        let edit = server
            .rename(&rename_params(&uri, Position::new(1, 8), "worker"))
            .expect("a workspace edit");
        let changes = edit.changes.expect("changes present");
        let edits = changes.get(&uri).expect("edits for this file");
        // Declaration (line 0) + both uses (lines 1, 2) = three edits, each inserting the new name.
        let lines: Vec<u32> = edits.iter().map(|e| e.range.start.line).collect();
        assert!(
            lines.contains(&0) && lines.contains(&1) && lines.contains(&2),
            "declaration + both uses rewritten: {lines:?}"
        );
        assert!(
            edits.iter().all(|e| e.new_text == "worker"),
            "every edit inserts the new name: {edits:?}"
        );
    }

    #[test]
    fn rename_off_a_name_is_none_so_the_editor_declines() {
        // A cursor on a literal (not a renamable name) yields None — the editor declines the rename
        // rather than applying an empty WorkspaceEdit.
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        server
            .handle_notification(did_open_note(&uri, "def answer = 42"))
            .expect("didOpen dispatches");
        // Cursor on the `42` literal (col 13).
        assert!(
            server
                .rename(&rename_params(&uri, Position::new(0, 13), "x"))
                .is_none(),
            "rename off a name must be None (editor declines)"
        );
    }

    #[test]
    #[allow(clippy::mutable_key_type)] // `WorkspaceEdit.changes` is the LSP-mandated `HashMap<Uri, _>`.
    fn rename_on_a_local_shadowing_a_top_level_does_not_touch_the_top_level() {
        // The same shadowing guard references uses must apply to rename: renaming a LOCAL binder that
        // shadows a top-level of the same spelling must NOT rewrite the unrelated top-level's occurrences.
        // `helper` is a top-level def AND a parameter of `g` that shadows it.
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        let text = "def helper(x: Int64) -> Int64 = x\n\
                    def g(helper: Int64) -> Int64 = helper\n\
                    def m = helper(1)";
        server
            .handle_notification(did_open_note(&uri, text))
            .expect("didOpen dispatches");
        // Cursor on the LOCAL `helper` use in g's body (line 1). Renaming it must not edit the top-level
        // def (line 0) or its use in `m` (line 2) — the guard returns the local scope only (empty here,
        // since a node-keyed local-uses query is a later increment), so the rename declines rather than
        // wrongly rewriting the top-level.
        let text_line1 = "def g(helper: Int64) -> Int64 = ";
        let edit = server.rename(&rename_params(
            &uri,
            Position::new(1, text_line1.len() as u32),
            "z",
        ));
        if let Some(edit) = edit {
            let changes = edit.changes.unwrap_or_default();
            let all_lines: Vec<u32> = changes
                .values()
                .flat_map(|es| es.iter().map(|e| e.range.start.line))
                .collect();
            assert!(
                !all_lines.contains(&2),
                "the top-level use on line 2 must NOT be renamed by a local rename: {all_lines:?}"
            );
        }
        // (None is also acceptable — declining a local rename is correct when the local-uses query is not
        // yet node-keyed; the invariant is simply that the top-level is never wrongly rewritten.)
    }

    #[test]
    fn rename_capability_is_advertised() {
        let value = serde_json::to_value(capabilities()).expect("serializes");
        assert_eq!(
            value.get("renameProvider").and_then(|v| v.as_bool()),
            Some(true),
            "renameProvider must be advertised: {value}"
        );
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
    fn completion_offers_a_let_bound_local_with_its_inferred_type() {
        // A `let`-bound local (not a parameter) in scope at the cursor is a completion candidate, shown as
        // a VARIABLE with its INFERRED type as the detail — the `ScopeAt` query walks `let` binders, not
        // just params. Every prior local-completion test uses a function param; this pins the distinct
        // let-binder scope path (and that the detail carries the inferred type, since a `let` has no
        // written annotation). Complements the hover let-local coverage on the completion surface.
        let text = "def main() =\n  let total = 5\n  total";
        // Cursor INSIDE the `total` use in the trailing expression, where it is in scope (line 2, col 5).
        let items = completions_at(text, true, Position::new(2, 5));
        let total = items
            .iter()
            .find(|i| i.label == "total")
            .unwrap_or_else(|| panic!("let-local `total` should be a candidate: {items:?}"));
        assert_eq!(
            total.kind,
            Some(CompletionItemKind::VARIABLE),
            "a let-local is a Variable candidate: {total:?}"
        );
        assert!(
            total.detail.as_deref().is_some_and(|d| d.contains("Int")),
            "the let-local's detail should show its inferred Int type, got {:?}",
            total.detail
        );
    }

    #[test]
    fn completion_local_shadows_a_top_level_of_the_same_name() {
        // A local binding SHADOWS a top-level of the same spelling: `x` is BOTH a top-level `def` (a
        // value/Constant) AND a parameter of `g` (a Variable). Inside g's body, completion must offer `x`
        // exactly ONCE, as the LOCAL (Variable, with the param's type) — the local wins, matching how
        // resolution would bind the name. `completions_at` inserts top-level first then lets locals
        // OVERWRITE by name; this pins that dedup + precedence (the doc-comment's claim, previously
        // untested).
        let text = "def x = 42\ndef g(x: Int64) -> Int64 = x";
        // The `x` use in g's body — line 1, col 27 (the last char of the 28-char line).
        let items = completions_at(text, true, Position::new(1, 27));
        let xs: Vec<&CompletionItem> = items.iter().filter(|i| i.label == "x").collect();
        assert_eq!(
            xs.len(),
            1,
            "`x` must appear exactly once (deduped), got {xs:?}"
        );
        assert_eq!(
            xs[0].kind,
            Some(CompletionItemKind::VARIABLE),
            "the LOCAL param must win over the top-level value (Variable, not Constant): {:?}",
            xs[0]
        );
        assert!(
            xs[0].detail.as_deref().is_some_and(|d| d.contains("Int")),
            "the winning candidate is the local, shown with its type: {:?}",
            xs[0].detail
        );
    }

    #[test]
    fn completion_is_total_on_malformed_source() {
        // A buffer that does not parse yields a defined (possibly empty) candidate set, never a panic.
        let _ = completions_at("def (f x = (", true, Position::new(0, 5));
        let _ = completions_at("", true, Position::new(0, 0));
    }

    #[test]
    fn completion_on_a_mid_edit_ml_buffer_recovers_a_partial_set() {
        // The `completions_at` doc distinguishes the two totality branches: on the ML surface the reader
        // RECOVERS, so a mid-edit buffer (a complete def followed by a half-typed reference) still yields a
        // PARTIAL candidate set from the recovered tree — the earlier `helper` def is offered even though
        // the last line `def main() = hel` is unfinished. Pin the non-empty recovery so the "partial set"
        // claim can't silently degrade to the empty s-expr-hard-fail branch (the pr397 doc/code contract).
        let text = "def helper(n: Int64) -> Int64 = n + 1\ndef main() = hel";
        // Cursor at the end of the half-typed `hel` on line 1 (col 16).
        let items = completions_at(text, true, Position::new(1, 16));
        assert!(
            items.iter().any(|i| i.label == "helper"),
            "ML recovery should still offer the earlier `helper` def as a candidate, got: {:?}",
            items.iter().map(|i| &i.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn completion_on_a_hard_failing_sexpr_buffer_is_empty_not_a_panic() {
        // The other totality branch: an s-expr buffer that HARD-fails to parse has no recovered tree to
        // read `Symbols`/`ScopeAt` from, so completions are EMPTY (never a panic). This is the exact
        // behavior the `completions_at` doc promises for the s-expr surface — pinned so a future reader
        // change can't turn an empty answer into a crash or a stale-tree candidate set.
        let items = completions_at("(def (f x", false, Position::new(0, 6));
        assert!(
            items.is_empty(),
            "a hard-failing s-expr buffer yields no candidates, got: {:?}",
            items.iter().map(|i| &i.label).collect::<Vec<_>>()
        );
    }

    // A range covering the whole buffer — inlay hints for the entire document.
    fn whole_range() -> Range {
        Range::new(Position::new(0, 0), Position::new(u32::MAX, 0))
    }

    /// The PARAMETER-kind (call-site name) hint labels over the whole buffer — filters OUT the TYPE-kind
    /// hints (the def-param `: <inferred>` mode), so a param-name-hint test asserts only its own subject.
    fn param_name_hint_labels(text: &str) -> Vec<String> {
        inlay_hints_at(text, false, whole_range())
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::PARAMETER))
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn inlay_hints_do_not_leak_onto_a_defs_own_signature() {
        // A def's SIGNATURE list `(scale (: factor Int64))` is itself a name-headed list whose head is a
        // known callee — the hint walk must NOT treat it as a CALL and emit a spurious `factor:` hint on
        // the parameter declaration. With an ANNOTATED param the sig "arg" is a `(: factor Int64)` list
        // (not a bare name), so I3's name-match suppression does NOT mask it — only skipping def sigs does.
        // Expect exactly ONE hint: `factor:` on the `(scale 3)` CALL, nothing on the declaration.
        let text = "(module m (def (scale (: factor Int64)) (* factor 2)) (def (main) (scale 3)) (export main))";
        let hints = inlay_hints_at(text, false, whole_range());
        let labels: Vec<String> = hints
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert_eq!(
            labels,
            vec!["factor:".to_string()],
            "exactly one hint (on the `scale 3` call) — no hint leaked onto the def's signature; got {labels:?}"
        );
    }

    #[test]
    fn inlay_hints_label_positional_args_with_the_local_callees_param_names() {
        // At a call to a LOCALLY-defined function, each positional argument gets an inline `name:` hint
        // read from the callee's `(def (add a b) …)` signature — the rust-analyzer parameter-name hint.
        // This uses the callee's declared param NAMES (in the parse tree), NOT the blocked inferred-binder
        // type query. `(add 1 2)` → hints `a:` before `1` and `b:` before `2`.
        let text = "(module m (def (add a b) (+ a b)) (def (main) (add 1 2)) (export main))";
        // Scope to the PARAMETER-name hints — the def's own un-annotated params ALSO get TYPE hints now
        // (`a`/`b` → `: Int64`), which is a separate mode asserted elsewhere; here we pin the call-site
        // name hints.
        let labels = param_name_hint_labels(text);
        assert!(
            labels.contains(&"a:".to_string()) && labels.contains(&"b:".to_string()),
            "the two positional args should be hinted with the callee's param names `a:`/`b:`, got {labels:?}"
        );
    }

    #[test]
    fn inlay_hints_type_an_unannotated_inferable_param() {
        // TYPE-hint mode (unblocked by v-inference's inferred-param TypeAt): an un-annotated but inferable
        // param binder gets an inline `: <type>` hint. `def f(x) = x + 1` → `x` is solved to Int64 by the
        // body, so a TYPE hint `: Int64` sits right after the binder.
        let text = "def f(x) = x + 1";
        let type_hints: Vec<(String, u32)> = inlay_hints_at(text, true, whole_range())
            .iter()
            .filter(|h| h.kind == Some(InlayHintKind::TYPE))
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => (s.clone(), h.position.character),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert_eq!(
            type_hints,
            vec![(": Int64".to_string(), 7)],
            "the inferable param `x` gets a `: Int64` TYPE hint after the binder (col 7); got {type_hints:?}"
        );
    }

    #[test]
    fn inlay_hints_do_not_type_a_generic_or_annotated_param() {
        // No TYPE hint for a param that has no single inferred type: a fully-GENERIC param (`id`'s x → the
        // query answers `unknown`) OR an already-ANNOTATED param (`scale`'s factor — it shows its own
        // written type). Both cases: zero TYPE-kind hints.
        let generic = "def id(x) = x\ndef main() -> Int64 = id(5)";
        let annotated = "def scale(factor: Int64) -> Int64 = factor * 2";
        for text in [generic, annotated] {
            let type_hints = inlay_hints_at(text, true, whole_range())
                .into_iter()
                .filter(|h| h.kind == Some(InlayHintKind::TYPE))
                .count();
            assert_eq!(
                type_hints, 0,
                "no TYPE hint for a generic/annotated param, got {type_hints} in `{text}`"
            );
        }
    }

    #[test]
    fn inlay_hints_work_on_the_ml_surface() {
        // The ML surface (what editor users actually write) desugars `add(1, 2)` into the same
        // name-headed call list the s-expr surface produces, so the parameter-name hints apply there too —
        // the real-world path. `def add(a, b) = a + b` then `add(1, 2)` → `a:`/`b:` hints.
        let text = "def add(a, b) = a + b\ndef main() = add(1, 2)";
        let hints = inlay_hints_at(text, true, whole_range());
        let labels: Vec<String> = hints
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert!(
            labels.contains(&"a:".to_string()) && labels.contains(&"b:".to_string()),
            "ML `add(1, 2)` should get param-name hints `a:`/`b:`, got {labels:?}"
        );
    }

    #[test]
    fn inlay_hints_work_on_an_ml_annotated_param_def() {
        // The real-world ML path: an ANNOTATED-param def `def scale(factor: Int64) -> Int64 = …`. The ML
        // parser desugars the typed param into the same `(: factor Int64)` binder the s-expr surface uses,
        // so `param_binder_name` recovers `factor` and a call `scale(3)` is hinted `factor:`. Pins that the
        // annotated-param fix reaches the ML surface (the un-annotated ML case is covered above; this is
        // the typed form editor users most often write).
        let text = "def scale(factor: Int64) -> Int64 = factor * 2\ndef main() -> Int64 = scale(3)";
        let labels: Vec<String> = inlay_hints_at(text, true, whole_range())
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert_eq!(
            labels,
            vec!["factor:".to_string()],
            "ML annotated-param def `scale(factor: Int64)` → call `scale(3)` hinted `factor:` (no leak onto the def); got {labels:?}"
        );
    }

    #[test]
    fn inlay_hints_label_an_imported_callee_with_its_library_param_names() {
        // Increment 2 (cross-file): a callee IMPORTED from a sibling library gets param-name hints read
        // from the LIBRARY's `(def (name param…) …)` signature. `main.sexp` imports `add` from `lib.sexp`
        // and calls `(add 1 2)` → hints `a:`/`b:` come from lib's `(def (add a b) …)`, not the entry.
        let dir = std::env::temp_dir().join(format!("cdz-lsp-inlay-pkg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("lib.sexp"),
            "(module lib (def (add a b) (+ a b)) (export add))",
        )
        .expect("write lib");
        let main_path = dir.join("main.sexp");
        let main_text = "(do (import \"lib\" (add)) (def (main) (add 1 2)) (export main))";
        std::fs::write(&main_path, main_text).expect("write main");
        // A filesystem-reading resolver (the sibling lib lives on disk next to the entry).
        let open = |p: &std::path::Path| std::fs::read_to_string(p).ok();
        let hints = package_inlay_hints_at(
            &main_path.to_string_lossy(),
            &open,
            main_text,
            false,
            whole_range(),
        )
        .expect("the closure loads");
        let labels: Vec<String> = hints
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert!(
            labels.contains(&"a:".to_string()) && labels.contains(&"b:".to_string()),
            "an imported callee's args should be hinted with the LIBRARY's param names `a:`/`b:`, got {labels:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inlay_hints_are_empty_for_a_call_to_an_unknown_or_builtin_callee() {
        // Increment 1 only hints LOCAL defs. A call whose head is not a locally-defined function — here the
        // built-in `+` (no `(def (+ …) …)` in the buffer) — contributes no hints. (Cross-file/imported
        // callees are a later increment; until then an unknown callee is silently un-hinted, never wrong.)
        let text = "(module m (def (main) (+ 1 2)) (export main))";
        let hints = inlay_hints_at(text, false, whole_range());
        assert!(
            hints.is_empty(),
            "a call to a non-local callee (`+`) should produce no hints, got {hints:?}"
        );
    }

    #[test]
    fn inlay_hints_are_total_on_malformed_source() {
        // A buffer that does not parse yields no hints, never a panic (queries over incomplete source are
        // TOTAL — the same contract the other read handlers hold).
        let _ = inlay_hints_at("(def (add a b", false, whole_range());
        let _ = inlay_hints_at("", true, whole_range());
    }

    #[test]
    fn inlay_hints_outside_the_requested_range_are_excluded() {
        // The request carries the visible viewport; only arguments whose START falls in the range are
        // hinted. A zero-width range at the very start of the buffer covers no argument, so no hints —
        // pins that the range filter actually gates (not every call in the file every request).
        let text = "(module m (def (add a b) (+ a b)) (def (main) (add 1 2)) (export main))";
        let empty_range = Range::new(Position::new(0, 0), Position::new(0, 0));
        let hints = inlay_hints_at(text, false, empty_range);
        assert!(
            hints.is_empty(),
            "an empty range at offset 0 covers no argument → no hints, got {hints:?}"
        );
    }

    #[test]
    fn inlay_hints_suppress_a_hint_when_the_arg_name_matches_the_param() {
        // Increment 3 (noise suppression, rust-analyzer's rule): when the argument is a bare name IDENTICAL
        // to the parameter, the `name:` hint is pure redundancy — skip it. Here `(add a b)` inside a
        // `passthrough` whose OWN params are `a`/`b`: the args are the names `a`/`b`, matching `add`'s
        // params, so NO hints. But a differently-named arg still gets its hint (see the mixed case below).
        let text = "(module m (def (add a b) (+ a b)) (def (passthrough a b) (add a b)) (export passthrough))";
        // Scope to PARAMETER-name hints (the def params also get TYPE hints, asserted elsewhere).
        let labels = param_name_hint_labels(text);
        assert!(
            labels.is_empty(),
            "args that are the same names as the params should be suppressed, got {labels:?}"
        );
    }

    #[test]
    fn inlay_hints_suppress_only_the_matching_arg_not_the_others() {
        // Suppression is PER-ARGUMENT: in `(add a 2)` the first arg is the name `a` (matches param `a` →
        // suppressed) but the second arg `2` is a literal (≠ param `b` → still hinted `b:`). Pins that the
        // rule hides only the redundant hint, not the whole call's hints.
        let text = "(module m (def (add a b) (+ a b)) (def (g a) (add a 2)) (export g))";
        // Scope to PARAMETER-name hints (def params also get TYPE hints, asserted elsewhere).
        let labels = param_name_hint_labels(text);
        assert_eq!(
            labels,
            vec!["b:".to_string()],
            "only the literal arg `2` should be hinted (`b:`); the name arg `a` matching param `a` is suppressed"
        );
    }

    #[test]
    fn inlay_hints_are_total_on_a_partial_unclosed_call() {
        // A mid-edit UNCLOSED call `(add 1 ` (the as-you-type state) must not panic and must not produce a
        // spurious hint from a broken tree — total, like the other read handlers on incomplete source. The
        // s-expr surface hard-fails to parse an unclosed form → no arenas → empty (never a panic).
        let _ = inlay_hints_at(
            "(module m (def (add a b) (+ a b)) (def (main) (add 1 ",
            false,
            whole_range(),
        );
        // ML surface recovers; either way the call is total (no panic, defined result).
        let _ = inlay_hints_at(
            "def add(a, b) = a + b\ndef main() = add(1, ",
            true,
            whole_range(),
        );
    }

    #[test]
    fn inlay_hints_cover_a_nested_call_argument() {
        // A call passed AS an argument gets hints at BOTH levels: the outer call `(add (id 5) 9)` hints
        // its two args (`a:` on `(id 5)`, `b:` on `9`), and the inner call `(id 5)` independently hints
        // its own arg (`n:` on `5`). Pins that the whole-tree walk finds calls at every depth, not just
        // top-level statements.
        let text = "(module m (def (id n) n) (def (add a b) (+ a b)) (def (main) (add (id 5) 9)) (export main))";
        let labels: std::collections::HashSet<String> = inlay_hints_at(text, false, whole_range())
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        // `a:` (outer arg is the nested call), `b:` (outer literal 9), `n:` (inner arg 5).
        for want in ["a:", "b:", "n:"] {
            assert!(
                labels.contains(want),
                "nested call should hint `{want}` at its level, got {labels:?}"
            );
        }
    }

    #[test]
    fn inlay_hints_over_application_hints_only_up_to_the_param_count() {
        // A call with MORE args than the callee has params (over-application) hints only the args that
        // line up with a parameter — the extra args are silently un-hinted (zip stops at the param count),
        // never a panic or an out-of-bounds param name. `(one 1 2 3)` where `one` has a single param `x`
        // → exactly one hint `x:` on `1`.
        let text = "(module m (def (one x) x) (def (main) (one 1 2 3)) (export main))";
        let labels: Vec<String> = inlay_hints_at(text, false, whole_range())
            .iter()
            .map(|h| match &h.label {
                InlayHintLabel::String(s) => s.clone(),
                other => panic!("expected a string label, got {other:?}"),
            })
            .collect();
        assert_eq!(
            labels,
            vec!["x:".to_string()],
            "over-application hints only the single param `x:`, extra args un-hinted; got {labels:?}"
        );
    }

    #[test]
    fn format_document_canonicalizes_an_unformatted_sexpr_buffer() {
        // A buffer with noisy whitespace reprints to its canonical s-expr form — the in-memory core of
        // `cdz fmt`. Pins that the formatter collapses the extra spaces to the same output `cdz fmt` gives.
        let messy = "(module m (def (add a b)   (+ a b)) (export add))";
        let formatted = format_document(messy, false).expect("a clean parse formats");
        assert_eq!(
            formatted, "(module m\n  (def (add a b) (+ a b))\n\n  (export add))\n",
            "the formatter should canonicalize whitespace + newline-terminate, got {formatted:?}"
        );
    }

    #[test]
    fn format_document_canonicalizes_an_unformatted_ml_buffer() {
        // The ML surface reprints canonically too (spacing around `,`/`=`/operators normalized).
        let messy = "def   add(a,b)=a+b";
        let formatted = format_document(messy, true).expect("a clean parse formats");
        assert_eq!(
            formatted, "def add(a, b) = a + b\n",
            "the ML formatter should normalize spacing + newline-terminate, got {formatted:?}"
        );
    }

    #[test]
    fn format_document_preserves_an_ml_doc_comment() {
        // A formatter that silently drops a `///` doc comment would be a disaster — the comment is part of
        // the source's meaning (it feeds hover/`DocAt`). Pin that formatting an ML buffer PRESERVES its
        // doc comment (the ML surface round-trips `///` through the arena; the shared `cdz fmt` guard also
        // refuses a reprint that drops a `//`/`///` marker). A `def` already canonical except for the doc.
        let text = "/// Adds one.\ndef inc(n: Int64) -> Int64 = n + 1";
        let formatted = format_document(text, true).expect("a clean parse formats");
        assert!(
            formatted.contains("/// Adds one."),
            "formatting must preserve the doc comment, got {formatted:?}"
        );
        assert!(
            formatted.contains("def inc(n: Int64) -> Int64 = n + 1"),
            "the def body must survive too, got {formatted:?}"
        );
    }

    #[test]
    fn format_document_now_formats_an_ml_buffer_with_a_trailing_comment_at_an_if_branch() {
        // Mirror pin for the `cdz fmt` comment-capture fix (syntax #2797): a same-line `//` AFTER the
        // then-branch of an `if` (`if a then 1 // note`) used to make `cdz fmt` REFUSE the whole file (the
        // reader did not attach that mid-expression comment, so the reprint-drops-a-comment guard fired and
        // `format_document` returned None → the LSP formatting handler yielded NO edit). The formatter now
        // captures it, so this buffer FORMATS and the comment SURVIVES. Because the LSP formatting provider
        // delegates to this exact `format_document`/`cdz fmt` path, that newly-unblocked behavior flows
        // through `textDocument/formatting` unchanged — pin it so a `cdz fmt` regression that re-refuses (or
        // drops the comment) is caught on the LSP surface too.
        let text = "def f(a: Bool) -> Int64 = if a then 1 // note\n  else 2\n";
        let formatted =
            format_document(text, true).expect("the trailing-comment if-branch buffer now formats");
        assert!(
            formatted.contains("// note"),
            "the trailing comment must survive the reprint, got {formatted:?}"
        );
        // Re-parse safety: `//` runs to end-of-line, so `else` must sit on its OWN line or it would be
        // swallowed by the comment (the exact bug #2797 fixed). Assert the reprint keeps them apart.
        assert!(
            formatted.contains("// note\n"),
            "a hardbreak must follow the comment so `else` is not swallowed, got {formatted:?}"
        );
    }

    #[test]
    fn format_document_is_none_on_a_buffer_that_does_not_parse_cleanly() {
        // A broken buffer is NOT rewritten to a patched-up shape (matching `cdz fmt`'s fail-safe) — the
        // formatter returns None, so the LSP handler yields no edit rather than corrupting the source.
        assert!(
            format_document("(module m (def (add a b", false).is_none(),
            "an unparseable s-expr buffer must not be reformatted"
        );
    }

    #[test]
    fn formatting_handler_yields_a_full_document_edit_then_none_when_canonical() {
        // The handler end-to-end: opening a MESSY buffer yields ONE full-document TextEdit whose new_text
        // is the canonical form and whose range starts at (0,0); a buffer that is ALREADY canonical yields
        // an EMPTY edit list (no-op). Drives the real `Server::formatting` via an open document.
        let (mut server, _client) = memory_server();
        let uri: Uri = "file:///fmt.sexp".parse().unwrap();
        let messy = "(module m (def (add a b)   (+ a b)) (export add))";
        server.docs.insert(
            uri.clone(),
            Document {
                text: messy.to_string(),
                is_ml: false,
            },
        );
        let params = DocumentFormattingParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            options: lsp_types::FormattingOptions::default(),
            work_done_progress_params: Default::default(),
        };
        let edits = server.formatting(&params).expect("an open doc formats");
        assert_eq!(edits.len(), 1, "one full-document edit: {edits:?}");
        assert_eq!(
            edits[0].range.start,
            Position::new(0, 0),
            "edit starts at doc start"
        );
        assert_eq!(
            edits[0].new_text, "(module m\n  (def (add a b) (+ a b))\n\n  (export add))\n",
            "the edit carries the canonical text"
        );
        // Now make the doc canonical and re-format → no edit.
        server.docs.insert(
            uri.clone(),
            Document {
                text: "(module m\n  (def (add a b) (+ a b))\n\n  (export add))\n".to_string(),
                is_ml: false,
            },
        );
        let edits = server.formatting(&params).expect("still an open doc");
        assert!(
            edits.is_empty(),
            "an already-canonical buffer yields no edit: {edits:?}"
        );
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

    /// The flat `SymbolInformation` list from a workspace-symbol response (the only variant we emit).
    fn flat_symbols(resp: Option<WorkspaceSymbolResponse>) -> Vec<SymbolInformation> {
        match resp.expect("a workspace-symbol response") {
            WorkspaceSymbolResponse::Flat(v) => v,
            WorkspaceSymbolResponse::Nested(_) => panic!("we only emit the Flat variant"),
        }
    }

    /// A second document URI, for the cross-document workspace-symbol tests.
    fn test_uri2() -> Uri {
        use std::str::FromStr;
        Uri::from_str("file:///u.cdz").unwrap()
    }

    #[test]
    fn workspace_symbol_finds_a_matching_symbol_across_open_documents() {
        // Two open docs; a query matches symbols in BOTH, each carrying a Location pointing at its own
        // file. Case-insensitive SUBSTRING match ("elp" hits "helper").
        let (mut server, _client) = memory_server();
        let (u1, u2) = (test_uri(), test_uri2());
        server
            .handle_notification(did_open_note(&u1, "def helper(x: Int64) -> Int64 = x"))
            .expect("didOpen 1");
        server
            .handle_notification(did_open_note(
                &u2,
                "def helper2(y: Int64) -> Int64 = y\ndef other = 1",
            ))
            .expect("didOpen 2");
        let params = WorkspaceSymbolParams {
            query: "elp".to_string(),
            partial_result_params: Default::default(),
            work_done_progress_params: Default::default(),
        };
        let syms = flat_symbols(server.workspace_symbol(&params));
        // Both `helper` (u1) and `helper2` (u2) contain "elp"; `other` does not.
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"helper") && names.contains(&"helper2"),
            "both helper symbols found across files: {names:?}"
        );
        assert!(!names.contains(&"other"), "non-match excluded: {names:?}");
        // Each carries the URI of the file it came from.
        let helper = syms.iter().find(|s| s.name == "helper").unwrap();
        let helper2 = syms.iter().find(|s| s.name == "helper2").unwrap();
        assert_eq!(helper.location.uri, u1, "helper is located in u1");
        assert_eq!(helper2.location.uri, u2, "helper2 is located in u2");
    }

    #[test]
    fn workspace_symbol_empty_query_returns_every_open_symbol() {
        // VS Code sends an empty query to preload — it must return the full set (not empty).
        let (mut server, _client) = memory_server();
        server
            .handle_notification(did_open_note(&test_uri(), "def a = 1\ndef b = 2"))
            .expect("didOpen");
        let params = WorkspaceSymbolParams {
            query: String::new(),
            partial_result_params: Default::default(),
            work_done_progress_params: Default::default(),
        };
        let syms = flat_symbols(server.workspace_symbol(&params));
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"a") && names.contains(&"b"),
            "empty query returns all symbols: {names:?}"
        );
    }

    #[test]
    fn workspace_symbol_with_no_open_documents_is_empty_not_none() {
        // No documents open → an empty list (Some), not None and not a panic — total.
        let (server, _client) = memory_server();
        let params = WorkspaceSymbolParams {
            query: "anything".to_string(),
            partial_result_params: Default::default(),
            work_done_progress_params: Default::default(),
        };
        let syms = flat_symbols(server.workspace_symbol(&params));
        assert!(syms.is_empty(), "no open docs → no symbols: {syms:?}");
    }

    #[test]
    fn workspace_symbol_capability_is_advertised() {
        let value = serde_json::to_value(capabilities()).expect("serializes");
        assert_eq!(
            value
                .get("workspaceSymbolProvider")
                .and_then(|v| v.as_bool()),
            Some(true),
            "workspaceSymbolProvider must be advertised: {value}"
        );
    }

    #[test]
    fn folding_ranges_fold_each_multi_line_top_level_form() {
        // Two top-level defs, each spanning multiple lines; a single-line def in between folds nothing.
        // `helper` spans lines 0-1, `single` is one line (2), `main` spans lines 3-4.
        let text = "def helper(x: Int64) -> Int64 =\n  x\n\
                    def single = 1\n\
                    def main =\n  helper(single)";
        let ranges = folding_ranges_for(text, true);
        // The two multi-line forms fold; the one-line `single` does not.
        let spans: Vec<(u32, u32)> = ranges.iter().map(|r| (r.start_line, r.end_line)).collect();
        assert!(
            spans.contains(&(0, 1)),
            "helper (lines 0-1) folds: {spans:?}"
        );
        assert!(spans.contains(&(3, 4)), "main (lines 3-4) folds: {spans:?}");
        assert!(
            !spans.iter().any(|(s, e)| s == e),
            "no single-line (start==end) range is emitted: {spans:?}"
        );
    }

    #[test]
    fn folding_ranges_are_empty_for_a_single_line_program_and_total_on_malformed() {
        // An all-one-line program has nothing to fold → empty, not None-at-this-layer (the handler wraps
        // Some). A malformed buffer yields a defined (possibly empty) list, never a panic.
        assert!(folding_ranges_for("def a = 1", true).is_empty());
        let _ = folding_ranges_for("def (f x = (", true);
        let _ = folding_ranges_for("", true);
    }

    #[test]
    fn folding_ranges_recurse_into_module_members() {
        // A module with a multi-line member: the module block folds AND its inner multi-line def folds too
        // (the module-member refinement — a def nested in a module is foldable on its own, not just the
        // whole module). Use the s-expr surface for an unambiguous `(module …)` shape.
        // Lines: 0 `(module m`, 1 `  (def (inner x)`, 2 `    x)`, 3 `  (def (other) 1))`.
        let text = "(module m\n  (def (inner x)\n    x)\n  (def (other) 1))";
        let ranges = folding_ranges_for(text, false);
        let spans: Vec<(u32, u32)> = ranges.iter().map(|r| (r.start_line, r.end_line)).collect();
        // The whole module (line 0 → 3) folds.
        assert!(
            spans.iter().any(|&(s, e)| s == 0 && e == 3),
            "the module block folds (0-3): {spans:?}"
        );
        // The multi-line member `inner` (lines 1-2) folds as its own sub-region — the refinement.
        assert!(
            spans.iter().any(|&(s, e)| s == 1 && e == 2),
            "the multi-line module member `inner` (1-2) folds: {spans:?}"
        );
    }

    #[test]
    fn folding_range_capability_is_advertised() {
        let value = serde_json::to_value(capabilities()).expect("serializes");
        // `foldingRangeProvider` serializes to `true` for the Simple(true) capability.
        assert_eq!(
            value.get("foldingRangeProvider").and_then(|v| v.as_bool()),
            Some(true),
            "foldingRangeProvider must be advertised: {value}"
        );
    }

    #[test]
    fn folding_range_handler_returns_none_on_an_unopened_document() {
        // The handler's docs.get guard: a foldingRange over an unopened URI is None (total, no panic).
        let (server, _client) = memory_server();
        let params = FoldingRangeParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: test_uri() },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        assert!(
            server.folding_range(&params).is_none(),
            "foldingRange on an unopened document must be None"
        );
    }

    /// The chain of ranges (innermost → outermost) a `SelectionRange` links via `parent`, as byte-agnostic
    /// (line, char)-tuple pairs, for terse assertions.
    fn selection_chain(sr: &SelectionRange) -> Vec<((u32, u32), (u32, u32))> {
        let mut out = Vec::new();
        let mut cur = Some(sr);
        while let Some(s) = cur {
            out.push((lc(s.range.start), lc(s.range.end)));
            cur = s.parent.as_deref();
        }
        out
    }

    #[test]
    fn selection_range_is_a_strictly_nested_chain_from_the_cursor_outward() {
        // Cursor on `x` inside `(if (= n 0) x …)`: the expand chain steps out through the enclosing forms,
        // each range strictly containing the previous (innermost first). We assert the chain is non-empty,
        // starts at the tightest node covering the cursor, and each parent strictly contains its child.
        let text = "(do (def (f (: n Int64)) (if (= n 0) x n)) (export f))";
        // Byte offset of the lone `x` (the then-branch). Find it.
        let cursor_byte = text.find(" x n)").unwrap() + 1;
        let pos = byte_to_position(text, cursor_byte);
        let sr = selection_range_at(text, false, pos);
        let chain = selection_chain(&sr);
        assert!(
            chain.len() >= 2,
            "expand-selection yields a nested chain (node + enclosers): {chain:?}"
        );
        // Innermost range covers the cursor position.
        let (inner_start, inner_end) = chain[0];
        assert!(
            inner_start <= lc(pos) && lc(pos) <= inner_end,
            "innermost range covers the cursor: {chain:?}"
        );
        // Strictly nested: each successive (parent) range contains the previous (child) and is not smaller.
        for w in chain.windows(2) {
            let (child, parent) = (w[0], w[1]);
            assert!(
                parent.0 <= child.0 && child.1 <= parent.1,
                "each parent range contains its child: parent={parent:?} child={child:?}"
            );
        }
    }

    #[test]
    fn selection_range_handler_returns_one_entry_per_position() {
        // The protocol requires one SelectionRange per requested position, in order — even for a position
        // over no node (which yields a degenerate empty range, never a missing entry).
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        server
            .handle_notification(did_open_note(&uri, "def answer = 42"))
            .expect("didOpen");
        let params = SelectionRangeParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            positions: vec![Position::new(0, 4), Position::new(9, 9)], // on `answer`, and past the end
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let ranges = server.selection_range(&params).expect("a response");
        assert_eq!(
            ranges.len(),
            2,
            "one SelectionRange per requested position, in order"
        );
        // The out-of-range position degenerates to an empty range AT that position (start == end).
        assert_eq!(
            ranges[1].range.start, ranges[1].range.end,
            "a position over no node yields a degenerate empty range, not a missing entry"
        );
    }

    #[test]
    fn selection_range_handler_parses_once_and_matches_per_position() {
        // PR #538: the multi-position handler now parses the document ONCE and answers every position
        // against the shared span table (was O(positions × parse)). Pin that this optimization is
        // behavior-PRESERVING: each entry the handler returns must equal what the single-position
        // parse-per-call `selection_range_at` gives for the same position (same chain, same ranges).
        let text = "def f(x) = if x then x else 0";
        let (mut server, _client) = memory_server();
        let uri = test_uri();
        server
            .handle_notification(did_open_note(&uri, text))
            .expect("didOpen");
        let positions = vec![
            Position::new(0, 4),
            Position::new(0, 14),
            Position::new(0, 25),
        ];
        let params = SelectionRangeParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            positions: positions.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let batched = server.selection_range(&params).expect("a response");
        assert_eq!(batched.len(), positions.len(), "one entry per position");
        for (i, &pos) in positions.iter().enumerate() {
            // The single-parse batched entry equals the independent per-position parse — proving the
            // parse-once refactor changed only the parse COUNT, not the answer.
            let independent = selection_range_at(text, true, pos);
            assert_eq!(
                selection_chain(&batched[i]),
                selection_chain(&independent),
                "batched entry {i} (pos {pos:?}) matches the per-position parse"
            );
        }
    }

    #[test]
    fn selection_range_handler_returns_none_on_an_unopened_document() {
        let (server, _client) = memory_server();
        let params = SelectionRangeParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: test_uri() },
            positions: vec![Position::new(0, 0)],
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        assert!(
            server.selection_range(&params).is_none(),
            "selectionRange on an unopened document must be None"
        );
    }

    #[test]
    fn selection_range_handler_is_total_on_a_malformed_buffer_one_self_range_per_position() {
        // The multi-cursor arity contract (one SelectionRange per requested position, in order) must hold
        // even when the document does not parse. An s-expr buffer HARD-fails to parse (unlike the ML
        // surface, which recovers), driving the handler's `Err(_)` arm — which must still return `Some`
        // with exactly one DEGENERATE (start == end, no parent) self-range per position, never `None` and
        // never a dropped/reordered entry. A regression that returned `None` (or the recovered-tree arity)
        // on a parse failure would break every client that batches cursors over a mid-edit s-expr buffer.
        use std::str::FromStr;
        let uri = Uri::from_str("file:///t.sexp").expect("a .sexp uri");
        let (mut server, _client) = memory_server();
        server
            // Unbalanced parens: `read_spanned` and the `read_all_spanned` fallback both fail → `Err(_)`.
            .handle_notification(did_open_note(&uri, "(def (f x"))
            .expect("didOpen");
        let positions = vec![
            Position::new(0, 1),
            Position::new(0, 6),
            Position::new(5, 0),
        ];
        let params = SelectionRangeParams {
            text_document: lsp_types::TextDocumentIdentifier { uri: uri.clone() },
            positions: positions.clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let ranges = server
            .selection_range(&params)
            .expect("a malformed buffer still gets a response, not None");
        assert_eq!(
            ranges.len(),
            positions.len(),
            "one SelectionRange per requested position, in order — even on a parse failure"
        );
        for (i, (sr, &pos)) in ranges.iter().zip(positions.iter()).enumerate() {
            assert_eq!(
                sr.range,
                Range::new(pos, pos),
                "entry {i} must be the degenerate self-range at its requested position"
            );
            assert!(
                sr.parent.is_none(),
                "entry {i} on a malformed buffer has no enclosing node, so no parent chain"
            );
        }
    }

    #[test]
    fn selection_range_capability_is_advertised() {
        let value = serde_json::to_value(capabilities()).expect("serializes");
        assert_eq!(
            value
                .get("selectionRangeProvider")
                .and_then(|v| v.as_bool()),
            Some(true),
            "selectionRangeProvider must be advertised: {value}"
        );
    }

    #[test]
    fn signature_help_shows_the_callee_type_and_tracks_the_active_arg() {
        // `add` is a 2-arg function; inside a call `(add 1 2)` the signature popup shows add's arrow type,
        // and the active parameter advances as the cursor moves past each typed argument.
        let text = "(do (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) (add 1 2)))";
        // Cursor right after `(add ` — on the first arg `1`. Find the `1` in `(add 1 2)`.
        let first_arg = text.find("add 1 2").unwrap() + 4; // index of `1`
        let sh = signature_help_at(text, false, byte_to_position(text, first_arg))
            .expect("a signature inside the call");
        assert_eq!(sh.signatures.len(), 1, "one signature for the callee");
        // The label is the callee + its type RENDERED from the structured `KIND_TYPE_INFO` payload — a
        // CLEAN arrow, not the undecoded binary-AST wire bytes. Pin the exact spelling: the sidecar wire is
        // binary AST now, and reading it as raw text yielded a `add : cdzast\0…` label that still contained
        // `->` bytes (so a weaker `contains("->")` check passed while the user saw garbage). This exact
        // match is what catches a future rewire that skips `decode_type_info` + `render_ty_scheme`.
        assert_eq!(
            sh.signatures[0].label, "add : (-> Int64 (-> Int64 Int64))",
            "label is the callee + its cleanly-rendered curried arrow type"
        );
        // On the first arg (none fully typed-and-past yet), active parameter is 0.
        assert_eq!(sh.active_parameter, Some(0), "first arg active");
        // Cursor after the second arg `2` (on the closing paren) — both args typed → active 2 (clamped by
        // the client to the last slot). At minimum it must have advanced past 0.
        let after_second = text.find("1 2)").unwrap() + 3; // the `)` after `2`
        let sh2 = signature_help_at(text, false, byte_to_position(text, after_second))
            .expect("a signature still inside the call");
        assert!(
            sh2.active_parameter.unwrap() >= 1,
            "active parameter advanced past the first arg: {:?}",
            sh2.active_parameter
        );
    }

    #[test]
    fn arrow_parameter_components_unfold_the_curry() {
        // A 2-arg function renders CURRIED — unfold to `[param, param, return]`.
        assert_eq!(
            arrow_parameter_components("(-> Int64 (-> Int64 Int64))"),
            vec!["Int64", "Int64", "Int64"]
        );
        // A 3-arg curried arrow.
        assert_eq!(
            arrow_parameter_components("(-> Int64 (-> Int64 (-> Int64 Int64)))"),
            vec!["Int64", "Int64", "Int64", "Int64"]
        );
        // Currying is ambiguous: `(-> (List a) (-> A B))` is equally "1-arg returning `(-> A B)`" and
        // "2-arg `(List a)`,`A` returning `B`". Since the s-expr call `(f xs a)` applies BOTH levels, the
        // honest reading is a full unfold — every curried level is a parameter slot the caller can fill.
        assert_eq!(
            arrow_parameter_components("(-> (List a) (-> A B))"),
            vec!["(List a)", "A", "B"]
        );
        // A nullary arrow `(-> Ret)` has no parameters.
        assert!(arrow_parameter_components("(-> Int64)").is_empty());
        // A non-arrow (a nullary value's type) yields no components.
        assert!(arrow_parameter_components("Int64").is_empty());
    }

    #[test]
    fn signature_help_gives_per_parameter_label_offsets_and_bolds_the_active_one() {
        // `add : (-> Int64 Int64 Int64)` — 2 params (the trailing Int64 is the RETURN, not a slot).
        let text = "(do (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) (add 1 2)))";
        let first_arg = text.find("add 1 2").unwrap() + 4; // on `1`
        let sh = signature_help_at(text, false, byte_to_position(text, first_arg))
            .expect("a signature inside the call");
        let sig = &sh.signatures[0];
        let params = sig
            .parameters
            .as_ref()
            .expect("per-parameter labels emitted");
        // Two parameter slots (return type dropped).
        assert_eq!(
            params.len(),
            2,
            "two parameters, not three (return dropped)"
        );
        // Each label offset must index a substring of the signature label that reads as the param's type.
        let label_utf16: Vec<u16> = sig.label.encode_utf16().collect();
        for p in params {
            let ParameterLabel::LabelOffsets([s, e]) = p.label else {
                panic!("expected LabelOffsets, got {:?}", p.label);
            };
            assert!(
                (e as usize) <= label_utf16.len() && s < e,
                "offset [{s},{e}] must be a valid range into label {:?}",
                sig.label
            );
            let slice = String::from_utf16(&label_utf16[s as usize..e as usize]).unwrap();
            assert_eq!(
                slice, "Int64",
                "the highlighted slot reads as the param type"
            );
        }
        // On the first arg, active parameter is 0.
        assert_eq!(sh.active_parameter, Some(0), "first arg active");
        // Cursor well past the last arg → active clamps to the LAST parameter slot (index 1), never 2+.
        let after_second = text.find("1 2)").unwrap() + 3; // the `)` after `2`
        let sh2 = signature_help_at(text, false, byte_to_position(text, after_second))
            .expect("a signature still inside the call");
        assert_eq!(
            sh2.active_parameter,
            Some(1),
            "active clamps to the last parameter slot, not past it"
        );
    }

    #[test]
    fn signature_help_is_none_for_a_generic_callee_whose_type_is_unresolved() {
        // A FULLY-GENERIC def (`def (id x) x`) has no monomorphic type: the compiler's `TypeOf` query answers
        // `TypeInfo::Unknown` for it today (the still-open inferred-binder/polymorphic-scheme frontier), not a
        // concrete `(-> a a)`. Signature help must treat `Unknown` as "no callable signature" and return None —
        // NOT panic, and NOT leak a bogus label. This pins the `Unknown` arm of the binary-AST decode added in
        // #6264 (the `Found`/`NoDef` arms are covered by the label tests + the unbound-callee test); it is the
        // TOTALITY guard for the one decode arm those don't reach. If `TypeOf` ever starts resolving generic
        // schemes, this becomes the trigger to add positive Var-lettering coverage (render_ty_scheme letters).
        let text = "(do (def (id x) x) (def (main) (id 5)))";
        let on_arg = text.find("id 5").unwrap() + 3; // on the `5`
        assert!(
            signature_help_at(text, false, byte_to_position(text, on_arg)).is_none(),
            "a generic callee whose TypeOf is Unknown yields no signature (total, never a panic or bogus label)"
        );
    }

    #[test]
    fn signature_help_is_none_outside_a_call() {
        // A cursor NOT inside a `(callee arg…)` named call → no signature popup (total, never an error).
        let text = "(do (def answer 42) (def (main) answer))";
        // Cursor on the bare `42` literal (not inside a call).
        let on_literal = text.find("42").unwrap() + 1;
        assert!(
            signature_help_at(text, false, byte_to_position(text, on_literal)).is_none(),
            "no signature help outside a call"
        );
    }

    #[test]
    fn signature_help_is_total_on_a_mid_typed_unclosed_call() {
        // The realistic "as you type" editor scenario: the call is UNCLOSED because the user is still typing
        // (`(add 1 ` / `add(1, `). Queries over incomplete source must be TOTAL — never panic — per the
        // tooling spec. It's fine to return None here (an unclosed call may not parse into a named-call node
        // the finder recognizes); what MUST hold is no crash on the partial buffer.
        let sexpr = "(do (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) (add 1 ";
        let sexpr_cursor = sexpr.rfind("add 1 ").unwrap() + 6;
        // Must not panic; returns Option — either arm is acceptable, totality is the invariant.
        let _ = signature_help_at(sexpr, false, byte_to_position(sexpr, sexpr_cursor));
        let ml = "def add(a: Int64, b: Int64) -> Int64 = a + b\ndef main = add(1, ";
        let ml_cursor = ml.rfind("add(1, ").unwrap() + 7;
        let _ = signature_help_at(ml, true, byte_to_position(ml, ml_cursor));
        // Reaching here without a panic is the assertion (totality on partial/unclosed input).
    }

    #[test]
    fn signature_help_computes_label_offsets_for_a_compound_parameter_type() {
        // A parameter whose type is a PARENTHESISED compound (`(List Int64)`) exercises a different label-offset
        // path than the scalar `Int64` case (`signature_help_gives_per_parameter_label_offsets_…`): the offset
        // finder locates the multi-token substring `(List Int64)` in the label and must return a range that
        // reads back as exactly that compound. `sum : (-> (List Int64) Int64)` — one parameter (the trailing
        // Int64 is the return), so exactly one LabelOffsets slot spanning the compound type.
        let text = "(do (def (sum (: xs (List Int64))) 0) (def (main) (sum [1 2])))";
        let on_arg = text.find("sum [1 2]").unwrap() + 4; // inside the call args
        let sh = signature_help_at(text, false, byte_to_position(text, on_arg))
            .expect("a signature inside the call");
        let sig = &sh.signatures[0];
        assert_eq!(
            sig.label, "sum : (-> (List Int64) Int64)",
            "compound param type renders cleanly in the label"
        );
        let params = sig
            .parameters
            .as_ref()
            .expect("per-parameter labels emitted");
        assert_eq!(
            params.len(),
            1,
            "one parameter (the return Int64 is dropped)"
        );
        let ParameterLabel::LabelOffsets([s, e]) = params[0].label else {
            panic!("expected LabelOffsets, got {:?}", params[0].label);
        };
        let label_utf16: Vec<u16> = sig.label.encode_utf16().collect();
        assert!(
            (e as usize) <= label_utf16.len() && s < e,
            "offset [{s},{e}] must be a valid range into label {:?}",
            sig.label
        );
        let slice = String::from_utf16(&label_utf16[s as usize..e as usize]).unwrap();
        assert_eq!(
            slice, "(List Int64)",
            "the highlighted slot reads as the compound param type"
        );
    }

    #[test]
    fn signature_help_is_none_for_a_call_to_an_unbound_callee() {
        // A call whose head names NO definition — `TypeOf` answers an error string ("no such definition …"),
        // not an arrow, so the `->` guard rejects it and no bogus signature leaks. (Totality: never a panic
        // or an error-string-as-signature.)
        let text = "(do (def (main) (mystery 1 2)))";
        let on_arg = text.find("mystery 1 2").unwrap() + 8; // on the `1`
        assert!(
            signature_help_at(text, false, byte_to_position(text, on_arg)).is_none(),
            "no signature for a call to an unbound callee"
        );
    }

    #[test]
    fn signature_help_works_on_the_ml_surface_with_per_parameter_labels() {
        // Signature help must work on the ML SURFACE (what most editor users write), not just s-expr: the ML
        // parser desugars `add(1, 2)` into the same call shape the finder walks. Pins per-parameter labels
        // for an ML buffer (all other sighelp tests are s-expr `is_ml=false`, leaving this path un-covered).
        let text = "def add(a: Int64, b: Int64) -> Int64 = a + b\ndef main = add(1, 2)";
        let on_arg = text.rfind("add(1, 2)").unwrap() + 4; // on the `1`
        let sh = signature_help_at(text, true, byte_to_position(text, on_arg))
            .expect("a signature inside the ML call");
        assert!(
            sh.signatures[0].label.starts_with("add : "),
            "ML label names the callee + its type: {}",
            sh.signatures[0].label
        );
        let params = sh.signatures[0]
            .parameters
            .as_ref()
            .expect("per-parameter labels on the ML surface too");
        assert_eq!(params.len(), 2, "two parameters (return dropped)");
        assert_eq!(sh.active_parameter, Some(0), "first arg active");
    }

    #[test]
    fn signature_help_picks_the_innermost_enclosing_call_when_calls_nest() {
        // Nested calls `(add (id 5) 9)`: the finder picks the SMALLEST-span named call COVERING the cursor,
        // so a cursor on the OUTER argument `9` shows `add` (the inner `(id 5)` does not cover `9`), while a
        // cursor inside `(id 5)` shows `id`. This is what disambiguates which signature to surface as the
        // caret moves between an argument and a sub-call.
        let text = "(do (def (id (: x Int64)) x) \
                     (def (add (: a Int64) (: b Int64)) (+ a b)) \
                     (def (main) (add (id 5) 9)))";
        // Cursor on the outer call's second argument `9` → the enclosing call is `add`.
        let on_outer_arg = text.rfind("(id 5) 9").unwrap() + 7; // the `9`
        let outer = signature_help_at(text, false, byte_to_position(text, on_outer_arg))
            .expect("a signature at the outer arg");
        assert!(
            outer.signatures[0].label.starts_with("add : "),
            "cursor on the outer arg surfaces the OUTER callee `add`: {}",
            outer.signatures[0].label
        );
        // Cursor on the inner call's argument `5` → the enclosing call is `id` (the innermost).
        let on_inner_arg = text.rfind("id 5").unwrap() + 3; // the `5`
        let inner = signature_help_at(text, false, byte_to_position(text, on_inner_arg))
            .expect("a signature at the inner arg");
        assert!(
            inner.signatures[0].label.starts_with("id : "),
            "cursor inside the sub-call surfaces the INNER callee `id`: {}",
            inner.signatures[0].label
        );
    }

    #[test]
    fn signature_help_handler_returns_none_on_an_unopened_document() {
        let (server, _client) = memory_server();
        let params = SignatureHelpParams {
            context: None,
            text_document_position_params: lsp_types::TextDocumentPositionParams {
                text_document: lsp_types::TextDocumentIdentifier { uri: test_uri() },
                position: Position::new(0, 0),
            },
            work_done_progress_params: Default::default(),
        };
        assert!(
            server.signature_help(&params).is_none(),
            "signatureHelp on an unopened document must be None"
        );
    }

    #[test]
    fn signature_help_capability_is_advertised_with_trigger_chars() {
        let value = serde_json::to_value(capabilities()).expect("serializes");
        let shp = value.get("signatureHelpProvider").expect("advertised");
        let triggers = shp
            .get("triggerCharacters")
            .and_then(|v| v.as_array())
            .expect("trigger characters");
        assert!(
            triggers.iter().any(|c| c == "("),
            "`(` is a trigger char: {shp}"
        );
    }

    // FIXED (cdz/lsp, 2026-08-30): the batched `code_lenses_for` path returned ZERO lenses for a specialized
    // generic because it matched each `Instantiations` answer artifact by the def NAME (`a.name == name`),
    // but `rcdzc::compile` names a batched query answer by its POSITIONAL request index (`"0"`, `"1"`, …),
    // not the semantic name — so the match never hit. (v-inference confirmed the sidecar queries + compile
    // were correct; the stale name-based lookup was the whole bug.) Fixed by recovering the i-th answer by
    // index. This test (previously `#[ignore]`d to un-red the fleet gate) now passes and guards the fix.
    #[test]
    fn code_lens_reports_a_specialized_generics_monomorphizations() {
        // `loopn` is a recursive generic specialized at `x: Int64` and `x: String` — a lens above it lists
        // both concrete instances (the `Instantiations` query surfaced in the editor).
        let text = "(do (def (loopn (: n Int64) x) (if (= n 0) x (loopn (- n 1) x))) \
                    (def (main (: a Int64)) (+ (loopn 3 a) (String.scalar-len (loopn 2 \"hi\")))))";
        let lenses = code_lenses_for(text, false);
        assert_eq!(
            lenses.len(),
            1,
            "one lens (on the specialized `loopn`), got {lenses:?}"
        );
        let title = lenses[0]
            .command
            .as_ref()
            .map(|c| c.title.as_str())
            .unwrap_or("");
        assert!(
            title.starts_with("2 instances:"),
            "title should count instances: {title:?}"
        );
        assert!(
            title.contains("x: Int64") && title.contains("x: String"),
            "title should name both monomorphizations: {title:?}"
        );
        // The lens sits on `loopn`'s name occurrence (line 0).
        assert_eq!(lenses[0].range.start.line, 0);
        // The command id must be NON-EMPTY (LSP requirement; an empty id makes some clients drop the
        // lens) — it names the extension's no-op handler so the label lens is valid + non-actionable.
        assert_eq!(
            lenses[0].command.as_ref().map(|c| c.command.as_str()),
            Some("cadenza.showInstantiations"),
            "the lens must carry a non-empty command id: {:?}",
            lenses[0].command
        );
    }

    #[test]
    fn code_lens_is_empty_for_a_non_generic_program_and_total_on_malformed() {
        // A plain monomorphic program has nothing to specialize → no lenses.
        assert!(code_lenses_for("def double(x: Int64) -> Int64 = x + x", true).is_empty());
        // A buffer that does not parse yields no lenses, never a panic.
        let _ = code_lenses_for("def (f x = (", true);
        let _ = code_lenses_for("", true);
        // Mid-edit s-expr / truncated buffers are also total.
        let _ = code_lenses_for("(do (def (f", false);
        let _ = code_lenses_for("def f(x:", true);
    }

    #[test]
    fn code_lens_is_total_on_an_import_declaring_buffer() {
        // `code_lenses_for` is SINGLE-buffer (it does not follow the `(import …)` closure — instantiations
        // are a whole-program fact and a lone opened buffer only sees its own uses). A buffer that
        // declares an import whose library is NOT loaded must still be total: the unresolved import faults
        // as a diagnostic elsewhere, and the lens pass returns without a lens (or a panic) here.
        let text = "(do (import \"lib\" (helper)) (def (main) (helper 1)))";
        // Just must not panic; a specialized def could in principle still appear, but the point is totality.
        let _ = code_lenses_for(text, false);
        // The ML surface's import form too.
        let _ = code_lenses_for(
            "import { helper } from \"lib\"\ndef main() -> Int64 = helper(1)",
            true,
        );
    }

    #[test]
    fn instantiations_lens_title_only_titles_specialized_defs() {
        use cadenza_compile_abi::instantiations_wire::{Instance, Instantiations};
        let report = |dispositions: &[&str], instances: Vec<Instance>| Instantiations {
            known: true,
            name_node: Some(2),
            dispositions: dispositions.iter().map(|s| (*s).to_string()).collect(),
            instances,
        };
        // Not specialized (emitted/inlined) → no title.
        assert_eq!(
            instantiations_lens_title(&report(&["emitted"], vec![])),
            None
        );
        assert_eq!(
            instantiations_lens_title(&report(&["inlined"], vec![])),
            None
        );
        // Specialized with instances → a counted, bracketed title.
        let specialized = report(
            &["specialized"],
            vec![
                Instance {
                    spec_name: "f#mono2".into(),
                    args: vec!["x: Int64".into()],
                },
                Instance {
                    spec_name: "f#mono3".into(),
                    args: vec!["x: String".into()],
                },
            ],
        );
        assert_eq!(
            instantiations_lens_title(&specialized).as_deref(),
            Some("2 instances: [x: Int64] · [x: String]")
        );
        // `specialized` disposition but no instances (defensive) → no title.
        assert_eq!(
            instantiations_lens_title(&report(&["specialized"], vec![])),
            None
        );
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
