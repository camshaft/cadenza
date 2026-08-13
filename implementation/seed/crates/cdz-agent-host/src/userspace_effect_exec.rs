//! [`UserspaceEffectExecutor`] — the delegating executor for USERSPACE effect families (design
//! DESIGN-userspace-effects I3). A reducer performs an effect whose family is not a kernel built-in (e.g.
//! `weather`, `vector-search`) — a candidate for handler resolution ([`effect_ct::is_registered_effect_family`]).
//! When a HANDLER session has registered itself for that family (an `effect/<family>` pointer in the canonical
//! name store, resolved by [`HandlerResolver`]), this executor does NOT answer synchronously — it FORWARDS the
//! request to the handler session and returns [`EffectOutcome::Deferred`], leaving the caller's effect OPEN. The
//! handler later answers with an `effect/reply` effect that the host [`ReplyExecutor`](crate::effect_reply)
//! (I4) settles back onto the caller's open effect by [`EffectId`] — closing the request→forward→reply→settle
//! loop. This is the host mechanism THE OUTPOST-as-userspace-handler needs: MCP/federation/routing is all a
//! userspace handler session folding forwarded requests, the host never interprets the effect's meaning.
//!
//! **HOST = PLUMBING (policy is the handler's fold).** This executor makes NO routing/authz decision about the
//! effect's meaning: it resolves the family to a handler SessionId (mechanism — "is a handler registered?"),
//! mints an unforgeable one-shot reply-token bound to `(caller, effect-id)`, forwards the opaque request bytes
//! into the handler's Inbox, and defers. WHICH families have handlers (I1 registration), WHETHER a caller may
//! use a handler (authz), and what the handler DOES with the request are all decided outside this mechanism.
//!
//! **The forward is decoupled from the live canonical store via [`HandlerResolver`]** — the same seam split
//! `ws_exec`'s [`WsConnRegistry`](crate::ws_exec) used to stay hermetically testable without a live socket. An
//! [`Executor`] sees only a spawn-time by-value copy of the canonical [`NameStore`](cdz_kernel::name_store::NameStore),
//! so a resolver that reads the LIVE canonical store (handlers register at runtime) is a distinct loop-side
//! wiring slice; this module lands the delegating mechanism + its wire contract against the trait.

use crate::async_host::{Inbound, Inbox};
use crate::effect_reply::ReplyTokenRegistry;
use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::{ContentType, EffectOutcome, EventBody};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;
use std::rc::Rc;

/// The content-type family PREFIX stamped on a forwarded userspace-effect request: `effect-request/<family>`.
/// Host-owned (a transport framing detail, like [`WS_FRAME_FAMILY`](crate::ws_socket::WS_FRAME_FAMILY)), not a
/// kernel effect const — it names the INBOUND event a handler folds, never a dispatched effect. A handler
/// reducer matches `effect-request/<its-family>` to receive requests routed to it.
pub const EFFECT_REQUEST_FAMILY_PREFIX: &str = "effect-request/";

/// The content-type version stamped on a forwarded effect-request Inbound (v1 of the userspace-effect wire).
pub const EFFECT_REQUEST_VERSION: u32 = 1;

/// Resolve a userspace-effect `family` to the handler session registered for it — the MECHANISM dimension of
/// I3 ("is a handler registered for this family?"). The live implementation reads the host-owned CANONICAL
/// [`NameStore`](cdz_kernel::name_store::NameStore) via
/// [`resolve_effect_handler`](cdz_kernel::name_store::NameStore::resolve_effect_handler) (an `effect/<family>`
/// pointer at a handler SessionId — the genesis-hash hex), which is why it is a loop-side collaborator, not
/// something an [`Executor`] can read from its spawn-time store copy. A trait so this executor is hermetically
/// testable with a fixed map (no live store, no loop), matching the `WsConnRegistry` split.
pub trait HandlerResolver {
    /// The handler [`SessionId`] registered for `family`, or `None` if no handler is registered (the effect is
    /// unhandled). `family` is the bare userspace family (`weather`), NOT the `effect/<family>` store-name.
    fn resolve_handler(&self, family: &str) -> Option<SessionId>;
}

/// Build the forwarded-request [`Inbound`] a handler session folds. The framing carries the ROUTING metadata
/// (which caller, which open effect, the reply-token to echo) as a length-prefixed byte header followed by the
/// opaque request payload — the same fold-the-header-into-the-payload shape
/// [`ws_frame_inbound`](crate::ws_socket::ws_frame_inbound) uses (the host has no side-band metadata slot on
/// [`EventBody::Inbound`]):
///
/// `[caller_len: u32-le][caller-id bytes][token_len: u32-le][reply-token RAW 32 bytes][effect_id: u64-le][payload bytes]`
///
/// - `caller-id` — provenance: the session that performed the effect (a handler that wants to prove who asked
///   reads it; the host stays oblivious to request semantics).
/// - `reply-token` (raw 32 bytes) — the CAPABILITY the handler echoes on its `effect/reply` `target` to settle exactly
///   this `(caller, effect-id)` (unforgeable + one-shot, [`ReplyTokenRegistry`]).
/// - `effect_id` — the caller's open [`EffectId`] (provenance in the handler's log; the settle keys on it via
///   the token, so the handler need not parse it, but it is surfaced for symmetry with the durable frame).
/// - `payload` — the opaque request bytes VERBATIM (the handler defines the schema; host does not interpret).
pub fn effect_request_inbound(
    handler: SessionId,
    caller: SessionId,
    effect_id: EffectId,
    reply_token: Hash,
    family: &str,
    payload: &[u8],
) -> Inbound {
    // The caller id rides the framing as its RAW 32 bytes (a SessionId IS the genesis Hash — no hex).
    let caller_hash = caller.hash();
    let caller_bytes = caller_hash.as_bytes();
    // The reply-token rides the framing as its RAW 32 bytes (no hex) — the handler echoes them verbatim on
    // effect/reply's Arc<[u8]> target; the I4 ReplyExecutor validates them as bytes (operator zero-hex).
    let token_bytes = reply_token.as_bytes();
    let mut buf =
        Vec::with_capacity(4 + caller_bytes.len() + 4 + token_bytes.len() + 8 + payload.len());
    buf.extend_from_slice(&(caller_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(caller_bytes);
    buf.extend_from_slice(&(token_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(token_bytes);
    buf.extend_from_slice(&effect_id.0.to_le_bytes());
    buf.extend_from_slice(payload);
    Inbound {
        session: handler,
        body: EventBody::Inbound {
            content_type: ContentType {
                family: format!("{EFFECT_REQUEST_FAMILY_PREFIX}{family}").into(),
                version: EFFECT_REQUEST_VERSION,
            },
            payload: Payload::Inline(buf.into()),
        },
        cause: None,
        // The CALLER is the return-address: if the handler is gone/terminated the loop bounce (§lifecycle I5)
        // + the I6 terminate-prune can settle the caller's open effect with a delivery-failure rather than
        // leaving it hung. Cloning an `Arc<str>` (O(1) refcount bump).
        reply_to: Some(caller),
    }
}

/// The delegating userspace-effect executor (I3). Holds the family→handler [`HandlerResolver`], the host loop's
/// [`Inbox`] sender (to forward the request into the handler session — same injected-Inbox shape as
/// [`EmitExecutor`](crate::emit::EmitExecutor)), the shared [`ReplyTokenRegistry`] (minted here on forward,
/// consumed by the I4 [`ReplyExecutor`](crate::effect_reply) — one table, so an `Rc`), and `owner`: the CALLER
/// session this executor is registered in (whose effects it forwards + whose `(caller, id)` a reply-token binds).
pub struct UserspaceEffectExecutor<R: HandlerResolver> {
    resolver: R,
    inbox: Inbox,
    reply_tokens: Rc<ReplyTokenRegistry>,
    owner: SessionId,
}

impl<R: HandlerResolver> UserspaceEffectExecutor<R> {
    /// Build the executor for the caller session `owner`, over its `resolver` (family→handler), the host loop
    /// `inbox` (to forward requests), and the shared `reply_tokens` table (bound to the I4 reply path).
    pub fn new(
        resolver: R,
        inbox: Inbox,
        reply_tokens: Rc<ReplyTokenRegistry>,
        owner: SessionId,
    ) -> Self {
        UserspaceEffectExecutor {
            resolver,
            inbox,
            reply_tokens,
            owner,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl<R: HandlerResolver> Executor for UserspaceEffectExecutor<R> {
    async fn perform(
        &mut self,
        id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // `_idempotency_key` is unused for the same reason as `EmitExecutor`: dedup of a re-driven forward
        // belongs at a durable peer-inbox, which in-memory `Inbox` routing has none of — the key has nothing
        // to consult here (a redelivered forward mints a fresh token + re-forwards; the kernel's at-most-once
        // settle on an already-settled id is the real guard).
        let family = req.content_type.family.as_ref();

        // Structural: this executor serves ONLY registered userspace families (the syntactic partition —
        // never a kernel built-in like http/shell/ws, which have their own executors). A built-in reaching
        // here is a routing bug → PERMANENT (§17: observable Err, never a panic).
        if !effect_ct::is_registered_effect_family(family) {
            return EffectOutcome::err(format!(
                "UserspaceEffectExecutor serves only registered userspace-effect families, not the built-in family {family}"
            ));
        }

        // Resolve the family to its handler session. No handler → the effect is unhandled: PERMANENT (the
        // reducer folds the error arm + resumes; retrying the same dispatch won't conjure a handler). In
        // practice the router only reaches `perform` when `handles_family` saw a handler, so this is the
        // defensive arm for a deregistration between routing + perform.
        let handler = match self.resolver.resolve_handler(family) {
            Some(h) => h,
            None => {
                return EffectOutcome::err(format!(
                    "UserspaceEffectExecutor: no registered handler for effect family {family}"
                ));
            }
        };

        // The request payload rides VERBATIM into the handler's Inbound (opaque — the handler defines the
        // schema). Payload-less = an empty request body (a bare signal is legitimate). A blob-ref payload
        // can't be forwarded inline (no blob-store handle here, same as `EmitExecutor`) → PERMANENT; a
        // blob-forwarding userspace-effect path is a documented follow-up.
        let payload: &[u8] = match &req.payload {
            Some(Payload::Inline(bytes)) => bytes,
            None => &[],
            Some(Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "UserspaceEffectExecutor: a blob-ref request payload is unsupported — this executor has no blob-store access; inline the request",
                );
            }
        };

        // Mint the one-shot reply-token bound to (this caller, this effect-id) + thread it into the forwarded
        // framing. Minted AFTER the structural checks so a rejected request leaves no dangling token.
        let token = self.reply_tokens.mint(self.owner, id);
        let inbound = effect_request_inbound(handler, self.owner, id, token, family, payload);

        match self.inbox.send(inbound) {
            // Forwarded — DEFER: the kernel leaves the caller's effect OPEN; the handler's `effect/reply`
            // (validated + consumed by the I4 ReplyExecutor) settles the real outcome by this EffectId.
            Ok(()) => EffectOutcome::Deferred,
            // The host loop's receiver is gone (shutdown). The request couldn't be routed → clean up the
            // token we just minted (it can never be replied to) so it doesn't leak, then classify RETRYABLE
            // (transient — a supervisor may re-drive once the loop is back).
            Err(_) => {
                self.reply_tokens.validate_and_consume(token.as_bytes());
                EffectOutcome::err_retryable(
                    "UserspaceEffectExecutor: the host loop inbox is closed — cannot forward the request (host shutting down?)",
                )
            }
        }
    }

    /// Serves a family iff it is a registered userspace family AND a handler resolves for it — so the
    /// composite router delegates a userspace effect here only when it is actually handled (an unresolved
    /// family falls through to the kernel's unhandled-effect path rather than being claimed + errored).
    fn handles_family(&self, family: &str) -> bool {
        effect_ct::is_registered_effect_family(family)
            && self.resolver.resolve_handler(family).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdz_kernel::effect::Timeliness;
    use cdz_kernel::event::Retryability;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    /// A fixed family→handler map standing in for the live canonical-store resolver.
    struct MapResolver(HashMap<String, SessionId>);
    impl HandlerResolver for MapResolver {
        fn resolve_handler(&self, family: &str) -> Option<SessionId> {
            self.0.get(family).cloned()
        }
    }

    /// Build a family→handler resolver; each handler is addressed by the canonical genesis-hash id its
    /// label hashes to (`Hash::of(label)`), the same id the forwarded Inbound routes to.
    fn resolver(pairs: &[(&str, &str)]) -> MapResolver {
        MapResolver(
            pairs
                .iter()
                .map(|(f, h)| (f.to_string(), SessionId::new(Hash::of(h.as_bytes()))))
                .collect(),
        )
    }

    /// A userspace-effect request in `family` with an inline `payload` (or none).
    fn ue_req(family: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            family.to_string(),
            // target is opaque to this executor (the handler interprets it) — any value works.
            "target",
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    /// Parse the forwarded framing back out: (caller, reply-token RAW bytes, effect-id, payload). The caller
    /// id rides as its RAW 32 genesis-hash bytes (a SessionId IS the genesis Hash — no hex on this seam).
    fn parse_framing(bytes: &[u8]) -> (SessionId, Vec<u8>, u64, Vec<u8>) {
        let clen = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let mut o = 4;
        let caller_bytes: [u8; 32] = bytes[o..o + clen]
            .try_into()
            .expect("the caller id is 32 raw genesis-hash bytes");
        let caller = SessionId::new(Hash::from_bytes(caller_bytes));
        o += clen;
        let tlen =
            u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]) as usize;
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
    async fn a_resolved_effect_forwards_to_the_handler_and_defers() {
        // The core I3 path: a registered userspace family with a handler forwards an effect-request Inbound
        // to that handler + mints a reply-token bound to (caller, id) + returns Deferred (NOT a folded result).
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec = UserspaceEffectExecutor::new(
            resolver(&[("weather", "weather-handler")]),
            tx,
            tokens.clone(),
            SessionId::new(Hash::of(b"caller-a")),
        );

        let out = exec
            .perform(
                EffectId(42),
                &ue_req("weather", Some(b"forecast?")),
                Hash::of(b"k"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Deferred),
            "a forwarded userspace effect DEFERS (the handler settles it later), got {out:?}"
        );

        // Exactly one token was minted, bound to (caller-a, EffectId(42)).
        assert_eq!(tokens.len(), 1, "the forward minted one reply-token");

        let fwd = rx
            .try_recv()
            .expect("an effect-request Inbound was forwarded");
        assert_eq!(
            fwd.session,
            SessionId::new(Hash::of(b"weather-handler")),
            "forwarded to the resolved handler session"
        );
        assert_eq!(
            fwd.reply_to,
            Some(SessionId::new(Hash::of(b"caller-a"))),
            "reply_to is the caller (bounce/return address)"
        );
        match fwd.body {
            EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } => {
                assert_eq!(
                    content_type.family.as_ref(),
                    "effect-request/weather",
                    "family is effect-request/<family>"
                );
                assert_eq!(content_type.version, EFFECT_REQUEST_VERSION);
                let (caller, token_bytes, eid, req_payload) = parse_framing(&bytes);
                assert_eq!(
                    caller,
                    SessionId::new(Hash::of(b"caller-a")),
                    "framing carries the caller id"
                );
                assert_eq!(eid, 42, "framing carries the open effect id");
                assert_eq!(
                    req_payload, b"forecast?",
                    "the opaque request payload rides verbatim"
                );
                // The framed token is exactly the minted one — the handler echoes it on effect/reply, and it
                // validates + consumes to (caller-a, 42).
                let target = tokens
                    .validate_and_consume(&token_bytes)
                    .expect("the framed reply-token validates");
                assert_eq!(target.caller, SessionId::new(Hash::of(b"caller-a")));
                assert_eq!(target.effect_id, EffectId(42));
            }
            other => panic!("expected an Inbound with inline framing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_payloadless_request_forwards_an_empty_body() {
        // A bare userspace effect (no payload) is legitimate — it forwards an empty request body, not an error.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec = UserspaceEffectExecutor::new(
            resolver(&[("ping", "ping-handler")]),
            tx,
            tokens,
            SessionId::new(Hash::of(b"c")),
        );
        let out = exec
            .perform(EffectId(1), &ue_req("ping", None), Hash::of(b"k"))
            .await;
        assert!(matches!(out, EffectOutcome::Deferred));
        let fwd = rx.try_recv().expect("forwarded");
        match fwd.body {
            EventBody::Inbound {
                payload: Payload::Inline(bytes),
                ..
            } => {
                let (_caller, _token, _eid, req_payload) = parse_framing(&bytes);
                assert!(req_payload.is_empty(), "empty request body");
            }
            other => panic!("expected Inbound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unhandled_family_is_a_permanent_error_and_mints_no_token() {
        // A registered userspace family with NO handler resolves to nothing → PERMANENT (unhandled), and
        // crucially mints no dangling token.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec = UserspaceEffectExecutor::new(
            resolver(&[]),
            tx,
            tokens.clone(),
            SessionId::new(Hash::of(b"c")),
        );
        let out = exec
            .perform(EffectId(1), &ue_req("weather", Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("no registered handler")),
            "an unhandled family is PERMANENT, got {out:?}"
        );
        assert_eq!(tokens.len(), 0, "a rejected request mints no reply-token");
        assert!(rx.try_recv().is_err(), "nothing was forwarded");
    }

    #[tokio::test]
    async fn a_builtin_family_is_a_permanent_error() {
        // A kernel built-in family (http) reaching this executor is a routing bug — it is not a userspace
        // family, so it is refused PERMANENT (never claimed away from the real http executor).
        let (tx, _rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec =
            UserspaceEffectExecutor::new(resolver(&[]), tx, tokens, SessionId::new(Hash::of(b"c")));
        let out = exec
            .perform(EffectId(1), &ue_req(effect_ct::HTTP, None), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("built-in family")),
            "a built-in family is refused PERMANENT, got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_blob_ref_payload_is_a_permanent_error() {
        // No blob-store handle here (same as EmitExecutor) → a blob-ref request can't be forwarded inline →
        // PERMANENT, and no token is minted (the blob check precedes the mint).
        let (tx, _rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec = UserspaceEffectExecutor::new(
            resolver(&[("weather", "h")]),
            tx,
            tokens.clone(),
            SessionId::new(Hash::of(b"c")),
        );
        let req = EffectRequest::new_with_family(
            "weather",
            "t",
            Some(Payload::Blob(Hash::of(b"blob"))),
            Timeliness::Interactive,
        );
        let out = exec.perform(EffectId(1), &req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("blob-ref")),
            "a blob-ref payload is PERMANENT, got {out:?}"
        );
        assert_eq!(
            tokens.len(),
            0,
            "no token minted for a rejected blob request"
        );
    }

    #[tokio::test]
    async fn a_closed_inbox_is_retryable_and_cleans_up_the_minted_token() {
        // The loop's receiver is gone → the forward can't route → RETRYABLE, and the token minted for the
        // forward is CLEANED UP (it could never be replied to) so it doesn't leak.
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let mut exec = UserspaceEffectExecutor::new(
            resolver(&[("weather", "h")]),
            tx,
            tokens.clone(),
            SessionId::new(Hash::of(b"c")),
        );
        let out = exec
            .perform(EffectId(1), &ue_req("weather", Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Retryable && message.contains("inbox is closed")),
            "a closed inbox is RETRYABLE, got {out:?}"
        );
        assert_eq!(
            tokens.len(),
            0,
            "the token minted for a failed forward is cleaned up, not leaked"
        );
    }

    #[tokio::test]
    async fn handles_family_only_for_a_registered_family_with_a_handler() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let tokens = Rc::new(ReplyTokenRegistry::new());
        let exec = UserspaceEffectExecutor::new(
            resolver(&[("weather", "h")]),
            tx,
            tokens,
            SessionId::new(Hash::of(b"c")),
        );
        assert!(
            exec.handles_family("weather"),
            "a registered userspace family with a handler is served"
        );
        assert!(
            !exec.handles_family("stocks"),
            "a userspace family with NO handler is NOT claimed (falls through to unhandled)"
        );
        assert!(
            !exec.handles_family(effect_ct::HTTP),
            "a built-in family is never claimed (its own executor serves it)"
        );
        assert!(
            !exec.handles_family(effect_ct::EFFECT_REPLY),
            "effect/reply is a built-in routed family, not a userspace family"
        );
    }

    // ---- I3+I4 CROSS-EXECUTOR round-trip (converted from the deleted userspace_effect_round_trip_e2e
    // integration test, operator no-integration-tests mandate — hermetic: two executors + two in-process
    // channels + one shared Rc<ReplyTokenRegistry>, no loop/store/reducer). This is the request→forward→
    // reply→settle contract that neither executor's OWN unit tests cover (each mocks the other half): I3
    // MINTS a reply-token into the shared table + forwards the framing, I4 CONSUMES it from the SAME table
    // and settles the ORIGINAL caller effect the token was bound to. ----
    use crate::reply_exec::{reply_settle_channel, ReplyExecutor};

    #[tokio::test]
    async fn request_forwards_and_a_handler_reply_settles_the_original_caller_effect() {
        // ONE shared reply-token table — the I3 executor mints into it, the I4 executor consumes from it
        // (an Rc, as the factory wires per-loop). This sharing is the crux the round-trip proves.
        let reply_tokens = Rc::new(ReplyTokenRegistry::new());
        // The two host-loop channels the executors feed (drained by the loop in production; by the test here):
        // the handler Inbox (I3 forward target) + the reply-settle sink (I4 output).
        let (inbox_tx, mut inbox_rx) = mpsc::unbounded_channel();
        let (settle_tx, mut settle_rx) = reply_settle_channel();

        let caller = SessionId::new(Hash::of(b"caller-session"));

        // I3: the caller's delegating executor, resolving `weather` -> the handler session.
        let mut i3 = UserspaceEffectExecutor::new(
            resolver(&[("weather", "weather-handler")]),
            inbox_tx,
            reply_tokens.clone(),
            caller,
        );

        // 1. CALLER performs `weather` (effect id 99). It forwards + DEFERS (does NOT answer synchronously).
        let out = i3
            .perform(
                EffectId(99),
                &ue_req("weather", Some(b"forecast-for-seattle")),
                Hash::of(b"idem"),
            )
            .await;
        assert!(
            matches!(out, EffectOutcome::Deferred),
            "the userspace effect defers (the handler will settle it), got {out:?}"
        );
        assert_eq!(reply_tokens.len(), 1, "the forward minted one reply-token");

        // 2. Play the HANDLER: read the forwarded effect-request off the inbox + extract the raw token bytes.
        let fwd = inbox_rx
            .try_recv()
            .expect("an effect-request Inbound was forwarded");
        assert_eq!(
            fwd.session,
            SessionId::new(Hash::of(b"weather-handler")),
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
        assert_eq!(fwd_caller, caller);
        assert_eq!(
            fwd_eid, 99,
            "the framing carries the caller's open effect id"
        );
        assert_eq!(
            req_payload, b"forecast-for-seattle",
            "opaque request rides verbatim"
        );

        // 3. HANDLER answers via effect/reply, echoing the RAW token bytes as the target. Its PAYLOAD is the
        // reply outcome value-form (err-reply seam): a success reply encodes `Ok(Inline(response))` via
        // `encode_reply_outcome`, which the I4 ReplyExecutor decodes back to the caller's `EffectOutcome`. The
        // I4 executor validates + consumes the token against the SAME registry and enqueues the settle.
        let reply_payload = cdz_kernel::ast_marshal::encode_reply_outcome(&EffectOutcome::Ok(
            Some(Payload::Inline(b"sunny-and-72".to_vec().into())),
        ))
        .expect("encode the handler's Ok reply outcome");
        let mut i4 = ReplyExecutor::new(reply_tokens.clone(), settle_tx);
        let reply_out = i4
            .perform(
                EffectId(1), // the handler's own effect-id for this reply — irrelevant to the settle
                &EffectRequest::new_with_family(
                    effect_ct::EFFECT_REPLY,
                    token_bytes.clone(),
                    Some(Payload::Inline(reply_payload.into())),
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

        // The enqueued settle recovers the ORIGINAL (caller, effect-id) the I3 forward bound the token to,
        // and carries the handler's reply payload — the round-trip is closed.
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
        // The one-shot property ACROSS the two executors: after a valid reply consumes the token, a REPLAY of
        // the same token (a buggy/malicious handler answering twice) is refused + enqueues no second settle —
        // the double-settle defense holds through the real I3-mint → I4-consume path, not just the registry
        // unit test.
        let reply_tokens = Rc::new(ReplyTokenRegistry::new());
        let (inbox_tx, mut inbox_rx) = mpsc::unbounded_channel();
        let (settle_tx, mut settle_rx) = reply_settle_channel();

        let mut i3 = UserspaceEffectExecutor::new(
            resolver(&[("weather", "h")]),
            inbox_tx,
            reply_tokens.clone(),
            SessionId::new(Hash::of(b"c")),
        );
        i3.perform(EffectId(5), &ue_req("weather", None), Hash::of(b"k"))
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
            EffectRequest::new_with_family(
                effect_ct::EFFECT_REPLY,
                t,
                None,
                Timeliness::Interactive,
            )
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
}
