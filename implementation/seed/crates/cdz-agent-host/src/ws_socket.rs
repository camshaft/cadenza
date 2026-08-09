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
//! The identity AND the registry key are the `Hash` itself (a `Copy` [u8;32], cheaply clonable — the operator's
//! cheaply-clonable-everywhere rule, NOT an owned String); the hex form appears ONLY at the kernel effect-target
//! boundary (`req.target` is `Arc<str>`), where the `ws/send` executor parses it back to a `Hash` to look up.

use crate::async_host::{Inbound, Inbox};
use crate::host::SessionId;
use crate::ws_exec::{WsConnRegistry, WsSendResult};
use bytes::Bytes;
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
/// `AdminChannel` + lifecycle ops through a `LifecycleChannel`. The conn-id is a [`Hash`] (a `Copy` [u8;32],
/// cheaply clonable — NOT an owned String, per the operator's cheaply-clonable-everywhere rule).
#[derive(Debug)]
pub enum WsControlOp {
    /// A peer connected: register its conn-id -> outbound sink so a `ws/send` to that conn-id reaches it. The
    /// listener pairs this with emitting a `ws/connect` `Inbound` (via the [`Inbox`]) so the reducer learns of
    /// the peer; the register lands the routing so a subsequent `ws/send` finds the connection.
    Register {
        /// The conn-id [`Hash`] ([`mint_conn_id`]) the listener minted for this connection.
        conn_id: Hash,
        /// The sink the connection's writer task drains onto the wire.
        sink: OutboundFrameSink,
    },
    /// A peer's connection closed: deregister its conn-id so a later `ws/send` resolves `Unknown` (peer gone).
    /// Paired with emitting a `ws/disconnect` `Inbound` so the reducer prunes the peer from federation state.
    Deregister {
        /// The conn-id [`Hash`] whose connection went away.
        conn_id: Hash,
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
/// here and the writer task moves it to the wire. Carries ref-counted [`Bytes`] (not `Vec<u8>`): the frame
/// arrives as the already-ref-counted `Payload::Inline`, so pushing it here is an O(1) move, no memcpy on the
/// `!Send` host loop. Unbounded so a `ws/send` on the host loop never blocks on a slow peer (backpressure/
/// slow-peer policy is a later refinement, not a v0 host concern).
pub type OutboundFrameSink = tokio::sync::mpsc::UnboundedSender<bytes::Bytes>;

/// The loop-side live-connection registry: conn-id -> that connection's outbound sink. Populated by the
/// listener (register on accept, deregister on close) and read by the [`WsSendExecutor`] via the
/// [`WsConnRegistry`] impl below. Lives on the single-threaded host loop (`RefCell`, not a `Mutex` — the
/// executor + the register/deregister calls are all on the one loop thread; the per-connection READER/WRITER
/// tasks talk to it only through the mpsc channels, never touching the map directly). Mirrors the
/// `MemWsConnRegistry` test shape, but over real outbound sinks.
#[derive(Default)]
pub struct LiveWsConnRegistry {
    // Keyed by the conn-id `Hash` (a `Copy` [u8;32], cheaply clonable — NOT an owned String, per the
    // operator's cheaply-clonable-everywhere standing rule). The hex string form only appears at the kernel
    // effect-target boundary (`req.target` is `Arc<str>`), where `send_frame` parses it back to a `Hash`.
    conns: RefCell<HashMap<Hash, OutboundFrameSink>>,
}

impl LiveWsConnRegistry {
    /// A fresh empty registry (no connections). The listener registers connections as peers accept.
    pub fn new() -> Self {
        LiveWsConnRegistry {
            conns: RefCell::new(HashMap::new()),
        }
    }

    /// Register a newly-accepted connection under its conn-id [`Hash`], with the sink its writer task drains.
    /// Called by the listener right after it mints the conn-id + spawns the connection's read/write tasks
    /// (paired with the `ws/connect` event emission). A duplicate conn-id would overwrite — the listener
    /// mints unique ids, so this is insert-new in practice. `conn_id` is a `Hash` (cheap `Copy`), not a String.
    pub fn register(&self, conn_id: Hash, sink: OutboundFrameSink) {
        self.conns.borrow_mut().insert(conn_id, sink);
    }

    /// Deregister a closed connection (paired with the `ws/disconnect` event emission). After this, a
    /// `ws/send` to that conn-id resolves [`WsSendResult::Unknown`] (peer gone) — the reducer prunes on the
    /// disconnect event. Idempotent: deregistering an absent conn-id is a no-op.
    pub fn deregister(&self, conn_id: &Hash) {
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
    fn send_frame(&self, conn_id: &str, frame: Bytes) -> WsSendResult {
        // `conn_id` arrives as the kernel effect target (`req.target` is `Arc<str>` — a string the reducer
        // echoed), so parse it back to the `Hash` the registry keys by. A non-hex/wrong-length target names
        // no real connection (a reducer sent a malformed target) → Unknown, same as a gone peer.
        let Some(conn_id) = Hash::from_hex(conn_id) else {
            return WsSendResult::Unknown;
        };
        match self.conns.borrow().get(&conn_id) {
            // The connection is live: MOVE the (ref-counted) frame to its writer task's sink — no memcpy, the
            // Bytes rides the channel by move. A closed channel (the writer task ended — the peer dropped
            // mid-send before deregister ran) is a transient write failure: the disconnect event is in flight,
            // so a retry either lands (race resolves) or resolves Unknown.
            Some(sink) => match sink.send(frame) {
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
pub fn ws_connect_inbound(session: SessionId, conn_id: Hash) -> Inbound {
    // Payload = the conn-id hex bytes: the reducer receives the hex string it echoes back as the `ws/send`
    // target (`req.target` is `Arc<str>`). The identity is a `Hash` (cheap `Copy`); only this boundary + the
    // effect target render it as hex.
    ws_event_inbound(
        session,
        effect_ct::WS_CONNECT,
        conn_id.to_hex().into_bytes(),
    )
}

/// Build the `ws/disconnect` [`Inbound`] the host emits when a peer's connection closes: the reducer prunes
/// the peer from its federation state. Payload = the conn-id hex (which connection went away).
pub fn ws_disconnect_inbound(session: SessionId, conn_id: Hash) -> Inbound {
    ws_event_inbound(
        session,
        effect_ct::WS_DISCONNECT,
        conn_id.to_hex().into_bytes(),
    )
}

/// Build the inbound DATA-FRAME [`Inbound`] the host emits for each opaque message a peer sends. The framing
/// carries the conn-id (so the reducer knows WHICH peer) followed by the raw frame bytes, length-prefixed:
/// `[conn_id_len: u32-le][conn_id bytes][frame bytes]`. The host does NOT interpret the frame — it's opaque
/// application bytes (JSON-RPC/whatever is a userspace concern). Family is a distinct `ws/frame` content-type
/// so the reducer matches data frames apart from the connect/disconnect lifecycle.
pub fn ws_frame_inbound(session: SessionId, conn_id: Hash, frame: &[u8]) -> Inbound {
    let cid = conn_id.to_hex();
    let cid = cid.as_bytes();
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

    /// A deterministic conn-id [`Hash`] for tests (a real one is `mint_conn_id`'s random Hash; tests derive a
    /// stable one from a label so assertions are reproducible). The conn-id is a `Hash`, NOT a String.
    fn cid(label: &[u8]) -> Hash {
        Hash::of(label)
    }

    #[test]
    fn register_then_send_delivers_and_deregister_makes_it_unknown() {
        let reg = LiveWsConnRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        let c1 = cid(b"conn-1");
        reg.register(c1, tx);
        assert_eq!(reg.len(), 1);

        // A send to the live conn is delivered (the executor addresses it by the conn-id HEX = req.target).
        assert_eq!(
            reg.send_frame(&c1.to_hex(), Bytes::from_static(b"hello")),
            WsSendResult::Delivered
        );
        assert_eq!(rx.try_recv().unwrap(), Bytes::from_static(b"hello"));

        // After deregister the conn is gone -> Unknown (the reducer prunes on the disconnect event).
        reg.deregister(&c1);
        assert!(reg.is_empty());
        assert_eq!(
            reg.send_frame(&c1.to_hex(), Bytes::from_static(b"x")),
            WsSendResult::Unknown
        );
    }

    #[test]
    fn send_to_never_registered_or_non_hex_target_is_unknown() {
        let reg = LiveWsConnRegistry::new();
        // A valid-hex-but-unregistered conn-id: no such connection.
        assert_eq!(
            reg.send_frame(&cid(b"nope").to_hex(), Bytes::from_static(b"x")),
            WsSendResult::Unknown
        );
        // A non-hex target (a reducer sent a malformed ws/send target): names no connection -> Unknown.
        assert_eq!(
            reg.send_frame("not-a-hash", Bytes::from_static(b"x")),
            WsSendResult::Unknown
        );
    }

    #[test]
    fn a_closed_writer_sink_is_a_transient_write_failure() {
        let reg = LiveWsConnRegistry::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        let c = cid(b"conn-flaky");
        reg.register(c, tx);
        // The writer task ended (peer dropped mid-send before deregister): the receiver is gone, so the
        // channel send errors -> a transient write failure (the disconnect event resolves the race).
        drop(rx);
        assert!(matches!(
            reg.send_frame(&c.to_hex(), Bytes::from_static(b"x")),
            WsSendResult::WriteFailed(_)
        ));
    }

    #[test]
    fn connect_inbound_carries_the_conn_id_hex_under_ws_connect() {
        let c = cid(b"conn-42");
        let ev = ws_connect_inbound(SessionId::new("outpost"), c);
        assert_eq!(ev.session, SessionId::new("outpost"));
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, effect_ct::WS_CONNECT);
        assert_eq!(
            payload,
            c.to_hex().into_bytes(),
            "connect payload is the conn-id HEX (the reducer echoes it as the ws/send target)"
        );
        assert!(
            ev.reply_to.is_none(),
            "a ws transport event is external ingress, no bounce"
        );
    }

    #[test]
    fn disconnect_inbound_carries_the_conn_id_hex_under_ws_disconnect() {
        let c = cid(b"conn-42");
        let ev = ws_disconnect_inbound(SessionId::new("outpost"), c);
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, effect_ct::WS_DISCONNECT);
        assert_eq!(payload, c.to_hex().into_bytes());
    }

    #[test]
    fn frame_inbound_length_prefixes_the_conn_id_hex_then_the_opaque_frame() {
        let c = cid(b"cid");
        let ev = ws_frame_inbound(SessionId::new("outpost"), c, b"opaque-bytes");
        let (family, payload) = drain_family(&ev.body);
        assert_eq!(family, WS_FRAME_FAMILY);
        // [len:u32-le][conn-id-hex][frame]
        let cid_len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        let hex = c.to_hex();
        assert_eq!(cid_len, hex.len());
        assert_eq!(&payload[4..4 + cid_len], hex.as_bytes());
        assert_eq!(&payload[4 + cid_len..], b"opaque-bytes");
    }

    #[test]
    fn mint_conn_id_produces_distinct_hashes_whose_hex_is_the_routing_token() {
        // A conn-id is a Hash (like a session's genesis hash), minted from fresh entropy each accept -> two
        // connections get different ids. Its hex is the token the reducer echoes as the ws/send target.
        let a = mint_conn_id();
        let b = mint_conn_id();
        assert_ne!(
            a, b,
            "each accepted connection gets a fresh unique conn-id Hash"
        );
        // The hex round-trips as a canonical Hash hex (same scheme as SessionId = genesis-hash hex).
        let hex = a.to_hex();
        assert_eq!(hex.len(), 64, "a conn-id hex is a 64-char blake3 hash hex");
        assert_eq!(
            Hash::from_hex(&hex),
            Some(a),
            "the hex round-trips to the Hash"
        );
        // The registry keys by the Hash; the executor addresses it by that Hash's hex (= req.target).
        let reg = LiveWsConnRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        reg.register(a, tx);
        assert_eq!(
            reg.send_frame(&hex, Bytes::from_static(b"f")),
            WsSendResult::Delivered
        );
    }

    #[test]
    fn control_ops_from_the_listener_apply_against_the_loop_registry() {
        // The loop <-> listener seam: the listener sends Register/Deregister (conn-id = Hash) over the control
        // channel; the loop drains + applies each against the !Send registry.
        let (tx, mut rx) = ws_control_channel();
        let reg = LiveWsConnRegistry::new();
        let (sink, _peer_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

        let conn = mint_conn_id();
        tx.send(WsControlOp::Register {
            conn_id: conn,
            sink,
        })
        .unwrap();
        reg.apply_control(rx.try_recv().unwrap());
        assert_eq!(reg.len(), 1);
        assert_eq!(
            reg.send_frame(&conn.to_hex(), Bytes::from_static(b"hi")),
            WsSendResult::Delivered
        );

        tx.send(WsControlOp::Deregister { conn_id: conn }).unwrap();
        reg.apply_control(rx.try_recv().unwrap());
        assert!(reg.is_empty());
        assert_eq!(
            reg.send_frame(&conn.to_hex(), Bytes::from_static(b"x")),
            WsSendResult::Unknown
        );
    }

    #[test]
    fn emit_ws_event_enqueues_then_reports_false_on_a_closed_inbox() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Inbound>();
        let c = cid(b"c1");
        assert!(emit_ws_event(
            &tx,
            ws_connect_inbound(SessionId::new("outpost"), c)
        ));
        drop(rx);
        assert!(
            !emit_ws_event(&tx, ws_disconnect_inbound(SessionId::new("outpost"), c)),
            "a closed inbox (loop shutting down) reports the event was dropped"
        );
    }
}
