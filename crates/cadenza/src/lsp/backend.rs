//! LSP backend implementation using tower-lsp.

use cadenza_lsp::core;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tower_lsp::{Client, LanguageServer, jsonrpc::Result, lsp_types::*};

/// The main LSP backend for Cadenza.
pub struct CadenzaLspBackend {
    client: Client,
    documents: RwLock<HashMap<Url, String>>,
}

impl CadenzaLspBackend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: RwLock::new(HashMap::new()),
        }
    }

    async fn publish_diagnostics(&self, uri: Url, text: &str) {
        // Convert from cadenza_lsp diagnostics to tower_lsp diagnostics
        let diagnostics = core::parse_to_diagnostics(text)
            .into_iter()
            .map(|d| Diagnostic {
                range: d.range,
                severity: d.severity,
                code: d.code,
                code_description: d.code_description,
                source: d.source,
                message: d.message,
                related_information: d.related_information,
                tags: d.tags,
                data: d.data,
            })
            .collect();

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for CadenzaLspBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "cadenza-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![".".to_string()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        tracing::info!("Cadenza LSP server initialized");
    }

    async fn shutdown(&self) -> Result<()> {
        tracing::info!("Cadenza LSP server shutting down");
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;

        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        if let Some(change) = params.content_changes.into_iter().next() {
            let text = change.text;
            self.documents
                .write()
                .await
                .insert(uri.clone(), text.clone());
            self.publish_diagnostics(uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .write()
            .await
            .remove(&params.text_document.uri);
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let documents = self.documents.read().await;
        let text = match documents.get(&uri) {
            Some(text) => text,
            None => return Ok(None),
        };

        // For now, just show a simple hover with the word at the position
        let offset = core::position_to_offset(text, position);

        // Find the word boundaries around the offset
        let start = text[..offset]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| i + 1)
            .unwrap_or(0);

        let end = text[offset..]
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|i| offset + i)
            .unwrap_or(text.len());

        if start >= end {
            return Ok(None);
        }

        let word = &text[start..end];

        if word.is_empty() {
            return Ok(None);
        }

        // Create a simple hover message
        let contents = HoverContents::Scalar(MarkedString::String(format!(
            "Symbol: `{}`\n\nType information coming soon!",
            word
        )));

        Ok(Some(Hover {
            contents,
            range: None,
        }))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let _uri = params.text_document_position.text_document.uri;
        let _position = params.text_document_position.position;

        // For now, return some basic completions
        let items = vec![
            CompletionItem {
                label: "let".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Variable binding".to_string()),
                ..Default::default()
            },
            CompletionItem {
                label: "fn".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Function definition".to_string()),
                ..Default::default()
            },
        ];

        Ok(Some(CompletionResponse::Array(items)))
    }
}
