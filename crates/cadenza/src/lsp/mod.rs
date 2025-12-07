//! LSP (Language Server Protocol) implementation for Cadenza.
//!
//! This module provides a Language Server Protocol server that can be used
//! by editors and IDEs to provide intelligent code editing features.

mod backend;

use anyhow::Result;
use tower_lsp::{LspService, Server};

/// Start the LSP server on stdin/stdout.
pub async fn start_server() -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(backend::CadenzaLspBackend::new);

    tracing::info!("Starting Cadenza LSP server");

    Server::new(stdin, stdout, socket).serve(service).await;

    Ok(())
}
