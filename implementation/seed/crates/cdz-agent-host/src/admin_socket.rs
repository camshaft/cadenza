//! The admin control interface's TRANSPORT — a local Unix-domain-socket listener that carries
//! [`AdminCommandWire`]/[`AdminResponseWire`] frames between an admin client and the running daemon.
//!
//! This is the outermost slice of the admin control interface, and the only part that touches the OS: it
//! binds a Unix socket, and for each connection reads a length-prefixed JSON command frame, forwards it to
//! the host loop over the in-process [`AdminChannel`] (awaiting the reply on a oneshot), and writes the
//! response frame back. The socket task is `Send` (it only moves bytes + channel handles), so it can run
//! as its own tokio task while the single-threaded host loop applies the command against the `!Send`
//! registry — the Send/!Send split the [`crate::async_host`] loop was built for.
//!
//! **Feature-gated (`admin`).** The whole module is behind the `admin` cargo feature so the DEFAULT build
//! binds no socket + pulls no tokio `net` tree (the hermetic-gate discipline). The admin COMMAND layer
//! ([`crate::admin`]) and wire CODEC ([`crate::admin_wire`]) stay always-on and hermetically tested; only
//! this LISTENER is opt-in. Local IPC only — a Unix socket is not network egress (a remote admin transport
//! would be a later opt-in with its own auth); the file-permissions gate (owner-only) is the v0 auth, with
//! a Cedar `admin/*` policy layer to follow.

use crate::admin_wire::{decode_frame, encode_frame, AdminCommandWire, AdminResponseWire};
use crate::async_host::{AdminChannel, AdminRequest};
use std::io;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

/// A bound admin control socket. Construct with [`AdminSocket::bind`] (binds the path + sets owner-only
/// perms), then [`serve`](AdminSocket::serve) it — the accept loop that feeds admin commands to the host.
/// Dropping it removes the socket file (best-effort) so a restart can re-bind the same path.
pub struct AdminSocket {
    listener: UnixListener,
    path: PathBuf,
}

impl AdminSocket {
    /// Bind the admin control socket at `path`, replacing any stale socket file left by a previous run, and
    /// restrict it to OWNER-ONLY (mode `0o600`) — the v0 OS-level auth (admin = whoever owns the daemon
    /// process). `Err` if the bind or the perms-set fails.
    ///
    /// (A stale socket file from an unclean shutdown would make `bind` fail with `AddrInUse`; we remove a
    /// pre-existing file first. This is safe for a single-daemon deployment — two daemons sharing one admin
    /// socket path is a misconfiguration, not something to defend by leaving a stale file to block bind.)
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        // Remove a stale socket file (ignore "not found"); a real bind failure surfaces below.
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        // Owner-only: an admin command installs/stops sessions, so the socket must not be world-writable.
        // This is the v0 auth gate (a Cedar admin/* policy layer follows). Unix-only perms via a raw mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(AdminSocket { listener, path })
    }

    /// The bound socket path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accept admin connections forever, dispatching each command through `admin` to the host loop. Runs
    /// until `shutdown` fires (then returns) — typically spawned as a tokio task beside the host loop. Each
    /// accepted connection is served inline (admin traffic is low-volume + serialized through the one host
    /// loop anyway, so there's no win in spawning per-connection; keeping it inline needs no `Send` bound on
    /// the connection future and keeps the ordering obvious). An accept error is logged to stderr and the
    /// loop continues — one bad connection never takes the listener down.
    pub async fn serve(
        self,
        admin: AdminChannel,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, _addr)) => {
                            // Serve this connection to completion. An error mid-connection (client hung up,
                            // a malformed frame) ends only THIS connection — never the listener.
                            if let Err(e) = serve_connection(stream, &admin).await {
                                eprintln!("cdz-agent-daemon admin: connection ended with error: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("cdz-agent-daemon admin: accept failed: {e}");
                        }
                    }
                }
            }
        }
    }
}

impl Drop for AdminSocket {
    fn drop(&mut self) {
        // Best-effort cleanup so a restart can re-bind the same path (ignore errors — the process is going
        // away regardless).
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Serve one admin connection: read command frames, dispatch each to the host loop, write back the response
/// frame — until the client closes the connection (clean EOF) or an error occurs. A connection can carry
/// MULTIPLE commands (the client may pipeline), so this loops until EOF rather than one-shot.
async fn serve_connection(mut stream: UnixStream, admin: &AdminChannel) -> io::Result<()> {
    // Accumulate bytes; decode as many whole frames as have arrived, dispatch each, reply. This handles a
    // command split across reads (partial frame → keep reading) and multiple commands in one read.
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        // Drain every COMPLETE frame currently in `buf`.
        loop {
            match decode_frame::<AdminCommandWire>(&buf) {
                Ok(Some((cmd_wire, consumed))) => {
                    buf.drain(..consumed);
                    let response = dispatch(cmd_wire, admin).await;
                    let frame = encode_frame(&response).map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, format!("encode response: {e}"))
                    })?;
                    stream.write_all(&frame).await?;
                    stream.flush().await?;
                }
                // Not a full frame yet — go read more bytes.
                Ok(None) => break,
                // A malformed frame (oversized length / bad JSON): reply with an error frame, then stop
                // serving this connection (the stream framing is now untrustworthy).
                Err(e) => {
                    let err = AdminResponseWire::Error {
                        message: format!("bad admin frame: {e}"),
                    };
                    if let Ok(frame) = encode_frame(&err) {
                        let _ = stream.write_all(&frame).await;
                        let _ = stream.flush().await;
                    }
                    return Ok(());
                }
            }
        }
        // Read more bytes. 0 = clean EOF (client closed) → we're done with this connection.
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Forward one decoded command to the host loop and await its reply. Converts the wire command to the
/// domain [`crate::admin::AdminCommand`] (returning a wire error response if the frame's hash is malformed),
/// sends it on the [`AdminChannel`] with a fresh reply oneshot, and maps the domain response back to wire.
async fn dispatch(cmd_wire: AdminCommandWire, admin: &AdminChannel) -> AdminResponseWire {
    let command = match cmd_wire.to_domain() {
        Ok(c) => c,
        // A structurally-valid frame whose content is invalid (e.g. a non-canonical reducer hash) — answer
        // with an error, don't tear down the connection.
        Err(e) => {
            return AdminResponseWire::Error {
                message: format!("invalid admin command: {e}"),
            }
        }
    };
    let (reply, rx) = tokio::sync::oneshot::channel();
    if admin.send(AdminRequest { command, reply }).is_err() {
        // The host loop is gone (shutting down) — report it rather than hang.
        return AdminResponseWire::Error {
            message: "daemon host loop is not accepting admin commands".into(),
        };
    }
    match rx.await {
        Ok(resp) => AdminResponseWire::from_domain(&resp),
        // The loop dropped the reply without answering (shutdown mid-command).
        Err(_) => AdminResponseWire::Error {
            message: "daemon shut down before replying to the admin command".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{AdminCommand, InstallSpec, SessionFactory};
    use crate::async_host::AsyncAgentHost;
    use crate::host::{AgentHost, HostedSession, SessionId};
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::event::Event;
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::hash::Hash;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    struct StubAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for StubAgent {
        async fn fold(&self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
            FoldOutput::none()
        }
    }

    struct StubFactory;
    #[async_trait::async_trait(?Send)]
    impl SessionFactory for StubFactory {
        async fn build(&mut self, spec: &InstallSpec) -> Result<HostedSession, String> {
            Ok(HostedSession::genesis(
                spec.reducer_hash,
                Box::new(StubAgent),
                Box::new(Authorizer::deny_all()),
                CompositeExecutor::new(),
            ))
        }
    }

    /// A unique temp path for a test socket (no `Date`/`rand` in this env; the test name + a counter arg
    /// keeps it unique across the two socket tests).
    fn socket_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("cdz-admin-test-{tag}.sock"))
    }

    /// Round-trip a command over a real client `UnixStream` to the served socket: write a frame, read the
    /// response frame, decode it.
    async fn client_call(stream: &mut UnixStream, cmd: AdminCommand) -> AdminResponseWire {
        let frame = encode_frame(&AdminCommandWire::from(&cmd)).unwrap();
        stream.write_all(&frame).await.unwrap();
        stream.flush().await.unwrap();
        // Read until a full response frame decodes.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            if let Some((resp, _)) = decode_frame::<AdminResponseWire>(&buf).unwrap() {
                return resp;
            }
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0, "server closed before a full response");
            buf.extend_from_slice(&chunk[..n]);
        }
    }

    #[tokio::test]
    async fn install_and_list_over_a_real_unix_socket() {
        let path = socket_path("install-list");
        let sock = AdminSocket::bind(&path).expect("bind admin socket");

        // Owner-only perms on the socket file (the v0 auth gate).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "socket is owner-only");
        }

        // The host loop (with a factory so installs work) + its admin channel.
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory));
        let admin = async_host.admin_channel();
        let (host_sd_tx, host_sd_rx) = tokio::sync::oneshot::channel();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel();

        // Client work: connect, install a session, list it back.
        let client = async {
            let mut stream = UnixStream::connect(&path).await.expect("connect");
            let installed = client_call(
                &mut stream,
                AdminCommand::InstallSession(InstallSpec {
                    id: SessionId::new("s1"),
                    reducer_hash: Hash::of(b"s1"),
                    goal: None,
                }),
            )
            .await;
            assert_eq!(installed, AdminResponseWire::Installed { id: "s1".into() });

            let listed = client_call(&mut stream, AdminCommand::ListSessions).await;
            assert_eq!(
                listed,
                AdminResponseWire::Sessions {
                    ids: vec!["s1".into()]
                }
            );
            // Done: close the client stream, then stop the socket + host loop.
            drop(stream);
            let _ = sock_sd_tx.send(());
            let _ = host_sd_tx.send(());
        };

        // Run the host loop, the socket server, and the client concurrently (single-threaded — everything
        // is on one task set via join!, matching the daemon's single-threaded shape).
        let (_host, (), ()) = tokio::join!(
            async_host.run(host_sd_rx, || 0),
            sock.serve(admin, sock_sd_rx),
            client,
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_garbage_frame_gets_an_error_response_not_a_crash() {
        let path = socket_path("garbage");
        let sock = AdminSocket::bind(&path).expect("bind");
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory));
        let admin = async_host.admin_channel();
        let (host_sd_tx, host_sd_rx) = tokio::sync::oneshot::channel();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel();

        let client = async {
            let mut stream = UnixStream::connect(&path).await.unwrap();
            // A well-framed but non-JSON body: 4-byte length header + garbage bytes.
            let body = b"this is not json";
            let mut frame = (body.len() as u32).to_be_bytes().to_vec();
            frame.extend_from_slice(body);
            stream.write_all(&frame).await.unwrap();
            stream.flush().await.unwrap();

            // Read the error response.
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let resp = loop {
                if let Some((resp, _)) = decode_frame::<AdminResponseWire>(&buf).unwrap() {
                    break resp;
                }
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "server closed without an error response");
                buf.extend_from_slice(&chunk[..n]);
            };
            assert!(
                matches!(resp, AdminResponseWire::Error { message } if message.contains("bad admin frame")),
                "a garbage frame gets an error response"
            );
            drop(stream);
            let _ = sock_sd_tx.send(());
            let _ = host_sd_tx.send(());
        };

        let (_host, (), ()) = tokio::join!(
            async_host.run(host_sd_rx, || 0),
            sock.serve(admin, sock_sd_rx),
            client,
        );
        let _ = std::fs::remove_file(&path);
    }
}
