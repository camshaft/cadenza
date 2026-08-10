//! THE OUTPOST's live websocket CLIENT dial-out (federation transport, behind `live-ws`) — the OS-touching
//! outer slice that CONNECTS OUT to a hub, the symmetric counterpart to [`ws_listen`](crate::ws_listen)'s
//! inbound listener. Where the listener ACCEPTS peer connections, this DIALS a configured hub URL and drives
//! the SAME always-on [`ws_socket`](crate::ws_socket) loop-core: it registers the connection's outbound sink
//! with the host loop (via a [`WsControlOp`]), surfaces the hub's frames + its connect/disconnect lifecycle as
//! `Inbound` events, and drains the reducer's outbound frames onto the wire.
//!
//! **This is the FEDERATION transport primitive** (operator directive: "the harness should be able to specify
//! a hub and connect to it and start federating"). The host is PLUMBING ONLY — it moves opaque frames both
//! ways and makes NO federation decision. ALL federation POLICY (the handshake, what federates —
//! sessions/messages/state, the hub topology) lives in the outpost reducer's fold, authored against the same
//! seam the listener uses: the reducer reads `ws/frame` `Inbound`s from the hub and emits `ws/send` frames +
//! `emit`/`lifecycle`/`store` effects. The wire payload is BINARY (operator: binary ASTs everywhere) — the
//! transport is byte-opaque, so the reducer owns the codec.
//!
//! **PURE BYTE TRANSPORT.** A hub's ws message becomes an `Inbound` carrying its opaque bytes; a reducer's
//! `ws/send` frame is written back verbatim. NO JSON-RPC / MCP / federation framing here — that is a userspace
//! concern layered over the raw transport, exactly as [`crate::ws_listen`] documents for the inbound side.
//!
//! **Send/!Send split (mirrors [`crate::ws_listen`]).** The dial + frame-pump runs on a `Send` task (it moves
//! a ws stream + cloned channel handles, never the `!Send` registry), routing register/deregister through the
//! [`WsControlSender`] the host loop drains and pushing inbound events through the shared [`Inbox`]. This is
//! the same seam [`crate::ws_socket`] defined — the dialer and the listener are two producers into one loop.
//!
//! **`live-ws`-gated.** The default build dials nothing + pulls no tokio-net / tungstenite tree; only this
//! module is opt-in. The routing + framing it drives is unit-tested hermetically in [`crate::ws_socket`]; this
//! module is the thin OS wiring, exercised by a `live-ws` loopback E2E (dial an in-process [`WsListener`], so
//! it is hermetic — no external hub/network).

use crate::async_host::Inbox;
use crate::host::SessionId;
use crate::ws_socket::{
    emit_ws_event, mint_conn_id, ws_connect_inbound, ws_disconnect_inbound, ws_frame_inbound,
    WsControlOp, WsControlSender,
};
use futures_util::{SinkExt, StreamExt};
use std::io;
use tokio_tungstenite::tungstenite::Message;

/// Dial `hub_url` and serve the connection as a federated ws session until the hub closes or `shutdown` fires.
/// The symmetric counterpart to [`WsListener::serve`](crate::ws_listen::WsListener::serve): where the listener
/// accepts and serves inbound peers, this connects OUT to one hub and serves that single connection. `session`
/// is the outpost session the hub's inbound events are addressed to; `control_tx` routes register/deregister
/// to the host loop; `inbox` carries the connect/frame/disconnect events.
///
/// Returns `Err` if the initial dial/handshake fails (bad URL, hub unreachable, non-ws endpoint) — a caller
/// that wants ret/reconnect drives that at a higher layer (v0 is a single connect; reconnect policy is a
/// federation-protocol decision, coordinated with `design-hub-federation`, not baked into this transport).
/// Once connected, a mid-stream failure (hub hung up, read error) ends the connection cleanly with a
/// `ws/disconnect` announced — NOT an `Err` (the connection lived, then ended, exactly like the listener's
/// per-connection path).
pub async fn dial_hub(
    hub_url: &str,
    session: SessionId,
    inbox: Inbox,
    control_tx: WsControlSender,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> io::Result<()> {
    // Connect + ws handshake. A failed dial (unreachable hub / non-ws endpoint / bad URL) is the one hard
    // error this returns — the connection never came up, so there's nothing to announce or deregister.
    let (ws, _resp) = tokio_tungstenite::connect_async(hub_url)
        .await
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("ws dial {hub_url}: {e}"),
            )
        })?;
    serve_hub_connection(ws, session, inbox, control_tx, shutdown).await
}

/// Drive one established hub connection: mint the conn-id, register + announce it, then pump frames both ways
/// until the hub closes, a pump end fails, or `shutdown` fires. Split out from [`dial_hub`] so the connect
/// error (which the caller may want to retry) is distinct from the connection lifecycle (which always ends
/// with a clean `ws/disconnect`). Generic over the ws stream so a test can drive a connected pair without a
/// real dial.
async fn serve_hub_connection<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    session: SessionId,
    inbox: Inbox,
    control_tx: WsControlSender,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut write, mut read) = ws.split();

    // Mint the conn-id (a Hash) + the per-connection outbound sink; register the sink with the host loop +
    // announce the hub via ws/connect BEFORE reading frames, so a reducer's ws/send can address it and the
    // frames that follow are attributable. If the loop's control/inbox channel is already closed (host
    // shutting down), abandon the connection. Identical to the listener's per-connection setup — the outpost
    // reducer sees a federated hub as just another ws conn-id.
    let conn_id = mint_conn_id();
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

    // Pump both directions until the hub closes, an end fails, or shutdown fires. The writer half drains the
    // reducer's outbound frames (out_rx, fed by ws/send via the registry sink) onto the wire; the reader half
    // turns each inbound hub message into a ws/frame Inbound. select! ends the connection when ANY arm
    // finishes. Byte-identical to the listener's pump (crate::ws_listen::serve_connection) — same seam, other
    // direction.
    loop {
        tokio::select! {
            // A shutdown signal (host stopping / caller closing the federation link): end cleanly.
            _ = &mut shutdown => break,
            // Reducer -> hub: an outbound frame the ws/send executor pushed to this conn's sink.
            outbound = out_rx.recv() => {
                match outbound {
                    Some(bytes) => {
                        // tungstenite 0.24's `Message::Binary` takes `Vec<u8>`, so the ref-counted frame is
                        // materialized to a Vec here — the one remaining copy, at the wire edge (matches the
                        // listener; eliminated end-to-end by a future tungstenite >=0.26 bump, v-nix-coordinated).
                        if write.send(Message::Binary(bytes.into())).await.is_err() {
                            break; // hub write failed — connection is going away
                        }
                    }
                    None => break, // the sink was dropped (deregistered) — stop writing
                }
            }
            // Hub -> reducer: an inbound ws message. Binary/Text carry opaque application bytes -> a ws/frame
            // Inbound. A Close ends the connection. Ping/Pong are handled by tungstenite; ignore here.
            inbound = read.next() => {
                match inbound {
                    Some(Ok(Message::Binary(bytes))) => {
                        if !emit_ws_event(&inbox, ws_frame_inbound(session, conn_id, &bytes)) {
                            break; // inbox gone
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        if !emit_ws_event(&inbox, ws_frame_inbound(session, conn_id, text.as_bytes())) {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => break,
                    // Ping/Pong/Frame: transport-level, no application payload to surface.
                    Some(Ok(_)) => {}
                    // A read error (hub reset mid-frame) or clean end of stream: the connection is done.
                    Some(Err(_)) | None => break,
                }
            }
        }
    }

    // The connection closed (any reason). Announce ws/disconnect so the reducer prunes the hub peer, and
    // deregister the sink so a later ws/send to this conn-id resolves Unknown. Both best-effort: if the loop's
    // channels are already gone, there's nothing to prune.
    let _ = emit_ws_event(&inbox, ws_disconnect_inbound(session, conn_id));
    let _ = control_tx.send(WsControlOp::Deregister { conn_id });
    Ok(())
}

#[cfg(test)]
mod tests {
    // THE OUTPOST federation CLIENT dial-out END-TO-END (live-ws). HERMETIC: an in-process WsListener plays
    // the HUB and our dial_hub connects OUT to it over loopback, in the SAME process — no external hub/network
    // egress. It drives the DIALER directly (draining its WsControlSender + Inbox as the host loop would),
    // asserting the NEW thing: the client dial + frame-pump moves opaque frames both ways over a real socket +
    // emits the right connect/frame/disconnect events with the right conn-id. The symmetric twin of
    // ws_listen's live_ws_transport_round_trips_connect_frame_send_disconnect.
    use super::*;
    use crate::ws_socket::{ws_control_channel, WS_FRAME_FAMILY};
    use cdz_kernel::effect::{effect_ct, Payload};
    use cdz_kernel::event::EventBody;
    use cdz_kernel::hash::Hash;

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
    async fn dial_hub_round_trips_connect_frame_send_disconnect() {
        use crate::ws_listen::WsListener;

        // The HUB: an in-process WsListener on an ephemeral loopback port. We drain its side to act as the hub
        // — accept the dialer's connection, receive its frame, echo one back, then close.
        let hub = WsListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback hub listener");
        let hub_addr = hub.local_addr().expect("hub addr");
        let (hub_inbox_tx, mut hub_inbox_rx) = tokio::sync::mpsc::unbounded_channel();
        let (hub_control_tx, mut hub_control_rx) = ws_control_channel();
        let (hub_shutdown_tx, hub_shutdown_rx) = tokio::sync::oneshot::channel();
        let hub_session = SessionId::new(Hash::of(b"hub-e2e"));
        let hub_serve =
            tokio::spawn(hub.serve(hub_session, hub_inbox_tx, hub_control_tx, hub_shutdown_rx));

        // The OUTPOST DIALER: connect OUT to the hub. We play the outpost host loop by draining the dialer's
        // control_tx + inbox. dial_hub runs on its own task (it blocks pumping the connection).
        let (out_inbox_tx, mut out_inbox_rx) = tokio::sync::mpsc::unbounded_channel();
        let (out_control_tx, mut out_control_rx) = ws_control_channel();
        let (dial_shutdown_tx, dial_shutdown_rx) = tokio::sync::oneshot::channel();
        let outpost_session = SessionId::new(Hash::of(b"outpost-dialer-e2e"));
        let url = format!("ws://{hub_addr}/");
        let dial = tokio::spawn(async move {
            dial_hub(
                &url,
                outpost_session,
                out_inbox_tx,
                out_control_tx,
                dial_shutdown_rx,
            )
            .await
        });

        // 1. CONNECT: the dialer minted a conn-id, Registered its outbound sink, + emitted ws/connect (the hub
        // is just another ws conn-id to the outpost reducer).
        let reg = out_control_rx.recv().await.expect("a Register control op");
        let (conn_id, out_sink) = match reg {
            WsControlOp::Register { conn_id, sink } => (conn_id, sink),
            other => panic!("first control op is Register, got {other:?}"),
        };
        let connect_ev = out_inbox_rx.recv().await.expect("a ws/connect Inbound");
        let (family, payload) = family_payload(&connect_ev.body);
        assert_eq!(family, effect_ct::WS_CONNECT);
        assert_eq!(
            payload,
            conn_id.to_hex().into_bytes(),
            "ws/connect payload is the conn-id hex the reducer echoes as the ws/send target"
        );

        // The hub's side accepted the dialer: drain its Register + ws/connect so we can echo back below.
        let hub_reg = hub_control_rx.recv().await.expect("hub Register");
        let hub_sink = match hub_reg {
            WsControlOp::Register { sink, .. } => sink,
            other => panic!("hub first control op is Register, got {other:?}"),
        };
        let _ = hub_inbox_rx.recv().await.expect("hub ws/connect");

        // 2. OUTPOST -> HUB: pushing a frame to the dialer's registered sink (what ws/send does) reaches the
        // hub over the real socket, surfacing as a ws/frame Inbound on the hub side.
        out_sink
            .send(bytes::Bytes::from_static(b"hello-hub"))
            .expect("push an outbound frame to the dialer sink");
        let hub_frame = hub_inbox_rx.recv().await.expect("hub receives a ws/frame");
        let (hub_family, hub_payload) = family_payload(&hub_frame.body);
        assert_eq!(hub_family, WS_FRAME_FAMILY);
        assert!(
            hub_payload.ends_with(b"hello-hub"),
            "the outpost's frame reached the hub verbatim (pure byte transport)"
        );

        // 3. HUB -> OUTPOST: the hub pushes a frame to ITS sink; it reaches the dialer + surfaces as a
        // ws/frame Inbound carrying [len][conn-id-hex][frame] on the OUTPOST side.
        hub_sink
            .send(bytes::Bytes::from_static(b"welcome-outpost"))
            .expect("hub pushes an outbound frame");
        let frame_ev = out_inbox_rx
            .recv()
            .await
            .expect("a ws/frame Inbound on the outpost");
        let (family, payload) = family_payload(&frame_ev.body);
        assert_eq!(family, WS_FRAME_FAMILY);
        let cid_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        assert_eq!(
            &payload[4..4 + cid_len],
            conn_id.to_hex().as_bytes(),
            "the hub frame is tagged with the dialer's conn-id hex"
        );
        assert_eq!(
            &payload[4 + cid_len..],
            b"welcome-outpost",
            "the opaque hub frame bytes surface verbatim on the outpost"
        );

        // 4. CLOSE: shutting down the dialer ends the connection cleanly — ws/disconnect + Deregister on the
        // outpost side (order between the two is not guaranteed; drain until both seen).
        let _ = dial_shutdown_tx.send(());
        let mut saw_disconnect = false;
        let mut saw_deregister = false;
        for _ in 0..4 {
            tokio::select! {
                ev = out_inbox_rx.recv() => {
                    if let Some(ev) = ev {
                        let (family, payload) = family_payload(&ev.body);
                        if family == effect_ct::WS_DISCONNECT {
                            assert_eq!(payload, conn_id.to_hex().into_bytes(), "disconnect names the conn-id");
                            saw_disconnect = true;
                        }
                    }
                }
                op = out_control_rx.recv() => {
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
        assert!(
            saw_disconnect,
            "dialer shutdown emitted a ws/disconnect Inbound"
        );
        assert!(saw_deregister, "dialer shutdown Deregistered the conn-id");

        // The dialer task returns Ok once the connection ends.
        assert!(
            matches!(dial.await, Ok(Ok(()))),
            "dial_hub returns cleanly after the connection closes"
        );

        // Tear down the hub.
        let _ = hub_shutdown_tx.send(());
        let _ = hub_serve.await;
    }

    #[tokio::test]
    async fn dial_hub_errors_on_an_unreachable_hub() {
        // A dial to a port nothing is listening on is a hard Err (the connection never came up) — never a
        // panic, never a silent hang. The caller (a future reconnect layer) decides what to do with the Err.
        let (inbox_tx, _inbox_rx) = tokio::sync::mpsc::unbounded_channel();
        let (control_tx, _control_rx) = ws_control_channel();
        let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        // 127.0.0.1:1 — port 1 is privileged + unused, so the connect refuses fast.
        let out = dial_hub(
            "ws://127.0.0.1:1/",
            SessionId::new(Hash::of(b"outpost-noconnect")),
            inbox_tx,
            control_tx,
            shutdown_rx,
        )
        .await;
        assert!(
            out.is_err(),
            "dialing an unreachable hub is an Err, got {out:?}"
        );
    }
}
