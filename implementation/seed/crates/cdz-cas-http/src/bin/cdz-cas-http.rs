//! The `cdz-cas-http` server binary — serves a content-addressed store over HTTP.
//!
//! Usage: `cdz-cas-http [listen-addr]` (default `127.0.0.1:8080`). Configured from the environment:
//! - `CDZ_CAS_STORE_DIR` — if set, blobs are persisted on disk under this directory (survives restart, for
//!   deploy tooling); unset ⇒ an in-memory store (lost on restart). The backend is a swappable `BlobStore`
//!   trait object either way, so the HTTP wire is identical.
//! - `CDZ_CAS_READ_CREDENTIAL` — if set, `GET`/`HEAD` require `Authorization: Bearer {it}`; unset ⇒ open
//!   reads (the store is unpermissioned, the hash is the capability).
//! - `CDZ_CAS_WRITE_CREDENTIAL` — required to enable the `PUT` write path; unset ⇒ a read-only server
//!   (`PUT` ⇒ `405`).

use cdz_cas_http::{CasServer, DiskBlobStore};
use cdz_platform::{BlobStore, InMemoryBlobStore};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string())
        .parse()
        .expect("listen address must be host:port");

    // On-disk when CDZ_CAS_STORE_DIR is set (persistent across restart), else in-memory.
    let (store, backend): (Box<dyn BlobStore>, String) = match std::env::var("CDZ_CAS_STORE_DIR") {
        Ok(dir) => (
            Box::new(DiskBlobStore::open(&dir)?),
            format!("on-disk at {dir}"),
        ),
        Err(_) => (Box::new(InMemoryBlobStore::new()), "in-memory".to_string()),
    };

    let mut server = CasServer::new(store);
    if let Ok(read) = std::env::var("CDZ_CAS_READ_CREDENTIAL") {
        server = server.with_read_credential(read);
    }
    let writes_enabled = if let Ok(write) = std::env::var("CDZ_CAS_WRITE_CREDENTIAL") {
        server = server.with_write_credential(write);
        true
    } else {
        false
    };
    let server = Arc::new(server);

    let listener = TcpListener::bind(addr).await?;
    eprintln!(
        "cdz-cas-http: listening on {addr} (store {backend}, writes {})",
        if writes_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    server.serve(listener).await
}
