//! The conformance-harness DRIVER library (`DESIGN-http-outpost-conformance-harness.md` §3.2).
//!
//! The driver spins up the SUT processes and executes a run-spec's steps against them. This module starts
//! with the **admin client** — the driver's half of the binary-AST admin channel to the mock control server:
//! it ENCODES an [`AdminCommand`] (via `cdz_http_control_mock::admin`), POSTs it to the mock's `/admin`
//! endpoint, and DECODES the [`AdminReply`]. (The process-orchestration, the ML run-spec interpreter, and the
//! gateway HTTP client are following pieces; this is the reusable control-injection + observation client.)

use cdz_http_control_mock::admin::{AdminCommand, AdminReply, decode_reply, encode_command};
use std::net::SocketAddr;

/// The run-spec parser — decode a conformance run's binary-AST value into a [`spec::RunSpec`].
pub mod spec;

/// Process orchestration — spawn each SUT bin, wait for its ready line, kill it on drop.
pub mod process;

/// Per-server spawners — typed handles (CAS / mock / gateway) with their parsed bound addresses.
pub mod servers;

/// The gateway HTTP client — make a run-spec's `http` requests + capture the response for `Expect`.
pub mod gateway;

/// The CAS seeding client — publish programs by hash into the store the gateway fetches from.
pub mod cas;

/// Program resolution — a name→wasm-path manifest + the canonical `ProgramHash` computation.
pub mod programs;

/// The run-loop's scenario-execution core — bin resolution, step driving, and the pass/fail verdict.
pub mod run;

/// A client for one mock control server's admin channel — binary-AST commands/replies over HTTP. Holds ONE
/// pooled [`reqwest::Client`]; connections are kept alive + reused across commands (not one-off per command).
/// Cheap to `Clone` (the client shares its connection pool).
#[derive(Debug, Clone)]
pub struct AdminClient {
    endpoint: String,
    http: reqwest::Client,
}

impl AdminClient {
    /// A client targeting the mock's admin endpoint at `addr`.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            endpoint: format!("http://{addr}/admin"),
            http: reqwest::Client::new(),
        }
    }

    /// Send an [`AdminCommand`] and return the mock's [`AdminReply`], reusing the pooled connection. Errs (a
    /// `String`) on any transport / decode failure so the driver can surface it as a scenario setup error.
    ///
    /// # Errors
    /// Any transport failure, a non-2xx status from the mock, or a reply that fails to decode.
    pub async fn send(&self, cmd: &AdminCommand) -> Result<AdminReply, String> {
        let resp = self
            .http
            .post(&self.endpoint)
            .body(encode_command(cmd))
            .send()
            .await
            .map_err(|e| format!("admin send: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("admin status {}", resp.status()));
        }
        let body = resp.bytes().await.map_err(|e| format!("admin body: {e}"))?;
        decode_reply(&body).ok_or_else(|| "admin reply decode failed".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
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
