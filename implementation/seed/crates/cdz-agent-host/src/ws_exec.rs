//! The `ws/send` executor — the OUTBOUND half of THE OUTPOST's websocket plumbing (O1). A reducer emits a
//! `ws/send` effect naming a peer connection (`req.target` = the opaque conn-id the host minted on accept and
//! surfaced to the reducer in a `ws/connect` Inbound event) with the outbound frame bytes (`req.payload`); this
//! executor maps the conn-id to the live connection's outbound sink and writes the frame.
//!
//! **THIN MECHANISM ONLY (operator standing-order: minimize host logic — the host is INEVOLVABLE).** This
//! executor does only the send: conn-id -> sink -> write bytes. It carries NO policy: WHICH peers a reducer may
//! address is EVOLVABLE POLICY in the Cedar WASM authorizer on the log (the kernel Cedar-authorizes each
//! `ws/send` effect on its resolved `target` = conn-id before dispatch, SEC-F1), NOT baked into this host code.
//! WHO connected + which conn-ids are live federation state is likewise the reducer's fold over the
//! `ws/connect`/`ws/disconnect` Inbound events (emitted by the listener slice) — this executor just delivers.
//!
//! **Wire shape (reconciled with v-agent-harness, the `ws/*` kernel-family owner, #2807).** The conn-id rides
//! `req.target` (opaque `Arc<[u8]>`, read as UTF-8 hex via [`EffectRequest::target_str`] — exactly like `shell`
//! target=program / `blob/get` target=hex-hash) — the kernel never interprets it; the host mints it and both
//! sides pass it verbatim; a non-UTF-8 target is a fail-closed PERMANENT error. `req.payload` = the outbound
//! frame bytes (`Payload::Inline`). Result:
//! - delivered -> `Ok(None)` ("sent, nothing to fold back" — like a fire-and-forget write).
//! - unknown / gone conn-id -> `Err` PERMANENT (the peer is no longer connected; a blind retry to the same dead
//!   conn-id re-fails — the reducer learns the send didn't land + prunes on the paired `ws/disconnect`).
//! - a transient sink write hiccup -> `err_retryable` (the connection is still up; a retry may succeed).
//!
//! **Connection registry seam.** The set of live connections is shared between the listener (which inserts on
//! accept, removes on close) and this executor (which looks up by conn-id to write). That shared state is behind
//! the [`WsConnRegistry`] trait so this executor is HERMETICALLY unit-testable (a [`MemWsConnRegistry`] backed by
//! in-memory channels) with NO tokio-net / no live socket — the `live-ws` listener slice provides the real
//! registry over live websocket sinks. NOT feature-gated: the executor + the registry trait are always-on
//! (hermetic), like `blob_exec`; only the socket LISTENER that fills the registry is the `live-ws` piece.

use bytes::Bytes;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The outcome of writing one outbound frame to a peer connection's sink — the registry's send result, mapped
/// by the executor onto an [`EffectOutcome`]. Kept deliberately small (mechanism, not policy): the registry
/// reports only what happened to the write, never any authorization judgment.
#[derive(Debug, PartialEq, Eq)]
pub enum WsSendResult {
    /// The frame was handed to the live connection's outbound sink.
    Delivered,
    /// No live connection is registered under this conn-id (never opened, or already closed/pruned).
    Unknown,
    /// The connection exists but the write failed transiently (e.g. a full/closing sink) — a retry may land.
    WriteFailed(String),
}

/// The shared live-connection registry: the seam between the `live-ws` listener (inserts on accept, removes on
/// close) and the [`WsSendExecutor`] (looks up by conn-id to write an outbound frame). Abstracted as a trait so
/// the executor is hermetically testable without a live socket. `?Send`-friendly (the host loop is
/// single-threaded `!Send`, mirroring the rest of the executor set).
pub trait WsConnRegistry {
    /// Write `frame` to the peer connection registered under `conn_id`. Returns [`WsSendResult`] describing the
    /// write; MUST NOT make any policy decision (the kernel already Cedar-authorized the send).
    ///
    /// Takes the frame by OWNED [`Bytes`] (not `&[u8]`): the connection sink stores/forwards the frame anyway,
    /// so a borrow would force the impl to defensively `to_vec()`-copy every outbound frame on the hot send
    /// path. `Payload::Inline` is already ref-counted `Bytes`, so the executor MOVES it in (O(1) clone, no
    /// memcpy) — the operator's cheaply-clonable/ownership rule: take the owned cheaply-clonable value, don't
    /// borrow-then-copy.
    fn send_frame(&self, conn_id: &str, frame: Bytes) -> WsSendResult;
}

/// An `Rc<R>` is a [`WsConnRegistry`] whenever `R` is — so a `WsSendExecutor<Rc<LiveWsConnRegistry>>` sends
/// through the SAME shared node registry the [`AsyncAgentHost`](crate::AsyncAgentHost) loop drains
/// `WsControlOp`s into (federation F0→F1 wiring). The registry uses interior mutability (`&self` methods over a
/// `RefCell`), so a shared `Rc` handle is the right way to give the loop AND every session's executor the one
/// node-scoped map without moving ownership. Single-threaded (`Rc`, not `Arc`) — the loop + executors are all
/// on the one `!Send` task.
impl<R: WsConnRegistry> WsConnRegistry for std::rc::Rc<R> {
    fn send_frame(&self, conn_id: &str, frame: Bytes) -> WsSendResult {
        (**self).send_frame(conn_id, frame)
    }
}

/// The `ws/send` executor over a connection registry `R`. Owns no policy — the kernel Cedar-authorized the
/// effect's `target` (conn-id) before dispatch; this executor only routes the frame to the sink.
pub struct WsSendExecutor<R: WsConnRegistry> {
    registry: R,
}

impl<R: WsConnRegistry> WsSendExecutor<R> {
    /// Construct over a connection registry. No configuration — WHICH peers are reachable is the Cedar policy's
    /// call (decided before this executor is reached), and WHICH conn-ids are live is the listener's doing.
    pub fn new(registry: R) -> Self {
        WsSendExecutor { registry }
    }
}

#[async_trait::async_trait(?Send)]
impl<R: WsConnRegistry> Executor for WsSendExecutor<R> {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        let family = req.content_type.family.as_ref();
        // The kernel routes by family, but be explicit: this executor serves ONLY ws/send (the outbound
        // effect). ws/connect + ws/disconnect are INBOUND events the listener emits, never effects dispatched
        // here, so a non-ws/send ws family reaching this executor is a mis-route.
        if family != effect_ct::WS_SEND {
            return EffectOutcome::err(format!(
                "WsSendExecutor only handles {}, got {family}",
                effect_ct::WS_SEND
            ));
        }
        // target = the opaque conn-id (hex) the host minted + surfaced in ws/connect. The target is now
        // opaque Arc<[u8]>; the conn-id hex is UTF-8, so a non-UTF-8 target is malformed → structural
        // PERMANENT (fail-closed). Empty target is also structural.
        let Ok(conn_id) = req.target_str() else {
            return EffectOutcome::err(
                "ws/send: target is not valid UTF-8 (expected the peer conn-id)",
            );
        };
        if conn_id.is_empty() {
            return EffectOutcome::err("ws/send: empty target (expected the peer conn-id)");
        }
        // payload = the outbound frame bytes. No payload = nothing to send (structural, PERMANENT). The
        // inline bytes are ref-counted `Bytes`; CLONE (O(1) ref-count bump, no memcpy) + move into the sink.
        let frame = match &req.payload {
            Some(Payload::Inline(b)) => b.clone(),
            // A blob-ref payload would require this executor to resolve a hash first (not its job) — malformed.
            Some(Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "ws/send: a blob-ref payload is unsupported — inline the frame bytes",
                );
            }
            None => return EffectOutcome::err("ws/send: no payload frame bytes to send"),
        };
        match self.registry.send_frame(conn_id, frame) {
            // Delivered -> nothing to fold back (a send has no return value; the peer's reply, if any, arrives
            // later as its own inbound frame -> a fresh Inbound event, not this effect's result).
            WsSendResult::Delivered => EffectOutcome::Ok(None),
            // The peer is gone (never opened / already closed). PERMANENT: retrying the same dead conn-id
            // re-fails; the reducer folds the Err + prunes the peer on its paired ws/disconnect event.
            WsSendResult::Unknown => EffectOutcome::err(format!(
                "ws/send: no live connection for conn-id {conn_id:?} (peer gone)"
            )),
            // The connection is still up but the write hiccuped — TRANSIENT, a retry may land.
            WsSendResult::WriteFailed(e) => {
                EffectOutcome::err_retryable(format!("ws/send: write to {conn_id:?} failed: {e}"))
            }
        }
    }

    /// Serves ONLY `ws/send`. The other `ws/*` names ([`effect_ct::WS_CONNECT`]/[`effect_ct::WS_DISCONNECT`])
    /// are INBOUND events the listener emits, not effects — so this executor claims just the outbound one, not
    /// the whole `ws/` prefix (a `ws/connect` reaching an executor would be a bug, caught by `perform`'s guard).
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::WS_SEND
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use std::cell::RefCell;

    /// A hermetic in-memory registry: records delivered frames per conn-id, lets a test register/close conn-ids
    /// and force a transient write failure, exercising every [`WsSendResult`] arm with no socket. The real
    /// `live-ws` listener supplies a registry over live websocket sinks with the same trait.
    #[derive(Default)]
    struct MemWsConnRegistry {
        /// conn-id -> the frames written to it (in order), for delivered connections.
        live: RefCell<std::collections::HashMap<String, Vec<Vec<u8>>>>,
        /// conn-ids that should report a transient write failure (connection up, sink hiccup).
        failing: RefCell<std::collections::HashSet<String>>,
    }

    impl MemWsConnRegistry {
        fn open(&self, conn_id: &str) {
            self.live
                .borrow_mut()
                .insert(conn_id.to_string(), Vec::new());
        }
        fn fail(&self, conn_id: &str) {
            self.open(conn_id);
            self.failing.borrow_mut().insert(conn_id.to_string());
        }
        fn frames(&self, conn_id: &str) -> Vec<Vec<u8>> {
            self.live.borrow().get(conn_id).cloned().unwrap_or_default()
        }
    }

    impl WsConnRegistry for MemWsConnRegistry {
        fn send_frame(&self, conn_id: &str, frame: Bytes) -> WsSendResult {
            if self.failing.borrow().contains(conn_id) {
                return WsSendResult::WriteFailed("sink full".into());
            }
            match self.live.borrow_mut().get_mut(conn_id) {
                Some(frames) => {
                    frames.push(frame.to_vec());
                    WsSendResult::Delivered
                }
                None => WsSendResult::Unknown,
            }
        }
    }

    fn send_req(conn_id: &str, frame: &[u8]) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::WS_SEND,
            conn_id,
            Some(Payload::Inline(frame.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn send_to_a_live_conn_delivers_the_frame_and_folds_ok_none() {
        let reg = MemWsConnRegistry::default();
        reg.open("conn-1");
        let mut exec = WsSendExecutor::new(reg);
        let out = exec
            .perform(EffectId(0), &send_req("conn-1", b"hello"), Hash::of(b"k"))
            .await;
        assert_eq!(
            out,
            EffectOutcome::Ok(None),
            "a delivered send folds Ok(None)"
        );
        // The frame reached the sink verbatim.
        assert_eq!(
            exec.registry.frames("conn-1"),
            vec![b"hello".to_vec()],
            "the exact frame bytes were written to the live connection"
        );
    }

    #[tokio::test]
    async fn send_to_an_unknown_conn_id_is_a_permanent_err() {
        let mut exec = WsSendExecutor::new(MemWsConnRegistry::default());
        let out = exec
            .perform(EffectId(0), &send_req("gone", b"x"), Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Err { retryability, .. } => assert_eq!(
                retryability,
                cdz_kernel::event::Retryability::Permanent,
                "a gone peer is permanent (retry to a dead conn-id re-fails)"
            ),
            other => panic!("unknown conn-id should Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_transient_write_failure_is_retryable() {
        let reg = MemWsConnRegistry::default();
        reg.fail("conn-flaky");
        let mut exec = WsSendExecutor::new(reg);
        let out = exec
            .perform(EffectId(0), &send_req("conn-flaky", b"x"), Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Err { retryability, .. } => assert_eq!(
                retryability,
                cdz_kernel::event::Retryability::Retryable,
                "a sink hiccup on a live connection is retryable"
            ),
            other => panic!("a transient write failure should be retryable Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_target_and_missing_payload_are_structural_errs() {
        let mut exec = WsSendExecutor::new(MemWsConnRegistry::default());
        // empty conn-id
        assert!(matches!(
            exec.perform(EffectId(0), &send_req("", b"x"), Hash::of(b"k"))
                .await,
            EffectOutcome::Err { .. }
        ));
        // no payload
        let no_payload = EffectRequest::new_with_family(
            effect_ct::WS_SEND,
            "conn-1",
            None,
            Timeliness::Interactive,
        );
        assert!(matches!(
            exec.perform(EffectId(0), &no_payload, Hash::of(b"k")).await,
            EffectOutcome::Err { .. }
        ));
    }

    #[tokio::test]
    async fn handles_only_ws_send_not_connect_or_disconnect() {
        let exec = WsSendExecutor::new(MemWsConnRegistry::default());
        assert!(exec.handles_family(effect_ct::WS_SEND));
        // ws/connect + ws/disconnect are INBOUND events, not effects — this executor must NOT claim them.
        assert!(!exec.handles_family(effect_ct::WS_CONNECT));
        assert!(!exec.handles_family(effect_ct::WS_DISCONNECT));
        assert!(!exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn a_misrouted_non_ws_send_family_is_a_structural_err() {
        let mut exec = WsSendExecutor::new(MemWsConnRegistry::default());
        // ws/connect must never be dispatched as an effect; if it reaches perform, that's a mis-route.
        let misrouted = EffectRequest::new_with_family(
            effect_ct::WS_CONNECT,
            "conn-1",
            Some(Payload::Inline(b"x".to_vec().into())),
            Timeliness::Interactive,
        );
        assert!(matches!(
            exec.perform(EffectId(0), &misrouted, Hash::of(b"k")).await,
            EffectOutcome::Err { .. }
        ));
    }
}
