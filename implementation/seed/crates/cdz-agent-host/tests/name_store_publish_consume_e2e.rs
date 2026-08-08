//! End-to-end (env-gated): the §4c v0.2 PUBLISH → CONSUME arc through a hosted agent — compile-artifact
//! pointer edition. One hosted session, two phases, over its attached per-session [`NameStore`]:
//!
//!   PUBLISH: a real wasm reducer component (the "compiled program" — nix builds it, e.g. rcdzc→wasm) is
//!            `put` into a blob store; the agent `store/set`s the well-known pointer
//!            [`NameStore::COMPILER_LATEST`] → that blob's content hash.
//!   CONSUME: the agent `store/resolve`s the pointer, we `blob-get` the bytes at the resolved hash, load
//!            them as an [`AsyncComponentReducer`], and RUN one fold — the resolved program actually
//!            executes. This is the piece the in-session store round-trip (name_store_e2e) can't cover:
//!            the pointer resolves to a REAL artifact that runs.
//!
//! v0.2 is SINGLE-SESSION (set + resolve on ONE session's store): a true cross-agent publish/consume needs
//! a SHARED store, which is the later durable-store slice (per-session by-value seam here). This proves the
//! full arc without cross-session sharing.
//!
//! GATED on `CDZ_LIVE_REDUCER_COMPONENT` (a path to a real wasm reducer component): unset → SKIP cleanly (a
//! plain `cargo test` has no such artifact), so the hermetic default gate is unaffected. Set it (the nix
//! build produces one) to exercise the real resolve→load→run path.

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

/// The pointer both phases use — one source of truth (the kernel-side well-known name).
const POINTER: &str = NameStore::COMPILER_LATEST;

/// A hard ceiling on running the resolved artifact's fold turn. `HostedSession::deliver` drives the
/// reducer→effect loop to quiescence with no step bound, so a misbehaving/looping live reducer (this test
/// runs a REAL, externally-supplied wasm component) could hang the suite. Bounding it surfaces a runaway as
/// a clear error instead of a wedge — same discipline as the live-net e2es (#1853/#1857).
const FOLD_RUN_TIMEOUT: Duration = Duration::from_secs(30);

/// A publisher/consumer reducer parameterized by the blob hash to publish. On inbound it `store/set`s the
/// well-known pointer → `artifact_hash`; when that settles it `store/resolve`s the pointer; when THAT
/// settles it records the resolved hash's hex in KV so the test can blob-get + run the artifact.
struct PublishThenResolve {
    artifact_hash: Hash,
}
#[async_trait::async_trait(?Send)]
impl Reducer for PublishThenResolve {
    async fn fold(&mut self, event: &Event, kv: &mut Kv) -> FoldOutput {
        match &event.body {
            EventBody::Inbound { .. } => {
                let payload = encode_name_set(POINTER, &self.artifact_hash);
                FoldOutput::with(vec![EffectRequest::new_with_family(
                    effect_ct::STORE_SET,
                    POINTER,
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
                        POINTER,
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

fn inbound_go() -> EventBody {
    EventBody::Inbound {
        content_type: ContentType {
            family: "message".into(),
            version: 1,
        },
        payload: Payload::Inline(b"go".to_vec().into()),
    }
}

/// A publisher that may set + resolve the well-known (`system/…`) pointer.
fn publisher_authz() -> Authorizer {
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

#[tokio::test]
async fn a_published_compiler_pointer_resolves_to_a_runnable_artifact() {
    let Some(component) = reducer_component_bytes() else {
        eprintln!(
            "SKIP name_store_publish_consume_e2e::a_published_compiler_pointer_resolves_to_a_runnable_\
             artifact: CDZ_LIVE_REDUCER_COMPONENT unset — set it to a real wasm reducer component (the nix \
             build produces one) to exercise the publish→resolve→load→run arc."
        );
        return;
    };

    // The blob store holding compiled artifacts. `put` is content-addressed → the hash we publish is the
    // hash the resolve will hand back, and blob-get at it returns these exact bytes.
    let mut blobs = MemBlobStore::new();
    let artifact_hash = blobs
        .put(&component)
        .await
        .expect("put the compiled wasm component into the blob store");

    // PUBLISH + RESOLVE (one session, two phases): the agent sets COMPILER_LATEST → artifact_hash, then
    // resolves it back. Its own per-session NameStore holds the pointer; a system/-prefix grant authorizes.
    let mut session = HostedSession::genesis(
        Hash::of(b"compiler-publisher-v1"),
        Box::new(PublishThenResolve { artifact_hash }),
        Box::new(publisher_authz()),
        CompositeExecutor::new(),
    )
    .with_name_store(NameStore::new());

    session.deliver(inbound_go(), None).await.unwrap();
    assert_eq!(session.open_effects(), 0, "both store effects settled");

    // The pointer resolved to the hash we published.
    let resolved_hex = session
        .session()
        .kv()
        .get(b"resolved")
        .expect("the agent recorded the resolved hash");
    assert_eq!(
        resolved_hex,
        artifact_hash.to_hex().as_bytes(),
        "COMPILER_LATEST resolved to the published artifact's hash"
    );

    // CONSUME: blob-get the bytes at the resolved hash, and prove they're a RUNNABLE program — load them as
    // an AsyncComponentReducer. (Loading validates the component + binds fold.apply; a broken/mismatched
    // blob would fail here.) This closes the arc: a resolved pointer → a real artifact that the host can run.
    let fetched = blobs
        .get(&artifact_hash)
        .await
        .expect("blob-get succeeds")
        .expect("the resolved hash is present in the blob store");
    assert_eq!(
        fetched, component,
        "blob-get returned the exact published bytes"
    );

    let resolved_reducer = AsyncComponentReducer::from_component_bytes(&fetched)
        .expect("the resolved artifact loads as a runnable reducer component (fold.apply bound)");

    // RUN the resolved artifact: host a fresh session driven by the reducer we just fetched-by-pointer, and
    // deliver one inbound event — the resolved program actually FOLDS (executes a real turn through the
    // kernel loop), not merely loads. We grant nothing and attach no store, so whatever effects it emits are
    // denied/decline cleanly — the point is that the turn RUNS to quiescence without panicking (§17
    // totality), closing the pointer→fetch→RUN loop end to end. The reducer's genesis id IS the published
    // pointer's resolved hash (`artifact_hash`) — content-addressed by the same bytes the pointer resolves
    // to, so the running reducer's identity == what COMPILER_LATEST points at (no redundant re-hash).
    let mut running = HostedSession::genesis(
        artifact_hash,
        Box::new(resolved_reducer),
        Box::new(Authorizer::new(vec![])),
        CompositeExecutor::new(),
    );
    // Bounded (see FOLD_RUN_TIMEOUT): a runaway live reducer surfaces as a timeout error, not a hung suite.
    tokio::time::timeout(FOLD_RUN_TIMEOUT, running.deliver(inbound_go(), None))
        .await
        .expect("the resolved artifact's fold turn completes within FOLD_RUN_TIMEOUT (not a runaway loop)")
        .expect("the resolved artifact runs one fold turn to quiescence (no panic, §17)");
    assert_eq!(
        running.open_effects(),
        0,
        "the resolved artifact's turn settled (any emitted effects resolved/denied — it ran)"
    );
}
