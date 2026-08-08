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
//! The host only: mint a conn-id on accept, surface connect/frames/disconnect as events, and route a
//! `ws/send` frame to the addressed connection.
//!
//! **Connection identity = a [`Hash`], like a session (operator unification, #2820 review).** A ws connection
//! is a stateful host-managed resource, so its identity uses the SAME scheme as reducer-backed sessions: a
//! [`SessionId`] IS its genesis-hash hex, and a conn-id is likewise a `Hash` ([`mint_conn_id`]) whose hex is
//! the opaque routing token the reducer echoes as the `ws/send` target. This unifies host-managed resources
//! with sessions under one id scheme (the capstone: anything stateful looks like any other session/handler).
//! The registry keys by that hex string (opaque to the executor); the IDENTITY is a `Hash`, not an ad-hoc id.

use crate::async_host::{Inbound, Inbox};
use crate::host::SessionId;
use crate::ws_exec::{WsConnRegistry, WsSendResult};
use cdz_kernel::effect::{effect_ct, Payload};
use cdz_kernel::event::{ContentType, EventBody};
use cdz_kernel::hash::Hash;
use std::cell::RefCell;
use std::collections::HashMap;

/// The content-type version stamped on every ws transport `Inbound` (connect / frame / disconnect). v1 — the
/// framing is a single byte version so a later envelope change is an additive version bump the reducer matches.
pub const WS_EVENT_VERSION: u32 = 1;

/// One registry mutation the ws LISTENER asks the host loop to apply. The [`LiveWsConnRegistry`] lives on the
/// single-threaded `!Send` host loop, but a peer connection is accepted + served on its own `Send` tokio task
/// (so a slow/stalled peer can't block `accept`, mirroring [`crate::admin_socket`]'s per-connection tasks). A
/// `Send` task therefore CANNOT touch the `!Send` registry directly — it sends a [`WsControlOp`] over the
/// [`WsControlSender`] and the loop applies it against the registry, exactly as admin commands route through an
/// `AdminChannel` + lifecycle ops through a `LifecycleChannel`. This is the loop-vs-listener seam; keeping it a
/// plain enum + mpsc (not tangled into the accept code) is what lets the routing be unit-tested with no socket.
#[derive(Debug)]
pub enum WsControlOp {
    /// A peer connected: register its conn-id -> outbound sink so a `ws/send` to that conn-id reaches it. The
    /// listener pairs this with emitting a `ws/connect` `Inbound` (via the [`Inbox`]) so the reducer learns of
    /// the peer; the register lands the routing so a subsequent `ws/send` finds the connection.
    Register {
        /// The opaque conn-id ([`mint_conn_id`] hex) the listener minted for this connection.
        conn_id: String,
        /// The sink the connection's writer task drains onto the wire.
        sink: OutboundFrameSink,
    },
    /// A peer's connection closed: deregister its conn-id so a later `ws/send` resolves `Unknown` (peer gone).
    /// Paired with emitting a `ws/disconnect` `Inbound` so the reducer prunes the peer from federation state.
    Deregister {
        /// The conn-id whose connection went away.
        conn_id: String,
    },
}

/// The sending half the ws listener holds to submit [`WsControlOp`]s into the host loop, which applies each
/// against the `!Send` [`LiveWsConnRegistry`] on the loop task. Cloneable (each accepted connection's task
/// clones it). Unbounded so the accept/close path never blocks the loop (a register/deregister is a cheap map
/// op; there is no backpressure need on control mutations).
pub type WsControlSender = tokio::sync::mpsc::UnboundedSender<WsControlOp>;

/// The receiving half the host loop drains, applying each [`WsControlOp`] against the registry via
/// [`LiveWsConnRegistry::apply_control`]. Paired with [`WsControlSender`] by [`ws_control_channel`].
pub type WsControlReceiver = tokio::sync::mpsc::UnboundedReceiver<WsControlOp>;

/// Create the loop <-> listener control channel: the listener holds the [`WsControlSender`], the host loop
/// drains the [`WsControlReceiver`] + applies each op against its registry. Mirrors how the admin + lifecycle
/// channels are constructed for the same Send/!Send split.
pub fn ws_control_channel() -> (WsControlSender, WsControlReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// Mint a fresh CONNECTION IDENTITY for a newly-accepted peer — 32 OS-random bytes hashed into a [`Hash`],
/// exactly as [`crate::host::mint_spawn_nonce`] mints a session's genesis nonce. A ws connection is a
/// stateful host-managed RESOURCE, and the operator's unification (review on #2820) is that such resources
/// share the SAME identity scheme as reducer-backed sessions: a session's [`SessionId`] IS its genesis-hash
/// hex, so a connection's id is likewise a `Hash` (its hex is the routing token the reducer echoes as the
/// `ws/send` target). `Hash::of` over the entropy keeps the id a real content hash in the blake3 domain,
/// uniform with every other `Hash` in the system, NOT an ad-hoc counter. `getrandom` failing is unsurvivable
/// (no entropy = can't mint a unique id), so it is a hard error, not a weak-id fallback — same as the nonce.
pub fn mint_conn_id() -> Hash {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("OS entropy (getrandom) for a ws connection id");
    Hash::of(&bytes)
}

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

    /// Apply one [`WsControlOp`] the listener sent — the loop-side entry point that lets a `Send` accept task
    /// mutate the `!Send` registry indirectly (the loop drains the [`WsControlReceiver`] + calls this). Register
    /// inserts the conn-id -> sink; Deregister removes it. This is the ONLY place the listener's Send tasks
    /// affect the registry (they never hold a `&LiveWsConnRegistry`), keeping the Send/!Send split clean.
    pub fn apply_control(&self, op: WsControlOp) {
        match op {
            WsControlOp::Register { conn_id, sink } => self.register(conn_id, sink),
            WsControlOp::Deregister { conn_id } => self.deregister(&conn_id),
        }
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
    fn mint_conn_id_produces_distinct_hashes_whose_hex_is_the_routing_token() {
        // A conn-id is a Hash (like a session's genesis hash), minted from fresh entropy each accept -> two
        // connections get different ids. Its hex is the token the registry keys by + the reducer echoes.
        let a = mint_conn_id();
        let b = mint_conn_id();
        assert_ne!(
            a, b,
            "each accepted connection gets a fresh unique conn-id Hash"
        );
        // The hex round-trips as a canonical Hash hex (same scheme as SessionId = genesis-hash hex).
        let hex = a.to_hex();
        assert_eq!(
            hex.len(),
            64,
            "a conn-id hex is a 64-char blake3 hash hex, like a session id"
        );
        assert_eq!(
            Hash::from_hex(&hex),
            Some(a),
            "the conn-id hex round-trips to the Hash"
        );
        // The registry + emission take the hex as the conn-id string (identity is the Hash, token is its hex).
        let reg = LiveWsConnRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        reg.register(hex.clone(), tx);
        assert_eq!(reg.send_frame(&hex, b"f"), WsSendResult::Delivered);
    }

    #[test]
    fn control_ops_from_the_listener_apply_against_the_loop_registry() {
        // Simulate the loop <-> listener seam: the listener sends Register/Deregister over the control channel;
        // the loop drains + applies each against the !Send registry. (Here we drive it synchronously — the
        // channel is the Send/!Send bridge, apply_control is the loop-side entry point.)
        let (tx, mut rx) = ws_control_channel();
        let reg = LiveWsConnRegistry::new();
        let (sink, _peer_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

        // Listener (Send task): a peer connected -> send Register with the minted conn-id + its sink.
        let conn = mint_conn_id().to_hex();
        tx.send(WsControlOp::Register {
            conn_id: conn.clone(),
            sink,
        })
        .unwrap();
        // Loop: drain + apply -> the conn is now routable.
        reg.apply_control(rx.try_recv().unwrap());
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.send_frame(&conn, b"hi"), WsSendResult::Delivered);

        // Listener: peer closed -> Deregister; loop applies -> a later send is Unknown.
        tx.send(WsControlOp::Deregister {
            conn_id: conn.clone(),
        })
        .unwrap();
        reg.apply_control(rx.try_recv().unwrap());
        assert!(reg.is_empty());
        assert_eq!(reg.send_frame(&conn, b"x"), WsSendResult::Unknown);
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
