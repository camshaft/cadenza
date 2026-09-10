//! The `cdz-cas-http` server binary — configured by a **binary-AST config document**, not CLI flags
//! (operator: binary-AST is THE data-exchange format). Give the config as the first argument (or via
//! `CDZ_CAS_CONFIG`): a file path, or `-` to read it from STDIN (pipe the binary-AST bytes in). The config
//! is a cadenza-ast binary the caller produced from any surface syntax (`cdz convert --to bin`). With no
//! config, the server defaults to a single in-memory store on `127.0.0.1:8080`. See [`cdz_cas_http::config`]
//! for the record schema (listen, credentials, and the mem/disk/S3 tiers).

use cdz_cas_http::{CasServer, ServerConfig};
use std::io::Read;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match config_path().as_deref() {
        // `-` reads the binary-AST config from stdin (pipe it in).
        Some("-") => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|e| format!("read config from stdin: {e}"))?;
            ServerConfig::decode(&bytes)?
        }
        Some(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("read config {path}: {e}"))?;
            ServerConfig::decode(&bytes)?
        }
        None => ServerConfig::default(),
    };

    let addr: std::net::SocketAddr = config
        .listen_addr()
        .parse()
        .map_err(|e| format!("bad listen address {:?}: {e}", config.listen_addr()))?;

    // Assemble the configured tier stack (mem → disk → S3), then serve it.
    let store = config.build_store().await?;
    let mut server = CasServer::new(store);
    if let Some(read) = &config.read_credential {
        server = server.with_read_credential(read.clone());
    }
    if let Some(write) = &config.write_credential {
        server = server.with_write_credential(write.clone());
    }
    if let Some(max) = config.max_body_bytes {
        server = server.with_max_body_bytes(max);
    }
    let server = Arc::new(server);

    let listener = TcpListener::bind(addr).await?;
    eprintln!(
        "cdz-cas-http: listening on {addr} (writes {})",
        if config.write_credential.is_some() {
            "enabled"
        } else {
            "disabled"
        }
    );
    server.serve(listener).await?;
    Ok(())
}

/// The config source: the first CLI argument, else `CDZ_CAS_CONFIG`, else `None` (run with defaults). A
/// value of `-` means "read the config from stdin".
fn config_path() -> Option<String> {
    std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CDZ_CAS_CONFIG").ok())
}
