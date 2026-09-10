//! The `cdz-cas-http` server binary — serves an (in-memory v0) content-addressed store over HTTP.
//!
//! Usage: `cdz-cas-http [listen-addr]` (default `127.0.0.1:8080`). Credentials come from the environment:
//! - `CDZ_CAS_READ_CREDENTIAL` — if set, `GET`/`HEAD` require `Authorization: Bearer {it}`; unset ⇒ open
//!   reads (the store is unpermissioned, the hash is the capability).
//! - `CDZ_CAS_WRITE_CREDENTIAL` — required to enable the `PUT` write path; unset ⇒ a read-only server
//!   (`PUT` ⇒ `405`).
//!
//! v0 backs the store in memory (lost on restart); the backend is a swappable `BlobStore` trait object, so
//! an on-disk/S3 backend drops in later with no wire change.

use cdz_cas_http::CasServer;
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

    let mut server = CasServer::in_memory();
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
        "cdz-cas-http: listening on {addr} (writes {})",
        if writes_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    server.serve(listener).await
}
