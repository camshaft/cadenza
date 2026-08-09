//! [`ReplyExecutor`] — the executor for the `effect/reply` family (design DESIGN-userspace-effects I4). A
//! userspace-effect HANDLER, having been forwarded a request by the I3 `UserspaceEffectExecutor` (which minted
//! a one-shot reply-token bound to the original `(caller, effect-id)` and threaded its hex into the forwarded
//! framing), answers by performing an
//! `effect/reply` effect whose `target` is that reply-token hex and whose `payload` is the response. This
//! executor validates + CONSUMES the token (reply-forgery + double-settle defense, [`ReplyTokenRegistry`]),
//! recovers the bound `(caller, effect-id)`, and hands the loop a [`ReplySettle`] command so the loop
//! `settle_effect_result`s the caller's OPEN (Deferred) effect — closing the request→forward→reply→settle loop.
//!
//! **Why a settle COMMAND, not a direct settle.** [`Session::settle_effect_result`](cdz_kernel::kernel::Session::settle_effect_result)
//! is a `&mut Session` loop operation that resumes the CALLER's continuation (it needs the caller's reducer +
//! authz + executor). This executor runs in the HANDLER's session context and holds none of the caller's loop
//! state, so it can't settle directly — it emits a [`ReplySettle`] over a channel the host loop drains + applies
//! against the caller session (the same Send/loop split [`WsControlOp`](crate::ws_socket::WsControlOp) uses for
//! ws register/deregister). This module lands the executor core + the settle-command seam; the loop-side drain
//! (`settle_effect_result` per command, like [`settle_signature_query`](crate::host)) is the wiring slice.
//!
//! **HOST = PLUMBING.** A reply is fire-and-forget from the handler's view: this executor acks `Ok(None)` (the
//! reply was accepted + routed for settle) the moment the command is enqueued; it does NOT interpret the
//! response bytes. A v1 reply always settles the caller with a SUCCESS `Ok(payload)` — a handler that wants to
//! signal a failure encodes it IN the payload (the caller's reducer folds the semantics), keeping the host
//! oblivious to the effect's meaning. The response [`Payload`](cdz_kernel::effect::Payload) passes through
//! VERBATIM: Inline OR Blob (unlike
//! the I3 forward, which folds into a byte envelope + can't carry a blob-ref — an [`EffectOutcome`] holds a
//! `Payload` natively, so a blob-ref reply settles the caller with that blob ref unchanged).

use crate::effect_reply::ReplyTokenRegistry;
use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest};
use cdz_kernel::event::EffectOutcome;
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use std::rc::Rc;

/// A loop command to settle a caller's open (Deferred) effect with a handler's reply outcome. The host loop
/// drains these and calls [`Session::settle_effect_result`](cdz_kernel::kernel::Session::settle_effect_result)
/// on the `caller` session for `effect_id` with `outcome` — resuming the caller's continuation. Emitted by
/// [`ReplyExecutor::perform`] once it has validated + consumed the reply-token; the loop is the only place
/// that holds the caller's reducer/authz/executor to actually run the settle.
#[derive(Debug)]
pub struct ReplySettle {
    /// The caller session whose open effect this settles (recovered from the consumed reply-token binding).
    pub caller: SessionId,
    /// The caller's open [`EffectId`] to settle (the `settle_effect_result` key).
    pub effect_id: EffectId,
    /// The outcome to fold onto the caller's continuation — a v1 reply is always `Ok(payload)` (success; a
    /// handler encodes any failure in the payload for the caller's reducer to interpret).
    pub outcome: EffectOutcome,
}

/// The loop-drained sender the [`ReplyExecutor`] enqueues [`ReplySettle`] commands on (an mpsc sender, the
/// same shape as the [`Inbox`](crate::async_host::Inbox) / ws control channel). Unbounded: a settle is a small
/// fire-and-forget command + the loop drains promptly; a bounded channel would let a slow drain wedge a
/// replying handler's turn.
pub type ReplySettleSink = tokio::sync::mpsc::UnboundedSender<ReplySettle>;

/// A fresh reply-settle channel: the [`ReplySettleSink`] the [`ReplyExecutor`] holds + the receiver the host
/// loop drains to apply each [`ReplySettle`] against its caller session.
pub fn reply_settle_channel() -> (
    ReplySettleSink,
    tokio::sync::mpsc::UnboundedReceiver<ReplySettle>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

/// The `effect/reply` executor (I4). Holds the shared [`ReplyTokenRegistry`] (minted by the I3 forward,
/// consumed here — one table, so an `Rc`) and the loop's [`ReplySettleSink`] (to hand the loop the settle
/// command, since this executor can't settle the caller's session directly).
pub struct ReplyExecutor {
    reply_tokens: Rc<ReplyTokenRegistry>,
    settle_tx: ReplySettleSink,
}

impl ReplyExecutor {
    /// Build the executor over the shared `reply_tokens` table (bound to the I3 forward's mints) and the
    /// loop's `settle_tx` (where validated replies enqueue their [`ReplySettle`]).
    pub fn new(reply_tokens: Rc<ReplyTokenRegistry>, settle_tx: ReplySettleSink) -> Self {
        ReplyExecutor {
            reply_tokens,
            settle_tx,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for ReplyExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // `_id` is the HANDLER's own effect-id for THIS effect/reply dispatch (not the caller's) — irrelevant
        // here: the caller effect to settle is recovered from the reply-TOKEN binding, not this id. The kernel
        // folds this executor's returned `Ok(None)` onto the handler's own continuation as usual.

        // Structural: serve ONLY the effect/reply family. Anything else reaching here is a routing bug →
        // PERMANENT (§17: observable Err, never a panic).
        if !req.content_type.matches_family(effect_ct::EFFECT_REPLY) {
            return EffectOutcome::err(format!(
                "ReplyExecutor only handles the {} family, got {}",
                effect_ct::EFFECT_REPLY,
                req.content_type.family
            ));
        }

        // The reply-token hex rides on `req.target` (the handler echoes what the I3 forward framed). An empty
        // target carries no token → PERMANENT.
        let token_hex = req.target.as_ref();
        if token_hex.is_empty() {
            return EffectOutcome::err(
                "ReplyExecutor: an effect/reply requires the reply-token (hex) as its target",
            );
        }

        // Validate + CONSUME the token one-shot. `None` = the token is malformed/non-hex, never-minted
        // (forged), or already consumed (a duplicate/replayed reply) — REFUSE it PERMANENT (retrying won't
        // make a forged/consumed token valid; this is the reply-forgery + double-settle defense, enforced
        // BEFORE any settle reaches the kernel). The host settles NOTHING on a refused reply.
        let target = match self.reply_tokens.validate_and_consume(token_hex) {
            Some(t) => t,
            None => {
                return EffectOutcome::err(
                    "ReplyExecutor: the reply-token is unknown, malformed, or already consumed — refused (reply-forgery/double-settle defense)",
                );
            }
        };

        // The response payload settles the caller VERBATIM. A v1 reply is a SUCCESS outcome; the payload
        // passes through as-is — Inline OR Blob (an EffectOutcome carries a Payload natively, so unlike the
        // I3 forward this need not inline a blob-ref). A payload-less reply settles the caller `Ok(None)`.
        let outcome = EffectOutcome::Ok(req.payload.clone());

        let settle = ReplySettle {
            caller: target.caller,
            effect_id: target.effect_id,
            outcome,
        };
        match self.settle_tx.send(settle) {
            // Enqueued for the loop to settle the caller — fire-and-forget ack to the handler.
            Ok(()) => EffectOutcome::Ok(None),
            // The loop's receiver is gone (host shutting down). The token is already consumed (a redrive
            // would find it gone + be refused), but the reply couldn't be routed to a settle → classify
            // RETRYABLE (transient — the settle drain is unavailable, not a malformed reply). Given the drain
            // being gone means host shutdown, a redrive is moot in practice; the classification is for
            // consistency with the other transport executors.
            Err(_) => EffectOutcome::err_retryable(
                "ReplyExecutor: the host loop settle channel is closed — cannot route the reply settle (host shutting down?)",
            ),
        }
    }

    /// Serves exactly the `effect/reply` family (the handler's reply verb).
    fn handles_family(&self, family: &str) -> bool {
        family == effect_ct::EFFECT_REPLY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{Payload, Timeliness};
    use cdz_kernel::event::Retryability;

    /// An effect/reply request echoing `token_hex` as its target with an inline `payload` (or none).
    fn reply_req(token_hex: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::EFFECT_REPLY,
            token_hex.to_string(),
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn a_valid_reply_consumes_the_token_and_enqueues_a_settle_for_the_caller() {
        // The core I4 path: a handler echoing a valid reply-token consumes it, recovers (caller, effect-id),
        // and enqueues a ReplySettle carrying Ok(payload); the executor acks Ok(None).
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new("caller-a"), EffectId(42));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens.clone(), settle_tx);

        let out = exec
            .perform(
                EffectId(7), // the handler's own effect-id — irrelevant to the settle
                &reply_req(&token.to_hex(), Some(b"the-answer")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "a routed reply acks Ok(None) (fire-and-forget), got {out:?}"
        );
        // The token was consumed one-shot.
        assert!(tokens.is_empty(), "the reply-token was consumed");

        let settle = settle_rx.try_recv().expect("a ReplySettle was enqueued");
        assert_eq!(
            settle.caller,
            SessionId::new("caller-a"),
            "settles the caller"
        );
        assert_eq!(
            settle.effect_id,
            EffectId(42),
            "settles the caller's open effect"
        );
        assert!(
            matches!(&settle.outcome, EffectOutcome::Ok(Some(Payload::Inline(b))) if &b[..] == b"the-answer"),
            "the reply payload settles the caller verbatim, got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn a_payloadless_reply_settles_the_caller_with_ok_none() {
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new("c"), EffectId(1));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(&token.to_hex(), None),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(settle.outcome, EffectOutcome::Ok(None)),
            "a payload-less reply settles Ok(None)"
        );
    }

    #[tokio::test]
    async fn a_blob_ref_reply_payload_passes_through_to_the_settle() {
        // Unlike the I3 forward (which folds into a byte envelope + rejects a blob-ref), the settle carries a
        // Payload natively, so a blob-ref reply settles the caller with that blob ref unchanged.
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new("c"), EffectId(3));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let blob = Hash::of(b"a-big-response-blob");
        let req = EffectRequest::new_with_family(
            effect_ct::EFFECT_REPLY,
            token.to_hex(),
            Some(Payload::Blob(blob)),
            Timeliness::Interactive,
        );
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(settle.outcome, EffectOutcome::Ok(Some(Payload::Blob(h))) if h == blob),
            "a blob-ref reply passes through to the caller's settle verbatim, got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn a_forged_or_consumed_token_is_refused_and_settles_nothing() {
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new("c"), EffectId(1));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens.clone(), settle_tx);

        // A never-minted (forged) token is refused PERMANENT + settles nothing.
        let forged = Hash::of(b"never-minted").to_hex();
        let out = exec
            .perform(EffectId(0), &reply_req(&forged, Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("refused")),
            "a forged token is refused PERMANENT, got {out:?}"
        );
        assert!(
            settle_rx.try_recv().is_err(),
            "nothing was enqueued for a forged reply"
        );
        assert_eq!(
            tokens.len(),
            1,
            "the forged attempt didn't consume the real token"
        );

        // The real token works ONCE.
        assert!(matches!(
            exec.perform(
                EffectId(0),
                &reply_req(&token.to_hex(), Some(b"ok")),
                Hash::of(b"k")
            )
            .await,
            EffectOutcome::Ok(None)
        ));
        let _ = settle_rx
            .try_recv()
            .expect("the valid reply enqueued a settle");
        // A SECOND reply with the same (now consumed) token is refused — double-settle defense.
        let dup = exec
            .perform(
                EffectId(0),
                &reply_req(&token.to_hex(), Some(b"again")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&dup, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("refused")),
            "a replayed/consumed token is refused PERMANENT (double-settle defense), got {dup:?}"
        );
        assert!(
            settle_rx.try_recv().is_err(),
            "a refused duplicate enqueues no settle"
        );
    }

    #[tokio::test]
    async fn an_empty_target_is_a_permanent_error() {
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let (settle_tx, _rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(EffectId(0), &reply_req("", Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("requires the reply-token")),
            "an empty target is PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_non_reply_family_is_a_permanent_error() {
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let (settle_tx, _rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let req = EffectRequest::new_with_family(
            effect_ct::HTTP,
            "t".to_string(),
            None,
            Timeliness::Interactive,
        );
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("only handles")),
            "a non-reply family is PERMANENT, got {out:?}"
        );
        assert!(
            exec.handles_family(effect_ct::EFFECT_REPLY) && !exec.handles_family(effect_ct::HTTP)
        );
    }

    #[tokio::test]
    async fn a_closed_settle_channel_is_retryable() {
        // The loop's settle receiver is gone → the reply can't be routed to a settle → RETRYABLE. (The token
        // is consumed by then, but the drain being gone means host shutdown, so a redrive is moot.)
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new("c"), EffectId(1));
        let (settle_tx, settle_rx) = reply_settle_channel();
        drop(settle_rx);
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(&token.to_hex(), Some(b"x")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Retryable && message.contains("settle channel is closed")),
            "a closed settle channel is RETRYABLE, got {out:?}"
        );
    }
}
