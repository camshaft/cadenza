//! End-to-end: a HOSTED agent's §4c `store/*` effects round-trip through a per-session [`NameStore`].
//!
//! This is the host half of the §4c mutable-name store (v0.2): [`HostedSession::with_name_store`] attaches
//! a per-session `NameStore`, and a reducer's `store/set` / `store/resolve` effects — authorized by the
//! session's real [`Authorizer`] via `Capability::for_family` prefix grants — flow through the kernel's
//! store arm (NOT an executor) and fold back. It proves, over the host wiring a deployed agent uses:
//! - a `set` then `resolve` of the same name returns the hash that was set (the store round-trips);
//! - write authority is a SEPARATE grant from read: an agent granted only `store/resolve` is DENIED a
//!   `store/set` (allow-read-deny-write — the §4c prefix-authority model, fail-closed by default);
//! - a session with NO store attached folds an observable `Err` for a `store/*` effect, never a panic.

use cdz_agent_host::HostedSession;
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

const NAME: &str = "system/compiler/latest";

/// A reducer that publishes then reads back a well-known pointer: on inbound it `store/set`s
/// `system/compiler/latest → <hash>`; when that settles it `store/resolve`s the same name; when the
/// resolve settles it records the resolved hash's hex in KV so a test can assert the round-trip.
struct SetThenResolve;
#[async_trait::async_trait(?Send)]
impl Reducer for SetThenResolve {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                let payload = encode_name_set(NAME, &Hash::of(b"compiler-wasm-v1"));
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    NAME,
                    Some(Payload::Inline(payload.into())),
                    Timeliness::Interactive,
                )])
            }
            EventBody::EffectResult {
                result: EffectOutcome::Ok(body),
                ..
            } => match kv.get(b"phase") {
                None => {
                    kv.put(b"phase".to_vec(), b"resolving".to_vec());
                    FoldOutput::with(vec![EffectRequest::new_with_family(
                        effect_ct::STORE_RESOLVE,
                        NAME,
                        None,
                        Timeliness::Interactive,
                    )])
                }
                Some(_) => {
                    if let Some(Payload::Inline(bytes)) = body {
                        if let Ok((_n, h)) = decode_name_set(bytes) {
                            kv.put(b"resolved".to_vec(), h.to_hex().into_bytes());
                        }
                    }
                    FoldOutput::none()
                }
            },
            _ => FoldOutput::none(),
        }
    }
}

/// A reducer that only tries to WRITE — on inbound it emits a single `store/set` (no resolve). Lets a test
/// assert the write is denied without a resolve muddying the picture.
struct SetOnly;
#[async_trait::async_trait(?Send)]
impl Reducer for SetOnly {
    async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
        if matches!(event.body, EventBody::Inbound { .. }) {
            let payload = encode_name_set(NAME, &Hash::of(b"compiler-wasm-v1"));
            FoldOutput::with(vec![EffectRequest::new_with_family(
                effect_ct::STORE_SET,
                NAME,
                Some(Payload::Inline(payload.into())),
                Timeliness::Interactive,
            )])
        } else {
            FoldOutput::none()
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

/// Grant both store actions on the `system/` prefix (the §4c write+read authority a publisher agent gets).
fn set_and_resolve_system() -> Authorizer {
    Authorizer::new(vec![]).with_family_grants(vec![
        Capability::for_family(
            effect_ct::STORE_SET,
            ResourcePredicate::Prefix("system/".into()),
        ),
        Capability::for_family(
            effect_ct::STORE_RESOLVE,
            ResourcePredicate::Prefix("system/".into()),
        ),
    ])
}

/// Grant ONLY resolve on `system/` — a read-only consumer. A `store/set` is denied (allow-read-deny-write).
fn resolve_only_system() -> Authorizer {
    Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
        effect_ct::STORE_RESOLVE,
        ResourcePredicate::Prefix("system/".into()),
    )])
}

#[tokio::test]
async fn a_hosted_agents_store_set_then_resolve_round_trips_through_its_attached_name_store() {
    let mut session = HostedSession::genesis(
        Hash::of(b"publisher-v1"),
        Box::new(SetThenResolve),
        Box::new(set_and_resolve_system()),
        CompositeExecutor::new(),
    )
    .with_name_store(NameStore::new());

    session.deliver(inbound_go(), None).await.unwrap();

    // The set applied and the resolve read the latest — through the host's attached store. The resolved
    // hash the reducer recorded equals the hash it set.
    assert_eq!(
        session.session().kv().get(b"resolved"),
        Some(Hash::of(b"compiler-wasm-v1").to_hex().as_bytes()),
        "store/set → store/resolve round-tripped through the attached NameStore"
    );
    assert_eq!(
        session.open_effects(),
        0,
        "both store effects settled (store/* is not executor-routed)"
    );
}

#[tokio::test]
async fn a_resolve_only_grant_denies_a_store_set_allow_read_deny_write() {
    let mut session = HostedSession::genesis(
        Hash::of(b"consumer-v1"),
        Box::new(SetOnly),
        Box::new(resolve_only_system()),
        CompositeExecutor::new(),
    )
    .with_name_store(NameStore::new());

    session.deliver(inbound_go(), None).await.unwrap();

    // Write authority is a SEPARATE grant: resolve-only → the store/set is denied at the gate. A denial is
    // on the log and nothing is left open (the §4c allow-read-deny-write property).
    assert!(
        session
            .session()
            .log()
            .iter()
            .any(|e| matches!(e.body, EventBody::AuthzDenied { .. })),
        "a store/set under a resolve-only grant is denied"
    );
    assert_eq!(session.open_effects(), 0);
}

#[tokio::test]
async fn a_store_effect_with_no_name_store_attached_folds_an_error_not_a_panic() {
    // Plain genesis (no with_name_store) → the session has no store bound. A store/* effect must fold an
    // observable Err (§9d/§17), never panic. The grant permits it, so this exercises the missing-store
    // path specifically, not an authz denial.
    let mut session = HostedSession::genesis(
        Hash::of(b"no-store-v1"),
        Box::new(SetOnly),
        Box::new(set_and_resolve_system()),
        CompositeExecutor::new(),
    );

    // Delivering completes without panicking; the store effect settles as an error outcome.
    session.deliver(inbound_go(), None).await.unwrap();
    assert_eq!(
        session.open_effects(),
        0,
        "the store effect settled (as an Err) — no hang, no panic"
    );
    assert!(
        session.session().log().iter().any(|e| matches!(
            &e.body,
            EventBody::EffectResult {
                result: EffectOutcome::Err { .. },
                ..
            }
        )),
        "a store/* effect with no attached store folds an observable Err"
    );
}
