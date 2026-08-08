//! End-to-end (env-gated): the §4c TRUE TWO-AGENT publish→consume loop — a PUBLISHER agent sets a
//! pointer, and a SEPARATE CONSUMER agent resolves it and runs the artifact it names. This is the payoff
//! the shared-store recipe unblocked (v-agent-harness's read-back accessor + replay primitives).
//!
//! v0.2 has PER-SESSION stores (the `attach_name_store` seam takes a `NameStore` by value; no shared
//! handle / interior mutability), so two sessions can't share a live store. The host bridges them EXPLICITLY
//! — it owns the sharing POLICY, the kernel stays share-free:
//!
//!   1. PUBLISHER (session A) `store/set`s `COMPILER_LATEST` → the wasm artifact's blob hash.
//!   2. HOST reads A's store back out (`Session::name_store()` — the read dual of `attach_name_store`) and
//!      exports its set-event stream (`NameStore::to_set_entries`).
//!   3. HOST replays that stream into a fresh store (`NameStore::replay_set_entries`) and attaches it to the
//!      CONSUMER (session B) — B now sees exactly what A published, with no shared handle.
//!   4. CONSUMER (session B) `store/resolve`s the pointer → gets A's hash → the host blob-gets the bytes and
//!      RUNS them (loads as an `AsyncComponentReducer` + folds a turn).
//!
//! A true SHARED/federated store (live cross-session, not this explicit export/replay bridge) is the later
//! durable-store slice; this proves the cross-agent loop works over v0.2's per-session seam today.
//!
//! GATED on `CDZ_LIVE_REDUCER_COMPONENT` (unset → SKIP cleanly), like the sibling live/publish-consume
//! e2es — the hermetic default gate never builds a wasm artifact or runs the loop.

mod common;

use cdz_agent_host::HostedSession;
use cdz_kernel::authz::Authorizer;
use cdz_kernel::blob::{BlobStore, MemBlobStore};
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
use cdz_kernel::wasm_host::AsyncComponentReducer;
use common::reducer_component_bytes;
use std::time::Duration;

const POINTER: &str = NameStore::COMPILER_LATEST;

/// A hard ceiling on running the resolved artifact's fold turn (unbounded `deliver` on a real external
/// reducer is a hang risk — same discipline as the live-net e2es #1853/#1857/#1887).
const FOLD_RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// PUBLISHER: on inbound, `store/set`s `COMPILER_LATEST` → the artifact hash it was built with. One hop.
struct Publisher {
    artifact_hash: Hash,
}
#[async_trait::async_trait(?Send)]
impl Reducer for Publisher {
    async fn fold(&mut self, event: &Event, _kv: &mut Kv) -> FoldOutput {
        if matches!(event.body, EventBody::Inbound { .. }) {
            let payload = encode_name_set(POINTER, &self.artifact_hash);
            FoldOutput::with(vec![EffectRequest::new_with_family(
                effect_ct::STORE_SET,
                POINTER,
                Some(Payload::Inline(payload.into())),
                Timeliness::Interactive,
            )])
        } else {
            FoldOutput::none()
        }
    }
}

/// CONSUMER: on inbound, `store/resolve`s the pointer; when the result arrives, records the resolved hash's
/// hex in KV so the host can blob-get + run the artifact.
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

fn set_system() -> Authorizer {
    Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
        effect_ct::STORE_SET,
        ResourcePredicate::Prefix("system/".into()),
    )])
}
fn resolve_system() -> Authorizer {
    Authorizer::new(vec![]).with_family_grants(vec![Capability::for_family(
        effect_ct::STORE_RESOLVE,
        ResourcePredicate::Prefix("system/".into()),
    )])
}

#[tokio::test]
async fn a_consumer_agent_resolves_and_runs_what_a_separate_publisher_agent_published() {
    let Some(component) = reducer_component_bytes() else {
        eprintln!(
            "SKIP name_store_two_agent_e2e::a_consumer_agent_resolves_and_runs_what_a_separate_publisher_\
             agent_published: CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component to \
             exercise the true 2-agent publish→consume loop."
        );
        return;
    };

    // The shared artifact store (host-owned). `put` is content-addressed → the hash the publisher writes is
    // the hash the consumer resolves, and blob-get at it returns these exact bytes.
    let mut blobs = MemBlobStore::new();
    let artifact_hash = blobs.put(&component).await.expect("put the wasm artifact");

    // (1) PUBLISHER: session A sets COMPILER_LATEST → artifact_hash into its OWN per-session store.
    let mut publisher = HostedSession::genesis(
        Hash::of(b"publisher-agent-v1"),
        Box::new(Publisher { artifact_hash }),
        Box::new(set_system()),
        CompositeExecutor::new(),
    )
    .with_name_store(NameStore::new());
    publisher.deliver(inbound_go(), None).await.unwrap();
    assert_eq!(publisher.open_effects(), 0, "the publish set settled");

    // (2)+(3) HOST BRIDGE: read A's store back out, export its set-event stream, and replay it into a fresh
    // store for the consumer — the explicit host-owned sharing policy (no shared handle; kernel stays
    // share-free). This is the crux the read-back accessor + replay primitives unblocked.
    let published = publisher
        .session()
        .name_store()
        .expect("the publisher has a name store attached")
        .to_set_entries();
    assert!(
        published
            .iter()
            .any(|(n, h)| n == POINTER && *h == artifact_hash),
        "the publisher's store carries COMPILER_LATEST → artifact_hash"
    );
    let consumer_store =
        NameStore::replay_set_entries(published.iter().map(|(n, h)| (n.as_str(), *h)))
            .expect("replay the published set-stream into the consumer's store");

    // (4) CONSUMER: session B (a DIFFERENT agent, resolve-only grant) resolves the pointer A published.
    let mut consumer = HostedSession::genesis(
        Hash::of(b"consumer-agent-v1"),
        Box::new(Consumer),
        Box::new(resolve_system()),
        CompositeExecutor::new(),
    )
    .with_name_store(consumer_store);
    consumer.deliver(inbound_go(), None).await.unwrap();
    assert_eq!(consumer.open_effects(), 0, "the resolve settled");

    // The consumer resolved the SAME hash the publisher wrote — cross-agent, through the host bridge.
    let resolved_hex = consumer
        .session()
        .kv()
        .get(b"resolved")
        .expect("the consumer recorded the resolved hash");
    assert_eq!(
        resolved_hex,
        artifact_hash.to_hex().as_bytes(),
        "the consumer resolved COMPILER_LATEST to the exact hash the publisher set (cross-agent)"
    );

    // ...and the resolved artifact RUNS: blob-get the bytes at that hash, load as a reducer, fold a turn.
    let fetched = blobs
        .get(&artifact_hash)
        .await
        .expect("blob-get succeeds")
        .expect("the resolved hash is present in the shared blob store");
    let resolved_reducer = AsyncComponentReducer::from_component_bytes(&fetched)
        .expect("the artifact the consumer resolved loads as a runnable reducer");
    let mut running = HostedSession::genesis(
        artifact_hash,
        Box::new(resolved_reducer),
        Box::new(Authorizer::new(vec![])),
        CompositeExecutor::new(),
    );
    tokio::time::timeout(FOLD_RUN_TIMEOUT, running.deliver(inbound_go(), None))
        .await
        .expect(
            "the resolved artifact's fold turn completes within FOLD_RUN_TIMEOUT (not a runaway)",
        )
        .expect(
            "the artifact the consumer resolved runs one fold turn to quiescence (no panic, §17)",
        );
    assert_eq!(
        running.open_effects(),
        0,
        "the resolved artifact's turn settled — publisher published, a separate consumer resolved + RAN it"
    );
}
