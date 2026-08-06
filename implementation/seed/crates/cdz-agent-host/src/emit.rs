//! [`EmitExecutor`] — the `Emit` effect executor: cross-session messaging (one agent signals another).
//!
//! A reducer in session A performs an `Emit` effect (`EffectKind::Emit` / [`effect_ct::EMIT`]) whose
//! `target` names a PEER session id and whose `payload` is the message. The kernel authorizes it (SEC-F1 —
//! a session may `Emit` only where its capability grants that target, gated BEFORE dispatch, so this
//! executor never re-authorizes — same discipline as [`crate::http::HttpExecutor`]), then dispatches it
//! here. This executor ROUTES the signal to the peer: it feeds an [`Inbound`] event into the host's shared
//! [`Inbox`] addressed to `target`, where the async loop delivers it into that session's log as an
//! `EventBody::Inbound` and the peer reducer folds it — an agent messaging another agent, end to end.
//!
//! The routing target is the [`AsyncAgentHost`](crate::AsyncAgentHost)'s shared `Inbox` (an mpsc sender the
//! loop drains) — the "peer-emit bridge" the loop was designed to accept ([`Inbound`] doc). The emit is
//! FIRE-AND-FORGET (§9b routing hint + async cross-session): the executor returns `Ok(None)` — a unit ack
//! that the signal was accepted + enqueued — the moment the send succeeds; the SENDER does NOT await the
//! peer's processing (delivery-confirmation would be a v2). The kernel folds that `Ok(None)` as a normal
//! `EffectResult` on the sender's log.
//!
//! WIRE CONTRACT (agreed with v-agent-harness, the kernel-side authority):
//! - `target` is the RAW host [`SessionId`] string (the [`AgentHost`](crate::AgentHost) registry key); the
//!   kernel treats it as an opaque routing hint (§9b), no namespacing.
//! - the routed [`Inbound`] carries `content_type.family = "message"` (the same family an ordinary inbound
//!   message uses, so a receiver reducer folds it with the existing pattern) + the sender's `payload`
//!   VERBATIM (opaque — the reducer defines the message schema, §4; a sender that wants to prove provenance
//!   encodes its own id INTO the payload, keeping the host oblivious to message semantics).
//! - `cause` is the emitting effect's dispatch id when available (provenance in the peer's log), else `None`.

use crate::async_host::{Inbound, Inbox};
use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectRequest, Payload};
use cdz_kernel::event::{ContentType, EffectOutcome, EventBody};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The content-type version stamped on a routed peer message (v1 of the cross-session wire).
const MESSAGE_VERSION: u32 = 1;

/// Routes an `Emit` effect to a peer session's inbox — cross-session messaging. Holds the host loop's
/// [`Inbox`] sender (injected at construction, since [`Executor::perform`] has no loop handle); each emit
/// becomes an [`Inbound`] fed to that sender, addressed to the effect's `target` session.
pub struct EmitExecutor {
    inbox: Inbox,
}

impl EmitExecutor {
    /// Build the executor over the host loop's [`Inbox`] sender (from
    /// [`AsyncAgentHost::inbox`](crate::AsyncAgentHost::inbox)). Register it under [`effect_ct::EMIT`] in the
    /// session's `CompositeExecutor`.
    pub fn new(inbox: Inbox) -> Self {
        EmitExecutor { inbox }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for EmitExecutor {
    async fn perform(&mut self, req: &EffectRequest, _idempotency_key: Hash) -> EffectOutcome {
        // `_idempotency_key` is DELIBERATELY dropped in v0 (#2351 review c2): the dedup/provenance it exists
        // for lives at the PERSISTENCE layer (a durable inbox de-duping a redelivered emit across a
        // crash-recovery re-drive), which this in-memory-`Inbox` routing has no way to consult — there's no
        // durable peer-inbox to check the key against yet. So using it here would be a no-op today. When the
        // peer inbox becomes durable (the delivery-confirmation / at-least-once v2), the key threads into
        // that dedup — a routed Inbound stamped with the key, deduped on the peer's persisted log. Until
        // then a re-driven emit CAN double-deliver on crash-recovery; that's an accepted v0 property (the
        // routing is in-memory + the daemon's durable-restart path is a later slice), not an oversight.
        // Family-keyed (seq-39), matching the router + authz decision. A non-Emit family is structural →
        // PERMANENT (§17: observable Err, never a panic).
        if !req.content_type.matches_family(effect_ct::EMIT) {
            return EffectOutcome::Err(crate::retry::permanent(format!(
                "EmitExecutor only handles the {} family, got {}",
                effect_ct::EMIT,
                req.content_type.family
            )));
        }

        // `target` is the peer session id (raw SessionId string, opaque routing hint). An empty target has
        // no peer to route to — structural PERMANENT.
        if req.target.is_empty() {
            return EffectOutcome::Err(crate::retry::permanent(
                "EmitExecutor: an Emit effect requires a non-empty target (the peer session id to route to)",
            ));
        }
        // Clone the `Arc<str>` (O(1) refcount bump) rather than round-tripping `as_ref()` → a fresh alloc
        // (#2351 review c3): `SessionId` IS an `Arc<str>` and `req.target` already is one.
        let target = SessionId::new(req.target.clone());

        // The message payload rides VERBATIM into the peer's Inbound (opaque — the reducer defines the
        // schema). A payload-less emit routes an empty-payload message (a bare signal is legitimate); a
        // blob-ref payload can't be forwarded inline (no blob-store handle here) → structural PERMANENT.
        let payload = match &req.payload {
            Some(Payload::Inline(bytes)) => Payload::Inline(bytes.clone()),
            None => Payload::Inline(Vec::new().into()),
            Some(Payload::Blob(_)) => {
                return EffectOutcome::Err(crate::retry::permanent(
                    "EmitExecutor: a blob-ref Emit payload is unsupported — this executor has no blob-store access; inline the message",
                ));
            }
        };

        // Route: feed an Inbound addressed to the target into the host loop's shared Inbox. The loop
        // delivers it into the target session's log as an EventBody::Inbound (family "message"), which the
        // peer reducer folds. `cause = None` for v1 (a provenance link to the emitting dispatch is a cheap
        // v2 add once the dispatch id is threaded here).
        let inbound = Inbound {
            session: target,
            body: EventBody::Inbound {
                content_type: ContentType {
                    family: "message".into(),
                    version: MESSAGE_VERSION,
                },
                payload,
            },
            cause: None,
        };
        match self.inbox.send(inbound) {
            // Enqueued for the target session — fire-and-forget: Ok(None) is the unit ack that the signal
            // was accepted + routed (NOT the peer's processing result; cross-session is asynchronous).
            Ok(()) => EffectOutcome::Ok(None),
            // The loop's receiver is gone (host shutting down / dropped). The signal couldn't be routed;
            // classify RETRYABLE — a supervisor may re-drive after the loop is back (transient, not a
            // malformed request).
            Err(_) => EffectOutcome::Err(crate::retry::retryable(
                "EmitExecutor: the host loop inbox is closed — cannot route the Emit (host shutting down?)",
            )),
        }
    }

    /// This single-family executor serves exactly the `Emit` family (the capability-manifest mechanism
    /// dimension when used bare; a `CompositeExecutor`'s own `by_family` answers when composed).
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::EMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use tokio::sync::mpsc;

    /// Build an Emit effect request to `target` with an inline `payload` (or none).
    fn emit_req(target: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::EMIT,
            target.to_string(),
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn emit_routes_an_inbound_message_to_the_target_session() {
        // The core cross-session routing: an Emit(target=B, payload) becomes an Inbound addressed to B on
        // the shared inbox, carrying family "message" + the payload verbatim, and the executor acks Ok(None).
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx);

        let out = exec
            .perform(&emit_req("session-b", Some(b"hello-peer")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "a routed emit acks Ok(None) (fire-and-forget), got {out:?}"
        );

        let routed = rx.try_recv().expect("an Inbound was routed to the inbox");
        assert_eq!(
            routed.session.as_str(),
            "session-b",
            "routed to the target session"
        );
        match routed.body {
            EventBody::Inbound {
                content_type,
                payload,
            } => {
                assert_eq!(
                    content_type.family.as_ref(),
                    "message",
                    "peer message family"
                );
                assert_eq!(content_type.version, MESSAGE_VERSION);
                assert_eq!(
                    payload,
                    Payload::Inline(b"hello-peer".to_vec().into()),
                    "the sender's payload rides verbatim into the peer's Inbound"
                );
            }
            other => panic!("expected an Inbound body, got {other:?}"),
        }
        assert!(routed.cause.is_none(), "v1 routes with no cause link");
    }

    #[tokio::test]
    async fn a_payloadless_emit_routes_an_empty_message() {
        // A bare signal (no payload) is legitimate — it routes an empty-payload message, not an error.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx);
        let out = exec.perform(&emit_req("b", None), Hash::of(b"k")).await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let routed = rx.try_recv().expect("routed");
        match routed.body {
            EventBody::Inbound { payload, .. } => {
                assert_eq!(
                    payload,
                    Payload::Inline(Vec::new().into()),
                    "empty-payload message"
                );
            }
            other => panic!("expected Inbound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_empty_target_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx);
        let out = exec
            .perform(&emit_req("", Some(b"x")), Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Err(reason) => {
                assert!(
                    reason.starts_with("PERMANENT:"),
                    "empty target is structural: {reason}"
                );
                assert!(reason.contains("non-empty target"));
            }
            other => panic!("expected a PERMANENT Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_emit_family_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx);
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "b".to_string(),
            None,
            Timeliness::Interactive,
        );
        let out = exec.perform(&req, Hash::of(b"k")).await;
        assert!(
            matches!(out, EffectOutcome::Err(r) if r.starts_with("PERMANENT:") && r.contains("only handles")),
            "a non-Emit family is rejected PERMANENT"
        );
        assert!(exec.handles_family(effect_ct::EMIT) && !exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn a_closed_inbox_is_a_retryable_error() {
        // The host loop's receiver is gone (shutdown) → the emit can't route → RETRYABLE (transient), not a
        // malformed-request PERMANENT.
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // loop's receiver gone
        let mut exec = EmitExecutor::new(tx);
        let out = exec
            .perform(&emit_req("b", Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err(r) if r.starts_with("RETRYABLE:") && r.contains("inbox is closed")),
            "a closed inbox is a transient RETRYABLE, got {out:?}"
        );
    }
}
