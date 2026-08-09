//! Userspace-effects request→forward→reply→settle ROUND-TRIP across the two landed executors (design
//! DESIGN-userspace-effects I3+I4), over ONE shared [`ReplyTokenRegistry`] — the contract each executor's
//! own unit tests exercise only in ISOLATION. This drives the real public API end to end:
//!
//! 1. a CALLER performs a userspace effect (`weather`); the I3 [`UserspaceEffectExecutor`] resolves it to a
//!    registered handler, MINTS a one-shot reply-token bound to `(caller, effect-id)`, forwards an
//!    `effect-request/weather` [`Inbound`] carrying the framing, and returns [`EffectOutcome::Deferred`];
//! 2. the test plays the HANDLER: it reads the forwarded framing off the inbox, extracts the reply-token the
//!    I3 executor minted, and answers by performing an `effect/reply` whose target IS that token;
//! 3. the I4 [`ReplyExecutor`] validates+CONSUMES the token against the SAME registry and enqueues a
//!    [`ReplySettle`] carrying the ORIGINAL `(caller, effect-id)` + the reply payload — proving the token the
//!    I3 forward minted round-trips to recover exactly the caller effect it was bound to.
//!
//! HERMETIC: two executors + two in-process channels + a shared `Rc<ReplyTokenRegistry>`, no loop / no live
//! store / no reducer. The point is the CROSS-EXECUTOR contract (I3 mint → framing → I4 consume → settle),
//! which neither module's in-crate tests cover (they each mock the other half).

use cdz_agent_host::{
    reply_settle_channel, HandlerResolver, ReplyExecutor, ReplyTokenRegistry, SessionId,
    UserspaceEffectExecutor,
};
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload, Timeliness};
use cdz_kernel::event::{EffectOutcome, EventBody};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use std::collections::HashMap;
use std::rc::Rc;

/// A fixed family→handler resolver standing in for the live canonical-store read (the I3 seam).
struct MapResolver(HashMap<String, SessionId>);
impl HandlerResolver for MapResolver {
    fn resolve_handler(&self, family: &str) -> Option<SessionId> {
        self.0.get(family).cloned()
    }
}

/// Parse the I3 forward framing `[caller_len|caller|token_len|token-RAW-32B|effect_id-u64le|payload]` back
/// into its parts (mirrors the wire the handler reducer would decode).
fn parse_framing(bytes: &[u8]) -> (String, Vec<u8>, u64, Vec<u8>) {
    let clen = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut o = 4;
    let caller = String::from_utf8(bytes[o..o + clen].to_vec()).unwrap();
    o += clen;
    let tlen = u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize;
    o += 4;
    let token = bytes[o..o + tlen].to_vec();
    o += tlen;
    let eid = u64::from_le_bytes([
        bytes[o],
        bytes[o + 1],
        bytes[o + 2],
        bytes[o + 3],
        bytes[o + 4],
        bytes[o + 5],
        bytes[o + 6],
        bytes[o + 7],
    ]);
    o += 8;
    (caller, token, eid, bytes[o..].to_vec())
}

#[tokio::test]
async fn request_forwards_and_a_handler_reply_settles_the_original_caller_effect() {
    // ONE shared reply-token table — the I3 executor mints into it, the I4 executor consumes from it. This
    // sharing (an Rc, as the factory will wire per-loop) is the crux the round-trip proves.
    let reply_tokens = Rc::new(ReplyTokenRegistry::new());

    // The host loop's two channels the executors feed (drained by the loop in production; drained by the test
    // here): the handler Inbox (I3 forward target) + the reply-settle sink (I4 output).
    let (inbox_tx, mut inbox_rx) = tokio::sync::mpsc::unbounded_channel();
    let (settle_tx, mut settle_rx) = reply_settle_channel();

    let caller = SessionId::new("caller-session");
    let handler = SessionId::new("weather-handler");

    // I3: the caller's delegating executor, resolving `weather` -> the handler session.
    let mut i3 = UserspaceEffectExecutor::new(
        MapResolver(HashMap::from([("weather".to_string(), handler.clone())])),
        inbox_tx,
        reply_tokens.clone(),
        caller.clone(),
    );

    // 1. CALLER performs `weather` (effect id 99). It forwards + DEFERS (does NOT answer synchronously).
    let out = i3
        .perform(
            EffectId(99),
            &EffectRequest::new_with_family(
                "weather",
                "querytarget",
                Some(Payload::Inline(b"forecast-for-seattle".to_vec().into())),
                Timeliness::Interactive,
            ),
            Hash::of(b"idem"),
        )
        .await;
    assert!(
        matches!(out, EffectOutcome::Deferred),
        "the userspace effect defers (the handler will settle it), got {out:?}"
    );
    assert_eq!(reply_tokens.len(), 1, "the forward minted one reply-token");

    // 2. The test plays the HANDLER: read the forwarded effect-request off the inbox + extract the token.
    let fwd = inbox_rx
        .try_recv()
        .expect("an effect-request Inbound was forwarded");
    assert_eq!(
        fwd.session.as_str(),
        "weather-handler",
        "forwarded to the resolved handler"
    );
    let (fwd_caller, token_bytes, fwd_eid, req_payload) = match fwd.body {
        EventBody::Inbound {
            content_type,
            payload: Payload::Inline(bytes),
        } => {
            assert_eq!(content_type.family.as_ref(), "effect-request/weather");
            parse_framing(&bytes)
        }
        other => panic!("expected an effect-request Inbound, got {other:?}"),
    };
    assert_eq!(fwd_caller, "caller-session");
    assert_eq!(
        fwd_eid, 99,
        "the framing carries the caller's open effect id"
    );
    assert_eq!(
        req_payload, b"forecast-for-seattle",
        "opaque request rides verbatim"
    );

    // 3. HANDLER answers via effect/reply, echoing the token as the target. The I4 executor validates +
    // consumes it against the SAME registry and enqueues the settle.
    let mut i4 = ReplyExecutor::new(reply_tokens.clone(), settle_tx);
    let reply_out = i4
        .perform(
            EffectId(1), // the handler's own effect-id for this reply — irrelevant to the settle
            &EffectRequest::new_with_family(
                effect_ct::EFFECT_REPLY,
                token_bytes.clone(),
                Some(Payload::Inline(b"sunny-and-72".to_vec().into())),
                Timeliness::Interactive,
            ),
            Hash::of(b"idem2"),
        )
        .await;
    assert!(
        matches!(reply_out, EffectOutcome::Ok(None)),
        "the reply acks Ok(None) fire-and-forget, got {reply_out:?}"
    );
    assert!(
        reply_tokens.is_empty(),
        "the token was consumed one-shot by the reply"
    );

    // The enqueued settle recovers the ORIGINAL (caller, effect-id) the I3 forward bound the token to, and
    // carries the handler's reply payload — the round-trip is closed.
    let settle = settle_rx.try_recv().expect("a ReplySettle was enqueued");
    assert_eq!(
        settle.caller, caller,
        "the settle targets the original caller session"
    );
    assert_eq!(
        settle.effect_id,
        EffectId(99),
        "the settle recovers the caller's original open effect id (not the handler's reply id)"
    );
    assert!(
        matches!(&settle.outcome, EffectOutcome::Ok(Some(Payload::Inline(b))) if &b[..] == b"sunny-and-72"),
        "the caller settles with the handler's reply payload verbatim, got {:?}",
        settle.outcome
    );
}

#[tokio::test]
async fn a_second_reply_with_the_same_token_is_refused_no_double_settle() {
    // The one-shot property ACROSS the two executors: after a valid reply consumes the token, a REPLAY of the
    // same token (a buggy/malicious handler answering twice) is refused + enqueues no second settle — the
    // double-settle defense holds through the real I3-mint → I4-consume path, not just the registry unit test.
    let reply_tokens = Rc::new(ReplyTokenRegistry::new());
    let (inbox_tx, mut inbox_rx) = tokio::sync::mpsc::unbounded_channel();
    let (settle_tx, mut settle_rx) = reply_settle_channel();
    let caller = SessionId::new("c");
    let handler = SessionId::new("h");

    let mut i3 = UserspaceEffectExecutor::new(
        MapResolver(HashMap::from([("weather".to_string(), handler)])),
        inbox_tx,
        reply_tokens.clone(),
        caller,
    );
    i3.perform(
        EffectId(5),
        &EffectRequest::new_with_family("weather", "t", None, Timeliness::Interactive),
        Hash::of(b"k"),
    )
    .await;
    let fwd = inbox_rx.try_recv().expect("forwarded");
    let token_bytes = match fwd.body {
        EventBody::Inbound {
            payload: Payload::Inline(bytes),
            ..
        } => parse_framing(&bytes).1,
        other => panic!("expected Inbound, got {other:?}"),
    };

    let mut i4 = ReplyExecutor::new(reply_tokens, settle_tx);
    let reply = |t: Vec<u8>| {
        EffectRequest::new_with_family(effect_ct::EFFECT_REPLY, t, None, Timeliness::Interactive)
    };
    // First reply settles.
    assert!(matches!(
        i4.perform(EffectId(0), &reply(token_bytes.clone()), Hash::of(b"k"))
            .await,
        EffectOutcome::Ok(None)
    ));
    settle_rx.try_recv().expect("first reply enqueued a settle");
    // Second reply with the same (consumed) token is refused, enqueues nothing.
    let dup = i4
        .perform(EffectId(0), &reply(token_bytes), Hash::of(b"k"))
        .await;
    assert!(
        matches!(&dup, EffectOutcome::Err { .. }),
        "a replayed token is refused, got {dup:?}"
    );
    assert!(
        settle_rx.try_recv().is_err(),
        "the refused duplicate enqueues no second settle (double-settle defense)"
    );
}
