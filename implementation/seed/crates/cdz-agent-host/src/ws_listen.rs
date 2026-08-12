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
    if !emit_ws_event(&inbox, ws_connect_inbound(session, conn_id)) {
        // Inbox closed after we registered: deregister so we don't leak the entry, then stop.
        let _ = control_tx.send(WsControlOp::Deregister { conn_id });
        return Ok(());
    }
    // The conn-id as RAW bytes for the pump loop — every inbound frame tags its payload with it. `as_bytes`
    // is free (a pointer into the Copy `Hash`, no alloc), so per-frame tagging is zero-cost — and no hex (the
    // runtime no-hex directive; the reducer echoes these raw bytes as the ws/send target).
    let conn_id_bytes = conn_id.as_bytes();

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
                        if !emit_ws_event(&inbox, ws_frame_inbound(session, conn_id_bytes, &bytes)) {
                            break; // inbox gone
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if !emit_ws_event(
                            &inbox,
                            ws_frame_inbound(session, conn_id_bytes, text.as_bytes()),
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

#[cfg(test)]
mod tests {
    // THE OUTPOST O1b live-ws transport END-TO-END (converted from the deleted ws_listener_e2e integration
    // test, operator no-integration-tests mandate). This whole module is behind `live-ws` (the file is), so
    // it gates cleanly under nix's sandbox (--features live-ws in cdzAgentHostNativeCheck). HERMETIC: a
    // loopback WsListener + an in-process websocket CLIENT in the SAME process, no external peer/network
    // egress. It drives the LISTENER directly (draining its WsControlSender + Inbox), rather than a full
    // reducer loop — the ws/send routing + connect/disconnect folding are covered by the ws_exec + ws_socket
    // unit tests; the NEW thing here is the live tungstenite listener moving opaque frames both ways over a
    // real socket + emitting the right events with the right conn-id.
    use super::*;
    use crate::ws_socket::{ws_control_channel, WS_FRAME_FAMILY};
    use cdz_kernel::effect::{effect_ct, Payload};
    use cdz_kernel::event::EventBody;
    use cdz_kernel::hash::Hash;
    use tokio_tungstenite::tungstenite::Message;

    /// Read the (family, payload-bytes) out of a ws transport `Inbound` body.
    fn family_payload(body: &EventBody) -> (String, Vec<u8>) {
        match body {
            EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } => (content_type.family.to_string(), bytes.to_vec()),
            other => panic!("expected a ws Inbound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn live_ws_transport_round_trips_connect_frame_send_disconnect() {
        // Bind the listener on an ephemeral loopback port. Any SessionId labels the outpost session the
        // transport events are addressed to.
        let listener = WsListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback ws listener");
        let addr = listener.local_addr().expect("listener addr");

        // The loop-side seam: the listener sends WsControlOp over control_tx + Inbound events over inbox; the
        // test plays the host loop by draining both.
        let (inbox_tx, mut inbox_rx) = tokio::sync::mpsc::unbounded_channel();
        let (control_tx, mut control_rx) = ws_control_channel();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let session = SessionId::new(Hash::of(b"outpost-ws-e2e"));
        let serve = tokio::spawn(listener.serve(session, inbox_tx, control_tx, shutdown_rx));

        // A real websocket CLIENT dials the listener over loopback.
        let url = format!("ws://{addr}/");
        let (mut client, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("client connects to the ws listener");

        // 1. ACCEPT: the listener minted a conn-id, Registered its outbound sink, + emitted ws/connect.
        let reg = control_rx.recv().await.expect("a Register control op");
        let (conn_id, sink) = match reg {
            WsControlOp::Register { conn_id, sink } => (conn_id, sink),
            other => panic!("first control op is Register, got {other:?}"),
        };
        let connect_ev = inbox_rx.recv().await.expect("a ws/connect Inbound");
        let (family, payload) = family_payload(&connect_ev.body);
        assert_eq!(family, effect_ct::WS_CONNECT);
        assert_eq!(
            payload,
            conn_id.as_bytes().to_vec(),
            "ws/connect payload is the raw conn-id bytes the reducer echoes as the ws/send target"
        );

        // 2. PEER -> HOST: the client sends a frame; it surfaces as a ws/frame Inbound carrying
        // [len][conn-id-bytes][frame].
        client
            .send(Message::Binary(b"hello-from-peer".to_vec()))
            .await
            .expect("client sends a frame");
        let frame_ev = inbox_rx.recv().await.expect("a ws/frame Inbound");
        let (family, payload) = family_payload(&frame_ev.body);
        assert_eq!(family, WS_FRAME_FAMILY);
        let cid_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        assert_eq!(
            &payload[4..4 + cid_len],
            conn_id.as_bytes(),
            "frame is tagged with the raw conn-id bytes"
        );
        assert_eq!(
            &payload[4 + cid_len..],
            b"hello-from-peer",
            "the opaque peer frame bytes surface verbatim (pure byte transport)"
        );

        // 3. HOST -> PEER: pushing a frame to the registered sink (what the ws/send executor does) reaches
        // the client over the real socket.
        sink.send(bytes::Bytes::from_static(b"echo-to-peer"))
            .expect("push an outbound frame to the connection sink");
        let got = client
            .next()
            .await
            .expect("client receives a frame")
            .expect("frame ok");
        let got_bytes = got.into_data();
        assert_eq!(
            &got_bytes[..],
            b"echo-to-peer",
            "the outbound frame reached the peer verbatim"
        );

        // 4. CLOSE: the client closes; the listener emits ws/disconnect + Deregisters the conn-id.
        client.close(None).await.expect("client closes");
        // The disconnect Inbound + the Deregister control op arrive (order between the two is not guaranteed;
        // drain until we've seen both).
        let mut saw_disconnect = false;
        let mut saw_deregister = false;
        for _ in 0..4 {
            tokio::select! {
                ev = inbox_rx.recv() => {
                    if let Some(ev) = ev {
                        let (family, payload) = family_payload(&ev.body);
                        if family == effect_ct::WS_DISCONNECT {
                            assert_eq!(payload, conn_id.as_bytes().to_vec(), "disconnect names the conn-id");
                            saw_disconnect = true;
                        }
                    }
                }
                op = control_rx.recv() => {
                    if let Some(WsControlOp::Deregister { conn_id: gone }) = op {
                        assert_eq!(gone, conn_id, "deregister names the closed conn-id");
                        saw_deregister = true;
                    }
                }
            }
            if saw_disconnect && saw_deregister {
                break;
            }
        }
        assert!(saw_disconnect, "close emitted a ws/disconnect Inbound");
        assert!(
            saw_deregister,
            "close Deregistered the conn-id from the registry"
        );

        // Tear down the listener.
        let _ = shutdown_tx.send(());
        let _ = serve.await;
    }
}
