//! End-to-end: the §4c v0.3 SHARED name store — an [`AgentHost`] with a canonical store gives LIVE
//! cross-session visibility, so a LATER-spawned consumer resolves what an earlier publisher set WITHOUT
//! the explicit export/replay bridge the two-agent e2e used.
//!
//! Lifecycle proven here (single-host-owned, conflict-free under single-writer-per-name):
//!   - `AgentHost::with_canonical_store` holds ONE canonical `NameStore`.
//!   - Spawn a PUBLISHER: it's born with a by-value replay of canonical; it `store/set`s a pointer; on
//!     `AgentHost::deliver` returning, the host folds its new write back into canonical (merge_appends_from).
//!   - Spawn a CONSUMER LATER: it's born with a replay of the NOW-updated canonical, so it already sees the
//!     publisher's pointer — a `store/resolve` returns the published hash with no host-side bridge wiring.
//!
//! This is hermetic (no wasm artifact / network) — it exercises the store lifecycle only, so it runs in the
//! default gate (unlike name_store_two_agent_e2e, which runs a real reducer component behind an env gate).

use cdz_agent_host::{AgentHost, HostedSession, SessionId};
use cdz_kernel::authz::Authorizer;
use cdz_kernel::effect::{
    effect_ct, Capability, EffectRequest, Payload, ResourcePredicate, Timeliness,
};
use cdz_kernel::event::{ContentType, EffectOutcome, Event, EventBody};
use cdz_kernel::event_ast::{decode_name_set, encode_name_set};
use cdz_kernel::executor::CompositeExecutor;
use cdz_kernel::hash::Hash;
use cdz_kernel::kv::Kv;
use cdz_kernel::name_store::NameStore;
use cdz_kernel::reducer::{FoldOutput, Reducer};

const POINTER: &str = NameStore::COMPILER_LATEST;

/// PUBLISHER: on inbound, `store/set`s the pointer → the hash it carries.
struct Publisher {
    hash: Hash,
}
#[async_trait::async_trait(?Send)]
impl Reducer for Publisher {
    async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
        if matches!(event.body, EventBody::Inbound { .. }) {
            FoldOutput::with(vec![EffectRequest::new_with_family(
                effect_ct::STORE_SET,
                POINTER,
                Some(Payload::Inline(encode_name_set(POINTER, &self.hash).into())),
                Timeliness::Interactive,
            )])
        } else {
            FoldOutput::none()
        }
    }
}

/// CONSUMER: on inbound, `store/resolve`s the pointer; records the resolved hash's hex in KV.
struct Consumer;
#[async_trait::async_trait(?Send)]
impl Reducer for Consumer {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => FoldOutput::with(vec![EffectRequest::new_with_family(
                effect_ct::STORE_RESOLVE,
                POINTER,
                None,
                Timeliness::Interactive,
            )]),
            EventBody::EffectResult {
                result: EffectOutcome::Ok(Some(Payload::Inline(bytes))),
                ..
            } => {
                if let Ok((_n, h)) = decode_name_set(bytes) {
                    kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                }
                FoldOutput::none()
            }
            _ => FoldOutput::none(),
        }
    }
}

fn inbound_go() -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"go".to_vec().into()),
    }
}

fn set_system() -> Box<Authorizer> {
    Box::new(
        Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/".into()),
        )]),
    )
}
fn resolve_system() -> Box<Authorizer> {
    Box::new(
        Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
            effect_ct::STORE_RESOLVE,
            ResourcePredicate::Prefix("system/".into()),
        )]),
    )
}

#[tokio::test]
async fn a_later_spawned_session_sees_what_an_earlier_session_published_via_the_canonical_store() {
    let published = Hash::of(b"compiler-wasm-v3");
    // A host with a canonical shared store — every session it spawns gets a replay-copy + merges back.
    let mut host = AgentHost::with_canonical_store(NameStore::new());

    // PUBLISH: spawn a publisher (born with a replay of the empty canonical), deliver a "go" so it sets the
    // pointer; on deliver return the host folds its write back into canonical.
    let pub_id = host.spawn(
        SessionId::new("publisher"),
        HostedSession::genesis(
            Hash::of(b"publisher-v1"),
            Box::new(Publisher { hash: published }),
            set_system(),
            CompositeExecutor::new(),
        ),
    );
    host.deliver(&pub_id, inbound_go(), None)
        .await
        .expect("publisher session exists")
        .expect("the publish turn ran");

    // CONSUME: spawn a DIFFERENT session LATER. It's born with a replay of the now-updated canonical, so it
    // already carries the publisher's pointer — NO explicit export/replay bridge between the two sessions.
    let con_id = host.spawn(
        SessionId::new("consumer"),
        HostedSession::genesis(
            Hash::of(b"consumer-v1"),
            Box::new(Consumer),
            resolve_system(),
            CompositeExecutor::new(),
        ),
    );
    host.deliver(&con_id, inbound_go(), None)
        .await
        .expect("consumer session exists")
        .expect("the resolve turn ran");

    // The later consumer resolved the exact hash the earlier publisher set — live cross-session visibility
    // through the host's canonical store, no bridge.
    let resolved = host
        .get(&con_id)
        .expect("consumer registered")
        .session()
        .kv()
        .get(b"resolved")
        .expect("consumer recorded a resolved hash");
    assert_eq!(
        resolved,
        published.to_hex().as_bytes(),
        "a later-spawned consumer resolved COMPILER_LATEST to what the earlier publisher set — via the \
         canonical shared store, no explicit bridge"
    );
}

#[tokio::test]
async fn a_share_less_host_leaves_sessions_store_less() {
    // Regression: a plain AgentHost::new() (no canonical) does NOT attach a store, so a store/* effect
    // folds an observable Err (never a panic) — the opt-in boundary. Spawn a resolver with a grant but no
    // canonical host store; its resolve settles as an error, session stays healthy.
    let mut host = AgentHost::new();
    let id = host.spawn(
        SessionId::new("no-store"),
        HostedSession::genesis(
            Hash::of(b"no-store-v1"),
            Box::new(Consumer),
            resolve_system(),
            CompositeExecutor::new(),
        ),
    );
    host.deliver(&id, inbound_go(), None)
        .await
        .expect("session exists")
        .expect("the turn ran (the store/* effect settled as an Err, no panic)");
    assert_eq!(
        host.get(&id).unwrap().open_effects(),
        0,
        "the resolve settled (as an Err — no store attached on a share-less host)"
    );
    assert!(
        host.get(&id)
            .unwrap()
            .session()
            .kv()
            .get(b"resolved")
            .is_none(),
        "nothing resolved — a share-less host attaches no store"
    );

    // No-cross-session-leak: even after a first session ran on this share-less host, a SECOND session
    // spawned on it is still store-less (the host holds no canonical store to hand down) — the opt-in
    // boundary means share-less sessions never see each other's (nonexistent) name space.
    let id2 = host.spawn(
        SessionId::new("no-store-2"),
        HostedSession::genesis(
            Hash::of(b"no-store-2-v1"),
            Box::new(Consumer),
            resolve_system(),
            CompositeExecutor::new(),
        ),
    );
    host.deliver(&id2, inbound_go(), None)
        .await
        .expect("session 2 exists")
        .expect("session 2's turn ran (store/* settled as an Err, no panic)");
    assert!(
        host.get(&id2).unwrap().session().kv().get(b"resolved").is_none(),
        "a second share-less session also resolves nothing — no store handed down, no cross-session leak"
    );
}
