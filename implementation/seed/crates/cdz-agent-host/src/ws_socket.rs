//! THE OUTPOST's websocket transport — the loop-side connection registry + the inbound/lifecycle event
//! emission (O1b-1). This is the always-on, hermetic CORE of the ws transport: it owns the live-connection
//! table the [`WsSendExecutor`](crate::ws_exec::WsSendExecutor) writes through, and it builds the `Inbound`
//! events (`ws/connect` on accept, opaque data frames, `ws/disconnect` on close) the host loop delivers to the
//! outpost session. The actual socket LISTENER (bind a websocket port, accept peers, split each stream into a
//! reader task + an outbound sink) is the `live-ws`-gated slice (O1b-2) that DRIVES this core — kept separate
//! so the routing + framing logic is unit-testable with no tokio-net / no tungstenite, exactly as
//! [`crate::ws_exec`] keeps the `ws/send` executor testable behind the [`WsConnRegistry`] trait.
//!
//! **PURE BYTE TRANSPORT (operator directive: transport-level only).** The host moves OPAQUE message frames:
//! a peer's inbound frame becomes an `Inbound` event carrying the raw bytes + the conn-id; the reducer's
//! `ws/send` writes raw bytes back. NO JSON-RPC / MCP framing here — that is a USERSPACE concern layered over
//! the raw transport (the litmus: JSON-RPC rides ws/stdin/tcp/http, so it is transport-agnostic = userspace).
//!
//! **THIN MECHANISM (host is INEVOLVABLE).** The transport carries no policy: WHO may connect / what a peer's
//! frames mean / which peers a reducer may address is the reducer's fold + the Cedar authorizer, never here.
//! The host only: mint an opaque conn-id on accept, surface connect/frames/disconnect as events, and route a
//! `ws/send` frame to the addressed connection.

use crate::async_host::{Inbound, Inbox};
use crate::host::SessionId;
use crate::ws_exec::{WsConnRegistry, WsSendResult};
use cdz_kernel::effect::{effect_ct, Payload};
use cdz_kernel::event::{ContentType, EventBody};
use std::cell::RefCell;
use std::collections::HashMap;

/// The content-type version stamped on every ws transport `Inbound` (connect / frame / disconnect). v1 — the
/// framing is a single byte version so a later envelope change is an additive version bump the reducer matches.
pub const WS_EVENT_VERSION: u32 = 1;

/// The outbound-frame sink for one live connection: an unbounded sender the listener's per-connection writer
/// task drains onto the real websocket sink. The registry holds one per conn-id; `ws/send` pushes a frame
/// here and the writer task moves it to the wire. Unbounded so a `ws/send` on the `!Send` host loop never
/// blocks on a slow peer (backpressure/slow-peer policy is a later refinement, not a v0 host concern).
pub type OutboundFrameSink = tokio::sync::mpsc::UnboundedSender<Vec<u8>>;

/// The loop-side live-connection registry: conn-id -> that connection's outbound sink. Populated by the
/// listener (register on accept, deregister on close) and read by the [`WsSendExecutor`] via the
/// [`WsConnRegistry`] impl below. Lives on the single-threaded host loop (`RefCell`, not a `Mutex` — the
/// executor + the register/deregister calls are all on the one loop thread; the per-connection READER/WRITER
/// tasks talk to it only through the mpsc channels, never touching the map directly). Mirrors the
/// `MemWsConnRegistry` test shape, but over real outbound sinks.
#[derive(Default)]
pub struct LiveWsConnRegistry {
    conns: RefCell<HashMap<String, OutboundFrameSink>>,
}

impl LiveWsConnRegistry {
    /// A fresh empty registry (no connections). The listener registers connections as peers accept.
    pub fn new() -> Self {
        LiveWsConnRegistry {
            conns: RefCell::new(HashMap::new()),
        }
    }

    /// Register a newly-accepted connection under its opaque conn-id, with the sink its writer task drains.
    /// Called by the listener right after it mints the conn-id + spawns the connection's read/write tasks
    /// (paired with the `ws/connect` event emission). A duplicate conn-id would overwrite — the listener
    /// mints unique ids, so this is insert-new in practice.
    pub fn register(&self, conn_id: String, sink: OutboundFrameSink) {
        self.conns.borrow_mut().insert(conn_id, sink);
    }

    /// Deregister a closed connection (paired with the `ws/disconnect` event emission). After this, a
    /// `ws/send` to that conn-id resolves [`WsSendResult::Unknown`] (peer gone) — the reducer prunes on the
    /// disconnect event. Idempotent: deregistering an absent conn-id is a no-op.
    pub fn deregister(&self, conn_id: &str) {
        self.conns.borrow_mut().remove(conn_id);
    }

    /// The count of live connections (for status/metrics + tests). Not a policy input — just an observable.
    pub fn len(&self) -> usize {
        self.conns.borrow().len()
    }

    /// Whether there are no live connections (clippy's `len`-companion; same observable).
    pub fn is_empty(&self) -> bool {
        self.conns.borrow().is_empty()
    }
}

impl WsConnRegistry for LiveWsConnRegistry {
    fn send_frame(&self, conn_id: &str, frame: &[u8]) -> WsSendResult {
        match self.conns.borrow().get(conn_id) {
            // The connection is live: hand the frame to its writer task's sink. A closed channel (the writer
            // task ended — the peer dropped mid-send before deregister ran) is a transient write failure: the
            // disconnect event is in flight, so a retry either lands (race resolves) or resolves Unknown.
            Some(sink) => match sink.send(frame.to_vec()) {
                Ok(()) => WsSendResult::Delivered,
                Err(_) => WsSendResult::WriteFailed("connection writer task closed".into()),
            },
            // No live connection under this conn-id (never opened, or already closed + deregistered).
            None => WsSendResult::Unknown,
        }
    }
}

/// Build the `ws/connect` [`Inbound`] the host emits when a peer connects: the reducer LEARNS a new peer
/// exists + can address `ws/send` to `conn_id`. Payload = the opaque conn-id bytes (the reducer echoes it as
/// the `ws/send` target while the peer is up). Addressed to the outpost `session`.
pub fn ws_connect_inbound(session: SessionId, conn_id: &str) -> Inbound {
    ws_event_inbound(session, effect_ct::WS_CONNECT, conn_id.as_bytes().to_vec())
}

/// Build the `ws/disconnect` [`Inbound`] the host emits when a peer's connection closes: the reducer prunes
/// the peer from its federation state. Payload = the conn-id bytes (which connection went away).
pub fn ws_disconnect_inbound(session: SessionId, conn_id: &str) -> Inbound {
    ws_event_inbound(
        session,
        effect_ct::WS_DISCONNECT,
        conn_id.as_bytes().to_vec(),
    )
}

/// Build the inbound DATA-FRAME [`Inbound`] the host emits for each opaque message a peer sends. The framing
/// carries the conn-id (so the reducer knows WHICH peer) followed by the raw frame bytes, length-prefixed:
/// `[conn_id_len: u32-le][conn_id bytes][frame bytes]`. The host does NOT interpret the frame — it's opaque
/// application bytes (JSON-RPC/whatever is a userspace concern). Family is a distinct `ws/frame` content-type
/// so the reducer matches data frames apart from the connect/disconnect lifecycle.
pub fn ws_frame_inbound(session: SessionId, conn_id: &str, frame: &[u8]) -> Inbound {
    let cid = conn_id.as_bytes();
    let mut payload = Vec::with_capacity(4 + cid.len() + frame.len());
    payload.extend_from_slice(&(cid.len() as u32).to_le_bytes());
    payload.extend_from_slice(cid);
    payload.extend_from_slice(frame);
    ws_event_inbound(session, WS_FRAME_FAMILY, payload)
}

/// The content-type family for an inbound peer DATA frame (distinct from the `ws/connect`/`ws/disconnect`
/// lifecycle families). Host-owned (a transport framing detail), not a kernel effect const: it's an INBOUND
/// event family the reducer matches, never a dispatched effect.
pub const WS_FRAME_FAMILY: &str = "ws/frame";

/// Shared builder: an `Inbound` addressed to `session` carrying `payload` under `family` at [`WS_EVENT_VERSION`].
/// `reply_to = None` — a ws transport event is external ingress (the peer is not a session that can be bounced
/// to); if the outpost session is gone the loop drops it (an external inbound never bounces, per the loop's
/// `reply_to`-None path).
fn ws_event_inbound(session: SessionId, family: &'static str, payload: Vec<u8>) -> Inbound {
    Inbound {
        session,
        body: EventBody::Inbound {
            content_type: ContentType {
                family: family.into(),
                version: WS_EVENT_VERSION,
            },
            payload: Payload::Inline(payload.into()),
        },
        cause: None,
        reply_to: None,
    }
}

/// Emit a ws transport event into the host loop's [`Inbox`]. A closed inbox (the loop is shutting down) drops
/// the event — the transport is being torn down anyway. Returns whether it was enqueued (for the listener to
/// decide whether to keep the connection alive).
pub fn emit_ws_event(inbox: &Inbox, event: Inbound) -> bool {
    inbox.send(event).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain_family(body: &EventBody) -> (String, Vec<u8>) {
        match body {
            EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } => (content_type.family.to_string(), bytes.to_vec()),
            other => panic!("expected an Inbound, got {other:?}"),
        }
    }

    #[test]
    fn register_then_send_delivers_and_deregister_makes_it_unknown() {
        let reg = LiveWsConnRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        reg.register("conn-1".into(), tx);
        assert_eq!(reg.len(), 1);

        // A send to the live conn is delivered + the exact frame reaches the sink.
        assert_eq!(reg.send_frame("conn-1", b"hello"), WsSendResult::Delivered);
        assert_eq!(rx.try_recv().unwrap(), b"hello".to_vec());

        // After deregister the conn is gone -> Unknown (the reducer prunes on the disconnect event).
        reg.deregister("conn-1");
        assert!(reg.is_empty());
        assert_eq!(reg.send_frame("conn-1", b"x"), WsSendResult::Unknown);
    }

    #[test]
    fn send_to_never_registered_conn_is_unknown() {
        let reg = LiveWsConnRegistry::new();
        assert_eq!(reg.send_frame("nope", b"x"), WsSendResult::Unknown);
    }

    #[test]
    fn a_closed_writer_sink_is_a_transient_write_failure() {
        let reg = LiveWsConnRegistry::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        reg.register("conn-flaky".into(), tx);
        // The writer task ended (peer dropped mid-send before deregister): the receiver is gone, so the
        // channel send errors -> a transient write failure (the disconnect event resolves the race).
        drop(rx);
        assert!(matches!(
            reg.send_frame("conn-flaky", b"x"),
            WsSendResult::WriteFailed(_)
        ));
    }

    #[test]
    fn connect_inbound_carries_the_conn_id_under_ws_connect() {
        let ev = ws_connect_inbound(SessionId::new("outpost"), "conn-42");
        assert_eq!(ev.session, SessionId::new("outpost"));
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, effect_ct::WS_CONNECT);
        assert_eq!(
            payload,
            b"conn-42".to_vec(),
            "connect payload is the conn-id"
        );
        assert!(
            ev.reply_to.is_none(),
            "a ws transport event is external ingress, no bounce"
        );
    }

    #[test]
    fn disconnect_inbound_carries_the_conn_id_under_ws_disconnect() {
        let ev = ws_disconnect_inbound(SessionId::new("outpost"), "conn-42");
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, effect_ct::WS_DISCONNECT);
        assert_eq!(payload, b"conn-42".to_vec());
    }

    #[test]
    fn frame_inbound_length_prefixes_the_conn_id_then_the_opaque_frame() {
        let ev = ws_frame_inbound(SessionId::new("outpost"), "cid", b"opaque-bytes");
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, WS_FRAME_FAMILY);
        // [len:u32-le][conn-id][frame]
        let cid_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        assert_eq!(cid_len, 3);
        assert_eq!(&payload[4..4 + cid_len], b"cid");
        assert_eq!(&payload[4 + cid_len..], b"opaque-bytes");
    }

    #[test]
    fn emit_ws_event_enqueues_then_reports_false_on_a_closed_inbox() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Inbound>();
        assert!(emit_ws_event(
            &tx,
            ws_connect_inbound(SessionId::new("outpost"), "c1")
        ));
        drop(rx);
        assert!(
            !emit_ws_event(&tx, ws_disconnect_inbound(SessionId::new("outpost"), "c1")),
            "a closed inbox (loop shutting down) reports the event was dropped"
        );
    }
}
