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
    /// Bind the admin control socket at `path` and restrict it to OWNER-ONLY (mode `0o600`) — the v0
    /// OS-level auth (admin = whoever owns the daemon process). `Err` if the bind or the perms-set fails.
    ///
    /// **Bind FIRST, unlink ONLY a dead socket (#1962 review).** A prior version unconditionally
    /// `remove_file`d the path before binding — which would delete ANY inode there: a regular file (a
    /// fat-fingered config path → silent data loss) or an ACTIVE socket another daemon is accepting on
    /// (unlink+rebind → the other daemon is left serving a now-nameless socket, unreachable, no error). So
    /// we bind first; only on `AddrInUse` do we probe whether the existing socket is DEAD (no live
    /// listener — a `connect` is refused), and if so unlink + rebind. A live socket or a non-socket file at
    /// the path is a hard error (a real conflict / misconfiguration), never silently clobbered.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            // The path is occupied. Only reclaim it if it's a DEAD socket left by a crashed instance;
            // anything else (a live daemon, a regular file) is a genuine conflict we must not clobber.
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => reclaim_dead_socket_then_bind(&path)?,
            Err(e) => return Err(e),
        };
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
    /// until `shutdown` fires (then returns) — typically spawned as a tokio task beside the host loop.
    ///
    /// **One task PER connection (#1962 review).** Each accepted connection is served on its own
    /// `tokio::spawn`ed task rather than inline, so a STALLED client (one that connects and never completes
    /// a frame or closes) can't hold the accept loop and block every other admin — the inline-serve local
    /// DoS. This does NOT change command ordering: the serialization point is the single host loop behind
    /// the [`AdminChannel`] (every command still funnels through it), not `accept`; spawning only keeps
    /// `accept()` live. The connection future is `Send` (it moves a `UnixStream` + a cloned `AdminChannel`
    /// and never touches the `!Send` registry), so it spawns freely. An accept error is logged and the loop
    /// continues — one bad connection never takes the listener down.
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
                            // Serve on a detached task so a slow/stalled client can't block accept().
                            let admin = admin.clone();
                            tokio::spawn(async move {
                                // An error mid-connection (client hung up, a malformed frame) ends only
                                // THIS connection — never the listener.
                                if let Err(e) = serve_connection(stream, &admin).await {
                                    eprintln!("cdz-agent-daemon admin: connection ended with error: {e}");
                                }
                            });
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

/// The `AddrInUse` recovery path: the socket path is occupied. Reclaim it ONLY if it's a DEAD Unix socket
/// (a socket inode whose owning process is gone, so a `connect` is refused); a LIVE socket or a non-socket
/// file is a hard conflict we refuse to clobber. Returns the freshly-bound listener on success.
fn reclaim_dead_socket_then_bind(path: &Path) -> io::Result<UnixListener> {
    // The inode must be a SOCKET; a regular file at the admin path is a misconfiguration, not a stale
    // socket — never delete it (that was the data-loss hazard). If the path is ALREADY GONE here (a race:
    // unlinked between our bind→AddrInUse and this stat), that's the recoverable case — just rebind on the
    // now-free path rather than aborting on the bare `?` (#1977 review, the earlier sibling of the #1971
    // remove_file NotFound fix).
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return UnixListener::bind(path),
        Err(e) => return Err(e),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if !meta.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "admin socket path {} exists and is not a socket — refusing to remove it",
                    path.display()
                ),
            ));
        }
    }
    // Is a listener LIVE on it? A successful connect (or a connect that isn't ECONNREFUSED/ENOENT) means
    // someone is (or may be) serving — do NOT clobber a live daemon. ECONNREFUSED = a dead socket inode
    // whose owner is gone: safe to reclaim.
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "admin socket {} is already served by a live daemon — refusing to take it over",
                    path.display()
                ),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
            // Dead socket — the owner is gone. Safe to unlink + rebind.
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // Raced away between bind and here; just rebind below.
        }
        // Any other connect error is ambiguous — fail safe rather than risk clobbering a live socket.
        Err(e) => return Err(e),
    }
    // Unlink the dead socket, then rebind. IGNORE a NotFound here (#1971 review): the connect NotFound arm
    // above, or a TOCTOU where another process unlinked between symlink_metadata and now, means the path is
    // already gone — that's a RECOVERABLE state (bind will succeed on the free path), so a NotFound from
    // remove_file must NOT abort the rebind. Any other remove error is real and propagates.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    UnixListener::bind(path)
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

/// The principal the local Unix-socket transport asserts for every command. Over the `0o600` owner-only
/// socket, every connecting client IS the daemon's owner — the file perms are the real identity gate — so
/// the transport asserts one fixed local-admin identity and the host's [`AdminAuthorizer`](crate::admin::AdminAuthorizer)
/// scopes WHICH actions it may take. (A remote transport, if ever added, would carry a real authenticated
/// principal per connection instead.) Pair this with an authorizer that grants this principal, e.g.
/// `AllowList::deny_all().allow_any_principal(...)` or `AllowList::allow_all_for_local_admin()`.
pub const LOCAL_ADMIN_PRINCIPAL: &str = "local-admin";

/// Forward one decoded command to the host loop and await its reply. Converts the wire command to the
/// domain [`crate::admin::AdminCommand`] (returning a wire error response if the frame's hash is malformed),
/// sends it on the [`AdminChannel`] with a fresh reply oneshot (asserting the [`LOCAL_ADMIN_PRINCIPAL`] —
/// the socket's owner-gate identity), and maps the domain response back to wire.
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
    let request = AdminRequest {
        command,
        principal: Some(LOCAL_ADMIN_PRINCIPAL.to_string()),
        reply,
    };
    if admin.send(request).is_err() {
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
    use crate::admin::{AdminCommand, AllowList, InstallSpec, SessionFactory};
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
        async fn fold(&mut self, _event: &Event, _kv: &mut Kv) -> FoldOutput {
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
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
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
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
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

    #[test]
    fn bind_refuses_to_clobber_a_non_socket_file() {
        // The #1962 unlink-hazard fix: a REGULAR FILE at the admin path (fat-fingered config) must NOT be
        // deleted — bind errors instead of silently removing it (the old unconditional remove_file would
        // have destroyed it).
        let path = socket_path("regular-file");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"important data, not a socket").unwrap();

        let err = match AdminSocket::bind(&path) {
            Ok(_) => panic!("bind must refuse a non-socket file, not succeed"),
            Err(e) => e,
        };
        // The file is untouched (not clobbered).
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"important data, not a socket",
            "the regular file must be left intact, not deleted"
        );
        assert!(
            format!("{err}").contains("not a socket"),
            "error names the hazard: {err}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn bind_reclaims_a_dead_socket_file() {
        // A stale socket file from a crashed instance (no live listener) IS reclaimable — bind unlinks +
        // rebinds. Simulate by binding then dropping a std UnixListener (leaves the socket inode, no live
        // accepter), then binding again over it.
        let path = socket_path("dead-socket");
        let _ = std::fs::remove_file(&path);
        {
            let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
            drop(stale); // inode remains; no live listener
        }
        // The socket file still exists (dropping the listener doesn't unlink it).
        assert!(path.exists(), "stale socket inode remains");
        // bind() reclaims it (dead socket → connect refused → unlink + rebind).
        let sock = AdminSocket::bind(&path).expect("a dead socket is reclaimable");
        assert_eq!(sock.path(), path);
        drop(sock);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_stalled_client_does_not_block_other_admins() {
        // The #1962 inline-serve DoS fix: one client that connects and NEVER sends a frame must not block
        // the accept loop — a second client's command still completes (per-connection tokio::spawn keeps
        // accept() live). Even on the current-thread runtime, the stalled connection task's read().await
        // cooperatively yields, so the second connection's task + the host loop still make progress; the
        // pre-fix inline serve_connection().await would have parked the accept loop on the staller.
        let path = socket_path("stalled-dos");
        let _ = std::fs::remove_file(&path);
        let sock = AdminSocket::bind(&path).expect("bind");
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
        let admin = async_host.admin_channel();
        let (host_sd_tx, host_sd_rx) = tokio::sync::oneshot::channel();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel();

        let client = async {
            // Client 1: connect and STALL (hold the connection open, never send a frame).
            let _staller = UnixStream::connect(&path).await.expect("staller connects");
            // Client 2: a normal command — must still complete even though client 1 is stalled.
            let mut live = UnixStream::connect(&path)
                .await
                .expect("second client connects");
            let listed = client_call(&mut live, AdminCommand::ListSessions).await;
            assert_eq!(
                listed,
                AdminResponseWire::Sessions { ids: vec![] },
                "a second admin's command completes despite a stalled first client"
            );
            drop(live);
            // _staller stays open (dropped at scope end) — the point is it didn't block client 2.
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

    #[tokio::test]
    async fn the_full_v0_admin_lifecycle_over_a_real_socket() {
        // The transport-completeness capstone: the sibling socket tests exercise only install + list; this
        // drives the WHOLE v0 command set — install → status → stop → list-confirms-gone, plus a
        // status-of-unknown error — end to end over a REAL UnixStream through the actual serve()/dispatch
        // transport + a live host loop. It proves every AdminCommand round-trips over the real socket (not
        // just in-process apply_admin), the completeness proof for "operate the running daemon over its
        // socket". (A subprocess smoke of the deployed binary itself would need live-net + AWS at boot —
        // aws_config::load_defaults — so it can't run hermetically in CI; this covers the transport layer
        // that CAN, under --features admin, with the real listener + codec + host loop.)
        let path = socket_path("full-lifecycle");
        let _ = std::fs::remove_file(&path);
        let sock = AdminSocket::bind(&path).expect("bind");
        let async_host = AsyncAgentHost::with_factory(AgentHost::new(), Box::new(StubFactory))
            .with_admin_authz(Box::new(AllowList::allow_all_for_local_admin()));
        let admin = async_host.admin_channel();
        let (host_sd_tx, host_sd_rx) = tokio::sync::oneshot::channel();
        let (sock_sd_tx, sock_sd_rx) = tokio::sync::oneshot::channel();

        let client = async {
            let mut stream = UnixStream::connect(&path).await.expect("connect");

            // INSTALL a session.
            let installed = client_call(
                &mut stream,
                AdminCommand::InstallSession(InstallSpec {
                    id: SessionId::new("w1"),
                    reducer_hash: Hash::of(b"w1"),
                    goal: Some("do the thing".into()),
                }),
            )
            .await;
            assert_eq!(installed, AdminResponseWire::Installed { id: "w1".into() });

            // STATUS: the installed session is observable over the socket (a status JSON object comes back).
            let status = client_call(
                &mut stream,
                AdminCommand::SessionStatus {
                    id: SessionId::new("w1"),
                },
            )
            .await;
            match status {
                AdminResponseWire::Status { status } => {
                    assert_eq!(
                        status.get("session_id").and_then(|v| v.as_str()),
                        Some("w1"),
                        "the status snapshot names the session: {status}"
                    );
                }
                other => panic!("expected a Status snapshot over the socket, got {other:?}"),
            }

            // STATUS of an UNKNOWN session → a clean Error frame (not a crash / not a torn-down connection —
            // the connection is still usable for the stop below).
            let unknown = client_call(
                &mut stream,
                AdminCommand::SessionStatus {
                    id: SessionId::new("nope"),
                },
            )
            .await;
            assert!(
                matches!(&unknown, AdminResponseWire::Error { message } if message.contains("nope")),
                "status of an unknown session is a clean error over the socket, got {unknown:?}"
            );

            // STOP the session over the socket.
            let stopped = client_call(
                &mut stream,
                AdminCommand::StopSession {
                    id: SessionId::new("w1"),
                },
            )
            .await;
            assert_eq!(stopped, AdminResponseWire::Stopped { id: "w1".into() });

            // LIST confirms it's gone — the full install→stop lifecycle round-tripped over the transport.
            let listed = client_call(&mut stream, AdminCommand::ListSessions).await;
            assert_eq!(
                listed,
                AdminResponseWire::Sessions { ids: vec![] },
                "after a socket stop, the session is gone from the registry"
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
