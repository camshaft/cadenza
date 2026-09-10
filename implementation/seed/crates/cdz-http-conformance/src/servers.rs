//! Per-server orchestration (`DESIGN-http-outpost-conformance-harness.md` §3.2), layered on the generic
//! [`crate::process::ServerProcess`]. The driver brings up the three SUTs on ephemeral (`:0`) sockets and
//! learns each REAL bound port from the process's ready line on stderr:
//!
//! - the CAS store `cdz-cas-http` — `cdz-cas-http: listening on <addr> (store …, writes …)`; positional
//!   listen-addr, store + credentials via env (`CDZ_CAS_STORE_DIR`, `CDZ_CAS_WRITE_CREDENTIAL`).
//! - the mock control server `cdz-http-control-mock` — `cdz-http-control-mock: admin=<a> control=<c>
//!   cas-url=<u>`; `--admin-addr`/`--control-addr`/`--cas-url` (+ optional `--cas-credential`).
//! - the stock gateway `cdz-http-gateway` — `gateway: listen=<b> control=<c>`; `--listen-addr` (binds `:0`,
//!   the line reports the real port) / `--control-addr` (the mock's control ws).
//!
//! Each `spawn_*` returns a typed handle owning the [`ServerProcess`] (so the child dies when the handle
//! drops) plus the parsed addresses the driver wires the next server / its HTTP client to. The run-loop
//! (spawn order CAS → mock → gateway, seed the CAS, drive the run-spec) sits on top of these handles.

use crate::process::{ServerProcess, field_after_eq};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

/// How long to wait for a server's ready line before treating the spawn as failed. Generous: a cold gateway
/// dials control + fetches the root router from the CAS before it reports `listen=…`.
const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// A running CAS store. `addr` is the real bound HTTP address (`http://{addr}/…` serves blobs by hash).
#[derive(Debug)]
pub struct CasServer {
    pub proc: ServerProcess,
    pub addr: SocketAddr,
}

/// A running mock control server. `admin_addr` is the driver-facing binary-AST admin channel; `control_addr`
/// is the gateway-facing control-plane ws; `cas_url` is the CAS base URL it ships to a gateway in its config.
#[derive(Debug)]
pub struct MockServer {
    pub proc: ServerProcess,
    pub admin_addr: SocketAddr,
    pub control_addr: SocketAddr,
    pub cas_url: String,
}

/// A running stock gateway. `listen_addr` is the real bound HTTP serve address the driver makes requests at;
/// `control_addr` echoes the control ws it dialed.
#[derive(Debug)]
pub struct GatewayServer {
    pub proc: ServerProcess,
    pub listen_addr: SocketAddr,
    pub control_addr: SocketAddr,
}

/// Spawn the CAS store, binding `listen` (pass `127.0.0.1:0` for an ephemeral port). `store_dir` (if set)
/// persists blobs on disk — the seeding strategy where the nix rig pre-fills a by-hash dir the CAS serves
/// read-only; `write_credential` (if set) enables the `PUT` write path so the driver can seed over HTTP.
///
/// # Errors
/// The bin cannot be spawned, or it does not print its `listening on <addr>` line before the ready timeout.
pub async fn spawn_cas(
    bin: &Path,
    listen: &str,
    store_dir: Option<&str>,
    write_credential: Option<&str>,
) -> Result<CasServer, String> {
    let mut envs: Vec<(&str, &str)> = Vec::new();
    if let Some(dir) = store_dir {
        envs.push(("CDZ_CAS_STORE_DIR", dir));
    }
    if let Some(cred) = write_credential {
        envs.push(("CDZ_CAS_WRITE_CREDENTIAL", cred));
    }
    let mut proc = ServerProcess::spawn_with_env(bin, &[listen], &envs)?;
    let addr = proc.wait_for_ready(READY_TIMEOUT, parse_cas_ready).await?;
    Ok(CasServer { proc, addr })
}

/// Spawn the mock control server on ephemeral admin + control sockets, configured to ship `cas_url` (+ an
/// optional read `cas_credential`) to a gateway.
///
/// # Errors
/// The bin cannot be spawned, or it does not print its `admin=… control=…` line before the ready timeout.
pub async fn spawn_mock(
    bin: &Path,
    admin: &str,
    control: &str,
    cas_url: &str,
    cas_credential: Option<&str>,
) -> Result<MockServer, String> {
    let mut args = vec![
        "--admin-addr",
        admin,
        "--control-addr",
        control,
        "--cas-url",
        cas_url,
    ];
    if let Some(cred) = cas_credential {
        args.push("--cas-credential");
        args.push(cred);
    }
    let mut proc = ServerProcess::spawn(bin, &args)?;
    let (admin_addr, control_addr, url) =
        proc.wait_for_ready(READY_TIMEOUT, parse_mock_ready).await?;
    Ok(MockServer {
        proc,
        admin_addr,
        control_addr,
        cas_url: url,
    })
}

/// Spawn the stock gateway, binding `listen` (`127.0.0.1:0` for ephemeral) and dialing the mock's control ws
/// at `control`. The gateway boots from the `ControlConfig` control ships on connect, then reports `listen=…`.
///
/// # Errors
/// The bin cannot be spawned, or it does not print its `listen=… control=…` line before the ready timeout
/// (e.g. it could not dial control or fetch the root router).
pub async fn spawn_gateway(
    bin: &Path,
    listen: &str,
    control: &str,
) -> Result<GatewayServer, String> {
    let mut proc =
        ServerProcess::spawn(bin, &["--listen-addr", listen, "--control-addr", control])?;
    let (listen_addr, control_addr) = proc
        .wait_for_ready(READY_TIMEOUT, parse_gateway_ready)
        .await?;
    Ok(GatewayServer {
        proc,
        listen_addr,
        control_addr,
    })
}

/// Parse the CAS ready line `cdz-cas-http: listening on <addr> (…)` → the bound address.
fn parse_cas_ready(line: &str) -> Option<SocketAddr> {
    line.split("listening on ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Parse the mock ready line `… admin=<a> control=<c> cas-url=<u>` → (admin, control, cas-url).
fn parse_mock_ready(line: &str) -> Option<(SocketAddr, SocketAddr, String)> {
    let admin = field_after_eq(line, "admin")?.parse().ok()?;
    let control = field_after_eq(line, "control")?.parse().ok()?;
    let cas_url = field_after_eq(line, "cas-url")?.to_string();
    Some((admin, control, cas_url))
}

/// Parse the gateway ready line `gateway: listen=<b> control=<c>` → (listen, control).
fn parse_gateway_ready(line: &str) -> Option<(SocketAddr, SocketAddr)> {
    let listen = field_after_eq(line, "listen")?.parse().ok()?;
    let control = field_after_eq(line, "control")?.parse().ok()?;
    Some((listen, control))
}

#[cfg(test)]
mod tests {
    //! The spawners are exercised against `/bin/sh` stand-ins that print each SUT's EXACT ready-line format
    //! (verified byte-for-byte against the bins' `eprintln!`s), so the parsing + typed-handle wiring is
    //! covered without building the real (heavy) servers. The full-process e2e (real bins under a nix rig)
    //! is a later slice. `/bin/sh` is always present in Nix's build sandbox (see `process::tests`).
    use super::*;

    /// Spawn a `/bin/sh` stand-in that echoes `ready_line` to stderr, then lingers like a real server.
    fn stub(ready_line: &str) -> ServerProcess {
        let script = format!("echo '{ready_line}' 1>&2; sleep 30");
        ServerProcess::spawn(Path::new("/bin/sh"), &["-c", &script]).expect("spawn stub")
    }

    #[tokio::test]
    async fn cas_ready_line_parses_to_the_bound_address() {
        let mut proc =
            stub("cdz-cas-http: listening on 127.0.0.1:45001 (store in-memory, writes disabled)");
        let addr = proc
            .wait_for_ready(Duration::from_secs(5), parse_cas_ready)
            .await
            .expect("cas ready");
        assert_eq!(addr, "127.0.0.1:45001".parse().unwrap());
    }

    #[tokio::test]
    async fn mock_ready_line_parses_admin_control_and_cas_url() {
        let mut proc = stub(
            "cdz-http-control-mock: admin=127.0.0.1:45002 control=127.0.0.1:45003 cas-url=http://127.0.0.1:45001/",
        );
        let (admin, control, cas_url) = proc
            .wait_for_ready(Duration::from_secs(5), parse_mock_ready)
            .await
            .expect("mock ready");
        assert_eq!(admin, "127.0.0.1:45002".parse().unwrap());
        assert_eq!(control, "127.0.0.1:45003".parse().unwrap());
        assert_eq!(cas_url, "http://127.0.0.1:45001/");
    }

    #[tokio::test]
    async fn gateway_ready_line_parses_listen_and_control() {
        let mut proc = stub("gateway: listen=127.0.0.1:45004 control=127.0.0.1:45003");
        let (listen, control) = proc
            .wait_for_ready(Duration::from_secs(5), parse_gateway_ready)
            .await
            .expect("gateway ready");
        assert_eq!(listen, "127.0.0.1:45004".parse().unwrap());
        assert_eq!(control, "127.0.0.1:45003".parse().unwrap());
    }

    #[test]
    fn parsers_reject_malformed_lines() {
        assert!(parse_cas_ready("cdz-cas-http: bound somewhere").is_none());
        assert!(
            parse_mock_ready("cdz-http-control-mock: admin=nope control=127.0.0.1:1 cas-url=x")
                .is_none()
        );
        assert!(parse_gateway_ready("gateway: listen=127.0.0.1:1").is_none());
    }
}
