//! THE OUTPOST O1b live-ws transport END-TO-END (behind `live-ws`): a real [`WsListener`] bound on loopback,
//! driven by a real in-process websocket CLIENT, exercising the full transport round-trip the O1b slices built:
//! accept -> mint conn-id -> `WsControlOp::Register` + `ws/connect` Inbound -> a peer frame surfaces as a
//! `ws/frame` Inbound -> a frame pushed to the registered sink reaches the client -> client close ->
//! `ws/disconnect` + `WsControlOp::Deregister`.
//!
//! This is HERMETIC (a loopback listener + a client in the SAME test process, no external peer, no network
//! egress) so it gates cleanly under nix's sandbox (`--features live-ws` in cdzAgentHostNativeCheck). It drives
//! the LISTENER directly (draining its `WsControlSender` + `Inbox` in the test) rather than a full reducer loop
//! — the reducer/executor side (ws/send routing, connect/disconnect folding) is covered by the `ws_exec` +
//! `ws_socket` unit tests; the NEW thing here is that the live tungstenite listener actually moves opaque
//! frames both ways over a real socket + emits the right events with the right conn-id.

#![cfg(feature = "live-ws")]

use cdz_agent_host::{SessionId, WsControlOp, WsListener};
use cdz_kernel::effect::{effect_ct, Payload};
use cdz_kernel::event::EventBody;
use futures_util::{SinkExt, StreamExt};
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
    // Bind the listener on an ephemeral loopback port. Any SessionId labels the outpost session the transport
    // events are addressed to.
    let listener = WsListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback ws listener");
    let addr = listener.local_addr().expect("listener addr");

    // The loop-side seam: the listener sends WsControlOp over control_tx + Inbound events over inbox; the test
    // plays the host loop by draining both.
    let (inbox_tx, mut inbox_rx) = tokio::sync::mpsc::unbounded_channel();
    let (control_tx, mut control_rx) = cdz_agent_host::ws_control_channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let session = SessionId::new("outpost-ws-e2e");
    let serve = tokio::spawn(listener.serve(session, inbox_tx, control_tx, shutdown_rx));

    // A real websocket CLIENT dials the listener over loopback.
    let url = format!("ws://{addr}/");
    let (mut client, _resp) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("client connects to the ws listener");

    // 1. ACCEPT: the listener minted a conn-id, Registered its outbound sink, + emitted ws/connect. Drain both.
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
        conn_id.to_hex().into_bytes(),
        "ws/connect payload is the conn-id hex the reducer echoes as the ws/send target"
    );

    // 2. PEER -> HOST: the client sends a frame; it surfaces as a ws/frame Inbound carrying [len][conn-id-hex][frame].
    client
        .send(Message::Binary(b"hello-from-peer".to_vec()))
        .await
        .expect("client sends a frame");
    let frame_ev = inbox_rx.recv().await.expect("a ws/frame Inbound");
    let (family, payload) = family_payload(&frame_ev.body);
    assert_eq!(family, cdz_agent_host::WS_FRAME_FAMILY);
    let cid_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
    let hex = conn_id.to_hex();
    assert_eq!(
        &payload[4..4 + cid_len],
        hex.as_bytes(),
        "frame is tagged with the conn-id hex"
    );
    assert_eq!(
        &payload[4 + cid_len..],
        b"hello-from-peer",
        "the opaque peer frame bytes surface verbatim (pure byte transport)"
    );

    // 3. HOST -> PEER: pushing a frame to the registered sink (what the ws/send executor does) reaches the
    // client over the real socket.
    sink.send(b"echo-to-peer".to_vec())
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
    // The disconnect Inbound + the Deregister control op arrive (order between the two is not guaranteed; drain
    // until we've seen both).
    let mut saw_disconnect = false;
    let mut saw_deregister = false;
    for _ in 0..4 {
        tokio::select! {
            ev = inbox_rx.recv() => {
                if let Some(ev) = ev {
                    let (family, payload) = family_payload(&ev.body);
                    if family == effect_ct::WS_DISCONNECT {
                        assert_eq!(payload, conn_id.to_hex().into_bytes(), "disconnect names the conn-id");
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
