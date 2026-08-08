//! End-to-end: session-directory MULTICAST (§design-session-directory I4) — a controller resolves a GROUP
//! name to its frozen member set, then fans out one by-id `Emit` per member, and each member session folds
//! the message. Proves the operator's "resolve multiple targets for a single name" over the REAL host
//! wiring, composing two ALREADY-LANDED pieces with NO new host mechanism:
//! - the `store/*` GROUP verbs (`store/add` / `store/resolve-all`, v-agent-harness kernel I1-I3b) through a
//!   per-session [`NameStore`], and
//! - the by-id [`EmitExecutor`](cdz_agent_host::EmitExecutor) cross-session path (target = each member's
//!   SessionId = its genesis-hash-hex).
//!
//! The multicast is "resolve the frozen set, then N unicasts" (design D4 v0: reducer-side loop reusing by-id
//! Emit). resolve-all FREEZES the membership into the controller's log (a query effect, §4b bridge rule), so
//! the fan-out is replay-deterministic even as membership keeps changing.

use cdz_agent_host::{EmitExecutor, HostedSession, Inbound, SessionId};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::event_ast::{decode_members, encode_member_op};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::name_store::NameStore;
use cdz_kernel::reducer::{FoldOutput, Reducer};
use tokio::sync::mpsc;

const GROUP: &str = "session/room/lobby";

/// The CONTROLLER reducer (§I4 multicast): on the initial inbound it `store/add`s each member to the group;
/// once the adds settle it `store/resolve-all`s the group; on the resolve-all result it DECODES the frozen
/// members and fans out one by-id `Emit` per member (the multicast). Records the member count in KV.
struct MulticastAgent {
    members: Vec<Hash>,
    origin: Hash,
    message: Vec<u8>,
}
#[async_trait::async_trait(?Send)]
impl Reducer for MulticastAgent {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            // Kick-off: add every member to the group (each op tagged (origin, seq) — the OR-set CRDT tag).
            EventBody::Inbound {
                content_type,
                payload: _,
            } if content_type.matches_family("message") => {
                let adds: Vec<EffectRequest> = self
                    .members
                    .iter()
                    .enumerate()
                    .map(|(seq, m)| {
                        let payload = encode_member_op(GROUP, true, m, &(self.origin, seq as u64));
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
                // Count settled effects. After all N adds settle → resolve-all. On the resolve-all result
                // (the only Ok carrying a members blob) → decode + fan out one Emit per member.
                if let Some(Payload::Inline(bytes)) = body {
                    if let Ok(members) = decode_members(bytes) {
                        kv.put(b"member_count".to_vec(), vec![members.len() as u8]);
                        // Fan out: one by-id Emit per member (target = member hex = its SessionId).
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

fn go() -> EventBody {
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
    // Two members' SessionIds = their genesis-hash-hex. The controller adds both to the group, resolve-alls,
    // and fans out an Emit to each. Driven directly (the loop's Emit-routing is covered elsewhere): we assert
    // the controller decoded 2 members + routed exactly 2 by-id Emits, and each targets a real member id.
    let (tx, mut rx) = mpsc::unbounded_channel::<Inbound>();

    // Build the two member sessions first so we know their genesis-hash ids (what the group stores + Emit
    // targets). Kept `mut` so we can DELIVER each routed multicast Inbound into its member below and assert it
    // FOLDS — closing the full end-to-end loop the module doc describes (#2444 Copilot c1).
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

    // The controller: authorized to store/add + store/resolve-all on the GROUP prefix (FamilyGrant, the §4c
    // prefix-authority) AND to Emit to each member. Has a NameStore (groups live there). Emits via the real
    // EmitExecutor over `tx`.
    let controller_id = SessionId::new("controller");
    // Authorized to Emit to any member id + (via FamilyGrant, the §4c prefix-authority) store/add +
    // store/resolve-all on the session/ group-name prefix.
    let authz = Authorizer::new(vec![Capability {
        kind: cdz_kernel::effect::EffectKind::Emit,
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
    // Emit flows through the EmitExecutor (registered under effect_ct::EMIT, owner = controller for reply_to);
    // store/* flows through the kernel store-arm (needs a NameStore, attached below).
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
        .deliver(go(), None)
        .await
        .expect("controller runs add → resolve-all → multicast without a kernel error");

    // It decoded exactly 2 members from the frozen resolve-all set.
    assert_eq!(
        controller.session().kv().get(b"member_count"),
        Some(&[2u8][..]),
        "resolve-all froze a 2-member set"
    );

    // Exactly two by-id Emits were routed, one per member, carrying the broadcast payload. Capture each
    // (target, body) so we can then DELIVER it into the addressed member (the loop's routing step) and prove
    // the member FOLDS it — the full multicast end-to-end the module doc describes.
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
    // KV["inbox"] — the end-to-end the module doc promises ("each member session folds the message"), not
    // just fan-out routing (#2444 Copilot c1: doc claimed a member fold the test didn't exercise).
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
        "member A folded the multicast message end to end (routed Emit → deliver → KV)"
    );
    assert_eq!(
        member_b.session().kv().get(b"inbox"),
        Some(&b"broadcast!"[..]),
        "member B folded the multicast message end to end (routed Emit → deliver → KV)"
    );
}
