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
//! - `target` is the host [`SessionId`] (the [`AgentHost`](crate::AgentHost) registry key), carried in the
//!   effect's opaque byte target ([`EffectRequest::target`] is `Arc<[u8]>`) and read as UTF-8 via
//!   [`EffectRequest::target_str`]; the kernel treats it as an opaque routing hint (§9b), no namespacing. A
//!   non-UTF-8 target is a fail-closed PERMANENT error.
//! - the routed [`Inbound`] carries `content_type.family = "message"` (the same family an ordinary inbound
//!   message uses, so a receiver reducer folds it with the existing pattern) + the sender's `payload`
//!   VERBATIM (opaque — the reducer defines the message schema, §4; a sender that wants to prove provenance
//!   encodes its own id INTO the payload, keeping the host oblivious to message semantics).
//! - `cause` is the emitting effect's dispatch id when available (provenance in the peer's log), else `None`.

use crate::async_host::{Inbound, Inbox};
use crate::host::SessionId;
use cdz_kernel::effect::{effect_ct, EffectId, EffectRequest, Payload};
use cdz_kernel::event::{ContentType, EffectOutcome, EventBody};
use cdz_kernel::executor::Executor;
use cdz_kernel::hash::Hash;

/// The content-type version stamped on a routed peer message (v1 of the cross-session wire).
const MESSAGE_VERSION: u32 = 1;

/// Routes an `Emit` effect to a peer session's inbox — cross-session messaging. Holds the host loop's
/// [`Inbox`] sender (injected at construction, since [`Executor::perform`] has no loop handle); each emit
/// becomes an [`Inbound`] fed to that sender, addressed to the effect's `target` session.
///
/// It also carries `owner` — the id of the session this executor belongs to (the EMITTER). It stamps that
/// onto the routed [`Inbound`]'s `reply_to` so the loop knows the return-address if the target turns out to
/// be gone/terminated and the delivery must BOUNCE back as a `delivery-failure` (§lifecycle I5). This is
/// host routing metadata, set at wiring time (the owning session's id is known then); it never touches the
/// guest payload (§4 opaque) and is not persisted into any log.
pub struct EmitExecutor {
    inbox: Inbox,
    owner: SessionId,
}

impl EmitExecutor {
    /// Build the executor over the host loop's [`Inbox`] sender (from
    /// [`AsyncAgentHost::inbox`](crate::AsyncAgentHost::inbox)) for the session `owner` (the emitter, whose
    /// `CompositeExecutor` this is registered in under [`effect_ct::EMIT`]). `owner` becomes the `reply_to`
    /// return-address on every routed [`Inbound`], so a bounce (target gone/terminated) can find the sender.
    pub fn new(inbox: Inbox, owner: SessionId) -> Self {
        EmitExecutor { inbox, owner }
    }
}

#[async_trait::async_trait(?Send)]
impl Executor for EmitExecutor {
    async fn perform(
        &mut self,
        _id: EffectId,
        req: &EffectRequest,
        _idempotency_key: Hash,
    ) -> EffectOutcome {
        // `_idempotency_key` is deliberately unused (#2351 review c2, present-tense per #2356 review B): the
        // dedup it exists for lives at the PERSISTENCE layer — de-duping a redelivered emit requires a
        // durable peer-inbox to check the key against, which in-memory `Inbox` routing has none of, so the
        // key has nothing to consult here and using it would be a no-op. A consequence: routing is
        // at-most-effort in-memory — a redelivered emit is not deduped by this executor (the durable inbox
        // that would key on it is where that dedup belongs). Dropping the key is the correct behavior given
        // the routing target, not an oversight.
        // Family-keyed (seq-39), matching the router + authz decision. A non-Emit family is structural →
        // PERMANENT (§17: observable Err, never a panic).
        if !req.content_type.matches_family(effect_ct::EMIT) {
            return EffectOutcome::err(format!(
                "EmitExecutor only handles the {} family, got {}",
                effect_ct::EMIT,
                req.content_type.family
            ));
        }

        // `target` is the peer session id (opaque routing hint). The target is now opaque Arc<[u8]>; a peer
        // session id is UTF-8, so a non-UTF-8 target is malformed → structural PERMANENT (fail-closed).
        let Ok(target_str) = req.target_str() else {
            return EffectOutcome::err(
                "EmitExecutor: an Emit target must be a valid UTF-8 peer session id",
            );
        };
        // An empty target has no peer to route to — structural PERMANENT.
        if target_str.is_empty() {
            return EffectOutcome::err(
                "EmitExecutor: an Emit effect requires a non-empty target (the peer session id to route to)",
            );
        }
        let target = SessionId::new(target_str);

        // The message payload rides VERBATIM into the peer's Inbound (opaque — the reducer defines the
        // schema). A payload-less emit routes an empty-payload message (a bare signal is legitimate); a
        // blob-ref payload can't be forwarded inline (no blob-store handle here) → structural PERMANENT.
        let payload = match &req.payload {
            Some(Payload::Inline(bytes)) => Payload::Inline(bytes.clone()),
            None => Payload::Inline(Vec::new().into()),
            Some(Payload::Blob(_)) => {
                return EffectOutcome::err(
                    "EmitExecutor: a blob-ref Emit payload is unsupported — this executor has no blob-store access; inline the message",
                );
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
            // The emitter's id: the loop's bounce path (§lifecycle I5) routes a `delivery-failure` here if
            // the target is gone/terminated. Cloning an `Arc<str>` (O(1) refcount bump).
            reply_to: Some(self.owner.clone()),
        };
        match self.inbox.send(inbound) {
            // Enqueued for the target session — fire-and-forget: Ok(None) is the unit ack that the signal
            // was accepted + routed (NOT the peer's processing result; cross-session is asynchronous).
            Ok(()) => EffectOutcome::Ok(None),
            // The loop's receiver is gone (host shutting down / dropped). The signal couldn't be routed;
            // classify RETRYABLE — a supervisor may re-drive after the loop is back (transient, not a
            // malformed request).
            Err(_) => EffectOutcome::err_retryable(
                "EmitExecutor: the host loop inbox is closed — cannot route the Emit (host shutting down?)",
            ),
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
    use cdz_kernel::event::Retryability;
    use tokio::sync::mpsc;

    /// Build an Emit effect request to `target` with an inline `payload` (or none).
    fn emit_req(target: &str, payload: Option<&[u8]>) -> EffectRequest {
        EffectRequest::new_with_family(
            effect_ct::EMIT,
            target,
            payload.map(|b| Payload::Inline(b.to_vec().into())),
            Timeliness::Interactive,
        )
    }

    #[tokio::test]
    async fn emit_routes_an_inbound_message_to_the_target_session() {
        // The core cross-session routing: an Emit(target=B, payload) becomes an Inbound addressed to B on
        // the shared inbox, carrying family "message" + the payload verbatim, and the executor acks Ok(None).
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));

        let out = exec
            .perform(
                EffectId(0),
                &emit_req("session-b", Some(b"hello-peer")),
                Hash::of(b"k"),
            )
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
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));
        let out = exec
            .perform(EffectId(0), &emit_req("b", None), Hash::of(b"k"))
            .await;
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
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));
        let out = exec
            .perform(EffectId(0), &emit_req("", Some(b"x")), Hash::of(b"k"))
            .await;
        match out {
            EffectOutcome::Err {
                message,
                retryability,
            } => {
                assert_eq!(
                    retryability,
                    Retryability::Permanent,
                    "empty target is structural: {message}"
                );
                assert!(message.contains("non-empty target"));
            }
            other => panic!("expected a PERMANENT Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_emit_family_is_a_permanent_error() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));
        let req =
            EffectRequest::new_with_family(effect_ct::HTTP, "b", None, Timeliness::Interactive);
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(
            matches!(out, EffectOutcome::Err { message, retryability } if retryability == Retryability::Permanent && message.contains("only handles")),
            "a non-Emit family is rejected PERMANENT"
        );
        assert!(exec.handles_family(effect_ct::EMIT) && !exec.handles_family(effect_ct::HTTP));
    }

    #[tokio::test]
    async fn a_blob_ref_emit_payload_is_a_permanent_error() {
        // The EmitExecutor has no blob-store handle, so it can't forward a blob-ref payload inline into the
        // peer's Inbound → structural PERMANENT (a malformed request for THIS executor, not transient). The
        // inline + payloadless paths are covered above; this pins the third payload arm. Distinct from the
        // closed-inbox RETRYABLE: a blob ref is a request-shape problem, retrying won't help.
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));
        let req = EffectRequest::new_with_family(
            effect_ct::EMIT,
            "b",
            Some(Payload::Blob(Hash::of(b"some-blob-ref"))),
            Timeliness::Interactive,
        );
        let out = exec.perform(EffectId(0), &req, Hash::of(b"k")).await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Permanent && message.contains("blob-ref")),
            "a blob-ref Emit payload is rejected PERMANENT (no blob-store access to forward it), got {out:?}"
        );
    }

    #[tokio::test]
    async fn a_closed_inbox_is_a_retryable_error() {
        // The host loop's receiver is gone (shutdown) → the emit can't route → RETRYABLE (transient), not a
        // malformed-request PERMANENT.
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // loop's receiver gone
        let mut exec = EmitExecutor::new(tx, SessionId::new("sender-a"));
        let out = exec
            .perform(EffectId(0), &emit_req("b", Some(b"x")), Hash::of(b"k"))
            .await;
        assert!(
            matches!(&out, EffectOutcome::Err { message, retryability } if *retryability == Retryability::Retryable && message.contains("inbox is closed")),
            "a closed inbox is a transient RETRYABLE, got {out:?}"
        );
    }

    // ---- session-directory MULTICAST end-to-end (converted from the deleted group_multicast_e2e integration
    // test, operator no-integration-tests mandate — hermetic: HostedSessions + Rust reducers + the real
    // EmitExecutor, no loop/network). A controller resolve-alls a GROUP name to its frozen member set then
    // fans out one by-id Emit per member, and each member folds the routed message. Proves the EmitExecutor's
    // by-id cross-session path composes with the kernel store/* group verbs into "resolve N targets, N
    // unicasts" — beyond the isolated single-emit routing above. ----
    use crate::host::HostedSession;
    use cdz_kernel::authz::Authorizer;
    use cdz_kernel::effect::{Capability, EffectKind, ResourcePredicate};
    use cdz_kernel::event::Event;
    use cdz_kernel::event_ast::{decode_members, encode_member_op};
    use cdz_kernel::executor::CompositeExecutor;
    use cdz_kernel::kv::Kv;
    use cdz_kernel::name_store::NameStore;
    use cdz_kernel::reducer::{FoldOutput, Reducer};

    const GROUP: &str = "session/room/lobby";

    /// The CONTROLLER reducer: on the initial inbound it `store/add`s each member to the group; once the adds
    /// settle it `store/resolve-all`s the group; on the resolve-all result it decodes the frozen members and
    /// fans out one by-id `Emit` per member (the multicast). Records the member count in KV.
    struct MulticastAgent {
        members: Vec<Hash>,
        origin: Hash,
        message: Vec<u8>,
    }
    #[async_trait::async_trait(?Send)]
    impl Reducer for MulticastAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            match &event.body {
                EventBody::Inbound {
                    content_type,
                    payload: _,
                } if content_type.matches_family("message") => {
                    let adds: Vec<EffectRequest> = self
                        .members
                        .iter()
                        .enumerate()
                        .map(|(seq, m)| {
                            let payload =
                                encode_member_op(GROUP, true, m, &(self.origin, seq as u64));
                            EffectRequest::new_with_family(
                                effect_ct::STORE_ADD,
                                GROUP,
                                Some(Payload::Inline(payload.into())),
                                Timeliness::Interactive,
                            )
                        })
                        .collect();
                    FoldOutput::with(adds)
                }
                EventBody::EffectResult {
                    result: EffectOutcome::Ok(body),
                    ..
                } => {
                    // After all N adds settle → resolve-all. On the resolve-all result (the only Ok carrying a
                    // members blob) → decode + fan out one Emit per member.
                    if let Some(Payload::Inline(bytes)) = body {
                        if let Ok(members) = decode_members(bytes) {
                            kv.put(b"member_count".to_vec(), vec![members.len() as u8]);
                            let emits: Vec<EffectRequest> = members
                                .iter()
                                .map(|m| {
                                    EffectRequest::new_with_family(
                                        effect_ct::EMIT,
                                        m.to_hex(),
                                        Some(Payload::Inline(self.message.clone().into())),
                                        Timeliness::Interactive,
                                    )
                                })
                                .collect();
                            return FoldOutput::with(emits);
                        }
                    }
                    let n = kv.get(b"settled").map(|v| v[0]).unwrap_or(0) + 1;
                    kv.put(b"settled".to_vec(), vec![n]);
                    if n as usize == self.members.len() {
                        return FoldOutput::with(vec![EffectRequest::new_with_family(
                            effect_ct::STORE_RESOLVE_ALL,
                            GROUP,
                            None,
                            Timeliness::Interactive,
                        )]);
                    }
                    FoldOutput::none()
                }
                _ => FoldOutput::none(),
            }
        }
    }

    /// A member reducer: folds a routed "message" Inbound into KV["inbox"] (proves it received the multicast).
    struct MemberAgent;
    #[async_trait::async_trait(?Send)]
    impl Reducer for MemberAgent {
        async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
            if let EventBody::Inbound {
                content_type,
                payload: Payload::Inline(bytes),
            } = &event.body
            {
                if content_type.matches_family("message") {
                    kv.put(b"inbox".to_vec(), bytes.to_vec());
                }
            }
            FoldOutput::none()
        }
    }

    fn multicast_go() -> EventBody {
        EventBody::Inbound {
            content_type: ContentType {
                family: "message".into(),
                version: 1,
            },
            payload: Payload::Inline(b"go".to_vec().into()),
        }
    }

    #[tokio::test]
    async fn a_controller_resolve_alls_a_group_and_multicasts_an_emit_to_each_member() {
        // Two members' SessionIds = their genesis-hash-hex. The controller adds both to the group,
        // resolve-alls, and fans out an Emit to each; then each member folds its routed message.
        let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();

        // Build the two member sessions first so we know their genesis-hash ids (what the group stores + Emit
        // targets). Kept `mut` so we can deliver each routed multicast Inbound into its member + assert it folds.
        let mut member_a = HostedSession::genesis(
            Hash::of(b"member-a-reducer"),
            Box::new(MemberAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let mut member_b = HostedSession::genesis(
            Hash::of(b"member-b-reducer"),
            Box::new(MemberAgent),
            Box::new(Authorizer::deny_all()),
            CompositeExecutor::new(),
        );
        let id_a = member_a.genesis_hash();
        let id_b = member_b.genesis_hash();

        // The controller: Emit to any member id + (FamilyGrant, the §4c prefix-authority) store/add +
        // store/resolve-all on the session/ group-name prefix. Groups live in its NameStore.
        let controller_id = SessionId::new("controller");
        let authz = Authorizer::new(vec![Capability {
            kind: EffectKind::Emit,
            predicate: ResourcePredicate::Any,
        }])
        .with_family_grants(vec![
            Capability::for_family(
                effect_ct::STORE_ADD,
                ResourcePredicate::Prefix("session/".into()),
            ),
            Capability::for_family(
                effect_ct::STORE_RESOLVE_ALL,
                ResourcePredicate::Prefix("session/".into()),
            ),
        ]);
        // Emit flows through the EmitExecutor (owner = controller for reply_to); store/* flows through the
        // kernel store-arm (needs the attached NameStore).
        let executor = CompositeExecutor::new().with_effect(
            effect_ct::EMIT,
            Box::new(EmitExecutor::new(tx.clone(), controller_id.clone())),
        );
        let mut controller = HostedSession::genesis(
            Hash::of(b"controller-reducer"),
            Box::new(MulticastAgent {
                members: vec![id_a, id_b],
                origin: Hash::of(b"controller-origin"),
                message: b"broadcast!".to_vec(),
            }),
            Box::new(authz),
            executor,
        )
        .with_name_store(NameStore::new());

        // Drive the controller to quiescence: inbound → adds → resolve-all → fan-out Emits.
        controller
            .deliver(multicast_go(), None)
            .await
            .expect("controller runs add → resolve-all → multicast without a kernel error");
        assert_eq!(
            controller.session().kv().get(b"member_count"),
            Some(&[2u8][..]),
            "resolve-all froze a 2-member set"
        );

        // Exactly two by-id Emits were routed, one per member, carrying the broadcast payload.
        let mut routed = Vec::new();
        while let Ok(inbound) = rx.try_recv() {
            assert!(
                matches!(&inbound.body, EventBody::Inbound { content_type, payload: Payload::Inline(b) }
                    if content_type.matches_family("message") && b.as_ref() == b"broadcast!"),
                "each multicast Inbound carries the broadcast message"
            );
            routed.push((inbound.session.as_str().to_string(), inbound.body));
        }
        let mut routed_targets: Vec<String> = routed.iter().map(|(t, _)| t.clone()).collect();
        routed_targets.sort();
        let mut want = vec![id_a.to_hex(), id_b.to_hex()];
        want.sort();
        assert_eq!(
            routed_targets, want,
            "multicast routed exactly one Emit to each group member (by-id, target = member genesis-hash-hex)"
        );

        // Deliver each routed multicast Inbound into its addressed member + assert the member FOLDED it into
        // KV["inbox"] — the full multicast end-to-end (routed Emit → deliver → KV).
        for (target, body) in routed {
            let member = if target == id_a.to_hex() {
                &mut member_a
            } else {
                &mut member_b
            };
            member
                .deliver(body, None)
                .await
                .expect("a member folds its routed multicast Inbound without a kernel error");
        }
        assert_eq!(
            member_a.session().kv().get(b"inbox"),
            Some(&b"broadcast!"[..]),
            "member A folded the multicast message end to end"
        );
        assert_eq!(
            member_b.session().kv().get(b"inbox"),
            Some(&b"broadcast!"[..]),
            "member B folded the multicast message end to end"
        );
    }
}
