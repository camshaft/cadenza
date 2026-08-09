//! THE OUTPOST's live websocket LISTENER (O1b-2, behind `live-ws`) — the OS-touching outer slice that DRIVES
//! the always-on [`ws_socket`](crate::ws_socket) loop-core. It binds a TCP port, accepts peer websocket
//! connections, and for each one: mints a conn-id [`Hash`], registers the connection's outbound sink with the
//! host loop (via a [`WsControlOp`]), surfaces the peer's frames + its connect/disconnect lifecycle as
//! `Inbound` events, and drains the reducer's outbound frames onto the wire.
//!
//! **Send/!Send split (mirrors [`crate::admin_socket`]).** Each accepted connection is served on its own
//! `tokio::spawn`ed task so a slow/stalled peer can't block `accept`. Those tasks are `Send` (they move a TCP
//! stream + cloned channel handles, never the `!Send` registry), so they can't touch [`LiveWsConnRegistry`]
//! directly — they route register/deregister through the [`WsControlSender`] the host loop drains, and push
//! inbound events through the shared [`Inbox`]. This is exactly the seam [`crate::ws_socket`] defined.
//!
//! **PURE BYTE TRANSPORT (operator: transport-level only).** A peer's ws message becomes an `Inbound` carrying
//! its opaque bytes; a reducer's `ws/send` frame is written back verbatim. NO JSON-RPC / MCP framing here —
//! that is a userspace concern layered over the raw transport.
//!
//! **`live-ws`-gated.** The default build binds no socket + pulls no tokio-net / tungstenite tree; only this
//! module is opt-in. The routing + framing it drives is unit-tested hermetically in [`crate::ws_socket`]; this
//! module is the thin OS wiring, exercised by the `live-ws` integration E2E (a loopback listener + in-process
//! client, so it is hermetic enough to gate under nix — no external peer/network).

use crate::async_host::Inbox;
use crate::host::SessionId;
use crate::ws_socket::{
    emit_ws_event, mint_conn_id, ws_connect_inbound, ws_disconnect_inbound, ws_frame_inbound,
    WsControlOp, WsControlSender,
};
use futures_util::{SinkExt, StreamExt};
use std::io;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;

/// A bound websocket listener. Construct with [`WsListener::bind`], then [`serve`](WsListener::serve) it — the
/// accept loop that turns each peer connection into a registered, event-surfacing ws session.
pub struct WsListener {
    listener: TcpListener,
}

impl WsListener {
    /// Bind the websocket listener at `addr` (e.g. `127.0.0.1:0` for an ephemeral loopback port in tests, or a
    /// configured host:port for the daemon). `Err` if the bind fails (port in use / bad address).
    pub async fn bind(addr: &str) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(WsListener { listener })
    }

    /// The bound local address (useful for tests that bind `:0` + need the assigned port).
    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.listener.local_addr()
    }

    /// Accept peer websocket connections forever, until `shutdown` fires. Each accepted connection is served on
    /// its own detached `Send` task ([`serve_connection`]) — a slow peer can't block `accept`. `session` is the
    /// outpost session the inbound events are addressed to; `control_tx` routes register/deregister to the host
    /// loop; `inbox` carries the connect/frame/disconnect events. An accept error is logged + the loop
    /// continues (one bad connection never takes the listener down).
    pub async fn serve(
        self,
        session: SessionId,
        inbox: Inbox,
        control_tx: WsControlSender,
        mut shutdown: tokio::sync::oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                accepted = self.listener.accept() => {
                    match accepted {
                        Ok((stream, _peer_addr)) => {
                            let session = session.clone();
                            let inbox = inbox.clone();
                            let control_tx = control_tx.clone();
                            tokio::spawn(async move {
                                if let Err(e) =
                                    serve_connection(stream, session, inbox, control_tx).await
                                {
                                    // A per-connection error (handshake failed, peer hung up mid-frame) ends
                                    // only THIS connection. NEVER log the peer's frame bytes (untrusted /
                                    // guest-controlled — the never-log-guest-strings seam); the io error is ours.
                                    eprintln!("cdz-agent-daemon ws: connection ended: {e}");
                                }
                            });
                        }
                        Err(e) => eprintln!("cdz-agent-daemon ws: accept failed: {e}"),
                    }
                }
            }
        }
    }
}

/// Serve one accepted TCP connection as a websocket peer session: handshake, mint the conn-id, register +
/// announce it, then pump frames both ways until the peer closes. Runs on its own `Send` task.
async fn serve_connection(
    stream: TcpStream,
    session: SessionId,
    inbox: Inbox,
    control_tx: WsControlSender,
) -> io::Result<()> {
    // Websocket handshake. A failed handshake (a non-ws client) ends this connection with no registration +
    // no events emitted (the peer never became a live conn).
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("ws handshake: {e}")))?;
    let (mut write, mut read) = ws.split();

    // Mint the conn-id (a Hash) + the per-connection outbound sink; register the sink with the host loop +
    // announce the peer via ws/connect BEFORE reading frames, so a reducer's ws/send can address it and the
    // frames that follow are attributable. If the loop's control/inbox channel is already closed (host
    // shutting down), abandon the connection.
    let conn_id = mint_conn_id();
    // The outbound sink carries ref-counted `Bytes` (matches `OutboundFrameSink`): the ws/send executor moves
    // the already-ref-counted `Payload::Inline` frame in with no memcpy on the host loop.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<bytes::Bytes>();
    if control_tx
        .send(WsControlOp::Register {
            conn_id,
            sink: out_tx,
        })
        .is_err()
    {
        return Ok(()); // loop gone
    }
    if !emit_ws_event(&inbox, ws_connect_inbound(session.clone(), conn_id)) {
        // Inbox closed after we registered: deregister so we don't leak the entry, then stop.
        let _ = control_tx.send(WsControlOp::Deregister { conn_id });
        return Ok(());
    }

    // Pump both directions until the peer closes or an end fails. The writer half drains the reducer's
    // outbound frames (out_rx, fed by ws/send via the registry sink) onto the wire; the reader half turns each
    // inbound peer message into a ws/frame Inbound. select! ends the connection when EITHER half finishes.
    loop {
        tokio::select! {
            // Reducer -> peer: an outbound frame the ws/send executor pushed to this conn's sink.
            outbound = out_rx.recv() => {
                match outbound {
                    Some(bytes) => {
                        // tungstenite 0.24's `Message::Binary` takes `Vec<u8>`, so the ref-counted frame is
                        // materialized to a Vec here — the one remaining copy, at the wire edge (was on the
                        // host loop before). Eliminated end-to-end by bumping tungstenite to >=0.26
                        // (`Message::Binary(Bytes)`); that bump touches the nix flake vendor, so it's a
                        // separate v-nix-coordinated change. See the ws-frame-bytes follow-up.
                        if write.send(Message::Binary(bytes.into())).await.is_err() {
                            break; // peer write failed — connection is going away
                        }
                    }
                    None => break, // the sink was dropped (deregistered) — stop writing
                }
            }
            // Peer -> reducer: an inbound ws message. Binary/Text carry opaque application bytes -> a ws/frame
            // Inbound. A Close ends the connection. Ping/Pong are handled by tungstenite; ignore here.
            inbound = read.next() => {
                match inbound {
                    Some(Ok(Message::Binary(bytes))) => {
                        if !emit_ws_event(&inbox, ws_frame_inbound(session.clone(), conn_id, &bytes)) {
                            break; // inbox gone
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if !emit_ws_event(
                            &inbox,
                            ws_frame_inbound(session.clone(), conn_id, text.as_bytes()),
                        ) {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    // Ping/Pong/Frame: transport-level, no application payload to surface.
                    Some(Ok(_)) => {}
                    // A read error (peer reset mid-frame) or clean end of stream: the connection is done.
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    // The connection closed (either direction). Announce ws/disconnect so the reducer prunes the peer, and
    // deregister the sink so a later ws/send to this conn-id resolves Unknown. Both best-effort: if the loop's
    // channels are already gone, there's nothing to prune.
    let _ = emit_ws_event(&inbox, ws_disconnect_inbound(session, conn_id));
    let _ = control_tx.send(WsControlOp::Deregister { conn_id });
    Ok(())
}
