//! [`ReplyExecutor`] — the executor for the `effect/reply` family (design DESIGN-userspace-effects I4). A
//! userspace-effect HANDLER, having been forwarded a request by the I3 `UserspaceEffectExecutor` (which minted
//! a one-shot reply-token bound to the original `(caller, effect-id)` and threaded its RAW bytes into the
//! forwarded framing), answers by performing an
//! `effect/reply` effect whose `target` is those raw reply-token bytes and whose `payload` is the response. This
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
//! reply was accepted + routed for settle) the moment the command is enqueued. The reply PAYLOAD is a value-form
//! OUTCOME the handler emits (err-reply seam, v-pc #1): the Ok/Err subset of the pinned
//! `outcome: option<Ok(Inline(bytes) | Blob(hash)) | Err{message,retryable}>` — decoded here via
//! [`decode_reply_outcome`](cdz_kernel::ast_marshal::decode_reply_outcome) and settled onto the caller as the
//! recovered [`EffectOutcome`]. So a handler can signal SUCCESS (`Ok`, carrying an `Inline` or `Blob` response
//! `Payload` — a large response need not inline) OR FAILURE (`Err` with a typed retryability the caller's reducer
//! folds). The host stays oblivious to the effect's MEANING (it never interprets the response bytes), but it
//! faithfully surfaces the ok/err/retryability DISCRIMINANT. FAIL-CLOSED: a reply whose payload is not a
//! well-formed outcome value-form (absent/blob-ref effect payload, `TimedOut`/unknown ctor, malformed record,
//! non-utf-8) is a malformed handler reply → a PERMANENT `Err` settle, never a spurious `Ok` (the outcome
//! value-form IS the reply contract — no legacy bare-payload success path).

use crate::effect_reply::ReplyTokenRegistry;
use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
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
    /// The outcome to fold onto the caller's continuation — the [`EffectOutcome`] the [`ReplyExecutor`]
    /// recovered from the handler's reply outcome value-form (`Ok(Inline|Blob)` success or `Err{..}` failure;
    /// a malformed reply fail-closes to a permanent `Err`).
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
        // PHASE-3 STEP C: schema-hash self-guard; diagnostic reports the mismatched request schema_hash
        // (content_type.family is deleted from EffectRequest in the S3 flip — identity is the schema_hash).
        if req.schema_hash
            != cdz_kernel::ast_marshal::effect_family_schema_hash(effect_ct::EFFECT_REPLY)
        {
            return EffectOutcome::err(format!(
                "ReplyExecutor only handles the {} family (schema_hash mismatch)",
                effect_ct::EFFECT_REPLY
            ));
        }

        // The reply-token rides on `req.target` as its RAW 32 bytes (the handler echoes what the I3 forward
        // framed; target is opaque Arc<[u8]>). An empty target carries no token → PERMANENT (fail-closed);
        // no hex parse (operator zero-hex — the token is binary end to end).
        let token_bytes = req.target.as_ref();
        if token_bytes.is_empty() {
            return EffectOutcome::err(
                "ReplyExecutor: an effect/reply requires the reply-token (raw bytes) as its target",
            );
        }

        // Validate + CONSUME the token one-shot. `None` = the bytes aren't a 32-byte token, never-minted
        // (forged), or already consumed (a duplicate/replayed reply) — REFUSE it PERMANENT (retrying won't
        // make a forged/consumed token valid; this is the reply-forgery + double-settle defense, enforced
        // BEFORE any settle reaches the kernel). The host settles NOTHING on a refused reply.
        let target = match self.reply_tokens.validate_and_consume(token_bytes) {
            Some(t) => t,
            None => {
                return EffectOutcome::err(
                    "ReplyExecutor: the reply-token is unknown, malformed, or already consumed — refused (reply-forgery/double-settle defense)",
                );
            }
        };

        // The reply PAYLOAD is a value-form OUTCOME the handler emits (err-reply seam, v-pc #1): the Ok/Err
        // subset of the pinned `outcome: option<Ok(bytes) | Err{message,retryable} | TimedOut>` value-form
        // (no TimedOut — a handler never replies it; the kernel injects TimedOut). Decode it via the kernel's
        // `decode_reply_outcome` (the codec lives in one place, symmetric with `effect_outcome_view` encode)
        // and settle the caller with the recovered `EffectOutcome`: an Ok arm → `Ok(Some(Inline(response)))`,
        // an Err arm → `Err{message, retryability}` (typed retryability recovered from the value-form). This
        // is what lets a handler signal a FAILURE reply (not just success), surfaced on the caller reducer's
        // outcome child once the caller-side wiring lands; today it already reaches the caller via the
        // effect-outcome flatten.
        //
        // FAIL-CLOSED (contract): the outcome value-form IS the reply contract, so a reply that does NOT
        // decode as it — an absent/blob payload (a value-form outcome is inline bytes), a `TimedOut`/unknown
        // ctor, a malformed Err record, or non-utf-8 — is a malformed handler reply, NOT a legacy success.
        // Settle a PERMANENT `Err` (never a spurious `Ok`), matching the token-invalid fail-closed posture;
        // the token is already consumed, so a redrive is refused (a handler must send a well-formed reply).
        let outcome = match &req.payload {
            Some(Payload::Inline(bytes)) => cdz_kernel::ast_marshal::decode_reply_outcome(bytes)
                .unwrap_or_else(|e| {
                    EffectOutcome::err(format!(
                        "ReplyExecutor: the reply payload is not a well-formed outcome value-form \
                         (Ok(bytes) | Err{{message,retryable}}) — malformed handler reply: {e:?}"
                    ))
                }),
            _ => EffectOutcome::err(
                "ReplyExecutor: an effect/reply requires an inline outcome value-form payload \
                 (Ok(bytes) | Err{message,retryable}); an absent or blob-ref reply payload is malformed",
            ),
        };

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
        cdz_kernel::ast_marshal::effect_family_schema_hash(family)
            == cdz_kernel::ast_marshal::effect_family_schema_hash(effect_ct::EFFECT_REPLY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::{Payload, Timeliness};
    use cdz_kernel::event::Retryability;

    /// An effect/reply request echoing the raw `token_bytes` as its target with an inline `payload` (or none).
    /// The `payload` bytes ARE the reply's outcome value-form (built by `reply_outcome_req` below); a `None`
    /// payload is a malformed (outcome-less) reply.
    fn reply_req(token_bytes: &[u8], payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::EFFECT_REPLY,
            token_bytes,
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    /// An effect/reply request whose payload is the value-form encoding of `outcome` — what a handler emits
    /// (the Ok/Err subset). Built via the kernel's `encode_reply_outcome` (the exact inverse of the executor's
    /// `decode_reply_outcome`), so a round-trip through the executor recovers the same `EffectOutcome`.
    fn reply_outcome_req(token_bytes: &[u8], outcome: &EffectOutcome) -> EffectRequest {
        let payload = cdz_kernel::ast_marshal::encode_reply_outcome(outcome)
            .expect("encode_reply_outcome for a handler-repliable Ok/Err outcome");
        reply_req(token_bytes, Some(&payload))
    }

    #[tokio::test]
    async fn a_valid_reply_consumes_the_token_and_enqueues_a_settle_for_the_caller() {
        // The core I4 path: a handler echoing a valid reply-token consumes it, recovers (caller, effect-id),
        // and enqueues a ReplySettle carrying Ok(payload); the executor acks Ok(None).
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"caller-a")), EffectId(42));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens.clone(), settle_tx);

        let out = exec
            .perform(
                EffectId(7), // the handler's own effect-id — irrelevant to the settle
                &reply_outcome_req(
                    token.as_bytes(),
                    &EffectOutcome::Ok(Some(Payload::Inline(b"the-answer".to_vec().into()))),
                ),
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
            SessionId::new(Hash::of(b"caller-a")),
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
    async fn a_payloadless_reply_is_malformed_and_settles_a_permanent_err() {
        // Under the err-reply value-form contract the reply payload IS the outcome value-form; a payload-LESS
        // reply carries no outcome → malformed → fail-closed PERMANENT Err (never a spurious Ok). The token is
        // still consumed so the caller's open effect settles (with an Err) rather than hanging.
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(1));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(token.as_bytes(), None),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Ok(None)),
            "still routed for settle (fire-and-forget ack)"
        );
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(&settle.outcome, EffectOutcome::Err { retryability: Retryability::Permanent, .. }),
            "a payload-less reply carries no outcome value-form → permanent Err (fail-closed), got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn a_blob_ref_reply_payload_passes_through_to_the_settle() {
        // Blob-ref replies are PRESERVED (operator Option B): the handler encodes an `Ok(Blob(hash))` outcome
        // value-form, `decode_reply_outcome` recovers `Ok(Some(Payload::Blob(hash)))`, and the settle carries
        // that blob ref to the caller unchanged — a large response need not inline. The reply PAYLOAD is the
        // outcome value-form (inline bytes encoding Ok(Blob h)); the Blob lives INSIDE the decoded Ok arm.
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(3));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let blob = Hash::of(b"a-big-response-blob");
        let out = exec
            .perform(
                EffectId(0),
                &reply_outcome_req(
                    token.as_bytes(),
                    &EffectOutcome::Ok(Some(Payload::Blob(blob))),
                ),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(settle.outcome, EffectOutcome::Ok(Some(Payload::Blob(h))) if h == blob),
            "a blob-ref reply passes through to the caller's settle verbatim (Option B preserve), got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn an_err_reply_settles_the_caller_with_a_typed_err_outcome() {
        // The err-reply raison d'être: a handler signals FAILURE. It encodes `Err{message, retryable}` in the
        // reply outcome value-form; the ReplyExecutor recovers `EffectOutcome::Err{message, retryability}` with
        // the retryability typed from the bool (true → Retryable), so the caller's reducer folds retry logic.
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(9));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_outcome_req(
                    token.as_bytes(),
                    &EffectOutcome::Err {
                        message: "upstream timeout".into(),
                        retryability: Retryability::Retryable,
                    },
                ),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(&settle.outcome,
                EffectOutcome::Err { message, retryability: Retryability::Retryable }
                if message == "upstream timeout"),
            "an Err reply settles a typed Retryable Err with the handler's message, got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn a_malformed_reply_payload_settles_a_permanent_err_fail_closed() {
        // Fail-closed: a reply payload that is NOT a well-formed outcome value-form (raw bytes that don't
        // decode as the outcome sum) is a malformed handler reply → PERMANENT Err, NEVER a spurious Ok(payload)
        // legacy path (the no-adapter directive: the outcome value-form IS the contract).
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(11));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(token.as_bytes(), Some(b"not a value-form outcome doc")),
                Hash::of(b"k"),
            )
            .await;
        assert!(matches!(out, EffectOutcome::Ok(None)));
        let settle = settle_rx.try_recv().expect("a settle was enqueued");
        assert!(
            matches!(
                &settle.outcome,
                EffectOutcome::Err {
                    retryability: Retryability::Permanent,
                    ..
                }
            ),
            "a non-value-form reply payload is malformed → permanent Err (fail-closed), got {:?}",
            settle.outcome
        );
    }

    #[tokio::test]
    async fn a_forged_or_consumed_token_is_refused_and_settles_nothing() {
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(1));
        let (settle_tx, mut settle_rx) = reply_settle_channel();
        let mut exec = ReplyExecutor::new(tokens.clone(), settle_tx);

        // A never-minted (forged) token is refused PERMANENT + settles nothing.
        let forged = Hash::of(b"never-minted");
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(forged.as_bytes(), Some(b"x")),
                Hash::of(b"k"),
            )
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
                &reply_req(token.as_bytes(), Some(b"ok")),
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
                &reply_req(token.as_bytes(), Some(b"again")),
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
            .perform(EffectId(0), &reply_req(b"", Some(b"x")), Hash::of(b"k"))
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
        let req =
            EffectRequest::new_with_family(effect_ct::HTTP, "t", None, Timeliness::Interactive);
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
        let token = tokens.mint(SessionId::new(Hash::of(b"c")), EffectId(1));
        let (settle_tx, settle_rx) = reply_settle_channel();
        drop(settle_rx);
        let mut exec = ReplyExecutor::new(tokens, settle_tx);
        let out = exec
            .perform(
                EffectId(0),
                &reply_req(token.as_bytes(), Some(b"x")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Retryable && message.contains("settle channel is closed")),
            "a closed settle channel is RETRYABLE, got {out:?}"
        );
    }
}
