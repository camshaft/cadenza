//! The conformance-harness DRIVER library (`DESIGN-http-outpost-conformance-harness.md` §3.2).
//!
//! The driver spins up the SUT processes and executes a run-spec's steps against them. This module starts
//! with the **admin client** — the driver's half of the binary-AST admin channel to the mock control server:
//! it ENCODES an [`AdminCommand`] (via `cdz_http_control_mock::admin`), POSTs it to the mock's `/admin`
//! endpoint, and DECODES the [`AdminReply`]. (The process-orchestration, the ML run-spec interpreter, and the
//! gateway HTTP client are following pieces; this is the reusable control-injection + observation client.)

use bytes::Bytes;
use cdz_http_control_mock::admin::{AdminCommand, AdminReply, decode_reply, encode_command};
use http_body_util::{BodyExt, Full};
use hyper_util::rt::TokioIo;
use std::net::SocketAddr;

/// A client for one mock control server's admin channel — binary-AST commands/replies over HTTP/1.
#[derive(Debug, Clone)]
pub struct AdminClient {
    addr: SocketAddr,
}

impl AdminClient {
    /// A client targeting the mock's admin endpoint at `addr`.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    /// Send an [`AdminCommand`] and return the mock's [`AdminReply`]. Errs (a `String`) on any transport /
    /// decode failure so the driver can surface it as a scenario setup error.
    ///
    /// # Errors
    /// Any connect/handshake/HTTP/decode failure, or a non-2xx status from the mock.
    pub async fn send(&self, cmd: &AdminCommand) -> Result<AdminReply, String> {
        let stream = tokio::net::TcpStream::connect(self.addr)
            .await
            .map_err(|e| format!("admin connect {}: {e}", self.addr))?;
        let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| format!("admin handshake: {e}"))?;
        tokio::spawn(conn);
        let req = hyper::Request::builder()
            .method(hyper::Method::POST)
            .uri("/admin")
            .body(Full::new(encode_command(cmd)))
            .map_err(|e| format!("admin request build: {e}"))?;
        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| format!("admin send: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("admin status {}", resp.status()));
        }
        let body: Bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| format!("admin body: {e}"))?
            .to_bytes();
        decode_reply(&body).ok_or_else(|| "admin reply decode failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_http_control_mock::MockState;
    use cdz_http_control_mock::server::{AdminCtx, serve_admin};
    use cdz_http_control_mock::ws::new_sessions;
    use cdz_str::Str;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// Boot the mock's admin server in-process (the driver spawns the bin in production; in-process is enough
    /// to exercise the admin client). Returns the admin address + the shared state.
    async fn boot_admin() -> (SocketAddr, Arc<Mutex<MockState>>) {
        let state = Arc::new(Mutex::new(MockState::new(HashMap::new())));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ctx = AdminCtx {
            state: state.clone(),
            sessions: new_sessions(),
            cas_url: Str::from("http://127.0.0.1:9/cas"),
            cas_credential: Bytes::new(),
        };
        tokio::spawn(serve_admin(listener, ctx));
        (addr, state)
    }

    #[tokio::test]
    async fn admin_client_round_trips_commands_against_the_mock() {
        let (addr, state) = boot_admin().await;
        let client = AdminClient::new(addr);

        // Seed a program, then set it as the root router — the mock resolves the name to the seeded hash.
        let hash = Bytes::from_static(b"a-33-byte-program-hash-goes-here.");
        assert_eq!(
            client
                .send(&AdminCommand::SetProgram {
                    name: Str::from("router-hello"),
                    hash: hash.clone(),
                })
                .await
                .unwrap(),
            AdminReply::Ok
        );
        assert_eq!(
            client
                .send(&AdminCommand::SetRootRouter {
                    program: Str::from("router-hello"),
                })
                .await
                .unwrap(),
            AdminReply::Ok
        );
        // The mock now ships a config carrying the resolved hash.
        assert_eq!(state.lock().unwrap().config().unwrap().root_router, hash);

        // GetControlUps returns an empty list (nothing captured yet).
        assert_eq!(
            client.send(&AdminCommand::GetControlUps).await.unwrap(),
            AdminReply::ControlUps { ups: vec![] }
        );
    }

    #[tokio::test]
    async fn an_unresolvable_program_surfaces_as_an_error_reply() {
        let (addr, _state) = boot_admin().await;
        let client = AdminClient::new(addr);
        match client
            .send(&AdminCommand::SetRootRouter {
                program: Str::from("nope"),
            })
            .await
            .unwrap()
        {
            AdminReply::Error { message } => assert!(message.contains("nope")),
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
